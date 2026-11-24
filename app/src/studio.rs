//! The Studio camera: a second camera, "Tiny 2 Lite Studio", for any video app, made by the `v4l2loopback`
//! kernel module and fed by the background service. It is the frame for filters on the camera's picture;
//! with none, it passes the picture through as it is.
//!
//! It is kept as light as it can be:
//! * Nothing runs on the real camera until an app opens the Studio camera. Until then a tiny placeholder
//!   (a dark frame, five times a second, at the camera's size) keeps it visible to apps (browsers only list
//!   it while something writes to it), and the real camera stays free to sleep.
//! * When an app opens it, `ffmpeg` reads the camera and writes to it. A few seconds after the last app
//!   lets go (at once when the app's own preview goes back to the camera) it stops again.
//! * A camera at 60 fps is brought to 30 first, if wanted.
//!
//! CPU, memory and the frame rate it really keeps up are measured and shown in the window.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The name the virtual camera has in every app.
pub const LABEL: &str = "Tiny 2 Lite Studio";
/// Its device number, out of the way of real cameras.
pub const DEVICE_NUMBER: u32 = 42;
/// After the last app lets go, the camera is held this long in case it comes straight back.
const LINGER: Duration = Duration::from_secs(5);
/// After a failure, wait this long before trying again.
// Short: the commonest failure is the camera still busy for a moment (the app's own preview letting go of
// it as the Studio picture starts).
const RETRY: Duration = Duration::from_secs(6);

// ── What is there ─────────────────────────────────────────────────────────

/// The virtual camera's node, if the module made it: the video device whose name is `LABEL`.
pub fn loopback_node() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/video4linux").ok()?;
    let mut found: Vec<String> = entries.flatten().filter_map(|e| {
        let name = std::fs::read_to_string(e.path().join("name")).ok()?;
        (name.trim() == LABEL).then(|| format!("/dev/{}", e.file_name().to_string_lossy()))
    }).collect();
    found.sort();
    found.into_iter().next()
}

pub fn module_loaded() -> bool { Path::new("/sys/module/v4l2loopback").exists() }

pub fn ffmpeg_found() -> bool { crate::setup::program_found("ffmpeg") }

/// What the camera sends, as ffmpeg needs to know it.
#[derive(Clone, Debug, PartialEq)]
pub struct Format {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// ffmpeg's name for the camera's format: `mjpeg` or `yuyv422`.
    pub input: String,
}

impl Format {
    pub fn from_mode(mode: &crate::video::VideoMode) -> Option<Format> {
        let input = match mode.pixel_format.as_str() { "MJPG" => "mjpeg", "YUYV" => "yuyv422", _ => return None };
        Some(Format { width: mode.width, height: mode.height, fps: f64::from(mode.frame_rate_milli) / 1000.0, input: input.into() })
    }
}

fn base_args() -> Vec<String> {
    ["-hide_banner", "-loglevel", "error", "-nostdin"].iter().map(|s| s.to_string()).collect()
}

/// The picture format the Studio camera sends: 4:2:2 (YUYV), the camera's own colour detail. (4:2:0 halves
/// the colour resolution again, which shows as jagged coloured edges, and costs more to make.)
pub const OUTPUT_FORMAT: &str = "yuyv422";

