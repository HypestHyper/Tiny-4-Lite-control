//! The Studio camera: a second camera, "Tiny 2 Lite Studio", with filters applied (denoise first; more can
//! follow), for any video app.
//!
//! The camera itself has no noise reduction, and Linux cannot change what other programs read from the
//! real camera. So the background service writes a cleaned copy into a virtual camera made by the
//! `v4l2loopback` kernel module, and apps choose that camera instead.
//!
//! It is kept as light as it can be:
//! * Nothing runs on the real camera until an app opens the Studio camera. Until then a tiny placeholder
//!   (a dark frame, five times a second, at the camera's size) keeps the virtual camera visible to apps
//!   (browsers only list it while something writes to it), and the real camera stays free to sleep.
//! * When an app opens it, `ffmpeg` reads the camera, denoises and writes. A few seconds after the last
//!   app lets go, it stops again.
//! * The denoise runs on the graphics chip when it can: VA-API's own noise reduction (Intel), else Vulkan's
//!   non-local means (AMD, Intel, NVIDIA; the best-looking filter). Otherwise on the processor with
//!   `atadenoise` (light, and it does not smear moving things). A camera at 60 fps is brought to 30 first.
//!
//! CPU and memory of the running `ffmpeg` are measured and shown in the window.

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

/// The first GPU render node, for VA-API.
pub fn render_node() -> Option<String> {
    (128..136).map(|n| format!("/dev/dri/renderD{n}")).find(|p| Path::new(p).exists())
}

/// Whether this ffmpeg has the VA-API denoise filter (asked once).
/// The filters this ffmpeg has (asked once).
fn ffmpeg_filters() -> &'static str {
    use std::sync::OnceLock;
    static FILTERS: OnceLock<String> = OnceLock::new();
    FILTERS.get_or_init(|| {
        Command::new("ffmpeg").args(["-hide_banner", "-filters"]).stdin(Stdio::null()).stderr(Stdio::null()).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
    })
}

/// Where the denoising runs. Tried in this order; one that fails is not tried again until the service
/// restarts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Route {
    /// The graphics chip through VA-API's own noise reduction: Intel's driver has it, AMD's (Mesa) does not.
    Vaapi,
    /// The graphics chip through Vulkan (non-local means): AMD, Intel and NVIDIA.
    Vulkan,
    /// The processor (atadenoise).
    Cpu,
}

impl Route {
    pub fn name(self) -> &'static str {
        match self { Route::Vaapi => "graphics chip (VA-API)", Route::Vulkan => "graphics chip (Vulkan)", Route::Cpu => "processor" }
    }
}

/// The Vulkan denoiser's strength (1 to 100 in, its own 1 to 10 out). Its patch and search sizes are kept
/// small, which is plenty for a webcam and light on the graphics chip.
pub fn vulkan_filter(strength: u8) -> String {
    let s = 1.0 + f64::from(strength.clamp(1, 100) - 1) * 9.0 / 99.0;
    format!("nlmeans_vulkan=s={s:.2}:p=5:r=7")
}

// ── The commands ──────────────────────────────────────────────────────────

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

/// The processor filter for a strength from 1 to 100: `atadenoise`, which averages each pixel with the
/// frames around it only while it barely changes, so grain goes and moving things do not smear or ghost
/// (as `hqdn3d` did). Measured at 1080p30: about a quarter of one core, less than `hqdn3d`. 50 is ffmpeg's
/// own default thresholds.
pub fn cpu_filter(strength: u8) -> String {
    let k = f64::from(strength.clamp(1, 100)) / 50.0;
    let (a, b) = (0.02 * k, 0.04 * k);
    format!("atadenoise=0a={a:.4}:0b={b:.4}:1a={a:.4}:1b={b:.4}:2a={a:.4}:2b={b:.4}:s=9")
}

/// The graphics-chip level for a strength from 1 to 100 (VA-API takes 0 to 64).
pub fn gpu_level(strength: u8) -> u32 { (u32::from(strength.clamp(1, 100)) * 64).div_ceil(100) }

