//! The background service: `tiny4lite-control service run`.
//!
//! It needs no display and no window. It watches for the camera being plugged in, puts the saved
//! capture mode back, optionally restores the settings (and a named preset), and, only if you turn it
//! on, puts the camera to sleep when it is idle, at shutdown and at suspend. It never wakes it.
//!
//! Safe by default: with the default configuration it only restores the capture mode. It never moves
//! the gimbal unless you allow it, and never sleeps the camera unless you turn that on.
//!
//! `install` writes or prints what each init system or window manager needs to start it.

use crate::backend::{self, current_config, update_config_checked};
use crate::config::CameraPreset;
use crate::presets;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The service sleeps in small slices so it can answer a stop quickly, and looks at the camera every
/// tenth slice (every two seconds).
const SLICE: Duration = Duration::from_millis(200);
const ROUND_SLICES: u32 = 10;
/// A new camera gets this long to settle before anything is sent to it.
const SETTLE: Duration = Duration::from_secs(2);
/// After the computer wakes, the camera gets this long to come back before it is put to sleep again,
/// and the service stops waiting for it after the second.
const RESUME_SETTLE: Duration = Duration::from_secs(4);
const RESUME_GIVE_UP: Duration = Duration::from_secs(90);
/// A control used in the window this recently counts as using the camera.
const ACTIVITY_SECONDS: u64 = 10;

// ── Decisions ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Connected(String),
    Restore,
    Disconnected,
    /// Send the sleep command. It does not matter whether the camera already sleeps: sending it again is
    /// harmless, so nothing is read first.
    Sleep,
}

/// What the service decides to do, from what it sees. It holds no hardware, so it is tested with
/// made-up time. It never sends wake: the camera wakes by itself when a program opens it (to be
/// confirmed on the camera, see docs/troubleshooting.md), and a second wake would only be a needless
/// extra command.
#[derive(Debug)]
pub struct Brain {
    pub auto_sleep: bool,
    pub sleep_after: Duration,
    device: Option<String>,
    connected_at: Option<Instant>,
    restored: bool,
    idle_since: Option<Instant>,
    /// Sleep was sent during this idle spell; it is not sent again until the camera has been used.
    slept: bool,
}

/// Everything the service looks at in one round.
pub struct Sight<'a> {
    pub camera: Option<&'a str>,
    /// A program other than this service has the camera open (a call, a recording, the preview).
    pub in_use: bool,
    /// The control window of this app is running, so someone is at the controls.
    pub window_open: bool,
}

impl Brain {
    pub fn new(auto_sleep: bool, sleep_after_minutes: u32) -> Brain {
        Brain {
            auto_sleep, sleep_after: Duration::from_secs(u64::from(sleep_after_minutes.max(1)) * 60),
            device: None, connected_at: None, restored: false, idle_since: None, slept: false,
        }
    }

    /// Change the policy while running (the settings were edited). The state it has built up stays.
    pub fn set_policy(&mut self, auto_sleep: bool, sleep_after_minutes: u32) {
        self.auto_sleep = auto_sleep;
        self.sleep_after = Duration::from_secs(u64::from(sleep_after_minutes.max(1)) * 60);
        if !auto_sleep { self.idle_since = None; }
    }

    pub fn step(&mut self, now: Instant, sight: &Sight) -> Vec<Action> {
        let mut actions = Vec::new();
        match (self.device.as_deref(), sight.camera) {
            (None, Some(new)) | (Some(_), Some(new)) if self.device.as_deref() != Some(new) => {
                self.device = Some(new.to_string());
                self.connected_at = Some(now);
                self.restored = false;
                self.idle_since = None;
                self.slept = false;
                actions.push(Action::Connected(new.to_string()));
            }
            (Some(_), None) => {
                self.device = None;
                self.connected_at = None;
                self.idle_since = None;
                self.slept = false;
                actions.push(Action::Disconnected);
                return actions;
            }
            _ => {}
        }
        if self.device.is_none() { return actions; }
        if !self.restored && self.connected_at.is_some_and(|at| now.duration_since(at) >= SETTLE) {
            self.restored = true;
            actions.push(Action::Restore);
        }
        if self.auto_sleep && self.restored {
            if sight.in_use {
                // Used: the idle spell is over, and the next one may send sleep again.
                self.idle_since = None;
                self.slept = false;
            } else if sight.window_open {
                // Someone is at the controls: never sleep it from under them.
                self.idle_since = None;
            } else {
                let since = *self.idle_since.get_or_insert(now);
                if now.duration_since(since) >= self.sleep_after && !self.slept {
                    self.slept = true;
                    actions.push(Action::Sleep);
                }
            }
        }
        actions
    }
}

// ── Files the service keeps ───────────────────────────────────────────────

fn user_id() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0)
}

/// Where the lock and the status file live: the runtime directory, which is private and cleared at logout.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("tiny4lite-control"),
        None => std::env::temp_dir().join(format!("tiny4lite-control-{}", user_id())),
    }
}

fn lock_path() -> PathBuf { runtime_dir().join("service.lock") }
pub(crate) fn state_path() -> PathBuf { runtime_dir().join("service.json") }
/// What a service started without systemd writes, as systemd would keep it in the journal.
pub(crate) fn log_path() -> PathBuf { runtime_dir().join("service.log") }

fn now_unix() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

/// Is `pid` this program? Judged by the file it runs, not its name on the command line: an AppImage
/// starts it as `/tmp/.mount_…/AppRun`, which says nothing about what it is.
pub(crate) fn is_our_program(pid: u32, words: &[String]) -> bool {
    let ours = |name: &str| name.starts_with("tiny4lite-control");
    if let Ok(target) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        if target.file_name().and_then(|n| n.to_str()).is_some_and(ours) { return true; }
        // The same file as this program (a renamed copy, or under test).
        if let Ok(me) = std::env::current_exe() { if me == target { return true; } }
    }
    words.first().is_some_and(|w| w.contains("tiny4lite-control")
        || w.rsplit('/').next().is_some_and(ours))
}