/// Real camera in, the picture out to the Studio camera (no filters yet: as it is).
/// `out_size` is the size the Studio camera already has while an app holds it open, if that differs from the
/// camera's (the camera's mode was changed meanwhile): the picture is scaled to it, since the Studio camera
/// cannot change size under an app (4K written into a 1080p Studio camera comes out as tiles of picture).
pub fn pipeline_args(camera: &str, loopback: &str, format: &Format, max_fps: Option<f64>, lut: Option<(crate::lut::Route, &Path)>, out_size: Option<(u32, u32)>) -> Vec<String> {
    let mut args = base_args();
    if let Some((route, _)) = lut { args.extend(crate::lut::hw_args(route)); }
    // Low delay (only options every ffmpeg takes for an input: newer ones refuse -thread_queue_size here):
    // no input buffering or probing (the format is known), the camera's frames unpacked on one
    // core (unpacking spread over cores holds a frame back per core), and frames passed on as they come
    // rather than re-timed (re-timing duplicates or drops frames, and catching up builds lag).
    args.extend(["-fflags".into(), "nobuffer".into(), "-flags".into(), "low_delay".into(), "-probesize".into(), "32".into(),
        "-analyzeduration".into(), "0".into(), "-threads".into(), "1".into()]);
    args.extend([
        "-f".into(), "v4l2".into(), "-input_format".into(), format.input.clone(),
        "-video_size".into(), format!("{}x{}", format.width, format.height),
        "-framerate".into(), format!("{:.3}", format.fps), "-i".into(), camera.into(),
    ]);
    // A camera set to 60 fps is brought down first, which halves all the work after it.
    let mut limit = match max_fps { Some(cap) if format.fps > cap + 0.5 => format!("fps={cap:.0},"), _ => String::new() };
    if let Some((w, h)) = out_size.filter(|size| *size != (format.width, format.height)) { limit.push_str(&format!("scale={w}:{h}:flags=bilinear,")); }
    let filters = match lut {
        Some((route, path)) => crate::lut::filter(route, path, OUTPUT_FORMAT),
        None => format!("format={OUTPUT_FORMAT}"),
    };
    args.extend(["-vf".into(), format!("{limit}{filters}")]);
    // Progress every second (frames done so far), to measure the frame rate the Studio camera really gets.
    args.extend(["-progress".into(), progress_path().to_string_lossy().to_string(), "-stats_period".into(), "1".into()]);
    args.extend(["-fps_mode".into(), "passthrough".into(), "-f".into(), "v4l2".into(), loopback.into()]);
    args
}

/// The stand-in while no app is watching: a dark frame at the camera's size, five times a second, in the
/// same format the real picture comes in.
pub fn placeholder_args(loopback: &str, format: &Format) -> Vec<String> {
    let mut args = base_args();
    args.extend([
        "-re".into(), "-f".into(), "lavfi".into(),
        "-i".into(), format!("color=c=0x101418:s={}x{}:r=5", format.width, format.height),
        "-vf".into(), format!("format={OUTPUT_FORMAT}"), "-f".into(), "v4l2".into(), loopback.into(),
    ]);
    args
}

/// While the camera is asleep: the chosen picture, scaled to fit and centred on a dark background at the
/// size the Studio camera has, five times a second, instead of the camera's dark frames.
pub fn picture_args(loopback: &str, width: u32, height: u32, picture: &Path) -> Vec<String> {
    let mut args = base_args();
    args.extend([
        "-re".into(), "-loop".into(), "1".into(), "-framerate".into(), "5".into(), "-i".into(), picture.to_string_lossy().to_string(),
        "-vf".into(), format!("scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=0x101418,format={OUTPUT_FORMAT}"),
        "-f".into(), "v4l2".into(), loopback.into(),
    ]);
    args
}

/// Where the sleep picture is kept: a copy in the app's settings folder, so moving the original does no harm.
pub fn import_sleep_picture(source: &Path) -> Result<PathBuf, String> {
    let extension = source.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_else(|| "png".into());
    if !["png", "jpg", "jpeg", "webp", "bmp"].contains(&extension.as_str()) { return Err("Choose a PNG, JPEG, WebP or BMP picture.".into()); }
    let folder = crate::config::config_path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let target = folder.join(format!("sleep-picture.{extension}"));
    std::fs::copy(source, &target).map_err(|e| format!("Could not copy the picture: {e}"))?;
    Ok(target)
}

/// Ask the service to let go of the camera now (the preview went back to the camera itself), instead of
/// after the usual pause.
pub fn request_release() {
    let path = crate::service::runtime_dir().join("studio-release");
    if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
    let _ = std::fs::write(path, "");
}

/// Whether a release was asked for (looked at often, so it only checks a file).
pub fn release_requested() -> bool { crate::service::runtime_dir().join("studio-release").exists() }

fn take_release() -> bool { std::fs::remove_file(crate::service::runtime_dir().join("studio-release")).is_ok() }