fn base_args() -> Vec<String> {
    ["-hide_banner", "-loglevel", "error", "-nostdin"].iter().map(|s| s.to_string()).collect()
}

/// The picture format the Studio camera sends: 4:2:2 (YUYV), the camera's own colour detail. (4:2:0 halves
/// the colour resolution again, which shows as jagged coloured edges, and costs more to make.)
pub const OUTPUT_FORMAT: &str = "yuyv422";

/// How the graphics chip is set up for a route.
fn hw_args(route: Route, render: Option<&str>) -> Vec<String> {
    match (route, render) {
        (Route::Vaapi, Some(node)) => vec!["-init_hw_device".into(), format!("vaapi=va:{node}"), "-filter_hw_device".into(), "va".into()],
        (Route::Vulkan, _) => vec!["-init_hw_device".into(), "vulkan=vk".into(), "-filter_hw_device".into(), "vk".into()],
        _ => Vec::new(),
    }
}

/// The filters, from the decoded camera picture to what the Studio camera sends. `limit` drops frames
/// first (a 60 fps camera to 30). Vulkan works in 4:2:2 where the driver can, 4:2:0 where it cannot.
pub fn filter_chain(denoise: Option<u8>, route: Route, vulkan_422: bool, limit: &str) -> String {
    let out = OUTPUT_FORMAT;
    match (denoise, route) {
        (Some(strength), Route::Vaapi) => format!("{limit}format=nv12,hwupload,denoise_vaapi=denoise={},hwdownload,format=nv12,format={out}", gpu_level(strength)),
        (Some(strength), Route::Vulkan) => {
            let inner = if vulkan_422 { "yuv422p" } else { "yuv420p" };
            format!("{limit}format={inner},hwupload,{},hwdownload,format={inner},format={out}", vulkan_filter(strength))
        }
        (Some(strength), Route::Cpu) => format!("{limit}{},format={out}", cpu_filter(strength)),
        (None, _) => format!("{limit}format={out}"),
    }
}