/// The words a process was started with.
fn command_words(pid: u32) -> Option<Vec<String>> {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(cmdline.split(|b| *b == 0).filter(|w| !w.is_empty()).map(|w| String::from_utf8_lossy(w).to_string()).collect())
}

/// Is `pid` a running tiny4lite-control service?
pub(crate) fn is_service_process(pid: u32) -> bool {
    let Some(words) = command_words(pid) else { return false };
    is_our_program(pid, &words) && words.iter().any(|w| w == "service") && words.iter().any(|w| w == "run")
}

/// Only one service runs at a time. A lock left behind by a process that is gone is taken over.
pub struct Lock { path: PathBuf }

impl Lock {
    pub fn acquire(path: &Path) -> Result<Lock, String> {
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        if let Some(pid) = std::fs::read_to_string(path).ok().and_then(|t| t.trim().parse::<u32>().ok()) {
            if pid != std::process::id() && is_service_process(pid) {
                return Err(format!("The service is already running (process {pid})."));
            }
        }
        std::fs::write(path, std::process::id().to_string()).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        Ok(Lock { path: path.to_path_buf() })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).ok().and_then(|t| t.trim().parse::<u32>().ok()) == Some(std::process::id()) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub pid: u32,
    pub started: u64,
    pub camera: Option<String>,
    pub last_event: String,
    pub last_event_at: u64,
    pub restored: bool,
    pub asleep_by_service: bool,
    /// The Studio camera: what runs and what it costs.
    #[serde(default)]
    pub denoise: Option<crate::denoise::Report>,
}

/// The running service's status, if one is running: for the window to show.
pub fn running_status() -> Option<Status> {
    let text = std::fs::read_to_string(state_path()).ok()?;
    let status: Status = serde_json::from_str(&text).ok()?;
    if is_service_process(status.pid) { Some(status) } else { None }
}

fn write_status(status: &Status) {
    let path = state_path();
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let temporary = path.with_extension("json.tmp");
    if let Ok(text) = serde_json::to_string_pretty(status) {
        if std::fs::write(&temporary, text).is_ok() { let _ = std::fs::rename(&temporary, &path); }
    }
}

// ── Looking around ────────────────────────────────────────────────────────


// ── Doing it ──────────────────────────────────────────────────────────────

/// What a restore sends, as a preset: the named one, or the last values this app set, without any
/// gimbal position unless that is allowed.
pub fn restore_preset(config: &crate::config::AppConfig) -> Option<CameraPreset> {
    if !config.apply_on_connect { return None; }
    let mut preset = match config.service.connect_preset.as_deref().filter(|n| !n.is_empty()) {
        Some(name) => config.presets.iter().find(|p| p.name.eq_ignore_ascii_case(name)).cloned()?,
        None => {
            let settings: BTreeMap<String, i64> = config.v4l2_controls.iter().map(|(k, v)| (format!("v4l2:{k}"), *v)).collect();
            CameraPreset { name: "last settings".into(), settings }
        }
    };
    if !config.service.allow_gimbal_move {
        preset.settings.retain(|key, _| !matches!(key.as_str(), "v4l2:pan_absolute" | "v4l2:tilt_absolute"));
    }
    if preset.settings.is_empty() { None } else { Some(preset) }
}

fn log(message: &str) { eprintln!("[tiny4lite-control service] {message}"); }

/// Write a line to the service's log (for the parts of the service that live in other files).
pub(crate) fn note(message: &str) { log(message); }

fn perform(action: &Action, device: &str, status: &mut Status) {
    let note = match action {
        Action::Connected(node) => format!("Camera found at {node}."),
        Action::Disconnected => "Camera disconnected.".to_string(),
        Action::Restore => {
            let mut parts = Vec::new();
            if let Some(message) = backend::apply_saved_video_mode(device) { parts.push(message); }
            let config = current_config();
            match restore_preset(&config) {
                Some(preset) => match presets::apply(device, &preset) {
                    Ok(done) => parts.push(done),
                    Err(error) => parts.push(format!("Restore: {error}")),
                },
                None if config.apply_on_connect => parts.push("Restore is on, but there is nothing to restore.".into()),
                None => {}
            }
            status.restored = true;
            if parts.is_empty() { "Camera ready.".to_string() } else { parts.join(" ") }
        }
        Action::Sleep => match send_sleep(device) {
            Ok(()) => { status.asleep_by_service = true; "The camera was idle, so it was put to sleep.".to_string() }
            Err(error) => format!("Could not put the camera to sleep: {error}"),
        },
    };
    log(&note);
    status.last_event = note;
    status.last_event_at = now_unix();
}

/// Send the sleep command, trying a few times if it is refused. Whether the camera is already asleep is not
/// looked at: sending it to a sleeping camera does no harm.
fn send_sleep(device: &str) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match backend::set_sleep_mode(device, true) {
            Ok(()) => return Ok(()),
            Err(error) => last = error,
        }
        if attempt < 2 { std::thread::sleep(Duration::from_millis(200)); }
    }
    Err(last)
}

/// When the settings file was last written, to notice an edit made in the window.
fn config_stamp() -> Option<SystemTime> {
    std::fs::metadata(crate::config::config_path()).and_then(|m| m.modified()).ok()
}

/// Put the camera to sleep on the way out (shutdown, logout), and never take long: there is little time
/// before the computer goes.
fn sleep_for_shutdown(device: &str, status: &mut Status) {
    match send_sleep(device) {
        Ok(()) => {
            log("Shutting down, so the camera was put to sleep.");
            status.last_event = "The camera was put to sleep as the computer shut down.".into();
            status.last_event_at = now_unix();
            write_status(status);
        }
        Err(error) => log(&format!("Shutting down, but the camera could not be put to sleep: {error}")),
    }
}