// ── Deciding ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Want { Nothing, Placeholder, Pipeline }

/// What should run, from what is seen. `idle_for` is how long no app has had the virtual camera open
/// while the pipeline ran.
pub fn decide(enabled: bool, loopback: bool, camera: bool, watched: bool, pipeline_running: bool, idle_for: Option<Duration>) -> Want {
    if !enabled || !loopback { return Want::Nothing; }
    if !camera { return Want::Nothing; }
    if watched { return Want::Pipeline; }
    if pipeline_running && idle_for.is_some_and(|idle| idle < LINGER) { return Want::Pipeline; }
    Want::Placeholder
}

// ── Running it ────────────────────────────────────────────────────────────

/// What the window shows about it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    /// off, unavailable, waiting, running or error.
    pub state: String,
    pub detail: String,
    pub pid: Option<u32>,
    pub cpu_percent: Option<f64>,
    /// Frames a second the Studio camera actually gets, and what it should get.
    #[serde(default)]
    pub fps: Option<f64>,
    #[serde(default)]
    pub target_fps: Option<f64>,
    pub memory_mb: Option<f64>,
    pub device: Option<String>,
}

struct Running {
    child: Child,
    want: Want,
    /// What it was started with; a change (size, frame rate, LUT) restarts it.
    key: String,
    started: Instant,
    /// Where the LUT runs, if one is applied.
    lut: Option<crate::lut::Route>,
}

#[derive(Default)]
pub struct Studio {
    running: Option<Running>,
    /// An app had the Studio camera open at the last look.
    watched: bool,
    idle_since: Option<Instant>,
    retry_after: Option<Instant>,
    last_error: Option<String>,
    cpu_sample: Option<(Instant, u64)>,
    /// The last frame count ffmpeg reported, and when, for the frame rate.
    frame_samples: std::collections::VecDeque<(Instant, u64)>,
    format: Option<(String, Format)>,
    /// Whether the graphics chip can apply LUTs here (tested once, the first time one is needed), and if
    /// not, what it said.
    lut_gpu: Option<Result<(), String>>,
}

fn log_path() -> PathBuf { crate::service::runtime_dir().join("studio.log") }

/// Where ffmpeg writes its progress (frames done) every second.
fn progress_path() -> PathBuf { crate::service::runtime_dir().join("studio.progress") }

/// The last frame count in the progress file.
fn last_frame_count() -> Option<u64> {
    let text = std::fs::read_to_string(progress_path()).ok()?;
    text.lines().rev().find_map(|l| l.strip_prefix("frame=")).and_then(|v| v.trim().parse().ok())
}

fn last_lines(path: &Path, count: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join(" ")
}

/// CPU time used by a process so far, in clock ticks (user and system).
fn cpu_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is in brackets and may hold spaces: count fields after the last ')'.
    let rest = &stat[stat.rfind(')')? + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}

fn memory_mb(pid: u32) -> Option<f64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

impl Studio {
    pub fn new() -> Studio { Studio::default() }

