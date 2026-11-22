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
//! * The denoise runs on the graphics chip (VA-API `denoise_vaapi`) when there is one, otherwise on the
//!   processor with `hqdn3d`, the cheapest good filter (it also compares neighbouring frames).
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
const RETRY: Duration = Duration::from_secs(30);

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
fn ffmpeg_has_vaapi_denoise() -> bool {
    use std::sync::OnceLock;
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        Command::new("ffmpeg").args(["-hide_banner", "-filters"]).stdin(Stdio::null()).stderr(Stdio::null()).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("denoise_vaapi")).unwrap_or(false)
    })
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

/// The processor filter for a strength from 1 to 100: `hqdn3d` with its usual proportions, from barely
/// there to strong (50 is ffmpeg's own default).
pub fn cpu_filter(strength: u8) -> String {
    let k = f64::from(strength.clamp(1, 100)) / 50.0;
    format!("hqdn3d={:.2}:{:.2}:{:.2}:{:.2}", 4.0 * k, 3.0 * k, 6.0 * k, 4.5 * k)
}

/// The graphics-chip level for a strength from 1 to 100 (VA-API takes 0 to 64).
pub fn gpu_level(strength: u8) -> u32 { (u32::from(strength.clamp(1, 100)) * 64).div_ceil(100) }

fn base_args() -> Vec<String> {
    ["-hide_banner", "-loglevel", "error", "-nostdin"].iter().map(|s| s.to_string()).collect()
}

/// Real camera in, the filtered picture out to the Studio camera. No filter on: the picture as it is.
pub fn pipeline_args(camera: &str, loopback: &str, format: &Format, denoise: Option<u8>, gpu: Option<&str>) -> Vec<String> {
    // The graphics chip is only worth setting up when there is a filter for it to run.
    let gpu = gpu.filter(|_| denoise.is_some());
    let mut args = base_args();
    if let Some(node) = gpu {
        args.extend(["-init_hw_device".into(), format!("vaapi=va:{node}"), "-filter_hw_device".into(), "va".into()]);
    }
    args.extend([
        "-f".into(), "v4l2".into(), "-input_format".into(), format.input.clone(),
        "-video_size".into(), format!("{}x{}", format.width, format.height),
        "-framerate".into(), format!("{:.3}", format.fps), "-i".into(), camera.into(),
    ]);
    let filter = match (denoise, gpu) {
        (Some(strength), Some(_)) => format!("format=nv12,hwupload,denoise_vaapi=denoise={},hwdownload,format=nv12,format=yuv420p", gpu_level(strength)),
        (Some(strength), None) => format!("{},format=yuv420p", cpu_filter(strength)),
        (None, _) => "format=yuv420p".to_string(),
    };
    args.extend(["-vf".into(), filter, "-f".into(), "v4l2".into(), loopback.into()]);
    args
}

/// The stand-in while no app is watching: a dark frame at the camera's size, five times a second.
pub fn placeholder_args(loopback: &str, format: &Format) -> Vec<String> {
    let mut args = base_args();
    args.extend([
        "-re".into(), "-f".into(), "lavfi".into(),
        "-i".into(), format!("color=c=0x101418:s={}x{}:r=5", format.width, format.height),
        "-vf".into(), "format=yuv420p".into(), "-f".into(), "v4l2".into(), loopback.into(),
    ]);
    args
}

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
    gpu: bool,
    started: Instant,
}

#[derive(Default)]
pub struct Denoiser {
    running: Option<Running>,
    /// An app had the Studio camera open at the last look.
    watched: bool,
    idle_since: Option<Instant>,
    retry_after: Option<Instant>,
    gpu_failed: bool,
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
            self.stop();
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
                let (want, gpu, lived) = (running.want, running.gpu, running.started.elapsed());
                self.running = None;
                let said = last_lines(&log_path(), 3);
                if want == Want::Pipeline && gpu {
                    // The graphics chip did not manage it: use the processor from now on.
                    self.gpu_failed = true;
                    self.last_error = Some(format!("The graphics chip could not denoise ({said}); using the processor."));
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
        let want = decide(true, true, camera.is_some(), watched, pipeline_running, self.idle_since.map(|at| at.elapsed()));
        if want != Want::Pipeline { self.idle_since = None; }

        let waiting = self.retry_after.is_some_and(|at| Instant::now() < at);
        let gpu = if self.gpu_failed || !config.use_gpu || !ffmpeg_has_vaapi_denoise() { None } else { render_node() };
        let target = match (want, camera) {
            (Want::Nothing, _) | (_, None) => None,
            (want, Some(camera)) => self.format_for(camera).map(|format| {
                let args = if want == Want::Pipeline { pipeline_args(camera, &loopback, &format, config.denoise.then_some(config.strength), gpu.as_deref()) }
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
                            self.running = Some(Running { child, want, key, gpu: want == Want::Pipeline && gpu.is_some(), started: Instant::now() });
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
                report.gpu = running.gpu;
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
                        else if running.gpu { "Denoising on the graphics chip.".into() } else { "Denoising on the processor.".into() };
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
        assert_eq!(cpu_filter(50), "hqdn3d=4.00:3.00:6.00:4.50");
        assert_eq!(cpu_filter(0), cpu_filter(1));
        assert_eq!((gpu_level(1), gpu_level(50), gpu_level(100)), (1, 32, 64));
    }

    #[test]
    fn the_commands_read_the_camera_and_write_the_virtual_one() {
        let cpu = pipeline_args("/dev/video0", "/dev/video42", &fmt(), Some(50), None).join(" ");
        assert!(cpu.contains("-input_format mjpeg -video_size 1920x1080 -framerate 30.000 -i /dev/video0"), "{cpu}");
        assert!(cpu.contains("-vf hqdn3d=4.00:3.00:6.00:4.50,format=yuv420p -f v4l2 /dev/video42"), "{cpu}");
        let gpu = pipeline_args("/dev/video0", "/dev/video42", &fmt(), Some(50), Some("/dev/dri/renderD128")).join(" ");
        assert!(gpu.contains("-init_hw_device vaapi=va:/dev/dri/renderD128 -filter_hw_device va"), "{gpu}");
        assert!(gpu.contains("denoise_vaapi=denoise=32"), "{gpu}");
        let wait = placeholder_args("/dev/video42", &fmt()).join(" ");
        assert!(wait.contains("color=c=0x101418:s=1920x1080:r=5") && wait.ends_with("-f v4l2 /dev/video42"), "{wait}");
        assert!(!wait.contains("/dev/video0"), "the placeholder never touches the camera");
        // No filter on: the picture passes through, and no graphics chip is set up for nothing.
        let plain = pipeline_args("/dev/video0", "/dev/video42", &fmt(), None, Some("/dev/dri/renderD128")).join(" ");
        assert!(plain.contains("-vf format=yuv420p -f v4l2") && !plain.contains("vaapi") && !plain.contains("hqdn3d"), "{plain}");
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