/// Apply a capture mode chosen while another program had the camera, now that nothing has it.
fn apply_pending_mode(device: &str, status: &mut Status) {
    let Some(pending) = current_config().pending_video_mode else { return };
    let mode = crate::video::VideoMode {
        width: pending.width, height: pending.height, pixel_format: pending.pixel_format.clone(), frame_rate_milli: pending.frame_rate_milli,
    };
    let note = match crate::video::set_mode(device, &mode) {
        Ok(()) => {
            let saved = pending.clone();
            let _ = backend::update_config_checked(|c| { c.video_mode = Some(saved); c.pending_video_mode = None; });
            format!("The camera was free, so the capture mode was changed to {} × {} {} {:.0} fps.", mode.width, mode.height, mode.pixel_format, f64::from(mode.frame_rate_milli) / 1000.0)
        }
        Err(error) => {
            // Not tried again and again: the choice is dropped, and the window can ask once more.
            let _ = backend::update_config_checked(|c| c.pending_video_mode = None);
            format!("The capture mode could not be changed: {error}")
        }
    };
    log(&note);
    status.last_event = note;
    status.last_event_at = now_unix();
}

/// Run until stopped. With `once`, stop after the camera has been found and restored.
///
/// SIGINT, SIGTERM and SIGHUP all mean the same: stop. That is what a shutdown, a reboot, a logout and
/// `systemctl stop` do to a program in every init system, so one handler covers them all.
pub fn run_service(once: bool, device_override: Option<String>) -> i32 {
    let _lock = match Lock::acquire(&lock_path()) {
        Ok(lock) => lock,
        Err(error) => { eprintln!("{error}"); return crate::cli::ALREADY_RUNNING; }
    };
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || stop.store(true, std::sync::atomic::Ordering::Relaxed));
    }
    let policy = |config: &crate::config::AppConfig| (config.service.auto_sleep_enabled, config.service.auto_sleep_timeout_minutes, config.service.sleep_on_shutdown, config.service.sleep_on_suspend);
    let config = current_config();
    let (auto, minutes, on_shutdown, on_suspend) = policy(&config);
    let mut brain = Brain::new(auto, minutes);
    let mut stamp = config_stamp();
    let mut status = Status { pid: std::process::id(), started: now_unix(), last_event: "Started.".into(), last_event_at: now_unix(), ..Status::default() };
    let words = |on: bool| if on { "on" } else { "off" };
    log(&format!("Started. Restore on connect: {}. Idle sleep: {}. Sleep at shutdown: {}. Sleep at suspend: {}.",
        words(config.apply_on_connect), if auto { format!("after {minutes} min") } else { "off".into() }, words(on_shutdown), words(on_suspend)));
    write_status(&status);
    // The camera's node, shared with the suspend watcher, which runs on its own thread.
    let shared_device: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let mut watcher: Option<crate::suspend::Watcher> = None;
    // The Studio camera (virtual, with filters), run while it is turned on.
    let mut denoiser = crate::denoise::Denoiser::new();
    let mut watching_suspend = false;
    let mut suspend_retry_at: Option<Instant> = None;
    // When the computer last woke from suspend, until the camera has been put back to sleep.
    let resumed_at: std::sync::Arc<std::sync::Mutex<Option<Instant>>> = Default::default();
    let mut restored_once = false;
    let mut last_device: Option<String> = None;
    let mut slice = 0u32;
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        // Looking around takes a round of slices: the wait is cut into small ones so a stop is answered
        // within a fifth of a second.
        if slice % ROUND_SLICES == 0 {
            // Settings edited in the window take effect at once, without restarting the service.
            let now_stamp = config_stamp();
            if now_stamp != stamp {
                // A file that cannot be read now (half written by hand) is tried again next round.
                if let Ok(loaded) = crate::config::load() {
                    stamp = now_stamp;
                    backend::install_config(loaded);
                    let (auto, minutes, on_shutdown, on_suspend) = policy(&current_config());
                    brain.set_policy(auto, minutes);
                    log(&format!("Settings reloaded. Idle sleep: {}. Sleep at shutdown: {}. Sleep at suspend: {}.",
                        if auto { format!("after {minutes} min") } else { "off".into() }, words(on_shutdown), words(on_suspend)));
                }
            }
            // Suspend is watched while it is wanted, and the watcher is let go when it is not.
            let want_suspend = current_config().service.sleep_on_suspend;
            // The watcher ends if its dbus-monitor does (the system bus restarted): start it again, but
            // not more than once a minute.
            if watching_suspend && watcher.as_ref().is_some_and(|w| !w.alive())
                && suspend_retry_at.is_none_or(|at| Instant::now() >= at) {
                log("The suspend watcher stopped; starting it again.");
                watcher = None;
                watching_suspend = false;
                suspend_retry_at = Some(Instant::now() + Duration::from_secs(60));
            }
            if want_suspend != watching_suspend {
                watching_suspend = want_suspend;
                if want_suspend {
                    let device = shared_device.clone();
                    let resumed = resumed_at.clone();
                    match crate::suspend::Watcher::start(move || {
                        let node = device.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        // One try only: there is no time for more before the computer sleeps.
                        match node.as_deref().map(|d| backend::set_sleep_mode(d, true)) {
                            Some(Ok(())) => log("The computer is suspending, so the camera was put to sleep."),
                            Some(Err(error)) => log(&format!("The computer is suspending, but the camera could not be put to sleep: {error}")),
                            None => {}
                        }
                    }, move || {
                        // The camera powers up again with the computer: put it back to sleep once it is back.
                        log("The computer woke up; the camera will be put back to sleep.");
                        *resumed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                    }) {
                        Ok(started) => { watcher = Some(started); log("Watching for suspend."); }
                        Err(error) => { log(&format!("Suspend is not watched: {error}")); status.last_event = format!("Suspend is not watched: {error}"); status.last_event_at = now_unix(); }
                    }
                } else {
                    watcher = None;
                    log("No longer watching for suspend.");
                }
            }
            let camera = device_override.clone().filter(|d| Path::new(d).exists() || backend::ui_test_mode())
                .or_else(|| backend::discover_tiny4lite().into_iter().next());
            *shared_device.lock().unwrap_or_else(|e| e.into_inner()) = camera.clone();
            // In use: a program streaming from the camera (this app's preview counts), or a control used in the
            // window just now. An open window on its own does not keep the camera awake.
            // Both ways in count: the camera itself, and the Studio camera (an app watching it keeps the
            // camera awake too, while the Studio camera reads it for that app).
            let in_use = brain.auto_sleep && camera.as_deref().is_some_and(|c| !backend::ui_test_mode()
                && (!backend::usage::streaming_holders(c).is_empty()
                    || denoiser.watched()
                    || backend::usage::seconds_since_activity().is_some_and(|s| s < ACTIVITY_SECONDS)));
            let sight = Sight { camera: camera.as_deref(), in_use, window_open: false };
            for action in brain.step(Instant::now(), &sight) {
                if let Some(device) = camera.as_deref() {
                    let restoring = action == Action::Restore;
                    perform(&action, device, &mut status);
                    if restoring { restored_once = true; }
                } else {
                    perform(&action, "", &mut status);
                }
                status.camera = camera.clone();
                write_status(&status);
            }
            // Back from suspend: the camera started up awake. Once it has been back a few seconds, and if no
            // program is using it, put it to sleep again. Give up after a minute and a half.
            let since_resume = resumed_at.lock().unwrap_or_else(|e| e.into_inner()).map(|at| at.elapsed());
            if let Some(elapsed) = since_resume {
                if elapsed > RESUME_GIVE_UP {
                    *resumed_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                } else if elapsed >= RESUME_SETTLE {
                    if let Some(device) = camera.as_deref() {
                        *resumed_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        let used = !backend::ui_test_mode() && !backend::usage::streaming_holders(device).is_empty();
                        let note = if used { "Back from suspend; a program is using the camera, so it was left awake.".to_string() } else {
                            match send_sleep(device) {
                                Ok(()) => "Back from suspend, so the camera was put to sleep again.".to_string(),
                                Err(error) => format!("Back from suspend, but the camera could not be put to sleep: {error}"),
                            }
                        };
                        log(&note);
                        status.last_event = note;
                        status.last_event_at = now_unix();
                    }
                }
            }
            // A capture mode that was asked for while another program had the camera is applied now that it is free.
            if let Some(device) = camera.as_deref() {
                if current_config().pending_video_mode.is_some() && (backend::ui_test_mode() || backend::usage::streaming_holders(device).is_empty()) {
                    apply_pending_mode(device, &mut status);
                }
            }
            if camera.is_some() { last_device = camera.clone(); }
            let denoise_config = current_config().denoise;
            status.denoise = (denoise_config.enabled || status.denoise.is_some())
                .then(|| denoiser.step(&denoise_config, if backend::ui_test_mode() { None } else { camera.as_deref() }));
            status.camera = camera;
            write_status(&status);
            if once && restored_once { break; }
        }
        std::thread::sleep(SLICE);
        slice = slice.wrapping_add(1);
    }
    drop(watcher);
    denoiser.stop_all();
    // Stopped by a signal (not just finished): a shutdown or a logout. Leave the camera asleep.
    if stop.load(std::sync::atomic::Ordering::Relaxed) && current_config().service.sleep_on_shutdown {
        if let Some(device) = last_device.as_deref().filter(|d| Path::new(d).exists() || backend::ui_test_mode()) {
            sleep_for_shutdown(device, &mut status);
        }
    }
    let _ = std::fs::remove_file(state_path());
    log("Stopped.");
    crate::cli::OK
}