    fn stop(&mut self) {
        if let Some(mut running) = self.running.take() {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
        self.cpu_sample = None;
        self.frame_samples.clear();
        let _ = std::fs::remove_file(progress_path());
    }

    pub fn stop_all(&mut self) { self.stop(); }

    /// Whether an app was watching the Studio camera at the last look: the camera is in use then, for
    /// the sleep timer, even in the moment before the Studio camera starts reading it.
    pub fn watched(&self) -> bool { self.watched }

    /// Whether the Studio camera is reading the camera right now (not a placeholder picture).
    pub fn reading_camera(&self) -> bool { self.running.as_ref().is_some_and(|r| r.want == Want::Pipeline) }

    fn our_pids(&self) -> Vec<u32> { self.running.as_ref().map(|r| vec![r.child.id()]).unwrap_or_default() }

    /// The camera's format. Read again whenever the Studio camera is not streaming from it (the mode may have
    /// been changed, 1080p to 4K, say); while it streams, the one it started with.
    fn format_for(&mut self, camera: &str, fresh: bool) -> Option<Format> {
        if let Some((node, format)) = &self.format { if node == camera && !fresh { return Some(format.clone()); } }
        let mode = crate::video::current_mode(camera)?;
        let format = Format::from_mode(&mode)?;
        self.format = Some((camera.to_string(), format.clone()));
        Some(format)
    }

    /// Look around once and start, stop or switch what runs. Called every round of the service.
    /// `asleep`: the camera reports it is asleep; with a sleep picture chosen, that is shown instead.
    pub fn step(&mut self, config: &crate::config::StudioConfig, camera: Option<&str>, asleep: bool) -> Report {
        let loopback = loopback_node();
        if !config.enabled {
            self.watched = false;
            // The virtual camera stays in apps' lists while the module is loaded (only root can unload it).
            // An app that still has it open would freeze on the last picture, which looks as if turning it
            // off did nothing: so it gets the dark placeholder until it lets go, and then nothing runs.
            let still_open = loopback.as_deref().is_some_and(|node| {
                let ours = self.our_pids();
                crate::backend::usage::openers(node).iter().any(|pid| !ours.contains(pid))
            });
            // The size the app holding it has, so the dark picture fits it.
            let held = loopback.as_deref().and_then(crate::video::current_mode).filter(|m| m.width > 0 && m.height > 0);
            let format = self.format.as_ref().map(|(_, f)| f.clone()).map(|f| match held { Some(m) => Format { width: m.width, height: m.height, ..f }, None => f });
            match (still_open, loopback.as_deref(), format) {
                (true, Some(node), Some(format)) if ffmpeg_found() => {
                    let args = placeholder_args(node, &format);
                    let key = args.join(" ");
                    if self.running.as_ref().map(|r| &r.key) != Some(&key) {
                        self.stop();
                        if let Ok(child) = Command::new("ffmpeg").args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                            self.running = Some(Running { child, want: Want::Placeholder, key, started: Instant::now(), lut: None });
                        }
                    }
                    return Report { state: "off".into(), detail: "Off: an app still has the Studio camera open, so it shows a dark picture until the app lets go.".into(), device: loopback, ..Report::default() };
                }
                _ => self.stop(),
            }
            return Report { state: "off".into(), device: loopback, ..Report::default() };
        }
        if !ffmpeg_found() || loopback.is_none() {
            self.stop();
            let detail = if !ffmpeg_found() { "ffmpeg is not installed." }
                else if !module_loaded() { "The v4l2loopback module is not loaded." }
                else { "The virtual camera is missing (v4l2loopback is loaded without it)." };
            return Report { state: "unavailable".into(), detail: format!("{detail} Use Set up… on the Studio camera card."), ..Report::default() };
        }
        let loopback = loopback.unwrap_or_default();

        // Has what runs ended by itself?
        if let Some(running) = self.running.as_mut() {
            if let Ok(Some(status)) = running.child.try_wait() {
                let (lived, lut) = (running.started.elapsed(), running.lut);
                self.running = None;
                let said = last_lines(&log_path(), 3);
                if lut == Some(crate::lut::Route::Gpu) && crate::lut::blames_gpu(&said) {
                    // The graphics chip did not manage it on the real picture: the processor from now on.
                    self.lut_gpu = Some(Err(format!("it stopped on the real picture: {said}")));
                } else if lived < Duration::from_secs(5) || !status.success() {
                    self.last_error = Some(if said.is_empty() { format!("ffmpeg stopped ({status}).") } else { said });
                    self.retry_after = Some(Instant::now() + RETRY);
                }
            }
        }

        // Is an app watching the virtual camera (anyone but our own ffmpeg)?
        let ours = self.our_pids();
        let watched = crate::backend::usage::openers(&loopback).iter().any(|pid| !ours.contains(pid));
        self.watched = watched;
        let pipeline_running = self.running.as_ref().is_some_and(|r| r.want == Want::Pipeline);
        if watched { self.idle_since = None; } else if pipeline_running { self.idle_since.get_or_insert(Instant::now()); }
        // The app's preview went back to the camera: let go now rather than after the pause.
        let hurry = take_release() && !watched;
        let idle_for = if hurry { Some(LINGER * 2) } else { self.idle_since.map(|at| at.elapsed()) };
        let want = decide(true, true, camera.is_some(), watched, pipeline_running, idle_for);
        if want != Want::Pipeline { self.idle_since = None; }

        let waiting = self.retry_after.is_some_and(|at| Instant::now() < at);
        let max_fps = config.limit_fps.then_some(30.0);
        // The LUT, if one is on: blended to its strength, on the graphics chip if it can (tested once).
        let mut lut_problem = None;
        let lut = match (&config.lut, config.lut_on, want) {
            (Some(name), true, Want::Pipeline) => match crate::lut::prepared(name, config.lut_strength) {
                Ok(path) => {
                    if config.lut_gpu && self.lut_gpu.is_none() { self.lut_gpu = Some(crate::lut::probe_gpu()); }
                    let route = if config.lut_gpu && matches!(self.lut_gpu, Some(Ok(()))) { crate::lut::Route::Gpu } else { crate::lut::Route::Cpu };
                    Some((route, path))
                }
                Err(error) => { lut_problem = Some(error); None }
            },
            _ => None,
        };
        let target = match (want, camera) {
            (Want::Nothing, _) | (_, None) => None,
            (want, Some(camera)) => self.format_for(camera, !pipeline_running).map(|format| {
                // An app holding the Studio camera fixes its size: keep to it (scaling if the camera's mode
                // changed since), until the app lets go.
                let held = if watched { crate::video::current_mode(&loopback).map(|m| (m.width, m.height)).filter(|(w, h)| *w > 0 && *h > 0) } else { None };
                let picture = config.sleep_picture.as_deref().filter(|p| asleep && p.exists());
                let args = match (want, picture) {
                    (Want::Pipeline, Some(picture)) => {
                        let (w, h) = held.unwrap_or((format.width, format.height));
                        picture_args(&loopback, w, h, picture)
                    }
                    (Want::Pipeline, None) => pipeline_args(camera, &loopback, &format, max_fps, lut.as_ref().map(|(r, p)| (*r, p.as_path())), held),
                    _ => placeholder_args(&loopback, &format),
                };
                (want, args)
            }),
        };
        match target {
            None => self.stop(),
            Some((want, args)) => {
                let key = args.join(" ");
                if self.running.as_ref().map(|r| &r.key) != Some(&key) && !waiting {
                    self.stop();
                    let log = std::fs::File::create(log_path()).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
                    match Command::new("ffmpeg").args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log).spawn() {
                        Ok(child) => {
                            crate::service::note(&format!("Studio camera: {}.", if want == Want::Pipeline { "an app opened it; running" } else { "waiting for an app to open it" }));
                            let lut_route = if want == Want::Pipeline { lut.as_ref().map(|(r, _)| *r) } else { None };
                            self.running = Some(Running { child, want, key, started: Instant::now(), lut: lut_route });
                            if want == Want::Pipeline { self.last_error = None; }
                        }
                        Err(error) => {
                            self.last_error = Some(format!("Could not start ffmpeg: {error}"));
                            self.retry_after = Some(Instant::now() + RETRY);
                        }
                    }
                }
            }
        }