/// Real camera in, the filtered picture out to the Studio camera. No filter on: the picture as it is.
pub fn pipeline_args(camera: &str, loopback: &str, format: &Format, denoise: Option<u8>, choice: Choice, render: Option<&str>, max_fps: Option<f64>) -> Vec<String> {
    // The graphics chip is only worth setting up when there is a filter for it to run.
    let route = if denoise.is_some() { choice.route } else { Route::Cpu };
    let mut args = base_args();
    args.extend(hw_args(route, render));
    args.extend([
        "-f".into(), "v4l2".into(), "-input_format".into(), format.input.clone(),
        "-video_size".into(), format!("{}x{}", format.width, format.height),
        "-framerate".into(), format!("{:.3}", format.fps), "-i".into(), camera.into(),
    ]);
    // A camera set to 60 fps is brought down first, which halves all the work after it.
    let limit = match max_fps { Some(cap) if format.fps > cap + 0.5 => format!("fps={cap:.0},"), _ => String::new() };
    args.extend(["-vf".into(), filter_chain(denoise, route, choice.vulkan_422, &limit), "-f".into(), "v4l2".into(), loopback.into()]);
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

/// Which route the denoise uses, and whether Vulkan can work in 4:2:2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choice { pub route: Route, pub vulkan_422: bool }

impl Choice { pub const CPU: Choice = Choice { route: Route::Cpu, vulkan_422: false }; }

/// Try a route on a small made-up picture (no camera involved): it works, or what ffmpeg said.
pub fn probe(route: Route, vulkan_422: bool, render: Option<&str>) -> Result<(), String> {
    let mut args = base_args();
    args.extend(hw_args(route, render));
    args.extend(["-f".into(), "lavfi".into(), "-i".into(), "testsrc2=s=640x360:r=30".into(), "-frames:v".into(), "8".into(),
        "-vf".into(), filter_chain(Some(40), route, vulkan_422, ""), "-f".into(), "null".into(), "-".into()]);
    let mut child = Command::new("ffmpeg").args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()
        .map_err(|e| format!("ffmpeg could not start: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut said = String::new();
                if let Some(mut err) = child.stderr.take() { use std::io::Read; let _ = err.read_to_string(&mut said); }
                let said: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
                let said = said[said.len().saturating_sub(2)..].join(" ");
                return if status.success() { Ok(()) } else { Err(if said.is_empty() { format!("ffmpeg stopped ({status})") } else { said }) };
            }
            Ok(None) if started.elapsed() > Duration::from_secs(15) => { let _ = child.kill(); let _ = child.wait(); return Err("it took too long".into()); }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Test the graphics-chip routes in order and pick the first that works; the processor otherwise. Also
/// returns what each tried route said, for the card and `studio check`.
pub fn choose_route() -> (Choice, Vec<(String, Result<(), String>)>) {
    let render = render_node();
    let filters = ffmpeg_filters();
    let mut tried = Vec::new();
    let mut found = None;
    if render.is_some() && filters.contains("denoise_vaapi") {
        let result = probe(Route::Vaapi, false, render.as_deref());
        if result.is_ok() { found = Some(Choice { route: Route::Vaapi, vulkan_422: false }); }
        tried.push(("Graphics chip (VA-API)".to_string(), result));
    } else {
        tried.push(("Graphics chip (VA-API)".to_string(), Err(if render.is_none() { "no graphics chip found (/dev/dri)".into() } else { "this ffmpeg has no denoise_vaapi".into() })));
    }
    if found.is_none() {
        if filters.contains("nlmeans_vulkan") {
            for (vulkan_422, name) in [(true, "Graphics chip (Vulkan, 4:2:2)"), (false, "Graphics chip (Vulkan, 4:2:0)")] {
                let result = probe(Route::Vulkan, vulkan_422, render.as_deref());
                let ok = result.is_ok();
                tried.push((name.to_string(), result));
                if ok { found = Some(Choice { route: Route::Vulkan, vulkan_422 }); break; }
            }
        } else {
            tried.push(("Graphics chip (Vulkan)".to_string(), Err("this ffmpeg has no nlmeans_vulkan (it needs ffmpeg 7 built with Vulkan)".into())));
        }
    }
    (found.unwrap_or(Choice::CPU), tried)
}

/// `tiny4lite-control studio check`: which route the denoise can use here, and why the others cannot.
pub fn check_command() -> i32 {
    if !ffmpeg_found() { println!("ffmpeg is not installed."); return crate::cli::FAILED; }
    println!("Testing the denoise routes on a small made-up picture (the camera is not used)...\n");
    let (choice, tried) = choose_route();
    for (name, result) in &tried {
        match result { Ok(()) => println!("works      {name}"), Err(why) => println!("no         {name}: {why}") }
    }
    println!("\nThe Studio camera will denoise on the {}.", choice.route.name());
    crate::cli::OK
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
    pub memory_mb: Option<f64>,
    pub gpu: bool,
    pub device: Option<String>,
}

struct Running {
    child: Child,
    want: Want,
    /// What it was started with; a change (strength, size, GPU) restarts it.
    key: String,
    route: Route,
    started: Instant,
}

#[derive(Default)]
pub struct Denoiser {
    running: Option<Running>,
    /// An app had the Studio camera open at the last look.
    watched: bool,
    idle_since: Option<Instant>,
    retry_after: Option<Instant>,
    /// Routes that failed here, with what ffmpeg said, so the next one is used and the card says why.
    failed: Vec<(Route, String)>,
    /// The route the test picked, and what each tried route said; tested once, when first needed.
    chosen: Option<(Choice, Vec<(String, Result<(), String>)>)>,
    last_error: Option<String>,
    cpu_sample: Option<(Instant, u64)>,
    format: Option<(String, Format)>,
}

fn log_path() -> PathBuf { crate::service::runtime_dir().join("denoise.log") }

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

impl Denoiser {
    pub fn new() -> Denoiser { Denoiser::default() }

    fn stop(&mut self) {
        if let Some(mut running) = self.running.take() {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
        self.cpu_sample = None;
    }

    pub fn stop_all(&mut self) { self.stop(); }

    /// Whether an app was watching the Studio camera at the last look: the camera is in use then, for
    /// the sleep timer, even in the moment before the Studio camera starts reading it.
    pub fn watched(&self) -> bool { self.watched }

    fn our_pids(&self) -> Vec<u32> { self.running.as_ref().map(|r| vec![r.child.id()]).unwrap_or_default() }

    /// The camera's format, read once per camera (it is only read while nothing streams).
    fn format_for(&mut self, camera: &str) -> Option<Format> {
        if let Some((node, format)) = &self.format { if node == camera { return Some(format.clone()); } }
        let mode = crate::video::current_mode(camera)?;
        let format = Format::from_mode(&mode)?;
        self.format = Some((camera.to_string(), format.clone()));
        Some(format)
    }

    /// Look around once and start, stop or switch what runs. Called every round of the service.
    pub fn step(&mut self, config: &crate::config::DenoiseConfig, camera: Option<&str>) -> Report {
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
            let format = self.format.as_ref().map(|(_, f)| f.clone());
            match (still_open, loopback.as_deref(), format) {
                (true, Some(node), Some(format)) if ffmpeg_found() => {
                    let args = placeholder_args(node, &format);
                    let key = args.join(" ");
                    if self.running.as_ref().map(|r| &r.key) != Some(&key) {
                        self.stop();
                        if let Ok(child) = Command::new("ffmpeg").args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                            self.running = Some(Running { child, want: Want::Placeholder, key, route: Route::Cpu, started: Instant::now() });
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
                let (want, route, lived) = (running.want, running.route, running.started.elapsed());
                self.running = None;
                let said = last_lines(&log_path(), 3);
                if want == Want::Pipeline && route != Route::Cpu {
                    // This route did not work here: the next one is used, and the card says why.
                    self.failed.push((route, if said.is_empty() { format!("ffmpeg stopped ({status})") } else { said }));
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
        // The route the test picked (tested once, the first time the denoise is needed), unless it then
        // failed on the real picture.
        let render = render_node();
        if config.use_gpu && config.denoise && want == Want::Pipeline && self.chosen.is_none() { self.chosen = Some(choose_route()); }
        let choice = match &self.chosen {
            Some((choice, _)) if config.use_gpu && !self.failed.iter().any(|(r, _)| *r == choice.route) => *choice,
            _ => Choice::CPU,
        };
        let max_fps = config.limit_fps.then_some(30.0);
        let target = match (want, camera) {
            (Want::Nothing, _) | (_, None) => None,
            (want, Some(camera)) => self.format_for(camera).map(|format| {
                let args = if want == Want::Pipeline { pipeline_args(camera, &loopback, &format, config.denoise.then_some(config.strength), choice, render.as_deref(), max_fps) }
                    else { placeholder_args(&loopback, &format) };
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
                            let route = if want == Want::Pipeline && config.denoise { choice.route } else { Route::Cpu };
                            self.running = Some(Running { child, want, key, route, started: Instant::now() });
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
                report.gpu = running.route != Route::Cpu;
                report.memory_mb = memory_mb(pid);
                if let Some(ticks) = cpu_ticks(pid) {
                    if let Some((at, before)) = self.cpu_sample {
                        let seconds = at.elapsed().as_secs_f64();
                        // /proc counts in USER_HZ, which is 100 on Linux.
                        if seconds > 0.1 { report.cpu_percent = Some(ticks.saturating_sub(before) as f64 / 100.0 / seconds * 100.0); }
                    }
                    self.cpu_sample = Some((Instant::now(), ticks));
                }
                if running.want == Want::Pipeline {
                    report.state = "running".into();
                    report.detail = if !config.denoise { "Passing the picture through (no filter on).".into() }
                        else { format!("Denoising on the {}.", running.route.name()) };
                    // Why not on the graphics chip: turned off, or what the routes tried here said.
                    if config.denoise && running.route == Route::Cpu {
                        let tested = self.chosen.as_ref().map(|(_, tried)| tried.iter().filter_map(|(name, r)| r.as_ref().err().map(|why| format!("{name}: {why}"))).collect::<Vec<_>>()).unwrap_or_default();
                        let failed_live = self.failed.iter().map(|(r, said)| format!("The {} stopped on the real picture: {said}", r.name()));
                        let why = if !config.use_gpu { "Use the graphics chip is off.".to_string() }
                            else { tested.into_iter().chain(failed_live).collect::<Vec<_>>().join(" \u{00b7} ") };
                        report.detail = format!("{} {why}", report.detail);
                    }
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
    if !program_found("ffmpeg") { packages.extend(crate::setup::denoise_package("ffmpeg", manager).map(String::from)); }
    let need_module = !module_loaded() && !module_installed();
    if need_module {
        match crate::setup::denoise_package("v4l2loopback", manager) {
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
    fn strength_maps_onto_each_filter() {
        assert_eq!(cpu_filter(50), "atadenoise=0a=0.0200:0b=0.0400:1a=0.0200:1b=0.0400:2a=0.0200:2b=0.0400:s=9");
        assert_eq!(cpu_filter(0), cpu_filter(1));
        assert_eq!((gpu_level(1), gpu_level(50), gpu_level(100)), (1, 32, 64));
    }

    #[test]
    fn the_commands_read_the_camera_and_write_the_virtual_one() {
        let cpu = pipeline_args("/dev/video0", "/dev/video42", &fmt(), Some(50), Choice::CPU, None, None).join(" ");
        assert!(cpu.contains("-input_format mjpeg -video_size 1920x1080 -framerate 30.000 -i /dev/video0"), "{cpu}");
        assert!(cpu.contains("-vf atadenoise=0a=0.0200:0b=0.0400:1a=0.0200:1b=0.0400:2a=0.0200:2b=0.0400:s=9,format=yuyv422 -f v4l2 /dev/video42"), "{cpu}");
        let gpu = pipeline_args("/dev/video0", "/dev/video42", &fmt(), Some(50), Choice { route: Route::Vaapi, vulkan_422: false }, Some("/dev/dri/renderD128"), None).join(" ");
        assert!(gpu.contains("-init_hw_device vaapi=va:/dev/dri/renderD128 -filter_hw_device va"), "{gpu}");
        assert!(gpu.contains("denoise_vaapi=denoise=32"), "{gpu}");
        let wait = placeholder_args("/dev/video42", &fmt()).join(" ");
        assert!(wait.contains("color=c=0x101418:s=1920x1080:r=5") && wait.ends_with("-f v4l2 /dev/video42"), "{wait}");
        assert!(!wait.contains("/dev/video0"), "the placeholder never touches the camera");
        // No filter on: the picture passes through, and no graphics chip is set up for nothing.
        let plain = pipeline_args("/dev/video0", "/dev/video42", &fmt(), None, Choice { route: Route::Vaapi, vulkan_422: false }, Some("/dev/dri/renderD128"), None).join(" ");
        assert!(plain.contains("-vf format=yuyv422 -f v4l2") && !plain.contains("vaapi") && !plain.contains("atadenoise"), "{plain}");
        // Vulkan (AMD and the rest): uploaded, denoised, downloaded.
        let vulkan = pipeline_args("/dev/video0", "/dev/video42", &fmt(), Some(50), Choice { route: Route::Vulkan, vulkan_422: true }, None, None).join(" ");
        assert!(vulkan.contains("-init_hw_device vulkan=vk -filter_hw_device vk"), "{vulkan}");
        assert!(vulkan.contains("format=yuv422p,hwupload,nlmeans_vulkan=s=5.45:p=5:r=7,hwdownload,format=yuv422p,format=yuyv422"), "{vulkan}");
        // A 60 fps camera is brought down to 30 first; a 30 fps one is left alone.
        let sixty = Format { fps: 60.0, ..fmt() };
        assert!(pipeline_args("/dev/video0", "/dev/video42", &sixty, Some(50), Choice::CPU, None, Some(30.0)).join(" ").contains("-vf fps=30,atadenoise"));
        assert!(!pipeline_args("/dev/video0", "/dev/video42", &fmt(), Some(50), Choice::CPU, None, Some(30.0)).join(" ").contains("fps=30,"));
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