// ── Commands ──────────────────────────────────────────────────────────────

/// What the `service` command line asks for.
pub struct Request<'a> {
    pub words: &'a [String],
    pub yes: bool,
    pub json: bool,
    pub write: bool,
    pub once: bool,
    pub device: Option<String>,
}

pub fn run(request: &Request) -> i32 {
    let word = |i: usize| request.words.get(i).map(String::as_str);
    match word(0) {
        Some("run") => run_service(request.once, request.device.clone()),
        Some("status") => status_command(request.json),
        Some("config") => { print_config(request.json); crate::cli::OK }
        Some("set") => set_command(word(1), word(2)),
        Some("targets") => { for (name, what) in TARGETS { println!("{name:<10} {what}"); } crate::cli::OK }
        Some("install") => install_command(word(1), request.write),
        Some("setup") => crate::setup::setup_command(request.yes),
        Some("start") => report(crate::setup::start()),
        Some("stop") => report(crate::setup::stop()),
        Some("restart") => report(crate::setup::restart()),
        Some("remove") => report(crate::setup::remove()),
        Some("uninstall") => uninstall_command(word(1)),
        _ => { eprintln!("service needs one of: run, status, config, set, install, uninstall, targets, setup, start, stop, restart, remove.\nTry: tiny4lite-control help service"); crate::cli::USAGE }
    }
}

fn report(result: Result<String, String>) -> i32 {
    match result { Ok(message) => { println!("{message}"); crate::cli::OK } Err(error) => { eprintln!("{error}"); crate::cli::FAILED } }
}