        // What it costs.
        let mut report = Report { device: Some(loopback), ..Report::default() };
        match self.running.as_ref() {
            Some(running) => {
                let pid = running.child.id();
                report.pid = Some(pid);
                report.memory_mb = memory_mb(pid);
                if running.want == Want::Pipeline {
                    // The frame rate it really keeps up, over the last ten seconds or so: a short window makes a
                    // steady 30 read 29 or 31, as frames and progress reports fall either side of it.
                    if let Some(frames) = last_frame_count() {
                        let now = Instant::now();
                        if self.frame_samples.back().is_some_and(|(_, last)| frames < *last) { self.frame_samples.clear(); }
                        self.frame_samples.push_back((now, frames));
                        while self.frame_samples.len() > 2 && self.frame_samples.front().is_some_and(|(at, _)| now.duration_since(*at) > Duration::from_secs(10)) {
                            self.frame_samples.pop_front();
                        }
                        if let (Some((first_at, first)), Some((_, last))) = (self.frame_samples.front(), self.frame_samples.back()) {
                            let seconds = now.duration_since(*first_at).as_secs_f64();
                            if seconds > 1.5 { report.fps = Some((last - first) as f64 / seconds); }
                        }
                    }
                    report.target_fps = self.format.as_ref().map(|(_, f)| if config.limit_fps { f.fps.min(30.0) } else { f.fps });
                }
                if let Some(ticks) = cpu_ticks(pid) {
                    if let Some((at, before)) = self.cpu_sample {
                        let seconds = at.elapsed().as_secs_f64();
                        // /proc counts in USER_HZ, which is 100 on Linux.
                        if seconds > 0.1 { report.cpu_percent = Some(ticks.saturating_sub(before) as f64 / 100.0 / seconds * 100.0); }
                    }
                    self.cpu_sample = Some((Instant::now(), ticks));
                }
                if running.want == Want::Pipeline && running.key.contains("-loop 1") {
                    report.state = "running".into();
                    report.detail = "The camera is asleep: showing your sleep picture.".into();
                } else if running.want == Want::Pipeline {
                    report.state = "running".into();
                    report.detail = match running.lut {
                        Some(route) => format!("Applying the LUT on the {}.", route.name()),
                        None => "Passing the picture through.".into(),
                    };
                    // Why the LUT is on the processor, which is heavy.
                    if running.lut == Some(crate::lut::Route::Cpu) {
                        let why = if !config.lut_gpu { "Use the graphics chip is off.".to_string() }
                            else if let Some(Err(said)) = &self.lut_gpu { format!("The graphics chip could not: {said}") }
                            else { String::new() };
                        report.detail = format!("{} {why}", report.detail).trim().to_string();
                    }
                    if let Some(problem) = &lut_problem { report.detail = format!("{} The LUT is not applied: {problem}", report.detail); }
                } else {
                    report.state = "waiting".into();
                    report.detail = "Ready: choose \u{201c}Tiny 2 Lite Studio\u{201d} in an app. The camera is not used until then.".into();
                }
            }
            None => {
                report.state = if self.last_error.is_some() { "error".into() } else { "waiting".into() };
                report.detail = if camera.is_none() { "The camera is not connected.".into() } else { String::new() };
            }
        }
        if let Some(error) = &self.last_error { report.detail = format!("{} {}", report.detail, error).trim().to_string(); }
        report
    }
}