fn status_command(json: bool) -> i32 {
    let text = std::fs::read_to_string(state_path()).ok();
    let status: Option<Status> = text.as_deref().and_then(|t| serde_json::from_str(t).ok());
    let running = status.as_ref().is_some_and(|s| is_service_process(s.pid));
    if json {
        println!("{}", serde_json::json!({ "running": running, "status": status }));
    } else {
        match (&status, running) {
            (Some(s), true) => {
                println!("Running (process {}), up {} s.", s.pid, now_unix().saturating_sub(s.started));
                println!("Camera:     {}", s.camera.as_deref().unwrap_or("not connected"));
                println!("Last event: {} ({} s ago)", s.last_event, now_unix().saturating_sub(s.last_event_at));
            }
            _ => println!("Not running. Start it with: tiny4lite-control service run\nOr have your session start it: tiny4lite-control service install"),
        }
    }
    if running { crate::cli::OK } else { crate::cli::FAILED }
}

fn print_config(json: bool) {
    let c = current_config();
    if json {
        println!("{}", serde_json::json!({
            "restore": c.apply_on_connect, "preset": c.service.connect_preset, "gimbal": c.service.allow_gimbal_move,
            "sleep": c.service.auto_sleep_enabled, "sleep_after_minutes": c.service.auto_sleep_timeout_minutes, "shutdown": c.service.sleep_on_shutdown, "suspend": c.service.sleep_on_suspend,
        }));
        return;
    }
    let on = |b: bool| if b { "on" } else { "off" };
    println!("restore          {}   Put the camera settings back when it is plugged in", on(c.apply_on_connect));
    println!("preset           {}   Restore this preset; with none, the last settings this app set", c.service.connect_preset.as_deref().unwrap_or("none"));
    println!("gimbal           {}   Let a restore turn the gimbal to a saved position", on(c.service.allow_gimbal_move));
    println!("sleep            {}   Put the camera to sleep when nothing has used it for a while", on(c.service.auto_sleep_enabled));
    println!("sleep-after      {} min", c.service.auto_sleep_timeout_minutes);
    println!("shutdown         {}   Put the camera to sleep when the computer shuts down or the session ends", on(c.service.sleep_on_shutdown));
    println!("suspend          {}   Put the camera to sleep when the computer suspends", on(c.service.sleep_on_suspend));
}

fn set_command(key: Option<&str>, value: Option<&str>) -> i32 {
    let (Some(key), Some(value)) = (key, value) else {
        eprintln!("service set KEY VALUE, where KEY is one of: restore, preset, gimbal, sleep, sleep-after, shutdown, suspend");
        return crate::cli::USAGE;
    };
    let flag = |v: &str| match v.to_lowercase().as_str() { "on" | "yes" | "true" | "1" => Some(true), "off" | "no" | "false" | "0" => Some(false), _ => None };
    let result = match key {
        "restore" | "gimbal" | "sleep" | "shutdown" | "suspend" => match flag(value) {
            Some(b) => update_config_checked(|c| match key {
                "restore" => c.apply_on_connect = b,
                "gimbal" => c.service.allow_gimbal_move = b,
                "shutdown" => c.service.sleep_on_shutdown = b,
                "suspend" => c.service.sleep_on_suspend = b,
                _ => c.service.auto_sleep_enabled = b,
            }),
            None => { eprintln!("{key} takes on or off."); return crate::cli::USAGE; }
        },
        "preset" => {
            let name = if value.eq_ignore_ascii_case("none") { None } else { Some(value.to_string()) };
            if let Some(name) = &name {
                if !current_config().presets.iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
                    eprintln!("There is no preset called `{name}`. Presets: {}", current_config().presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", "));
                    return crate::cli::FAILED;
                }
            }
            update_config_checked(|c| c.service.connect_preset = name.clone())
        }
        "sleep-after" => match value.parse::<u32>() {
            Ok(m) if (1..=1440).contains(&m) => update_config_checked(|c| c.service.auto_sleep_timeout_minutes = m),
            _ => { eprintln!("sleep-after takes a number of minutes from 1 to 1440."); return crate::cli::USAGE; }
        },
        other => { eprintln!("Unknown service setting `{other}`. Use restore, preset, gimbal, sleep, sleep-after, shutdown or suspend."); return crate::cli::USAGE; }
    };
    match result {
        Ok(()) => { println!("Saved. A running service picks it up within a couple of seconds."); crate::cli::OK }
        Err(error) => { eprintln!("Could not save: {error}"); crate::cli::FAILED }
    }
}

// ── Starting it with your session ─────────────────────────────────────────

pub const TARGETS: &[(&str, &str)] = &[
    ("systemd", "A systemd user service (most distributions: Fedora, Arch, Debian, Ubuntu, openSUSE, NixOS). Writes with --write."),
    ("xdg", "A desktop autostart entry (GNOME, KDE, Xfce, Cinnamon, MATE, LXQt and anything that follows the XDG spec). Writes with --write."),
    ("hyprland", "Hyprland 0.55 and later, which is configured in Lua (hyprland.lua)"),
    ("hyprland-legacy", "Hyprland before 0.55, or a hyprland.conf you still use"),
    ("niri", "niri's config.kdl (a one-line shell command; older niri gets the long form in a comment)"),
    ("sway", "A line for sway's config"),
    ("i3", "A line for i3's config"),
    ("river", "A line for river's init script"),
    ("mango", "A line for MangoWC (mango) config.conf"),
    ("labwc", "A line for labwc's autostart file"),
    ("openbox", "A line for openbox's autostart file"),
    ("bspwm", "A line for bspwmrc"),
    ("shell", "Any autostart script, .xinitrc or .xprofile (dwm, awesome, qtile, herbstluftwm and so on)"),
    ("openrc", "An OpenRC service script, for Alpine, Gentoo and Artix without systemd"),
    ("runit", "A runit user service, for Void"),
    ("s6", "An s6 service run script"),
    ("dinit", "A dinit user service, for Chimera and Artix"),
    ("cron", "A cron @reboot entry, where nothing else is available"),
    ("udev", "A udev rule that starts the systemd user service when the camera is plugged in"),
];