// ── Setting it up (needs root, run in a terminal) ─────────────────────────

// The names from when the Studio camera was called Denoise; kept, so a system set up then stays as it is.
const MODPROBE_FILE: &str = "/etc/modprobe.d/tiny4lite-denoise.conf";
const MODULES_FILE: &str = "/etc/modules-load.d/tiny4lite-denoise.conf";

pub fn module_options() -> String {
    format!("options v4l2loopback devices=1 video_nr={DEVICE_NUMBER} card_label=\"{LABEL}\" exclusive_caps=1\n")
}

/// `tiny4lite-control studio setup`: install ffmpeg and v4l2loopback, make the module load at boot with
/// the virtual camera, and load it now. Asks before each step that needs a password.
pub fn setup_command(yes: bool) -> i32 {
    use crate::setup::{confirm, install_command, manager, program_found, wait_to_close};
    println!("Tiny 2 Lite Control: Studio camera setup\n");
    let Some(manager) = manager() else {
        println!("No package manager was recognised, so nothing is installed from here.");
        if Path::new("/etc/NIXOS").exists() {
            println!("On NixOS, add to configuration.nix and rebuild:");
            println!("  environment.systemPackages = [ pkgs.ffmpeg ];");
            println!("  boot.extraModulePackages = [ config.boot.kernelPackages.v4l2loopback ];");
            println!("  boot.kernelModules = [ \"v4l2loopback\" ];");
            println!("  boot.extraModprobeConfig = ''{}'';", module_options().trim());
        } else {
            println!("Install ffmpeg and the v4l2loopback kernel module with your system's tools, then run this again.");
        }
        wait_to_close(yes);
        return crate::cli::FAILED;
    };
    let mut packages: Vec<String> = Vec::new();
    if !program_found("ffmpeg") { packages.extend(crate::setup::studio_package("ffmpeg", manager).map(String::from)); }
    let need_module = !module_loaded() && !module_installed();
    if need_module {
        match crate::setup::studio_package("v4l2loopback", manager) {
            Some(package) => {
                // A module built here needs the headers of the running kernel.
                packages.extend(crate::setup::kernel_headers_package(manager));
                packages.push(package.into());
            }
            None => println!("This distribution has no v4l2loopback package that this setup knows. Install the module yourself (see v4l2loopback's own instructions), then run this again.\n"),
        }
        if manager == crate::setup::Manager::Dnf {
            println!("Fedora: the v4l2loopback module comes from RPM Fusion (free). If it is not enabled, see rpmfusion.org/Configuration.\n");
        }
    }
    let packages: Vec<&str> = packages.iter().map(String::as_str).collect();
    if !packages.is_empty() {
        let command = install_command(manager, &packages);
        println!("1. Install {}:\n  {}\n", packages.join(", "), command.join(" "));
        if confirm("Go ahead? (asks for your password)", yes) {
            let _ = Command::new(&command[0]).args(&command[1..]).status();
        }
    } else {
        println!("1. ffmpeg and v4l2loopback are installed.");
    }
    if loopback_node().is_some() {
        println!("\n2. The virtual camera \u{201c}{LABEL}\u{201d} is already there.");
        wait_to_close(yes);
        return crate::cli::OK;
    }
    if module_loaded() {
        println!("\n2. v4l2loopback is already loaded (perhaps by OBS) without this app's camera. Loading it again with");
        println!("   this app's settings closes any other virtual camera (OBS's) until it is restarted.");
    }
    let sudo = crate::setup::gain_program();
    let run = |what: &str, args: &[&str]| -> bool {
        let mut command = Command::new(sudo.unwrap_or(args[0]));
        if sudo.is_some() { command.args(args); } else { command.args(&args[1..]); }
        let ok = command.status().is_ok_and(|s| s.success());
        if !ok { println!("   {what} did not work."); }
        ok
    };
    println!("\n2. Make the virtual camera now and at every start:");
    println!("   {MODPROBE_FILE}: {}", module_options().trim());
    println!("   {MODULES_FILE}: v4l2loopback");
    if !confirm("Go ahead? (asks for your password)", yes) { wait_to_close(yes); return crate::cli::FAILED; }
    let options = module_options();
    let wrote = run("Writing the module settings", &["sh", "-c", &format!("printf '%s' '{}' > {MODPROBE_FILE} && echo v4l2loopback > {MODULES_FILE}", options.replace('\'', "'\\''"))]);
    if module_loaded() { run("Unloading v4l2loopback", &["modprobe", "-r", "v4l2loopback"]); }
    let loaded = wrote && run("Loading v4l2loopback", &["modprobe", "v4l2loopback"]);
    std::thread::sleep(Duration::from_millis(500));
    let ok = loaded && loopback_node().is_some();
    if ok {
        println!("\nDone: \u{201c}{LABEL}\u{201d} is ready. Turn the Studio camera on in the app (Device tab) and choose it in your video app.");
    } else {
        println!("\nThe virtual camera did not appear. With Secure Boot on, an unsigned module cannot load; the kernel log says more:");
        println!("  journalctl -k -b | grep -i v4l2loopback");
    }
    wait_to_close(yes);
    if ok { crate::cli::OK } else { crate::cli::FAILED }
}

/// Whether the module is installed for the running kernel, even if not loaded.
pub fn module_installed() -> bool {
    Command::new("modinfo").arg("v4l2loopback").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt() -> Format { Format { width: 1920, height: 1080, fps: 30.0, input: "mjpeg".into() } }

    #[test]
    fn nothing_runs_on_the_camera_until_an_app_watches_and_it_lets_go_after_a_moment() {
        assert_eq!(decide(false, true, true, true, false, None), Want::Nothing);
        assert_eq!(decide(true, false, true, true, false, None), Want::Nothing);
        assert_eq!(decide(true, true, false, true, false, None), Want::Nothing);
        assert_eq!(decide(true, true, true, false, false, None), Want::Placeholder);
        assert_eq!(decide(true, true, true, true, false, None), Want::Pipeline);
        assert_eq!(decide(true, true, true, false, true, Some(Duration::from_secs(2))), Want::Pipeline);
        assert_eq!(decide(true, true, true, false, true, Some(Duration::from_secs(6))), Want::Placeholder);
    }

    #[test]
    fn the_sleep_picture_fits_the_studio_camera_and_loops() {
        let args = picture_args("/dev/video42", 1920, 1080, Path::new("/home/me/.config/tiny4lite-control/sleep-picture.png")).join(" ");
        assert!(args.contains("-loop 1") && args.contains("-i /home/me/.config/tiny4lite-control/sleep-picture.png"), "{args}");
        assert!(args.contains("scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:"), "{args}");
        assert!(args.ends_with("-f v4l2 /dev/video42"), "{args}");
    }

    #[test]
    fn the_commands_read_the_camera_and_write_the_virtual_one() {
        let pass = pipeline_args("/dev/video0", "/dev/video42", &fmt(), None, None, None).join(" ");
        assert!(pass.contains("-input_format mjpeg -video_size 1920x1080 -framerate 30.000 -i /dev/video0"), "{pass}");
        // Low delay: no buffering or probing, one decoding thread, frames passed on as they come.
        assert!(pass.contains("-fflags nobuffer -flags low_delay") && pass.contains("-threads 1 -f v4l2"), "{pass}");
        assert!(pass.contains("-fps_mode passthrough -f v4l2 /dev/video42"), "{pass}");
        assert!(pass.contains("-vf format=yuyv422 -progress") && pass.ends_with("-f v4l2 /dev/video42"), "{pass}");
        // A 60 fps camera is brought down to 30 first; a 30 fps one is left alone.
        let sixty = Format { fps: 60.0, ..fmt() };
        assert!(pipeline_args("/dev/video0", "/dev/video42", &sixty, Some(30.0), None, None).join(" ").contains("-vf fps=30,format=yuyv422"));
        assert!(!pipeline_args("/dev/video0", "/dev/video42", &fmt(), Some(30.0), None, None).join(" ").contains("fps=30,"));
        // 4K from the camera into a Studio camera an app holds at 1080p: scaled to fit, not tiled.
        let uhd = Format { width: 3840, height: 2160, fps: 30.0, input: "mjpeg".into() };
        let held = pipeline_args("/dev/video0", "/dev/video42", &uhd, Some(30.0), None, Some((1920, 1080))).join(" ");
        assert!(held.contains("-video_size 3840x2160") && held.contains("-vf scale=1920:1080:flags=bilinear,format=yuyv422"), "{held}");
        // The same size needs no scaling.
        assert!(!pipeline_args("/dev/video0", "/dev/video42", &uhd, None, None, Some((3840, 2160))).join(" ").contains("scale="));
        let wait = placeholder_args("/dev/video42", &fmt()).join(" ");
        assert!(wait.contains("color=c=0x101418:s=1920x1080:r=5") && wait.ends_with("-f v4l2 /dev/video42"), "{wait}");
        assert!(!wait.contains("/dev/video0"), "the placeholder never touches the camera");
    }

    #[test]
    fn the_module_makes_one_camera_with_this_name_that_browsers_list() {
        let options = module_options();
        assert!(options.contains("video_nr=42") && options.contains("card_label=\"Tiny 2 Lite Studio\"") && options.contains("exclusive_caps=1"));
    }

    #[test]
    fn cpu_time_is_read_past_a_command_name_with_spaces() {
        // Our own process always exists.
        assert!(cpu_ticks(std::process::id()).is_some());
        assert!(memory_mb(std::process::id()).is_some_and(|mb| mb > 0.0));
    }
}