/// The program to start. An AppImage's own file, not its temporary mount.
pub fn program_path() -> String {
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) { return appimage.to_string_lossy().to_string(); }
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "tiny4lite-control".into())
}

/// Quote for a shell or a config line: plain words stay plain.
fn quoted(path: &str) -> String {
    if path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@%:,=".contains(c)) { path.to_string() }
    else { format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\"")) }
}

/// Text for the inside of a Lua string in double quotes.
fn lua_escaped(text: &str) -> String { text.replace('\\', "\\\\").replace('"', "\\\"") }

/// Text for the inside of a KDL string in double quotes.
fn kdl_escaped(text: &str) -> String { text.replace('\\', "\\\\").replace('"', "\\\"") }

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

pub(crate) fn systemd_unit(program: &str) -> String {
    format!("[Unit]\nDescription=Tiny 2 Lite Control background service\nDocumentation=https://github.com/HypestHyper/Tiny-4-Lite-control\n\n[Service]\nType=simple\nExecStart={} service run\nRestart=on-failure\nRestartSec=3\nRestartPreventExitStatus=6\nNice=10\nTimeoutStopSec=15\n\n[Install]\nWantedBy=default.target\n", quoted(program))
}

pub(crate) fn desktop_entry(program: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName=Tiny 2 Lite Control service\nComment=Restores camera settings and watches for the camera\nExec={} service run\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n", quoted(program).replace('"', "\"").replace("%", "%%"))
}

/// The text for a target, and where it goes.
pub fn snippet(target: &str, program: &str) -> Option<(String, String)> {
    let p = quoted(program);
    let line = |s: String, where_: &str| Some((s, where_.to_string()));
    match target {
        "systemd" => line(systemd_unit(program), "~/.config/systemd/user/tiny4lite-control.service, then: systemctl --user daemon-reload && systemctl --user enable --now tiny4lite-control.service"),
        "xdg" => line(desktop_entry(program), "~/.config/autostart/tiny4lite-control-service.desktop"),
        // Hyprland 0.55 moved its configuration to Lua: a function that runs when it starts.
        "hyprland" => line(format!("hl.on(\"hyprland.start\", function()\n  hl.exec_cmd(\"{} service run\")\nend)\n", lua_escaped(&p)), "~/.config/hypr/hyprland.lua"),
        "hyprland-legacy" => line(format!("exec-once = {p} service run\n"), "~/.config/hypr/hyprland.conf"),
        // niri 25.08 added spawn-sh-at-startup, which takes one string like a shell would. Older
        // versions take each argument on its own.
        "niri" => line(format!("spawn-sh-at-startup \"{}\"\n// niri before 25.08:\n// spawn-at-startup \"{}\" \"service\" \"run\"\n",
            kdl_escaped(&format!("{p} service run")), kdl_escaped(program)), "~/.config/niri/config.kdl"),
        "sway" => line(format!("exec {p} service run\n"), "~/.config/sway/config"),
        "i3" => line(format!("exec --no-startup-id {p} service run\n"), "~/.config/i3/config"),
        "river" => line(format!("riverctl spawn \"{} service run\"\n", p.replace('"', "\\\"")), "~/.config/river/init"),
        "mango" => line(format!("exec-once={p} service run\n"), "~/.config/mango/config.conf (exec-once runs once at start; exec runs on every reload)"),
        "labwc" | "openbox" => line(format!("{p} service run &\n"), if target == "labwc" { "~/.config/labwc/autostart" } else { "~/.config/openbox/autostart" }),
        "bspwm" => line(format!("{p} service run &\n"), "~/.config/bspwm/bspwmrc"),
        "shell" => line(format!("{p} service run &\n"), "your autostart script, ~/.xinitrc or ~/.xprofile"),
        "openrc" => line(format!("#!/sbin/openrc-run\ndescription=\"Tiny 2 Lite Control background service\"\ncommand={p}\ncommand_args=\"service run\"\ncommand_user=\"YOUR_USER\"\ncommand_background=true\npidfile=\"/run/${{RC_SVCNAME}}.pid\"\nexport XDG_RUNTIME_DIR=\"/run/user/$(id -u YOUR_USER)\"\n\ndepend() {{\n\tneed localmount\n}}\n"),
            "/etc/init.d/tiny4lite-control, then: chmod +x it, replace YOUR_USER, rc-update add tiny4lite-control default, rc-service tiny4lite-control start"),
        "runit" => line(format!("#!/bin/sh\nexec 2>&1\nexec {p} service run\n"), "~/.config/sv/tiny4lite-control/run (chmod +x), then link it into your user service directory"),
        "s6" => line(format!("#!/bin/execlineb -P\n{p} service run\n"), "your s6 service directory, as tiny4lite-control/run"),
        "dinit" => line(format!("type = process\ncommand = {p} service run\nrestart = true\n"), "~/.config/dinit.d/tiny4lite-control, then: dinitctl enable tiny4lite-control"),
        "cron" => line(format!("@reboot {p} service run\n"), "crontab -e"),
        "udev" => line("ACTION==\"add\", SUBSYSTEM==\"usb\", ATTR{idVendor}==\"3564\", ATTR{idProduct}==\"fef9\", TAG+=\"systemd\", ENV{SYSTEMD_USER_WANTS}=\"tiny4lite-control.service\"\n".to_string(),
            "/etc/udev/rules.d/90-tiny4lite-control.rules (needs root), then: udevadm control --reload. Use it with the systemd target above"),
        _ => None,
    }
}

pub(crate) fn install_path(target: &str) -> Option<PathBuf> {
    match target {
        "systemd" => Some(config_home().join("systemd/user/tiny4lite-control.service")),
        "xdg" => Some(config_home().join("autostart/tiny4lite-control-service.desktop")),
        _ => None,
    }
}

/// The best target for this machine.
fn detect_target() -> &'static str {
    if crate::setup::systemd_user_available() { "systemd" } else { "xdg" }
}

fn install_command(target: Option<&str>, write: bool) -> i32 {
    let target = target.unwrap_or_else(|| detect_target());
    let program = program_path();
    let Some((text, place)) = snippet(target, &program) else {
        eprintln!("Unknown target `{target}`. Run `tiny4lite-control service targets` for the list.");
        return crate::cli::USAGE;
    };
    if write {
        let Some(path) = install_path(target) else {
            eprintln!("`{target}` is not written for you, because it belongs in your own configuration. Add this to {place}:\n\n{text}");
            return crate::cli::USAGE;
        };
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        return match std::fs::write(&path, &text) {
            Ok(()) => {
                println!("Wrote {}", path.display());
                if target == "systemd" { println!("Now start it: systemctl --user daemon-reload && systemctl --user enable --now tiny4lite-control.service"); }
                else { println!("It starts the next time you log in."); }
                crate::cli::OK
            }
            Err(error) => { eprintln!("Could not write {}: {error}", path.display()); crate::cli::FAILED }
        };
    }
    println!("# {target}: put this in {place}\n");
    print!("{text}");
    if matches!(target, "systemd" | "xdg") { println!("\n# Or let this program write it: tiny4lite-control service install {target} --write"); }
    crate::cli::OK
}

fn uninstall_command(target: Option<&str>) -> i32 {
    let target = target.unwrap_or_else(|| detect_target());
    let Some(path) = install_path(target) else {
        eprintln!("Only the systemd and xdg targets are written by this program; remove the other lines yourself.");
        return crate::cli::USAGE;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {
            println!("Removed {}", path.display());
            if target == "systemd" { println!("Then: systemctl --user disable --now tiny4lite-control.service && systemctl --user daemon-reload"); }
            crate::cli::OK
        }
        Err(error) => { eprintln!("Could not remove {}: {error}", path.display()); crate::cli::FAILED }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sight<'a>(camera: Option<&'a str>, in_use: bool, window_open: bool) -> Sight<'a> {
        Sight { camera, in_use, window_open }
    }

    #[test]
    fn a_new_camera_is_restored_after_it_settles_and_only_once() {
        let mut brain = Brain::new(false, 10);
        let t0 = Instant::now();
        assert_eq!(brain.step(t0, &sight(None, false, false)), vec![]);
        assert_eq!(brain.step(t0, &sight(Some("/dev/video1"), false, false)), vec![Action::Connected("/dev/video1".into())]);
        assert_eq!(brain.step(t0 + Duration::from_secs(1), &sight(Some("/dev/video1"), false, false)), vec![]);
        assert_eq!(brain.step(t0 + Duration::from_secs(3), &sight(Some("/dev/video1"), false, false)), vec![Action::Restore]);
        assert_eq!(brain.step(t0 + Duration::from_secs(5), &sight(Some("/dev/video1"), false, false)), vec![]);
        // Unplugged and plugged back in: restored again.
        assert_eq!(brain.step(t0 + Duration::from_secs(6), &sight(None, false, false)), vec![Action::Disconnected]);
        assert_eq!(brain.step(t0 + Duration::from_secs(10), &sight(Some("/dev/video2"), false, false)), vec![Action::Connected("/dev/video2".into())]);
        assert_eq!(brain.step(t0 + Duration::from_secs(13), &sight(Some("/dev/video2"), false, false)), vec![Action::Restore]);
    }

    fn settled(brain: &mut Brain, t0: Instant) {
        brain.step(t0, &sight(Some("/dev/video1"), false, false));
        brain.step(t0 + Duration::from_secs(3), &sight(Some("/dev/video1"), false, false));
    }

    #[test]
    fn it_never_sleeps_the_camera_unless_turned_on() {
        let mut brain = Brain::new(false, 1);
        let t0 = Instant::now();
        settled(&mut brain, t0);
        for minutes in 1..=60 {
            assert!(!brain.step(t0 + Duration::from_secs(60 * minutes), &sight(Some("/dev/video1"), false, false)).contains(&Action::Sleep));
        }
    }

    #[test]
    fn an_idle_camera_sleeps_once_after_the_time_and_again_only_after_it_was_used() {
        let mut brain = Brain::new(true, 10);
        let t0 = Instant::now();
        settled(&mut brain, t0);
        let at = |m: u64| t0 + Duration::from_secs(10 + 60 * m);
        assert_eq!(brain.step(at(0), &sight(Some("/dev/video1"), false, false)), vec![]);
        assert_eq!(brain.step(at(9), &sight(Some("/dev/video1"), false, false)), vec![]);
        assert_eq!(brain.step(at(10), &sight(Some("/dev/video1"), false, false)), vec![Action::Sleep]);
        // Not sent again and again while it stays unused.
        for m in 11..40 { assert_eq!(brain.step(at(m), &sight(Some("/dev/video1"), false, false)), vec![]); }
        // Used, then idle again: another full wait, then sleep once more. Never a wake.
        assert_eq!(brain.step(at(41), &sight(Some("/dev/video1"), true, false)), vec![]);
        assert_eq!(brain.step(at(42), &sight(Some("/dev/video1"), false, false)), vec![]);
        assert_eq!(brain.step(at(51), &sight(Some("/dev/video1"), false, false)), vec![]);
        assert_eq!(brain.step(at(52), &sight(Some("/dev/video1"), false, false)), vec![Action::Sleep]);
    }

    #[test]
    fn use_or_an_open_window_resets_the_idle_clock() {
        let mut brain = Brain::new(true, 5);
        let t0 = Instant::now();
        settled(&mut brain, t0);
        let at = |m: u64| t0 + Duration::from_secs(10 + 60 * m);
        brain.step(at(0), &sight(Some("/dev/video1"), false, false));
        // Someone at the window at minute 4 restarts the wait.
        brain.step(at(4), &sight(Some("/dev/video1"), false, true));
        assert_eq!(brain.step(at(8), &sight(Some("/dev/video1"), false, false)), vec![]);
        assert_eq!(brain.step(at(13), &sight(Some("/dev/video1"), false, false)), vec![Action::Sleep]);
    }

    #[test]
    fn editing_the_policy_while_running_keeps_what_it_has_seen() {
        let mut brain = Brain::new(true, 10);
        let t0 = Instant::now();
        settled(&mut brain, t0);
        let at = |m: u64| t0 + Duration::from_secs(10 + 60 * m);
        brain.step(at(0), &sight(Some("/dev/video1"), false, false));
        // The timer is shortened to 3 minutes while it counts: it sleeps at 3 minutes of idleness.
        brain.set_policy(true, 3);
        assert_eq!(brain.step(at(2), &sight(Some("/dev/video1"), false, false)), vec![]);
        assert_eq!(brain.step(at(3), &sight(Some("/dev/video1"), false, false)), vec![Action::Sleep]);
        // Turned off: nothing more is done.
        brain.set_policy(false, 3);
        assert_eq!(brain.step(at(30), &sight(Some("/dev/video1"), false, false)), vec![]);
    }

    #[test]
    fn a_restore_never_turns_the_gimbal_unless_allowed() {
        let mut config = crate::config::AppConfig::default();
        config.apply_on_connect = true;
        config.v4l2_controls.insert("brightness".into(), 40);
        config.v4l2_controls.insert("pan_absolute".into(), 3600);
        let preset = restore_preset(&config).unwrap();
        assert!(preset.settings.contains_key("v4l2:brightness"));
        assert!(!preset.settings.contains_key("v4l2:pan_absolute"));
        config.service.allow_gimbal_move = true;
        assert!(restore_preset(&config).unwrap().settings.contains_key("v4l2:pan_absolute"));
        // Off means nothing is restored.
        config.apply_on_connect = false;
        assert!(restore_preset(&config).is_none());
    }

    #[test]
    fn a_named_preset_is_restored_without_its_position_by_default() {
        let mut config = crate::config::AppConfig::default();
        config.apply_on_connect = true;
        config.service.connect_preset = Some("desk".into());
        let mut settings = BTreeMap::new();
        settings.insert("v4l2:contrast".to_string(), 55);
        settings.insert("v4l2:tilt_absolute".to_string(), -3600);
        config.presets.push(CameraPreset { name: "Desk".into(), settings });
        let preset = restore_preset(&config).unwrap();
        assert_eq!(preset.settings.len(), 1);
        // A name that no longer exists restores nothing, rather than something else.
        config.service.connect_preset = Some("gone".into());
        assert!(restore_preset(&config).is_none());
    }

    #[test]
    fn only_one_service_runs_and_a_dead_lock_is_taken_over() {
        let dir = std::env::temp_dir().join(format!("t4-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("service.lock");
        // A lock left by a process that does not exist.
        std::fs::write(&path, "999999999").unwrap();
        let lock = Lock::acquire(&path).expect("a dead lock is taken over");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), std::process::id().to_string());
        drop(lock);
        assert!(!path.exists());
    }

    #[test]
    fn every_target_has_a_snippet_that_starts_the_service_with_the_program() {
        for (name, _) in TARGETS {
            let (text, place) = snippet(name, "/opt/Tiny 2/tiny4lite-control").unwrap_or_else(|| panic!("no snippet for {name}"));
            assert!(!place.is_empty() && !text.is_empty());
            if *name != "udev" {
                assert!(text.contains("service") && text.contains("run"), "{name}: {text}");
                assert!(text.contains("Tiny 2"), "{name} dropped the path: {text}");
            }
        }
        assert!(snippet("nonsense", "x").is_none());
    }

    #[test]
    fn a_path_with_spaces_is_quoted_for_each_syntax() {
        let (legacy, _) = snippet("hyprland-legacy", "/a b/tiny").unwrap();
        assert_eq!(legacy, "exec-once = \"/a b/tiny\" service run\n");
        // Hyprland's Lua configuration: a start handler that runs the command through a Lua string.
        let (lua, place) = snippet("hyprland", "/a b/tiny").unwrap();
        assert_eq!(lua, "hl.on(\"hyprland.start\", function()\n  hl.exec_cmd(\"\\\"/a b/tiny\\\" service run\")\nend)\n");
        assert!(place.ends_with("hyprland.lua"));
        let (niri, _) = snippet("niri", "/a b/tiny").unwrap();
        assert!(niri.starts_with("spawn-sh-at-startup \"\\\"/a b/tiny\\\" service run\"\n"), "{niri}");
        assert!(niri.contains("// spawn-at-startup \"/a b/tiny\" \"service\" \"run\""));
        let (unit, _) = snippet("systemd", "/a b/tiny").unwrap();
        assert!(unit.contains("ExecStart=\"/a b/tiny\" service run\n"));
        let (plain, _) = snippet("sway", "/usr/bin/tiny4lite-control").unwrap();
        assert_eq!(plain, "exec /usr/bin/tiny4lite-control service run\n");
    }

    #[test]
    fn the_status_file_round_trips() {
        let status = Status { pid: 7, started: 5, camera: Some("/dev/video1".into()), last_event: "x".into(), last_event_at: 9, restored: true, asleep_by_service: false, denoise: Some(crate::denoise::Report { state: "running".into(), cpu_percent: Some(12.5), ..Default::default() }) };
        let text = serde_json::to_string(&status).unwrap();
        assert_eq!(serde_json::from_str::<Status>(&text).unwrap(), status);
    }
}
