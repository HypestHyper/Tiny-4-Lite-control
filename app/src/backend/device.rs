//! Camera access: V4L2 controls through `v4l2-ctl`, the UVC extension unit
//! used for OBSBOT vendor commands, and the framed command protocol.
//!
//! Everything here is blocking and GTK-free. The UI never calls it directly;
//! it goes through `backend::worker`.
#![allow(dead_code)]

use super::state::{current_config, update_config};
use crate::video;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::OnceLock;

/// Restore the remembered host capture mode only when this camera still
/// advertises it. This does not replay camera movement or vendor settings.
pub(crate) fn apply_saved_video_mode(device: &str) -> Option<String> {
    let saved = current_config().video_mode?;
    let mode = video::VideoMode {
        width: saved.width,
        height: saved.height,
        pixel_format: saved.pixel_format,
        frame_rate_milli: saved.frame_rate_milli,
    };
    let capabilities = match video::capabilities(device) {
        Ok(value) => value,
        Err(error) => return Some(format!("Could not check the saved video mode: {error}")),
    };
    let supported = capabilities.iter().any(|capability|
        capability.pixel_format == mode.pixel_format
            && capability.width == mode.width
            && capability.height == mode.height
            && capability.frame_rates_milli.contains(&mode.frame_rate_milli)
    );
    if !supported {
        return Some(format!(
            "Saved capture mode {} × {} · {} · {:.3} fps is no longer advertised; keeping the camera's current mode.",
            mode.width, mode.height, mode.pixel_format, mode.frame_rate_milli as f64 / 1000.0
        ));
    }
    let current = video::current_mode(device);
    if current.as_ref() == Some(&mode) { return None; }
    match video::set_mode(device, &mode) {
        Ok(()) => Some(format!(
            "Restored capture mode {} × {} · {} · {:.3} fps.",
            mode.width, mode.height, mode.pixel_format, mode.frame_rate_milli as f64 / 1000.0
        )),
        Err(error) => {
            if let Some(previous) = current {
                let _ = video::set_mode(device, &previous);
            }
            Some(format!("Could not restore the saved capture mode: {error}"))
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Control {
    pub name: String, pub kind: String, pub min: i64, pub max: i64, pub step: i64,
    pub value: i64, pub default_value: i64, pub disabled: bool, pub menu: Vec<(i64, String)>,
}


/// A pretend gimbal for the synthetic camera: it follows absolute moves, and turns at a steady rate
/// while a speed is set, so the position readout and the 3D model can be tested without hardware.
mod sim {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    use crate::backend::angles::{PAN_RANGE, TILT_RANGE};
    /// Degrees per second for each unit of speed.
    const RATE: f64 = 0.4;

    pub struct Gimbal { pub pan: f64, pub tilt: f64, pan_speed: f64, tilt_speed: f64, at: Instant }

    fn gimbal() -> &'static Mutex<Gimbal> {
        static G: OnceLock<Mutex<Gimbal>> = OnceLock::new();
        G.get_or_init(|| {
            // TINY4LITE_UI_TEST_GIMBAL="pan,tilt" starts the pretend gimbal somewhere, for screenshots.
            let (pan, tilt) = std::env::var("TINY4LITE_UI_TEST_GIMBAL").ok()
                .and_then(|v| v.split_once(',').and_then(|(p, t)| Some((p.trim().parse::<f64>().ok()?, t.trim().parse::<f64>().ok()?))))
                .unwrap_or((0.0, 0.0));
            Mutex::new(Gimbal { pan: pan.clamp(PAN_RANGE.0, PAN_RANGE.1), tilt: tilt.clamp(TILT_RANGE.0, TILT_RANGE.1), pan_speed: 0.0, tilt_speed: 0.0, at: Instant::now() })
        })
    }

    impl Gimbal {
        fn advance(&mut self) {
            let seconds = self.at.elapsed().as_secs_f64();
            self.at = Instant::now();
            self.pan = (self.pan + self.pan_speed * RATE * seconds).clamp(PAN_RANGE.0, PAN_RANGE.1);
            self.tilt = (self.tilt + self.tilt_speed * RATE * seconds).clamp(TILT_RANGE.0, TILT_RANGE.1);
        }
    }

    /// Where the pretend gimbal is now, as (pan, tilt) in degrees.
    pub fn position() -> (f64, f64) {
        let mut g = gimbal().lock().unwrap_or_else(|e| e.into_inner());
        g.advance();
        (g.pan, g.tilt)
    }

    /// Apply a `--set-ctrl=name=value,...` option the way the real camera would.
    pub fn apply_set_ctrl(option: &str) {
        let Some(list) = option.strip_prefix("--set-ctrl=") else { return };
        let mut g = gimbal().lock().unwrap_or_else(|e| e.into_inner());
        g.advance();
        for pair in list.split(',') {
            let Some((name, value)) = pair.split_once('=') else { continue };
            let Ok(value) = value.parse::<f64>() else { continue };
            match name {
                "pan_absolute" => g.pan = (value / crate::backend::angles::RAW_PER_DEGREE).clamp(PAN_RANGE.0, PAN_RANGE.1),
                "tilt_absolute" => g.tilt = (value / crate::backend::angles::RAW_PER_DEGREE).clamp(TILT_RANGE.0, TILT_RANGE.1),
                "pan_speed" => g.pan_speed = value,
                "tilt_speed" => g.tilt_speed = value,
                _ => {}
            }
        }
    }
}

pub(crate) fn ui_test_mode() -> bool {
    std::env::var("TINY4LITE_UI_TEST").is_ok_and(|value| value == "1")
}

pub(crate) fn mock_v4l2_output(option: &str) -> String {
    let fixture_state = std::env::var("TINY4LITE_UI_TEST_STATE").unwrap_or_default();
    let manual = fixture_state == "manual";
    let aperture_priority = fixture_state == "aperture";
    let exposure_mode = if manual { 1 } else if aperture_priority { 3 } else { 0 };
    let awb = if manual { 0 } else { 1 };
    let autofocus = if manual { 0 } else { 1 };
    let dependent_flags = |inactive: bool| if inactive { " flags=inactive" } else { "" };
    match option {
        "--list-ctrls-menus" => format!(r#"brightness (int) : min=0 max=100 step=1 default=50 value=50
contrast (int) : min=0 max=100 step=1 default=50 value=54
saturation (int) : min=0 max=100 step=1 default=50 value=50
hue (int) : min=0 max=100 step=1 default=50 value=50
white_balance_automatic (bool) : default=1 value={awb}
red_balance (int) : min=0 max=2048 step=1 default=1024 value=1043{}
blue_balance (int) : min=0 max=2048 step=1 default=1024 value=1312{}
gain (int) : min=1 max=64 step=1 default=1 value=1
power_line_frequency (menu) : min=0 max=2 step=1 default=3 value=2
	0: Disabled
	1: 50 Hz
	2: 60 Hz
white_balance_temperature (int) : min=2000 max=10000 step=100 default=5000 value=3000{}
sharpness (int) : min=0 max=100 step=1 default=50 value=50
backlight_compensation (int) : min=0 max=18 step=1 default=9 value=18
auto_exposure (menu) : min=0 max=3 step=1 default=0 value={exposure_mode}
	0: Auto
	1: Manual
	3: Aperture Priority
exposure_time_absolute (int) : min=1 max=2500 step=1 default=330 value=330{}
pan_absolute (int) : min=-468000 max=468000 step=3600 default=0 value=-50400
tilt_absolute (int) : min=-324000 max=324000 step=3600 default=0 value=-50400
focus_absolute (int) : min=0 max=100 step=1 default=0 value=100{}
focus_automatic_continuous (bool) : default=1 value={autofocus}
zoom_absolute (int) : min=0 max=100 step=1 default=0 value=15
zoom_continuous (int) : min=-100 max=100 step=1 default=100 value=245
pan_speed (int) : min=-80 max=80 step=1 default=10 value=0
tilt_speed (int) : min=-120 max=120 step=1 default=20 value=0
"#,
            dependent_flags(awb != 0),
            dependent_flags(awb != 0),
            dependent_flags(awb != 0),
            dependent_flags(exposure_mode != 1),
            dependent_flags(autofocus != 0),
        ),
        "--list-formats-ext" => String::from(
            "ioctl: VIDIOC_ENUM_FMT\n Type: Video Capture\n [0]: 'MJPG' (Motion-JPEG, compressed)\n   Size: Discrete 3840x2160\n     Interval: Discrete 0.033s (30.000 fps)\n   Size: Discrete 1920x1080\n     Interval: Discrete 0.033s (30.000 fps)\n     Interval: Discrete 0.017s (60.000 fps)\n   Size: Discrete 1280x720\n     Interval: Discrete 0.033s (30.000 fps)\n [1]: 'YUYV' (YUYV 4:2:2)\n   Size: Discrete 640x480\n     Interval: Discrete 0.033s (30.000 fps)\n"
        ),
        "--get-fmt-video" => String::from("Format Video Capture:\n\tWidth/Height      : 1920/1080\n\tPixel Format      : 'MJPG'\n"),
        "--get-parm" => String::from("Streaming Parameters Video Capture:\n\tFrames per second: 30.000 (30/1)\n"),
        value if value.starts_with("--get-ctrl=") => {
            let name = value.trim_start_matches("--get-ctrl=");
            let current = match name {
                "pan_absolute" => (sim::position().0 * 3600.0).round() as i64,
                "tilt_absolute" => (sim::position().1 * 3600.0).round() as i64,
                "pan_speed" | "tilt_speed" => 0,
                "zoom_absolute" => 15,
                "brightness" | "contrast" | "saturation" | "hue" | "sharpness" => 50,
                "gain" => 1,
                _ => 0,
            };
            format!("{name}: {current}")
        }
        value if value.starts_with("--set-ctrl=") || value.starts_with("--set-fmt-video=") || value.starts_with("--set-parm=") => String::new(),
        _ => String::new(),
    }
}

pub(crate) fn run_v4l2(device: &str, option: &str) -> Result<String, String> {
    if ui_test_mode() {
        // The screenshot harness can record what the UI sent to the synthetic camera.
        if let Ok(path) = std::env::var("TINY4LITE_UI_TEST_LOG") {
            if option.starts_with("--set-") {
                use std::io::Write;
                let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(file, "{millis} {option}");
                }
            }
        }
        sim::apply_set_ctrl(option);
        return Ok(mock_v4l2_output(option));
    }
    let (success, stdout, stderr) = run_with_timeout(
        Command::new("v4l2-ctl").arg("-d").arg(device).arg(option), v4l2_timeout(option))?;
    if !success {
        // v4l2-ctl prints some failures (a busy device) on standard output, and some say nothing at all.
        let said = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
        return Err(if said.is_empty() { format!("v4l2-ctl {option} failed without saying why") } else { said.to_string() });
    }
    Ok(stdout.trim().to_string())
}

/// How long a `v4l2-ctl` call may take. Changing the capture mode can make the camera renegotiate
/// with the USB host, which takes a while; everything else is quick.
pub(crate) fn v4l2_timeout(option: &str) -> std::time::Duration {
    let slow = option.starts_with("--set-fmt-video") || option.starts_with("--set-parm");
    std::time::Duration::from_secs(if slow { 15 } else { 6 })
}

/// Run a command and give up on it after `limit`: a camera that stops answering must not leave the
/// caller (and everything queued behind it) waiting for ever. Returns (succeeded, stdout, stderr).
pub(crate) fn run_with_timeout(command: &mut Command, limit: std::time::Duration) -> Result<(bool, String, String), String> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|e| format!("Could not run v4l2-ctl: {e}"))?;
    // Read both pipes on their own threads so a full pipe can never stop the command finishing.
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let out_thread = std::thread::spawn(move || { let mut s = String::new(); if let Some(o) = out.as_mut() { let _ = o.read_to_string(&mut s); } s });
    let err_thread = std::thread::spawn(move || { let mut s = String::new(); if let Some(o) = err.as_mut() { let _ = o.read_to_string(&mut s); } s });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("The camera did not answer within {} seconds, so the command was stopped. If the picture is frozen, unplug and replug the camera.", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(15)),
            Err(e) => break Err(format!("Could not wait for v4l2-ctl: {e}")),
        }
    };
    let (stdout, stderr) = (out_thread.join().unwrap_or_default(), err_thread.join().unwrap_or_default());
    status.map(|s| (s.success(), stdout, stderr))
}

pub(crate) fn get_control(device: &str, name: &str) -> Result<i64, String> {
    let output = run_v4l2(device, &format!("--get-ctrl={name}"))?;
    output.split(':').last().unwrap_or("").trim().parse::<i64>()
        .map_err(|_| format!("Could not parse {name} value: {output}"))
}

/// Make the camera use one way of manual white balance. It uses whichever of colour temperature and red/blue
/// balance was written last (measured: touching blue balance switched the picture to raw gains, touching the
/// temperature switched it back), so the chosen mode's current values are written again. Each is first
/// moved one step and back, in case the driver skips a write of the value it already has.
pub(crate) fn set_white_balance_by_gains(device: &str, gains: bool) -> Result<(), String> {
    let rewrite = |name: &str, step: i64, low: i64, high: i64| -> Result<(), String> {
        let value = get_control(device, name)?;
        let nudged = if value + step <= high { value + step } else { (value - step).max(low) };
        set_control(device, name, nudged)?;
        set_control(device, name, value)
    };
    if gains {
        rewrite("red_balance", 1, 0, 2048)?;
        rewrite("blue_balance", 1, 0, 2048)?;
    } else {
        rewrite("white_balance_temperature", 100, 2000, 10000)?;
    }
    crate::backend::update_config(|c| c.white_balance_by_gains = gains);
    Ok(())
}

pub(crate) fn set_control(device: &str, name: &str, value: i64) -> Result<(), String> {
    run_v4l2(device, &format!("--set-ctrl={name}={value}"))?;
    // Position and speed are transient movement commands, never preferences.
    if !matches!(name, "pan_absolute" | "tilt_absolute" | "pan_speed" | "tilt_speed") {
        update_config(|config| { config.v4l2_controls.insert(name.to_string(), value); });
    }
    Ok(())
}

pub(crate) fn usb_camera_present() -> bool {
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else { return false; };
    entries.flatten().any(|entry| {
        let path = entry.path();
        fs::read_to_string(path.join("idVendor")).unwrap_or_default().trim().eq_ignore_ascii_case("3564")
            && fs::read_to_string(path.join("idProduct")).unwrap_or_default().trim().eq_ignore_ascii_case("fef9")
    })
}

pub(crate) fn discover_tiny4lite() -> Vec<String> {
    if ui_test_mode() { return vec!["/dev/tiny4lite-headless-fixture".to_string()]; }
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/video4linux") else { return found; };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("video") { continue; }
        let Ok(mut path) = fs::canonicalize(entry.path().join("device")) else { continue; };
        let mut matches = false;
        loop {
            let vendor = fs::read_to_string(path.join("idVendor")).unwrap_or_default();
            let product = fs::read_to_string(path.join("idProduct")).unwrap_or_default();
            if vendor.trim().eq_ignore_ascii_case("3564") && product.trim().eq_ignore_ascii_case("fef9") {
                matches = true;
                break;
            }
            if !path.pop() { break; }
        }
        let node = format!("/dev/{name}");
        if matches && Path::new(&node).exists() { found.push(node); }
    }
    found.sort();
    found
}

pub(crate) fn parse_controls(text: &str) -> Vec<Control> {
    let mut controls: Vec<Control> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((index, label)) = trimmed.split_once(": ") {
            if let Ok(index) = index.parse::<i64>() {
                if let Some(control) = controls.last_mut() {
                    if control.kind.contains("menu") { control.menu.push((index, label.trim().to_string())); }
                }
                continue;
            }
        }
        let Some((left, right)) = trimmed.split_once(" : ") else { continue; };
        let Some((name_part, kind_part)) = left.split_once(" (") else { continue; };
        let name = name_part.split_whitespace().next().unwrap_or("").to_string();
        let kind = kind_part.trim_end_matches(')').to_string();
        if name.is_empty() { continue; }
        let mut fields = HashMap::new();
        for token in right.split_whitespace() {
            if let Some((key, value)) = token.split_once('=') {
                fields.insert(key.trim().to_string(), value.trim_matches(',').to_string());
            }
        }
        let number = |key: &str, fallback: i64| fields.get(key).and_then(|v| v.parse().ok()).unwrap_or(fallback);
        let flags = fields.get("flags").cloned().unwrap_or_default();
        controls.push(Control {
            name, kind, min: number("min", 0), max: number("max", 1),
            step: number("step", 1).max(1), value: number("value", number("default", 0)),
            default_value: number("default", number("value", 0)),
            disabled: flags.contains("inactive") || flags.contains("disabled") || flags.contains("read-only"),
            menu: Vec::new(),
        });
    }
    for control in &mut controls {
        if control.kind.contains("menu") {
            let valid = |value: i64| control.menu.iter().any(|(entry, _)| *entry == value);
            if !valid(control.default_value) {
                control.default_value = if valid(control.value) {
                    control.value
                } else {
                    control.menu.first().map(|(value, _)| *value).unwrap_or(control.min)
                };
            }
            if !valid(control.value) {
                control.value = control.default_value;
            }
        } else {
            let upper = control.max.max(control.min);
            control.default_value = control.default_value.clamp(control.min, upper);
            if control.value < control.min || control.value > upper {
                // Preserve the driver's raw value for diagnosis. The UI will
                // clamp the thumb, disable writes, and explain the bad report.
                control.disabled = true;
            }
        }
    }
    controls
}

// OBSBOT vendor UVC Extension Unit transport. This is deliberately separate
// from V4L2 controls: Tiny 2 Lite exposes its AI features through XU unit 2.
#[repr(C)]
pub(crate) struct UvcXuControlQuery {
    unit: u8,
    selector: u8,
    query: u8,
    _pad: u8,
    size: u16,
    _pad2: u16,
    data: *mut u8,
}

pub(crate) const UVCIOC_CTRL_QUERY: std::os::raw::c_ulong = 0xc010_7521;
pub(crate) const XU_UNIT: u8 = 2;
pub(crate) const UVC_SET_CUR: u8 = 0x01;
pub(crate) const UVC_GET_CUR: u8 = 0x81;

extern "C" {
    fn ioctl(fd: std::os::raw::c_int, request: std::os::raw::c_ulong, ...) -> std::os::raw::c_int;
}

pub(crate) fn xu_query(device: &str, selector: u8, request: u8, data: &mut [u8]) -> Result<(), String> {
    if ui_test_mode() {
        if selector == 0x06 && request == UVC_SET_CUR && data.len() >= 60 && data[0] == 0x16 && data[1] == 0x02 {
            LAST_UI_TEST_AI_MODE.store(u16::from_le_bytes([data[2], data[3]]), Ordering::Relaxed);
        } else if selector == 0x06 && request == UVC_SET_CUR && data.len() >= 60 && data[0] == 0x03 && data[1] == 0x01 {
            LAST_UI_TEST_AUTO_EXPOSURE_PRIORITY.store(data[2], Ordering::Relaxed);
        } else if selector == 0x06 && request == UVC_SET_CUR && data.len() >= 60 && data[0] == 0x14 && data[1] == 0x01 {
            ui_test_mirror().store(data[2] == 1, Ordering::Relaxed);
        } else if selector == 0x06 && request == UVC_SET_CUR && data.len() >= 60 && data[0] == 0x1c && data[1] == 0x01 {
            // The real camera restarts and comes back with the change; the pretend one just takes it.
            ui_test_microphone().store(data[2] == 1, Ordering::Relaxed);
            if let Ok(path) = std::env::var("TINY4LITE_UI_TEST_LOG") {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(file, "microphone={}", if data[2] == 1 { "on" } else { "off" });
                }
            }
        } else if selector == 0x02 && request == UVC_SET_CUR && data.len() >= 20
            && data[..2] == [0xaa, 0x25]
            && data[8..12] == [0x0a, 0x02, 0x02, 0x36]
        {
            LAST_UI_TEST_FOCUS_VALUE.store(
                u32::from_le_bytes(data[16..20].try_into().unwrap()),
                Ordering::Relaxed,
            );
        } else if selector == 0x02 && request == UVC_SET_CUR && data.len() >= 12
            && data[0] == 0xaa && data[1] == 0x01
        {
            LAST_UI_TEST_QUERY_SEQUENCE.store(u16::from_le_bytes([data[2], data[3]]), Ordering::Relaxed);
            LAST_UI_TEST_QUERY_COMMAND.store(u16::from_le_bytes([data[10], data[11]]), Ordering::Relaxed);
        } else if request == UVC_GET_CUR {
            data.fill(0);
            if selector == 0x02 && data.len() >= 60 {
                let command = LAST_UI_TEST_QUERY_COMMAND.load(Ordering::Relaxed);
                let sequence = LAST_UI_TEST_QUERY_SEQUENCE.load(Ordering::Relaxed);
                let payload: Vec<u8> = match command {
                    0x0043 => {
                        // Reply layout: pitch then yaw as signed hundredths of a degree at 8 and 10.
                        // The app reads tilt as the negative of pitch.
                        let (pan, tilt) = sim::position();
                        let mut reply = vec![0u8; 12];
                        reply[8..10].copy_from_slice(&((-tilt * 100.0).round() as i16).to_le_bytes());
                        reply[10..12].copy_from_slice(&((pan * 100.0).round() as i16).to_le_bytes());
                        reply
                    }
                    // The live zoom, matching the pretend zoom control (15, which is 1.47×).
                    0x6804 => 1.47f32.to_le_bytes().to_vec(),
                    // The tracking area's return position: pan 15°, tilt 10° up, zoom 1.2×.
                    0x4ec4 => zone_home_payload(15.0, 10.0, 1.2).to_vec(),
                    0x80c2 => 50i16.to_le_bytes().to_vec(),
                    // The AI status: right hand, zoom gesture on, gesture zoom 2.5×, Sport.
                    0x0104 => vec![0, 0, 0, 0, 1, 0, 0, 0x19, 0, 2, 0, 0],
                    0xa182 => 600u32.to_le_bytes().to_vec(),
                    0x3884 => {
                        let mut v = 3u32.to_le_bytes().to_vec();
                        for f in [0.0f32, 0.0, 0.0, 1.0, 0.0] { v.extend_from_slice(&f.to_le_bytes()); }
                        v
                    }
                    0x3e84 => {
                        let mut v = vec![0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
                        v.extend_from_slice(&[0x80, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
                        v.extend_from_slice(&[0; 4]);
                        v.extend_from_slice(&[0xff; 4]);
                        v.extend_from_slice(&[0; 16]);
                        v
                    }
                    _ => Vec::new(),
                };
                if command == 0x35c2 {
                    data[0..2].copy_from_slice(&[0xaa, 0x29]);
                    data[2..4].copy_from_slice(&sequence.to_le_bytes());
                    data[4..6].copy_from_slice(&12u16.to_le_bytes());
                    data[8..12].copy_from_slice(&[0x02, 0x0a, 0xc2, 0x35]);
                    data[12..14].copy_from_slice(&4u16.to_le_bytes());
                    // Laid out like a command frame: the payload CRC, then the number (1 Face, 0 Global).
                    let value = LAST_UI_TEST_FOCUS_VALUE.load(Ordering::Relaxed).to_le_bytes();
                    data[16..20].copy_from_slice(&value);
                    let payload_crc = crc16_usb(&data[12..20]);
                    data[14..16].copy_from_slice(&payload_crc.to_le_bytes());
                    let crc = crc16_usb(&data[..12]);
                    data[6..8].copy_from_slice(&crc.to_le_bytes());
                } else if !payload.is_empty() {
                    data[0] = 0xaa; data[1] = 0x29;
                    data[2..4].copy_from_slice(&sequence.to_le_bytes());
                    data[4..6].copy_from_slice(&12u16.to_le_bytes());
                    data[8..12].copy_from_slice(&[0x0a, 0x04, command as u8, (command >> 8) as u8]);
                    data[12..14].copy_from_slice(&(payload.len() as u16).to_le_bytes());
                    data[16..16 + payload.len()].copy_from_slice(&payload);
                }
            } else if selector == 0x06 && data.len() >= 0x25 {
                let mode = LAST_UI_TEST_AI_MODE.load(Ordering::Relaxed);
                data[0x18] = mode as u8;
                data[0x1c] = (mode >> 8) as u8;
                data[XU_AUTO_EXPOSURE_PRIORITY_OFFSET] = LAST_UI_TEST_AUTO_EXPOSURE_PRIORITY.load(Ordering::Relaxed);
                data[0x10] = 1;
                data[0x13] = u8::from(ui_test_mirror().load(Ordering::Relaxed));
                data[34] = 0x02 | if ui_test_microphone().load(Ordering::Relaxed) { 0x10 } else { 0 };
                data[0x21] = 1;
            } else if selector == 0x0d && !data.is_empty() {
                data[0] = 0x02;
            } else if selector == 0x08 {
                let model = b"OBSBOT Tiny 2 Lite";
                let count = model.len().min(data.len());
                data[..count].copy_from_slice(&model[..count]);
            }
        }
        return Ok(());
    }
    use std::os::fd::AsRawFd;
    let file = std::fs::OpenOptions::new().read(true).write(true).open(device)
        .map_err(|e| format!("Cannot open {device} for OBSBOT controls: {e}"))?;
    let mut query = UvcXuControlQuery {
        unit: XU_UNIT, selector, query: request, _pad: 0,
        size: data.len() as u16, _pad2: 0, data: data.as_mut_ptr(),
    };
    let result = unsafe { ioctl(file.as_raw_fd(), UVCIOC_CTRL_QUERY, &mut query) };
    if result < 0 {
        return Err(format!("OBSBOT XU selector 0x{selector:02x} request 0x{request:02x}: {}",
            std::io::Error::last_os_error()));
    }
    Ok(())
}

pub(crate) const UVC_CAMERA_TERMINAL_UNIT: u8 = 1;
pub(crate) const UVC_CT_PANTILT_ABSOLUTE: u8 = 0x0d;
// Existing V4L2 PTZ controls use 3600 raw units for a one-degree step.

/// The largest single step the app will send, in degrees. Movement tests start small.
pub(crate) const MAX_STEP_DEGREES: i64 = 5;

/// Turn the gimbal by a small, bounded angle using the standard absolute pan/tilt control.
/// Returns the new target in degrees. The target stays inside the range the driver reports.
pub(crate) fn step_gimbal(device: &str, control: &str, degrees: i64, min: i64, max: i64, from: Option<f64>) -> Result<f64, String> {
    if !matches!(control, "pan_absolute" | "tilt_absolute") {
        return Err(format!("{control} is not a pan or tilt control."));
    }
    if degrees == 0 || degrees.abs() > MAX_STEP_DEGREES {
        return Err(format!("A step is between 1 and {MAX_STEP_DEGREES} degrees."));
    }
    // Step from where the camera really is: the position the caller shows (or the step it just sent), else a
    // fresh read. Never from the value the driver remembers: the joystick and the tracking do not update it,
    // so it is usually the centre. The live degrees and the control agree in sign.
    let current_degrees = match from {
        Some(degrees) => degrees.round(),
        None => get_gimbal_position(device)
            .map(|(pan, tilt)| if control == "pan_absolute" { pan } else { tilt })
            .map_err(|_| "Could not read where the camera is pointing. Try again in a moment.".to_string())?
            .round(),
    };
    // Never past the range the camera is driven over, nor past what the driver itself allows.
    let (low, high) = if control == "pan_absolute" { super::angles::pan_range() } else { super::angles::tilt_range() };
    let (low_raw, high_raw) = (min.max(super::angles::degrees_to_raw(low)), max.min(super::angles::degrees_to_raw(high)));
    let wanted = super::angles::degrees_to_raw(current_degrees + degrees as f64);
    let target = wanted.clamp(low_raw, high_raw);
    if target == super::angles::degrees_to_raw(current_degrees) {
        return Err(format!("Already at the end of the range ({}).", super::angles::format(current_degrees)));
    }
    set_control(device, control, target)?;
    Ok(super::angles::raw_to_degrees(target))
}
/// Turn the camera to pan 0, tilt 0 (the middle of the V4L2 pan and tilt ranges). One command, so
/// both axes start together. The camera physically turns, so callers ask the user first.
pub(crate) fn centre_gimbal(device: &str) -> Result<(), String> {
    run_v4l2(device, "--set-ctrl=pan_absolute=0,tilt_absolute=0").map(|_| ())
}
pub(crate) use super::angles::{PAN_LIMIT as PAN_DISPLAY_LIMIT_DEGREES, TILT_LIMIT as TILT_DISPLAY_LIMIT_DEGREES};

pub(crate) fn camera_terminal_query(device: &str, selector: u8, request: u8, data: &mut [u8]) -> Result<(), String> {
    if ui_test_mode() {
        if request == UVC_GET_CUR { data.fill(0); }
        return Ok(());
    }
    use std::os::fd::AsRawFd;
    let file = std::fs::OpenOptions::new().read(true).write(true).open(device)
        .map_err(|e| format!("Cannot open {device} for UVC Camera Terminal read-back: {e}"))?;
    let mut query = UvcXuControlQuery {
        unit: UVC_CAMERA_TERMINAL_UNIT, selector, query: request, _pad: 0,
        size: data.len() as u16, _pad2: 0, data: data.as_mut_ptr(),
    };
    let result = unsafe { ioctl(file.as_raw_fd(), UVCIOC_CTRL_QUERY, &mut query) };
    if result < 0 {
        return Err(format!("UVC Camera Terminal 1 selector 0x{selector:02x} request 0x{request:02x}: {}",
            std::io::Error::last_os_error()));
    }
    Ok(())
}

pub(crate) fn decode_pan_tilt_absolute(data: &[u8; 8]) -> (f64, f64) {
    let pan_raw = i32::from_le_bytes(data[0..4].try_into().expect("four-byte pan field"));
    let tilt_raw = i32::from_le_bytes(data[4..8].try_into().expect("four-byte tilt field"));
    (
        super::angles::raw_to_degrees(i64::from(pan_raw)).clamp(-PAN_DISPLAY_LIMIT_DEGREES, PAN_DISPLAY_LIMIT_DEGREES),
        super::angles::raw_to_degrees(i64::from(tilt_raw)).clamp(-TILT_DISPLAY_LIMIT_DEGREES, TILT_DISPLAY_LIMIT_DEGREES),
    )
}

/// Read the Camera Terminal Pan/Tilt Absolute control directly.
/// Windows DirectShow testing showed that this control follows hand-moved
/// physical position. Linux still needs on-device confirmation.
pub(crate) fn obsbot_query_frame(receiver: u8, command: u16, sequence: u16) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = 0xaa;
    frame[1] = 0x01;
    frame[2..4].copy_from_slice(&sequence.to_le_bytes());
    frame[4..6].copy_from_slice(&12u16.to_le_bytes());
    frame[8..12].copy_from_slice(&[0x0a, receiver, command as u8, (command >> 8) as u8]);
    let crc = crc16_usb(&frame[..12]);
    frame[6..8].copy_from_slice(&crc.to_le_bytes());
    frame
}

pub(crate) fn query_obsbot(device: &str, receiver: u8, command: u16) -> Result<Vec<u8>, String> {
    let sequence = next_xu_sequence(xu_sequence_counter());
    let mut request = obsbot_query_frame(receiver, command, sequence);
    xu_query(device, 0x02, UVC_SET_CUR, &mut request)?;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut response = [0u8; 60];
        if xu_query(device, 0x02, UVC_GET_CUR, &mut response).is_err() { continue; }
        if response[0] != 0xaa || u16::from_le_bytes([response[2], response[3]]) != sequence {
            continue;
        }
        match response[1] {
            0x29 => {
                let len = usize::from(u16::from_le_bytes([response[12], response[13]]));
                if len > 44 { return Err(format!("OBSBOT reply 0x{command:04x} declares invalid payload length {len}.")); }
                return Ok(response[16..16 + len].to_vec());
            }
            0x89 => return Err(format!("OBSBOT rejected query 0x{command:04x}.")),
            0x09 if u16::from_le_bytes([response[10], response[11]]) == command => return Ok(Vec::new()),
            _ => {}
        }
    }
    Err(format!("No matching OBSBOT reply for query 0x{command:04x} (sequence 0x{sequence:04x})."))
}

pub(crate) fn decode_live_gimbal_position(data: &[u8]) -> Result<(f64, f64), String> {
    if data.len() < 12 { return Err(format!("Live gimbal reply is {} bytes; expected at least 12.", data.len())); }
    Ok(super::angles::from_live(i16::from_le_bytes([data[8], data[9]]), i16::from_le_bytes([data[10], data[11]])))
}

pub(crate) fn get_gimbal_position(device: &str) -> Result<(f64, f64), String> {
    let reply = query_obsbot(device, 0x03, 0x0043)?;
    decode_live_gimbal_position(&reply)
}

// ── Tracking area (OBSBOT's limited-zone tracking), from the SDK captures of 2026-11-24 ──
// The camera keeps AI tracking between a yaw (pan) and a pitch (tilt) range. Its yaw is our pan; its pitch is
// "down positive", the opposite of our tilt, so tilt limits are negated and swapped. Ranges: yaw ±120°,
// pitch ±90° (the SDK's documented valid ranges).

const ZONE_YAW_MIN_SET: u16 = 0x4c04;
const ZONE_YAW_MAX_SET: u16 = 0x4c84;
const ZONE_PITCH_MIN_SET: u16 = 0x4d04;
const ZONE_PITCH_MAX_SET: u16 = 0x4d84;
const ZONE_YAW_MIN_GET: u16 = 0x4c44;
const ZONE_YAW_MAX_GET: u16 = 0x4cc4;
const ZONE_PITCH_MIN_GET: u16 = 0x4d44;
const ZONE_PITCH_MAX_GET: u16 = 0x4dc4;
const ZONE_ENABLED_SET: u16 = 0x5084;
/// Zone tracking itself (aiSetZoneTrackStateR, "enable the zone tracking function"): OBSBOT Center's Zone tracking
/// switch. The limited-zone switch above alone did not keep tracking inside the area on the user's camera.
/// Captured from the SDK: write 04 0x08c4, u8.
const ZONE_TRACK_STATE_SET: u16 = 0x08c4;
const ZONE_AUTO_SELECT_SET: u16 = 0x4e04;
const ZONE_AUTO_SELECT_GET: u16 = 0x4e44;
// The hand-tracking area (aiSetHandZone{Yaw,Pitch}{Min,Max}R, undeclared in the SDK's headers): the same
// f32 degrees as the person-tracking area, set with 0x2044 / 0x20c4 / 0x2144 / 0x21c4 (capture of 2026-11-24).
// The camera does not answer reads of it, so it is sent, never shown as read back.
const HAND_YAW_MIN_SET: u16 = 0x2044;
const HAND_YAW_MAX_SET: u16 = 0x20c4;
const HAND_PITCH_MIN_SET: u16 = 0x2144;
const HAND_PITCH_MAX_SET: u16 = 0x21c4;
pub(crate) const ZONE_YAW_LIMIT: f64 = 120.0;
pub(crate) const ZONE_PITCH_LIMIT: f64 = 90.0;

/// Our pan and tilt limits as the camera's (yaw min, yaw max, pitch min, pitch max), clamped to its ranges.
pub(crate) fn tracking_area_to_camera(pan: (f64, f64), tilt: (f64, f64)) -> [f32; 4] {
    let yaw = |d: f64| d.clamp(-ZONE_YAW_LIMIT, ZONE_YAW_LIMIT) as f32;
    let pitch = |d: f64| d.clamp(-ZONE_PITCH_LIMIT, ZONE_PITCH_LIMIT) as f32;
    [yaw(pan.0), yaw(pan.1), pitch(-tilt.1), pitch(-tilt.0)]
}

/// The camera's (yaw min, yaw max, pitch min, pitch max) as our (pan low, pan high), (tilt low, tilt high).
pub(crate) fn tracking_area_from_camera(zone: [f32; 4]) -> ((f64, f64), (f64, f64)) {
    ((f64::from(zone[0]), f64::from(zone[1])), (-f64::from(zone[3]), -f64::from(zone[2])))
}

/// Send the tracking area's limits (our convention) to the camera.
pub(crate) fn set_tracking_area_limits(device: &str, pan: (f64, f64), tilt: (f64, f64)) -> Result<(), String> {
    let zone = tracking_area_to_camera(pan, tilt);
    for (command, value) in [(ZONE_YAW_MIN_SET, zone[0]), (ZONE_YAW_MAX_SET, zone[1]), (ZONE_PITCH_MIN_SET, zone[2]), (ZONE_PITCH_MAX_SET, zone[3])] {
        send_camera_preset_command(device, command, &value.to_le_bytes())?;
    }
    Ok(())
}

/// Turn the tracking area on or off. The camera does not answer a read of this on firmware 6.2.8.1.
pub(crate) fn set_tracking_area_enabled(device: &str, enabled: bool) -> Result<(), String> {
    send_camera_preset_command(device, ZONE_ENABLED_SET, &[u8::from(enabled)])?;
    send_camera_preset_command(device, ZONE_TRACK_STATE_SET, &[u8::from(enabled)])
}

pub(crate) fn set_tracking_area_auto_select(device: &str, enabled: bool) -> Result<(), String> {
    send_camera_preset_command(device, ZONE_AUTO_SELECT_SET, &[u8::from(enabled)])
}

pub(crate) fn get_tracking_area_auto_select(device: &str) -> Result<bool, String> {
    let reply = query_obsbot(device, 0x04, ZONE_AUTO_SELECT_GET)?;
    reply.first().map(|b| *b != 0).ok_or_else(|| "The camera sent no auto-select value.".to_string())
}

/// The tracking area the camera holds now, in our convention: (pan low, pan high), (tilt low, tilt high).
pub(crate) fn get_tracking_area(device: &str) -> Result<((f64, f64), (f64, f64)), String> {
    let mut zone = [0f32; 4];
    for (slot, command) in [ZONE_YAW_MIN_GET, ZONE_YAW_MAX_GET, ZONE_PITCH_MIN_GET, ZONE_PITCH_MAX_GET].into_iter().enumerate() {
        let reply = query_obsbot(device, 0x04, command)?;
        let bytes: [u8; 4] = reply.get(..4).ok_or("The camera sent a short tracking-area value.")?.try_into().unwrap();
        zone[slot] = f32::from_le_bytes(bytes);
    }
    Ok(tracking_area_from_camera(zone))
}

/// Send what the settings say: the area's limits (the gimbal limits, or the camera's full range for an axis
/// whose limit is off) and switch it on, or switch it off.
/// Done in the order OBSBOT's SDK documents for setting zone tracking: the special LED pattern on
/// (cameraSetLedCtrlU), the gimbal's auto-tracking off (aiSetZoneTrackGimbalEnabledR), the area set, then
/// auto-tracking and the LED back, and zone tracking switched on (limited zone and zone tracking state).
pub(crate) fn apply_tracking_area(device: &str, config: &crate::config::AppConfig) -> Result<(), String> {
    let full = ((-ZONE_YAW_LIMIT, ZONE_YAW_LIMIT), (-ZONE_PITCH_LIMIT, ZONE_PITCH_LIMIT));
    if !config.tracking_area {
        set_tracking_area_enabled(device, false)?;
        // Hand tracking has no on/off for its area, so off is the whole range.
        return set_hand_area_limits(device, full.0, full.1);
    }
    let (pan, tilt) = ((config.zone_pan_low, config.zone_pan_high), (config.zone_tilt_low, config.zone_tilt_high));
    set_led_pattern(device, true)?;
    let set = set_zone_track_gimbal(device, false)
        .and_then(|()| set_tracking_area_limits(device, pan, tilt))
        .and_then(|()| set_hand_area_limits(device, pan, tilt));
    // Whatever happened, tracking and the light go back to normal.
    let back = set_zone_track_gimbal(device, true).and(set_led_pattern(device, false));
    set.and(back)?;
    set_tracking_area_enabled(device, true)
}

/// The LED pattern OBSBOT shows while zone or hand tracking is being set (cameraSetLedCtrlU): status block
/// `18 01 xx`, as captured from the SDK.
pub(crate) fn set_led_pattern(device: &str, on: bool) -> Result<(), String> {
    let mut data = [0u8; 60];
    data[..3].copy_from_slice(&[0x18, 0x01, u8::from(on)]);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)
}

/// The gimbal's auto-tracking while the zone is set (aiSetZoneTrackGimbalEnabledR): 04 0x0844, u8, as captured.
const ZONE_TRACK_GIMBAL_SET: u16 = 0x0844;
pub(crate) fn set_zone_track_gimbal(device: &str, on: bool) -> Result<(), String> {
    send_camera_preset_command(device, ZONE_TRACK_GIMBAL_SET, &[u8::from(on)])
}

/// The hand-tracking area, from our pan and tilt limits (converted like the person-tracking area).
pub(crate) fn set_hand_area_limits(device: &str, pan: (f64, f64), tilt: (f64, f64)) -> Result<(), String> {
    let zone = tracking_area_to_camera(pan, tilt);
    for (command, value) in [(HAND_YAW_MIN_SET, zone[0]), (HAND_YAW_MAX_SET, zone[1]), (HAND_PITCH_MIN_SET, zone[2]), (HAND_PITCH_MAX_SET, zone[3])] {
        send_camera_preset_command(device, command, &value.to_le_bytes())?;
    }
    Ok(())
}

// The return position ("after exceeding the tracking range, the gimbal returns to the set position" in OBSBOT
// Center): 24 bytes, i32 id 0, then f32 yaw, pitch, 0, zoom, 0. Set and read as captured from the SDK on
// 2026-11-24 (cmd 0x4e84 / 0x4ec4); the field order is the one the SDK decoded from the start-up position
// reply (cmd 0x3884), which uses the same layout.
const ZONE_HOME_SET: u16 = 0x4e84;
const ZONE_HOME_GET: u16 = 0x4ec4;

pub(crate) fn zone_home_payload(pan: f64, tilt: f64, zoom: f64) -> [u8; 24] {
    let mut payload = [0u8; 24];
    payload[4..8].copy_from_slice(&(pan.clamp(-ZONE_YAW_LIMIT, ZONE_YAW_LIMIT) as f32).to_le_bytes());
    // 0.0 - tilt, not -tilt: level must be +0.0, as the camera sends it, not -0.0.
    payload[8..12].copy_from_slice(&((0.0 - tilt.clamp(-ZONE_PITCH_LIMIT, ZONE_PITCH_LIMIT)) as f32).to_le_bytes());
    payload[16..20].copy_from_slice(&(zoom.clamp(1.0, 4.0) as f32).to_le_bytes());
    payload
}

/// (pan, tilt, zoom) in our convention from the camera's 24-byte reply.
pub(crate) fn zone_home_from_reply(reply: &[u8]) -> Result<(f64, f64, f64), String> {
    let float = |at: usize| -> Result<f64, String> {
        let bytes: [u8; 4] = reply.get(at..at + 4).ok_or("The camera sent a short return position.")?.try_into().unwrap();
        Ok(f64::from(f32::from_le_bytes(bytes)))
    };
    let (yaw, pitch, zoom) = (float(4)?, float(8)?, float(16)?);
    if !yaw.is_finite() || !pitch.is_finite() || !zoom.is_finite() { return Err("The camera sent an unreadable return position.".into()); }
    Ok((yaw, -pitch, zoom))
}

pub(crate) fn set_zone_home(device: &str, pan: f64, tilt: f64, zoom: f64) -> Result<(), String> {
    send_camera_preset_command(device, ZONE_HOME_SET, &zone_home_payload(pan, tilt, zoom))
}

pub(crate) fn get_zone_home(device: &str) -> Result<(f64, f64, f64), String> {
    zone_home_from_reply(&query_obsbot(device, 0x04, ZONE_HOME_GET)?)
}

/// Set the return position and read it back: (what was sent, what the camera holds now), so a camera that
/// ignores it can be told apart from one that took it.
pub(crate) fn set_zone_home_checked(device: &str, pan: f64, tilt: f64, zoom: f64) -> Result<((f64, f64, f64), (f64, f64, f64)), String> {
    set_zone_home(device, pan, tilt, zoom)?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    Ok(((pan, tilt, zoom), get_zone_home(device)?))
}

/// The return position set to where the camera points now, at its current zoom.
pub(crate) fn set_zone_home_here(device: &str) -> Result<((f64, f64, f64), (f64, f64, f64)), String> {
    let (pan, tilt) = get_gimbal_position(device)?;
    let zoom = get_live_zoom_factor(device)?;
    set_zone_home_checked(device, pan, tilt, f64::from(zoom))
}

/// Turn the camera to the return position (with the app's own controls), so you can see where it is.
pub(crate) fn go_to_zone_home(device: &str) -> Result<(), String> {
    use super::angles::{clamp_pan, clamp_tilt, degrees_to_raw};
    let (pan, tilt, zoom) = get_zone_home(device)?;
    set_control(device, "pan_absolute", degrees_to_raw(clamp_pan(pan)))?;
    set_control(device, "tilt_absolute", degrees_to_raw(clamp_tilt(tilt)))?;
    set_control(device, "zoom_absolute", super::zoom::position(zoom).round() as i64)
}

pub(crate) fn get_live_zoom_factor(device: &str) -> Result<f32, String> {
    let reply = query_obsbot(device, 0x04, 0x6804)?;
    let bytes: [u8; 4] = reply.get(..4)
        .ok_or_else(|| format!("Live zoom reply is {} bytes; expected 4.", reply.len()))?
        .try_into().unwrap();
    let zoom = f32::from_le_bytes(bytes);
    if !zoom.is_finite() || !(1.0..=4.0).contains(&zoom) {
        return Err(format!("Camera returned invalid live zoom factor {zoom}."));
    }
    Ok(zoom)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InitialCameraState {
    pub source: u32,
    pub pan: f32,
    pub tilt: f32,
    pub zoom: f32,
}

pub(crate) fn read_initial_camera_state(device: &str) -> Result<InitialCameraState, String> {
    let reply = query_obsbot(device, 0x04, 0x3884)?;
    if reply.len() < 20 { return Err(format!("Initial-state pose reply is {} bytes; expected at least 20.", reply.len())); }
    let read_float = |offset: usize| -> Result<f32, String> {
        let bytes: [u8; 4] = reply[offset..offset + 4].try_into().unwrap();
        let value = f32::from_le_bytes(bytes);
        if value.is_finite() { Ok(value) } else { Err("Initial-state pose contains a non-finite value.".into()) }
    };
    Ok(InitialCameraState {
        source: u32::from_le_bytes(reply[0..4].try_into().unwrap()),
        pan: read_float(4)?,
        tilt: -read_float(8)?,
        zoom: read_float(16)?,
    })
}

pub(crate) fn initial_pose_payload(source: u32, pan: f32, tilt: f32, zoom: f32) -> Result<Vec<u8>, String> {
    if !pan.is_finite() || !tilt.is_finite() || pan.abs() > 180.0 || !(-180.0..=180.0).contains(&tilt) {
        return Err(format!("Initial pose is outside representable camera angles: pan/tilt must be within ±180° (received {pan:+.1}° / {tilt:+.1}°)."));
    }
    if !zoom.is_finite() || !(1.0..=4.0).contains(&zoom) {
        return Err(format!("Initial zoom must be 1.0–4.0× (received {zoom:.2}×)."));
    }
    let mut payload = Vec::with_capacity(24);
    payload.extend_from_slice(&source.to_le_bytes());
    // 0.0 - tilt: level is +0.0, as the camera and the SDK write it, not -0.0.
    for value in [pan, 0.0 - tilt, 0.0f32, zoom, 0.0f32] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    Ok(payload)
}

/// The start-up position, as OBSBOT's SDK sets it (aiSetGimbalBootPosR): one write, 04 0x3844, 24 bytes
/// (id 3 = a position of its own, yaw, pitch, 0, zoom, 0), then read back from 0x3884.
pub(crate) const BOOT_POSITION_SET: u16 = 0x3844;
pub(crate) fn set_initial_camera_state(device: &str) -> Result<InitialCameraState, String> {
    let (pan, tilt) = get_gimbal_position(device)?;
    let zoom = get_live_zoom_factor(device)?;
    let pose = initial_pose_payload(3, pan as f32, tilt as f32, zoom)?;
    send_camera_preset_command(device, BOOT_POSITION_SET, &pose)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    let saved = read_initial_camera_state(device)?;
    if (saved.pan - pan as f32).abs() > 0.2 || (saved.tilt - tilt as f32).abs() > 0.2 {
        return Err(format!("The start-up position was sent, but the camera reports Pan {:+.1}° / Tilt {:+.1}° instead of Pan {pan:+.1}° / Tilt {tilt:+.1}°.", saved.pan, saved.tilt));
    }
    Ok(saved)
}

pub(crate) fn decode_candidate_gimbal_position(data: &[u8; 60]) -> Result<(f64, f64), String> {
    let record = &data[..6];
    if record != [0x04, 0x0a, 0x44, 0x3a, 0x18, 0x00]
        && record != [0x02, 0x0a, 0x42, 0x29, 0x05, 0x00]
    {
        return Err(format!("Unrecognised position response record: {:02x?}", record));
    }
    let pan = f32::from_le_bytes(data[20..24].try_into().unwrap()) as f64;
    let tilt = f32::from_le_bytes(data[24..28].try_into().unwrap()) as f64;
    if !pan.is_finite() || !tilt.is_finite() {
        return Err("Position response contained non-finite coordinates.".into());
    }
    Ok((pan.clamp(-130.0, 130.0), tilt.clamp(-90.0, 90.0)))
}

/// Exposure compensation is the camera's standard V4L2 control `backlight_compensation`, 0 to 18.
/// (Reading it through the raw Processing Unit ioctl fails on this camera and kernel.)
pub(crate) fn get_exposure_compensation(device: &str) -> Result<u8, String> {
    let value = if ui_test_mode() {
        i64::from(LAST_UI_TEST_EXPOSURE_COMPENSATION.load(Ordering::Relaxed))
    } else {
        get_control(device, "backlight_compensation")?
    };
    u8::try_from(value).ok().filter(|value| *value <= 18)
        .ok_or_else(|| format!("The camera reported an exposure compensation of {value}; expected 0 to 18."))
}

pub(crate) fn set_exposure_compensation(device: &str, position: u8) -> Result<u8, String> {
    if position > 18 {
        return Err("Exposure compensation must be between 0 and 18.".into());
    }
    run_v4l2(device, &format!("--set-ctrl=backlight_compensation={position}"))?;
    if ui_test_mode() { LAST_UI_TEST_EXPOSURE_COMPENSATION.store(position, Ordering::Relaxed); }
    // Show and keep what the camera now says, not just what was asked for.
    let actual = get_exposure_compensation(device)?;
    update_config(|config| config.exposure_compensation_position = Some(actual));
    Ok(actual)
}

pub(crate) fn exposure_compensation_label(position: u8) -> String {
    if position == 9 { "0.0".into() }
    else { format!("{:+.1}", -3.0 + position as f64 / 3.0) }
}

pub(crate) fn xu_status(device: &str) -> Result<[u8; 60], String> {
    let mut data = [0u8; 60];
    xu_query(device, 0x06, UVC_GET_CUR, &mut data)?;
    Ok(data)
}

pub(crate) const XU_AUDIO_NOISE_REDUCTION_OFFSET: usize = 8;
pub(crate) const XU_AUDIO_MICROPHONE_DURING_SLEEP_OFFSET: usize = 16;
pub(crate) const XU_AUDIO_AUTO_GAIN_OFFSET: usize = 25;
pub(crate) const XU_AUDIO_MICROPHONE_DISABLED_OFFSET: usize = 2;
pub(crate) const XU_AUDIO_RADIO_DISTANCE_OFFSET: usize = 2;

pub(crate) fn microphone_disabled_wire_value(disabled: bool) -> u8 {
    if disabled { 0 } else { 1 }
}

pub(crate) fn microphone_during_sleep_state(status: &[u8; 60]) -> Option<bool> {
    match status[XU_AUDIO_MICROPHONE_DURING_SLEEP_OFFSET] {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}


pub(crate) fn audio_noise_reduction_label(level: u8) -> &'static str {
    match level {
        0 => "Off",
        1 => "Weak",
        2 => "Medium",
        3 => "Strong",
        _ => "Unknown",
    }
}

pub(crate) fn patch_xu_status_byte(mut data: [u8; 60], offset: usize, value: u8) -> Result<[u8; 60], String> {
    let byte = data.get_mut(offset)
        .ok_or_else(|| format!("OBSBOT status byte offset {offset} is outside the 60-byte block."))?;
    *byte = value;
    Ok(data)
}

pub(crate) fn set_xu_status_byte(device: &str, offset: usize, value: u8) -> Result<(), String> {
    // GET_CUR returns the current shared configuration block. Preserve every
    // unrelated field and change only the byte established by controlled capture.
    let current = xu_status(device)?;
    let mut data = patch_xu_status_byte(current, offset, value)?;
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)
}

pub(crate) fn microphone_during_sleep_payload(enabled: bool) -> [u8; 3] {
    [0x13, 0x01, u8::from(enabled)]
}

pub(crate) fn set_microphone_during_sleep(device: &str, enabled: bool) -> Result<(), String> {
    // Selector 0x06 status byte 0x10 is the read-back. The write is a
    // distinct tagged command: 13 01 <0|1>, not a status-block rewrite.
    let mut data = microphone_during_sleep_payload(enabled);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.audio_microphone_during_sleep = Some(enabled));
    Ok(())
}

pub(crate) fn set_microphone_disabled(device: &str, disabled: bool) -> Result<(), String> {
    // Unlike the GET_CUR status block, this captured command has its own
    // fixed 60-byte payload. Its first three bytes are 1c 01 <state>.
    let mut data = [0u8; 60];
    data[..2].copy_from_slice(&[0x1c, 0x01]);
    data[XU_AUDIO_MICROPHONE_DISABLED_OFFSET] = microphone_disabled_wire_value(disabled);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.audio_microphone_disabled = Some(disabled));
    MICROPHONE_PENDING.store(if disabled { 2 } else { 1 }, Ordering::Relaxed);
    Ok(())
}

/// What the microphone was last set to while the camera still reports the other state. The camera restarts
/// by itself to make the change, and until it is back it can still report the old state.
/// 0 nothing waiting, 1 on, 2 off.
static MICROPHONE_PENDING: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// `Some(disabled)` while a microphone change waits for the camera to restart.
pub(crate) fn microphone_pending() -> Option<bool> {
    match MICROPHONE_PENDING.load(Ordering::Relaxed) { 1 => Some(false), 2 => Some(true), _ => None }
}

/// The camera now reports what was asked for (or something else was chosen): nothing waits any more.
pub(crate) fn clear_microphone_pending() { MICROPHONE_PENDING.store(0, Ordering::Relaxed); }

#[cfg(test)]
pub(crate) fn set_microphone_pending_for_test(disabled: Option<bool>) {
    MICROPHONE_PENDING.store(match disabled { None => 0, Some(false) => 1, Some(true) => 2 }, Ordering::Relaxed);
}

pub(crate) fn radio_distance_label(distance: u8) -> &'static str {
    match distance { 0 => "Close", 1 => "Standard", 2 => "Far", _ => "Unknown" }
}

pub(crate) fn radio_distance_payload(distance: u8) -> Result<[u8; 60], String> {
    if distance > 2 { return Err("Radio distance must be Close, Standard, or Far.".into()); }
    let mut data = [0u8; 60];
    data[..2].copy_from_slice(&[0x1b, 0x01]);
    data[XU_AUDIO_RADIO_DISTANCE_OFFSET] = distance;
    Ok(data)
}

pub(crate) fn set_radio_distance(device: &str, distance: u8) -> Result<(), String> {
    let mut data = radio_distance_payload(distance)?;
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.audio_radio_distance = Some(distance));
    Ok(())
}

// Experimental Pop filter implementation: replay the captured 60-byte commands verbatim.
// The sequence/check fields are not decoded or regenerated.
pub(crate) fn pop_filter_payload(enabled: bool) -> [u8; 60] {
    let mut data = [0u8; 60];
    let captured: [u8; 32] = if enabled {
        [0xaa,0x25,0x10,0x00,0x0c,0x00,0x79,0x00,0x0a,0x02,0x82,0xc0,0x04,0x00,0xef,0x4f,
         0x06,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x50,0xc8,0xd7,0xb9,0x0f,0x22]
    } else {
        [0xaa,0x25,0x0f,0x00,0x0c,0x00,0x48,0xa5,0x0a,0x02,0x82,0xc0,0x04,0x00,0xbe,0x8f,
         0x06,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x50,0xc8,0xd7,0xb9,0x0f,0x22]
    };
    data[..32].copy_from_slice(&captured);
    data[32] = 0x01;
    data
}

pub(crate) fn set_pop_filter(device: &str, enabled: bool) -> Result<(), String> {
    let mut data = pop_filter_payload(enabled);
    xu_query(device, 0x02, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.pop_filter_enabled = Some(enabled));
    Ok(())
}

/// Mirror image command captured from three controlled OBSBOT Center transitions.
/// Only payload byte 2 changes: 1 = On, 0 = Off; the remaining 57 bytes are zero.
pub(crate) fn mirror_image_payload(enabled: bool) -> [u8; 60] {
    let mut data = [0u8; 60];
    data[..3].copy_from_slice(&[0x14, 0x01, u8::from(enabled)]);
    data
}

pub(crate) fn set_mirror_image(device: &str, enabled: bool) -> Result<(), String> {
    let mut data = mirror_image_payload(enabled);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.mirror_image_enabled = Some(enabled));
    Ok(())
}

/// Audio auto gain: the tagged selector 0x06 command `17 01 xx`, as OBSBOT's SDK sends it
/// (`cameraSetAudioAutoGainU`, captured 2026-11-24). It replaced rewriting status byte 25 in the whole block
/// (see docs/sdk-alignment-backup.md). The state still reads back from status byte 25.
pub(crate) fn auto_gain_payload(enabled: bool) -> [u8; 60] {
    let mut data = [0u8; 60];
    data[..3].copy_from_slice(&[0x17, 0x01, u8::from(enabled)]);
    data
}

pub(crate) fn set_auto_gain(device: &str, enabled: bool) -> Result<(), String> {
    let mut data = auto_gain_payload(enabled);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.audio_auto_gain = Some(enabled));
    Ok(())
}

pub(crate) fn set_noise_reduction(device: &str, level: u8) -> Result<(), String> {
    if level > 3 {
        return Err("Noise reduction must be Off, Weak, Medium, or Strong.".into());
    }
    set_xu_status_byte(device, XU_AUDIO_NOISE_REDUCTION_OFFSET, level)?;
    update_config(|config| config.audio_noise_reduction = Some(level));
    Ok(())
}


pub(crate) fn crc16_usb(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in bytes {
        crc ^= *byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xa001 } else { crc >> 1 };
        }
    }
    !crc
}

pub(crate) fn obsbot_frame(sequence: u16, function_group: [u8; 6], command: [u8; 6]) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = 0xaa;
    frame[1] = 0x25;
    frame[2..4].copy_from_slice(&sequence.to_le_bytes());
    frame[4..6].copy_from_slice(&12u16.to_le_bytes());
    // The six-byte captured function block is four command bytes plus
    // the two-byte payload length. Only the first four bytes belong in
    // the 12-byte header.
    frame[8..12].copy_from_slice(&function_group[..4]);
    let header_crc = crc16_usb(&frame[..12]);
    frame[6..8].copy_from_slice(&header_crc.to_le_bytes());

    let payload_len = u16::from_le_bytes([function_group[4], function_group[5]]) as usize;
    let payload_len = payload_len.min(4);
    frame[12..14].copy_from_slice(&(payload_len as u16).to_le_bytes());
    frame[14..16].fill(0);
    frame[16..16 + payload_len].copy_from_slice(&command[2..2 + payload_len]);
    let payload_crc = crc16_usb(&frame[12..16 + payload_len]);
    frame[14..16].copy_from_slice(&payload_crc.to_le_bytes());
    frame
}

// OBSBOT's frame sequence is a rolling transaction counter shared by
// framed XU commands. The capture shows consecutive values 0x000f..0x0012
// across Direction Flip and Dynamic Zoom commands, not fixed per-feature IDs.
pub(crate) static XU_SEQUENCE: OnceLock<AtomicU16> = OnceLock::new();
pub(crate) static LAST_UI_TEST_QUERY_COMMAND: AtomicU16 = AtomicU16::new(0);
pub(crate) static LAST_UI_TEST_QUERY_SEQUENCE: AtomicU16 = AtomicU16::new(0);
pub(crate) static LAST_UI_TEST_AI_MODE: AtomicU16 = AtomicU16::new(0);
pub(crate) static LAST_UI_TEST_AUTO_EXPOSURE_PRIORITY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
pub(crate) static LAST_UI_TEST_EXPOSURE_COMPENSATION: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(9);
pub(crate) static LAST_UI_TEST_FOCUS_VALUE: AtomicU32 = AtomicU32::new(1);
/// The pretend camera's microphone (status byte 34, bit 4). On unless TINY4LITE_UI_TEST_MIC=0.
fn ui_test_microphone() -> &'static std::sync::atomic::AtomicBool {
    static MIC: std::sync::OnceLock<std::sync::atomic::AtomicBool> = std::sync::OnceLock::new();
    MIC.get_or_init(|| std::sync::atomic::AtomicBool::new(std::env::var("TINY4LITE_UI_TEST_MIC").map_or(true, |v| v != "0")))
}

/// The pretend camera's Mirror image (status byte 19). TINY4LITE_UI_TEST_MIRROR=1 starts it on.
fn ui_test_mirror() -> &'static std::sync::atomic::AtomicBool {
    static MIRROR: std::sync::OnceLock<std::sync::atomic::AtomicBool> = std::sync::OnceLock::new();
    MIRROR.get_or_init(|| std::sync::atomic::AtomicBool::new(std::env::var("TINY4LITE_UI_TEST_MIRROR").is_ok_and(|v| v == "1")))
}

pub(crate) fn xu_sequence_counter() -> &'static AtomicU16 {
    XU_SEQUENCE.get_or_init(|| {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u16)
            .unwrap_or(0);
        AtomicU16::new(seed)
    })
}

pub(crate) fn next_xu_sequence(counter: &AtomicU16) -> u16 {
    counter.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

pub(crate) fn xu_send_frame(device: &str, function_group: [u8; 6], command: [u8; 6]) -> Result<(), String> {
    let sequence = next_xu_sequence(xu_sequence_counter());
    let mut frame = obsbot_frame(sequence, function_group, command);
    xu_query(device, 0x02, UVC_SET_CUR, &mut frame)
}

pub(crate) fn obsbot_frame_with_payload(sequence: u16, command: u16, payload: &[u8]) -> Result<[u8; 60], String> {
    obsbot_frame_to(0x04, sequence, command, payload)
}

/// A framed command to one of the camera's modules: 0x02 camera and audio, 0x04 AI and gimbal, 0x12 system.
pub(crate) fn obsbot_frame_to(module: u8, sequence: u16, command: u16, payload: &[u8]) -> Result<[u8; 60], String> {
    if payload.len() > 44 {
        return Err(format!("OBSBOT command payload is too large: {} bytes", payload.len()));
    }
    let mut frame = [0u8; 60];
    frame[0] = 0xaa;
    frame[1] = 0x25;
    frame[2..4].copy_from_slice(&sequence.to_le_bytes());
    frame[4..6].copy_from_slice(&12u16.to_le_bytes());
    frame[8..12].copy_from_slice(&[0x0a, module, command as u8, (command >> 8) as u8]);
    let header_crc = crc16_usb(&frame[..12]);
    frame[6..8].copy_from_slice(&header_crc.to_le_bytes());
    let len = payload.len() as u16;
    frame[12..14].copy_from_slice(&len.to_le_bytes());
    frame[16..16 + payload.len()].copy_from_slice(payload);
    let payload_crc = crc16_usb(&frame[12..16 + payload.len()]);
    frame[14..16].copy_from_slice(&payload_crc.to_le_bytes());
    Ok(frame)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CameraPresetEntry {
    pub slot: u8,
    pub name: String,
    pub pan_degrees: f32,
    pub tilt_degrees: f32,
    pub zoom: f32,
}

pub(crate) fn decode_preset_base64(input: &[u8]) -> Result<String, String> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut output = Vec::new();
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in input {
        if *byte == b'=' { break; }
        let digit = value(*byte).ok_or_else(|| format!("Invalid base64 byte 0x{byte:02x} in preset name"))?;
        accumulator = (accumulator << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
        }
    }
    String::from_utf8(output).map_err(|error| format!("Preset name is not UTF-8: {error}"))
}

pub(crate) fn parse_camera_preset_entry(data: &[u8; 60]) -> Result<Option<CameraPresetEntry>, String> {
    if data[0] != 0x00 { return Ok(None); }
    let name_end = data[10..].iter().position(|byte| *byte == 0).unwrap_or(50);
    let name = decode_preset_base64(&data[10..10 + name_end])?;
    let pitch = i16::from_le_bytes([data[4], data[5]]) as f32 / 100.0;
    let yaw = i16::from_le_bytes([data[6], data[7]]) as f32 / 100.0;
    let zoom = u16::from_le_bytes([data[8], data[9]]) as f32 / 100.0;
    Ok(Some(CameraPresetEntry { slot: data[1], name, pan_degrees: yaw, tilt_degrees: -pitch, zoom }))
}

/// Camera-native Tiny 2 Lite preset list. Selector 0x0D is a consuming
/// cursor, so selector 0x0C must be echo-written before every enumeration.
pub(crate) fn read_camera_presets(device: &str) -> Result<Vec<CameraPresetEntry>, String> {
    let mut list = [0u8; 60];
    xu_query(device, 0x0c, UVC_GET_CUR, &mut list)?;
    let count = usize::from(list[0]).min(3);
    let slots = list[1..1 + count].to_vec();
    if slots.is_empty() { return Ok(Vec::new()); }
    xu_query(device, 0x0c, UVC_SET_CUR, &mut list)?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    let mut entries = Vec::new();
    for _ in 0..slots.len() {
        let mut entry = [0u8; 60];
        xu_query(device, 0x0d, UVC_GET_CUR, &mut entry)?;
        match parse_camera_preset_entry(&entry)? {
            Some(value) if slots.contains(&value.slot) => entries.push(value),
            Some(_) => return Err("Camera preset cursor returned a slot absent from selector 0x0C.".into()),
            None => break,
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if entries.len() != slots.len() {
        return Err(format!("Camera reports {} preset slots but {} entries could be read.", slots.len(), entries.len()));
    }
    Ok(entries)
}

pub(crate) fn preset_slot_index(slot: u8) -> Result<u32, String> {
    if slot >= 3 { return Err("Tiny 2 Lite preset slot must be 1, 2, or 3.".into()); }
    Ok(u32::from(slot))
}

pub(crate) fn send_obsbot_command(device: &str, module: u8, command: u16, payload: &[u8]) -> Result<(), String> {
    let sequence = next_xu_sequence(xu_sequence_counter());
    let mut frame = obsbot_frame_to(module, sequence, command, payload)?;
    xu_query(device, 0x02, UVC_SET_CUR, &mut frame)?;
    Ok(())
}

// ── Found with the SDK's undeclared functions (capture of 2026-11-24, round 3) ──

const MIC_VOLUME_GET: u16 = 0x80c2;
const MIC_VOLUME_SET: u16 = 0x8102;
const CAMERA_SLEEP_GET: u16 = 0xa182;
const CAMERA_SLEEP_SET: u16 = 0xa1c2;
const LED_ENABLED_SET: u16 = 0x75cd;
const FOCUS_POINT_SET: u16 = 0x3582;
const SELECT_TARGET_SET: u16 = 0x0404;

/// Microphone volume (cameraGetAudioVolumeR / cameraSetAudioVolumeR): i16, module 0x02. Read 50 on the Lite.
pub(crate) fn get_mic_volume(device: &str) -> Result<i16, String> {
    let reply = query_obsbot(device, 0x02, MIC_VOLUME_GET)?;
    let bytes: [u8; 2] = reply.get(..2).ok_or("The camera sent a short volume.")?.try_into().unwrap();
    Ok(i16::from_le_bytes(bytes))
}

pub(crate) fn set_mic_volume(device: &str, volume: i16) -> Result<(), String> {
    send_obsbot_command(device, 0x02, MIC_VOLUME_SET, &volume.clamp(0, 100).to_le_bytes())
}

/// The camera's own sleep timer (cameraGetAutoSleepTimeR / cameraSetAutoSleepTimeR): u32 seconds, 0 is off.
pub(crate) fn get_camera_sleep_time(device: &str) -> Result<u32, String> {
    let reply = query_obsbot(device, 0x02, CAMERA_SLEEP_GET)?;
    let bytes: [u8; 4] = reply.get(..4).ok_or("The camera sent a short sleep time.")?.try_into().unwrap();
    Ok(u32::from_le_bytes(bytes))
}

pub(crate) fn set_camera_sleep_time(device: &str, seconds: u32) -> Result<(), String> {
    send_obsbot_command(device, 0x02, CAMERA_SLEEP_SET, &seconds.min(65535).to_le_bytes())
}

/// The status light's own switch (sysMgSetLedEnabledR), module 0x12. The camera does not answer a read.
pub(crate) fn set_led_enabled(device: &str, on: bool) -> Result<(), String> {
    send_obsbot_command(device, 0x12, LED_ENABLED_SET, &[u8::from(on)])
}

/// Focus on a point of the picture (cameraSetUserFocusPos): x and y from 0 to 1 across the picture, then
/// 8 zero bytes and 100, as the SDK sends them.
pub(crate) fn focus_point_payload(x: f32, y: f32) -> [u8; 20] {
    let mut payload = [0u8; 20];
    payload[0..4].copy_from_slice(&x.clamp(0.0, 1.0).to_le_bytes());
    payload[4..8].copy_from_slice(&y.clamp(0.0, 1.0).to_le_bytes());
    payload[16..20].copy_from_slice(&100u32.to_le_bytes());
    payload
}

pub(crate) fn set_focus_point(device: &str, x: f32, y: f32) -> Result<(), String> {
    send_obsbot_command(device, 0x02, FOCUS_POINT_SET, &focus_point_payload(x, y))
}

/// Track the person at a point of the picture (aiSetSelectTargetByPos): x, y from 0 to 1, then a 0 byte.
pub(crate) fn select_target_payload(x: f32, y: f32) -> [u8; 9] {
    let mut payload = [0u8; 9];
    payload[0..4].copy_from_slice(&x.clamp(0.0, 1.0).to_le_bytes());
    payload[4..8].copy_from_slice(&y.clamp(0.0, 1.0).to_le_bytes());
    payload
}

pub(crate) fn select_target_at(device: &str, x: f32, y: f32) -> Result<(), String> {
    send_obsbot_command(device, 0x04, SELECT_TARGET_SET, &select_target_payload(x, y))
}

/// The AI status (module 04, command 0x0104), as captured from the SDK on a Tiny 2 Lite (2026-11-25): byte 2 the
/// hand followed (0 right, 1 left), 3 target gesture, 4 zoom gesture, 5 dynamic zoom, 6 reverse dynamic zoom,
/// 7 the gesture zoom amount in tenths (0x19 = 2.5×), 9 tracking speed.
pub(crate) fn get_ai_status(device: &str) -> Result<Vec<u8>, String> {
    let reply = query_obsbot(device, 0x04, 0x0104)?;
    if reply.len() < 10 { return Err(format!("The AI status reply is {} bytes; expected 12.", reply.len())); }
    Ok(reply)
}

/// Which hand Hand tracking follows (aiSetHandTrackModeR): 0 right, 1 left; command 0x2244 (capture of 2026-11-25).
const HAND_SIDE_SET: u16 = 0x2244;
pub(crate) fn set_hand_side(device: &str, left: bool) -> Result<(), String> {
    send_camera_preset_command(device, HAND_SIDE_SET, &[u8::from(left)])
}


/// Follow the person at a point of the picture. With tracking off, Normal tracking is turned on first, since
/// picking a target only means something while the camera tracks.
pub(crate) fn track_at(device: &str, x: f32, y: f32) -> Result<(), String> {
    if xu_status(device).is_ok_and(|status| status[0x18] == 0) { set_ai_mode(device, (0x02, 0x00))?; }
    select_target_at(device, x, y)
}

pub(crate) fn send_camera_preset_command(device: &str, command: u16, payload: &[u8]) -> Result<(), String> {
    let sequence = next_xu_sequence(xu_sequence_counter());
    let mut frame = obsbot_frame_with_payload(sequence, command, payload)?;
    xu_query(device, 0x02, UVC_SET_CUR, &mut frame)?;
    Ok(())
}

pub(crate) fn save_camera_preset(device: &str, slot: u8, name: &str, update: bool) -> Result<(), String> {
    let index = preset_slot_index(slot)?;
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 32 || !trimmed.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
        return Err("Preset names must contain 1–32 printable ASCII characters.".into());
    }
    let (pan, tilt) = get_gimbal_position(device)?;
    let zoom = get_live_zoom_factor(device)?;
    let mut payload = index.to_le_bytes().to_vec();
    if !pan.is_finite() || !tilt.is_finite() || !(super::angles::PAN_RANGE.0..=super::angles::PAN_RANGE.1).contains(&pan)
        || !(super::angles::TILT_RANGE.0..=super::angles::TILT_RANGE.1).contains(&tilt) {
        return Err(format!("Current live pose is outside camera preset limits: pan {pan:+.1}°, tilt {tilt:+.1}°."));
    }
    if !(1.0..=4.0).contains(&zoom) {
        return Err(format!("Current live zoom {zoom:.2}× is outside preset limits 1.0–4.0×."));
    }
    // As OBSBOT's SDK writes it (aiAddGimbalPresetR 04 0x3944, aiUpdGimbalPresetR 04 0x3a04): id, yaw, pitch, 0,
    // zoom, 0; then the name (0x3a84).
    for value in [pan as f32, 0.0 - tilt as f32, 0.0f32, zoom, 0.0f32] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    send_camera_preset_command(device, if update { PRESET_UPDATE } else { PRESET_ADD }, &payload)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    rename_camera_preset(device, slot, trimmed)?;
    Ok(())
}

pub(crate) fn rename_camera_preset(device: &str, slot: u8, name: &str) -> Result<(), String> {
    let index = preset_slot_index(slot)?;
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 32 || !trimmed.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
        return Err("Preset names must contain 1–32 printable ASCII characters.".into());
    }
    let mut payload = index.to_le_bytes().to_vec();
    payload.extend_from_slice(trimmed.as_bytes());
    send_camera_preset_command(device, 0x3a84, &payload)
}

pub(crate) const PRESET_ADD: u16 = 0x3944;
pub(crate) const PRESET_UPDATE: u16 = 0x3a04;
pub(crate) const PRESET_DELETE: u16 = 0x3984;

/// Delete a saved position as OBSBOT's SDK does (aiDelGimbalPresetR): 04 0x3984 with the id, nothing else.
pub(crate) fn delete_camera_preset(device: &str, slot: u8) -> Result<(), String> {
    send_camera_preset_command(device, PRESET_DELETE, &preset_slot_index(slot)?.to_le_bytes())
}

pub(crate) fn recall_camera_preset(device: &str, slot: u8) -> Result<(), String> {
    let mut payload = preset_slot_index(slot)?.to_le_bytes().to_vec();
    for _ in 0..4 { payload.extend_from_slice(&1.0f32.to_le_bytes()); }
    send_camera_preset_command(device, 0x39c4, &payload)
}


pub(crate) fn led_brightness_payload(level: u8) -> Result<[u8; 3], String> {
    if level > 3 {
        return Err("LED setting must be Off (0) or brightness level 1–3.".into());
    }
    // The captured selector 0x06/tag 0x1A values are the four settings:
    // raw 0 = Off; raw 1, 2, 3 = brightness levels 1, 2, 3.
    // Do not subtract one: that incorrectly turns level 1 into Off.
    Ok([0x1a, 0x01, level])
}

pub(crate) fn set_led_brightness(device: &str, level: u8) -> Result<(), String> {
    let payload = led_brightness_payload(level)?;
    let mut data = [0u8; 60];
    data[..payload.len()].copy_from_slice(&payload);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    // A level also turns on the light's own switch, which the SDK probe of 2026-11-24 may have left off
    // (it cannot be read). Off is the brightness 0 command alone, as before.
    if level > 0 { set_led_enabled(device, true)?; }
    update_config(|config| config.led_brightness = Some(level));
    Ok(())
}

// Tiny 2 Lite HDR control, captured as a 60-byte selector 0x06 SET_CUR.
// Captures correlate third byte 0x01 with HDR On and 0x00 with HDR Off.
pub(crate) fn hdr_payload(enabled: bool) -> [u8; 3] {
    [0x01, 0x01, if enabled { 0x01 } else { 0x00 }]
}

pub(crate) fn hdr_state_from_status(status: &[u8; 60]) -> Option<bool> {
    match status[0x06] {
        0x00 => Some(false),
        0x01 => Some(true),
        _ => None,
    }
}

/// How long the camera needs between two HDR switches: switching reconfigures the sensor, and OBSBOT's own
/// guidance is to leave 3 seconds between them.
pub const HDR_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);
static LAST_HDR_SWITCH: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

pub(crate) fn set_hdr_enabled(device: &str, enabled: bool) -> Result<(), String> {
    // Never switch again within 3 seconds of the last switch (the window's switch rests that long, so this
    // only waits for the command line or presets).
    let since = LAST_HDR_SWITCH.lock().ok().and_then(|last| last.map(|at| at.elapsed()));
    if let Some(since) = since { if since < HDR_SETTLE { std::thread::sleep(HDR_SETTLE - since); } }
    let payload = hdr_payload(enabled);
    let mut data = [0u8; 60];
    data[..payload.len()].copy_from_slice(&payload);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    if let Ok(mut last) = LAST_HDR_SWITCH.lock() { *last = Some(std::time::Instant::now()); }
    update_config(|config| config.hdr_enabled = Some(enabled));
    Ok(())
}

// Tiny 2 Lite Face / Global Auto Exposure priority.
// XU unit 2, selector 0x06, 60-byte SET_CUR and GET_CUR. Controlled
// 2026-11-20 captures established the write value at byte 2 and the
// authoritative read-back value at byte 7. This is NOT autofocus priority.
pub(crate) const XU_AUTO_EXPOSURE_PRIORITY_OFFSET: usize = 7;

pub(crate) fn auto_exposure_mode_payload(face_priority: bool) -> [u8; 60] {
    let mut data = [0u8; 60];
    // As OBSBOT's SDK sends it (cameraSetFaceAER, captured 2026-11-24): 1 = face, 0 = global. It was the other
    // way round from an earlier capture (see docs/sdk-alignment-backup.md).
    data[..3].copy_from_slice(&[0x03, 0x01, if face_priority { 0x01 } else { 0x00 }]);
    data
}

pub(crate) fn auto_exposure_mode_from_status(status: &[u8; 60]) -> Option<bool> {
    match status[XU_AUTO_EXPOSURE_PRIORITY_OFFSET] {
        0x01 => Some(true),  // Face (as the SDK's cameraSetFaceAER(1) leaves it)
        0x00 => Some(false), // Global
        _ => None,
    }
}

pub(crate) fn get_auto_exposure_mode(device: &str) -> Result<bool, String> {
    let status = xu_status(device)?;
    auto_exposure_mode_from_status(&status).ok_or_else(|| format!(
        "Camera returned unsupported Auto Exposure priority value 0x{:02x} at status byte 0x{:02x}.",
        status[XU_AUTO_EXPOSURE_PRIORITY_OFFSET], XU_AUTO_EXPOSURE_PRIORITY_OFFSET
    ))
}

/// After a command, read the state back, waiting a little for the camera to catch up: it often
/// reports the old value for a moment after accepting a change. Returns the last reading, which is
/// only different from `wanted` if the camera really kept something else.
pub(crate) fn settle_readback<T: PartialEq + Copy>(wanted: T, read: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    settle_readback_with(wanted, read, if ui_test_mode() { std::time::Duration::ZERO } else { std::time::Duration::from_millis(200) })
}

pub(crate) fn settle_readback_with<T: PartialEq + Copy>(wanted: T, mut read: impl FnMut() -> Result<T, String>, wait: std::time::Duration) -> Result<T, String> {
    const ATTEMPTS: usize = 7;
    for attempt in 1..=ATTEMPTS {
        let actual = read()?;
        if actual == wanted || attempt == ATTEMPTS { return Ok(actual); }
        std::thread::sleep(wait);
    }
    unreachable!("the last attempt returns")
}

pub(crate) fn set_auto_exposure_mode(device: &str, face_priority: bool) -> Result<bool, String> {
    let mut data = auto_exposure_mode_payload(face_priority);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;

    // Do not treat a successful ioctl or the requested value as proof of
    // device state. Read the complete status block and trust byte 7.
    let actual_face_priority = settle_readback(face_priority, || get_auto_exposure_mode(device))
        .map_err(|error| format!("SET_CUR was submitted, but Auto Exposure priority read-back failed: {error}"))?;
    update_config(|config| config.auto_exposure_mode = Some(
        if actual_face_priority { "face" } else { "global" }.into()
    ));
    Ok(actual_face_priority)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FaceFocusPriority {
    Face,
    Global,
}

impl FaceFocusPriority {
    /// The value sent: a 32-bit number at offset 16 of the frame, 1 for Face and 0 for Global, with the
    /// frame's payload CRC just before it. (The bytes `bf fb 01 00` and `be 07 00 00` that were once
    /// taken for the value are that CRC followed by the first two bytes of the number: they are the
    /// same two bytes that appear before `01 00 00 00` and `00 00 00 00` in the sleep and wake frames.)
    pub(crate) fn value(self) -> [u8; 4] {
        match self {
            Self::Face => 1u32.to_le_bytes(),
            Self::Global => 0u32.to_le_bytes(),
        }
    }

    /// Bytes 14 to 18 of a frame carrying this value: its CRC and the first half of the number. This is
    /// how the camera's reply looks, and how the earlier notes wrote it down.
    pub(crate) fn payload(self) -> [u8; 4] {
        match self {
            Self::Face => [0xbf, 0xfb, 0x01, 0x00],
            Self::Global => [0xbe, 0x07, 0x00, 0x00],
        }
    }

    pub(crate) fn from_payload(payload: [u8; 4]) -> Result<Self, String> {
        match payload {
            [0xbf, 0xfb, 0x01, 0x00] => Ok(Self::Face),
            [0xbe, 0x07, 0x00, 0x00] => Ok(Self::Global),
            value => Err(format!("Camera returned unknown autofocus-priority value {:02x?}.", value)),
        }
    }

    pub(crate) fn config_value(self) -> &'static str {
        match self { Self::Face => "face", Self::Global => "global" }
    }

    pub(crate) fn label(self) -> &'static str {
        match self { Self::Face => "Face", Self::Global => "Global" }
    }
}

pub(crate) fn face_focus_get_frame(sequence: u16) -> [u8; 60] {
    let mut frame = [0u8; 60];
    frame[0] = 0xaa;
    frame[1] = 0x01; // Header-only vendor GET request.
    frame[2..4].copy_from_slice(&sequence.to_le_bytes());
    frame[4..6].copy_from_slice(&12u16.to_le_bytes());
    frame[8..12].copy_from_slice(&[0x0a, 0x02, 0xc2, 0x35]);
    // CRC16/USB covers the complete 12-byte header with bytes 6-7 zeroed.
    let crc = crc16_usb(&frame[..12]);
    frame[6..8].copy_from_slice(&crc.to_le_bytes());
    frame
}

pub(crate) fn decode_face_focus_reply(data: &[u8], sequence: u16) -> Result<Option<FaceFocusPriority>, String> {
    if data.len() < 18 || data[..2] != [0xaa, 0x29]
        || data[2..4] != sequence.to_le_bytes()
        || data[10..12] != [0xc2, 0x35]
    {
        return Ok(None);
    }
    if data[4..6] != [0x0c, 0x00] || data[8..10] != [0x02, 0x0a] {
        return Err("CAM_GET_FACE_FOCUS reply has an unexpected header.".into());
    }
    if u16::from_le_bytes([data[12], data[13]]) != 4 {
        return Err("CAM_GET_FACE_FOCUS reply did not contain a four-byte priority value.".into());
    }
    let mut header = data[..12].to_vec();
    let received_crc = u16::from_le_bytes([header[6], header[7]]);
    header[6] = 0;
    header[7] = 0;
    if crc16_usb(&header) != received_crc {
        return Err("CAM_GET_FACE_FOCUS reply header CRC is invalid.".into());
    }
    // The number sits at 16 to 20: 1 is Face, 0 is Global. The older reading of bytes 14 to 18 (the
    // CRC and half the number) is kept as a second opinion for replies laid out that way.
    match [data[16], data[17], data[18], data[19]] {
        [1, 0, 0, 0] => return Ok(Some(FaceFocusPriority::Face)),
        [0, 0, 0, 0] if data[14..16] == FaceFocusPriority::Global.payload()[..2] => return Ok(Some(FaceFocusPriority::Global)),
        _ => {}
    }
    let payload = [data[14], data[15], data[16], data[17]];
    FaceFocusPriority::from_payload(payload).map(Some)
}

pub(crate) fn get_face_focus(device: &str) -> Result<FaceFocusPriority, String> {
    let sequence = next_xu_sequence(xu_sequence_counter());
    let mut request = face_focus_get_frame(sequence);
    xu_query(device, 0x02, UVC_SET_CUR, &mut request)?;

    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(30));
        let mut response = [0u8; 60];
        xu_query(device, 0x02, UVC_GET_CUR, &mut response)?;
        if let Some(priority) = decode_face_focus_reply(&response, sequence)? {
            return Ok(priority);
        }
    }
    Err("Timed out waiting for a matching CAM_GET_FACE_FOCUS reply.".into())
}

pub(crate) fn set_face_focus(device: &str, requested: FaceFocusPriority) -> Result<FaceFocusPriority, String> {
    let group = [0x0a, 0x02, 0x02, 0x36, 0x04, 0x00];
    let mut command = [0u8; 6];
    command[2..6].copy_from_slice(&requested.value());
    xu_send_frame(device, group, command)?;
    if std::env::var_os("TINY4LITE_DEBUG_XU").is_some() {
        eprintln!("[xu] autofocus priority: sent {} (value {:02x?})", requested.label(), requested.value());
    }

    // A successful SET_CUR is only host-transport evidence. Confirm the
    // camera's state with the independent CAM_GET_FACE_FOCUS transaction.
    let actual = settle_readback(requested, || {
        let reading = get_face_focus(device);
        if std::env::var_os("TINY4LITE_DEBUG_XU").is_some() {
            eprintln!("[xu] autofocus priority: camera answered {:?}", reading.as_ref().map(|p| p.label()));
        }
        reading
    })?;
    update_config(|config| config.focus_mode = Some(actual.config_value().into()));
    Ok(actual)
}

pub(crate) fn set_ai_mode(device: &str, mode: (u8, u8)) -> Result<(), String> {
    // Selector 0x06 uses [opcode, payload length, m, n].
    let mut data = [0u8; 60];
    data[0] = 0x16;
    data[1] = 0x02;
    data[2] = mode.0;
    data[3] = mode.1;
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    // The camera takes a moment to report the new mode, and it works either way, so wait a little
    // for the report to catch up and never treat a late one as a failure. The window reads the
    // state again afterwards and shows whatever the camera says.
    for attempt in 0..5 {
        match xu_status(device) {
            Ok(status) if (status[0x18], status[0x1c]) == mode => break,
            _ if attempt < 4 && !ui_test_mode() => std::thread::sleep(std::time::Duration::from_millis(150)),
            _ => break,
        }
    }
    let name = match mode { (0x00, 0x00) => "off", (0x02, 0x00) => "normal", (0x02, 0x01) => "upper", (0x02, 0x02) => "close", (0x02, 0x03) => "headless", (0x02, 0x04) => "lower", (0x03, 0x00) => "hand", (0x01, 0x00) => "group", _ => "unknown" };
    update_config(|config| config.ai_tracking_mode = Some(name.into()));
    Ok(())
}

pub(crate) fn set_sleep_mode(device: &str, sleep: bool) -> Result<(), String> {
    // CAM_SET_DEV_STATUS (0xA0C2), confirmed on Tiny 2 Lite.
    // Power-state verification is deliberately separate: the camera's
    // status byte may lag behind the command.
    if ui_test_mode() {
        // The synthetic camera records what was asked, for the tests.
        if let Ok(path) = std::env::var("TINY4LITE_UI_TEST_LOG") {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "sleep={}", if sleep { "on" } else { "off" });
            }
        }
    }
    let group = [0x0a, 0x02, 0xc2, 0xa0, 0x04, 0x00];
    let command = if sleep {
        [0x00, 0x00, 0x01, 0x00, 0x00, 0x00]
    } else {
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    };
    xu_send_frame(device, group, command)
}

/// Vertical tracking (command 0x0cc4): 0 Standard, 1 Headroom, 2 Motion, as OBSBOT's SDK names it
/// (`aiSetTrackingModeR(AiVerticalTrackType)`, captured 2026-11-24). It was "Tracking speed", Standard / Sport
/// (wire 0 / 2); see docs/sdk-alignment-backup.md.
pub(crate) fn set_vertical_tracking(device: &str, mode: u8) -> Result<(), String> {
    if mode > 2 { return Err(format!("Vertical tracking is 0 (Standard), 1 (Headroom) or 2 (Motion), not {mode}.")); }
    send_camera_preset_command(device, 0x0cc4, &[mode])?;
    update_config(|config| config.tracking_speed = Some(vertical_tracking_name(mode).into()));
    Ok(())
}

pub(crate) fn vertical_tracking_name(mode: u8) -> &'static str {
    match mode { 1 => "headroom", 2 => "sport", _ => "standard" }
}

/// The saved name, including the old "sport" (which is Motion).
pub(crate) fn vertical_tracking_from_name(name: &str) -> Option<u8> {
    match name { "standard" => Some(0), "headroom" => Some(1), "motion" | "sport" => Some(2), _ => None }
}

/// Status byte 0x24: 0, 1, 2 as sent; 0x21 was seen for Motion (then called Sport) in an earlier capture.
pub(crate) fn vertical_tracking_from_status(byte: u8) -> Option<u8> {
    match byte { 0 => Some(0), 1 => Some(1), 2 | 0x21 => Some(2), _ => None }
}

pub(crate) fn set_gesture_zoom_ratio_payload_is_valid(ratio: f32) -> bool {
    ratio.is_finite() && (1.0..=4.0).contains(&ratio)
}

pub(crate) fn set_gesture_zoom_ratio(device: &str, ratio: f32) -> Result<(), String> {
    if !set_gesture_zoom_ratio_payload_is_valid(ratio) {
        return Err("Gesture zoom ratio must be between 1.00x and 4.00x.".into());
    }
    // AI_SET_GESTURE_ZOOM_RATIO (0x3244), captured on Tiny 2 Lite.
    // The payload is an IEEE-754 f32 in little-endian order.
    let group = [0x0a, 0x04, 0x44, 0x32, 0x04, 0x00];
    let mut command = [0u8; 6];
    command[2..6].copy_from_slice(&ratio.to_le_bytes());
    xu_send_frame(device, group, command)?;
    update_config(|config| config.gesture_zoom_ratio = Some(ratio));
    Ok(())
}

pub(crate) fn gesture_command(group: [u8; 6], enabled: bool) -> [u8; 6] {
    // On is 1 and off is 0 for every gesture, Dynamic Zoom Direction included, as OBSBOT's SDK sends them
    // (aiSetGestureCtrlIndividualR, captured 2026-11-24). Direction used to be inverted, from an earlier
    // capture (see docs/sdk-alignment-backup.md).
    let _ = group;
    if enabled {
        [0x27, 0xff, 0x01, 0x00, 0x00, 0x00]
    } else {
        [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00]
    }
}

pub(crate) fn set_gesture(device: &str, group: [u8; 6], enabled: bool) -> Result<(), String> {
    let command = gesture_command(group, enabled);
    xu_send_frame(device, group, command)?;
    let key = match group { [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00] => "target_selection", [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00] => "zoom_l_pose", [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00] => "dynamic_zoom", [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00] => "dynamic_zoom_direction", _ => "unknown" };
    if key != "unknown" { update_config(|config| { config.gesture_states.insert(key.into(), enabled); }); }
    Ok(())
}

pub(crate) fn ai_mode_label(status: &[u8; 60]) -> &'static str {
    match (status[0x18], status[0x1c]) {
        (0x00, 0x00) => "Off",
        (0x02, 0x00) => "Normal tracking",
        (0x02, 0x01) => "Upper body",
        (0x02, 0x02) => "Close-up",
        (0x02, 0x03) => "Headless",
        (0x02, 0x04) => "Lower body",
        (0x03, 0x00) => "Hand",
        (0x01, 0x00) => "Group",
        _ => "Unknown mode",
    }
}

pub(crate) fn usb_descriptor_info() -> Result<String, String> {
    if ui_test_mode() {
        return Ok("USB identity: 3564 / fef9\nManufacturer: OBSBOT (fixture)\nProduct: Tiny 2 Lite (headless test)\nUSB bcdDevice: simulated\nUSB bus/address: 0/0\n\nThis is a synthetic UI fixture. No camera is connected or controlled.".into());
    }
    let entries = fs::read_dir("/sys/bus/usb/devices")
        .map_err(|e| format!("Could not enumerate USB devices: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let vendor = fs::read_to_string(path.join("idVendor")).unwrap_or_default();
        let product = fs::read_to_string(path.join("idProduct")).unwrap_or_default();
        if vendor.trim().eq_ignore_ascii_case("3564")
            && product.trim().eq_ignore_ascii_case("fef9")
        {
            let read = |name: &str| fs::read_to_string(path.join(name))
                .unwrap_or_else(|_| "unavailable".to_string()).trim().to_string();
            return Ok(format!(
                "USB identity: {} / {}\nManufacturer: {}\nProduct: {}\nUSB bcdDevice: {}\nUSB bus/address: {}/{}\n\nNote: bcdDevice is the USB device-release field. It is not confirmed to be the user-visible OBSBOT firmware version.",
                read("idVendor"), read("idProduct"), read("manufacturer"), read("product"),
                read("bcdDevice"), read("busnum"), read("devnum")
            ));
        }
    }
    Err("Tiny 2 Lite USB device (3564:fef9) is not currently visible to Linux.".into())
}

pub(crate) fn hex_dump(data: &[u8]) -> String {
    let mut output = String::new();
    for (offset, chunk) in data.chunks(16).enumerate() {
        output.push_str(&format!("{:04x}: ", offset * 16));
        for byte in chunk { output.push_str(&format!("{:02x} ", byte)); }
        output.push('\n');
    }
    output
}

pub(crate) fn decode_xu_status(data: &[u8; 60]) -> String {
    let sleep = match data[0x02] { 0x00 => "Awake", 0x01 => "Sleep", _ => "Unknown" };
    let hdr = if data[0x06] == 0 { "Off" } else { "On / non-zero" };
    let fov = match data[0x11] { 1 => "Wide", 2 => "Normal", v => {
        return format!("Sleep: {sleep}\nHDR: {hdr}\nFOV raw: 0x{v:02x} (unknown)\nAI tuple: {:02x}/{:02x}\nTracking speed: 0x{:02x}",
            data[0x18], data[0x1c], data[0x24]);
    }};
    let crop = match data[0x04] { 0x05 => "Wide crop code", 0x0f => "Normal crop code", _ => "Unmapped crop code" };
    let mode = ai_mode_label(data);
    let speed = match data[0x24] { 0x00 => "Standard", 0x02 => "Sport", 0x21 => "Sport (legacy)", _ => "Unknown" };
    format!(
        "Decoded fields (confidence varies by field):\nSleep: {sleep} [0x02]\nCrop/zoom-related code: {crop} (0x{:02x}) [0x04]\nHDR: {hdr} [0x06]\nFOV: {fov} [0x11]\nAI mode: {mode} [0x18/0x1c]\nTracking speed: {speed} [0x24]",
        data[0x04]
    )
}

pub(crate) fn decode_xu_command_response(data: &[u8; 60]) -> String {
    let record = &data[9..15];
    let position_record = record == [0x04, 0x0a, 0x44, 0x3a, 0x18, 0x00]
        || record == [0x02, 0x0a, 0x42, 0x29, 0x05, 0x00];
    if !position_record {
        return format!("No recognised position-like response record at bytes 9..14: {:02x?}\nOffset-20 values are not decoded for this response.", record);
    }
    let pan = f32::from_le_bytes(data[20..24].try_into().unwrap());
    let tilt = f32::from_le_bytes(data[24..28].try_into().unwrap());
    if pan.is_finite() && tilt.is_finite() {
        format!("Candidate position record: {:02x?}\nCandidate pan: {pan:.2}\nCandidate tilt: {tilt:.2}\n\nUnverified: these floats are position-like in prior captures, but live movement correlation is still required.",
            record)
    } else {
        format!("Recognised record, but non-finite float values at offset 20: {pan:?}, {tilt:?}")
    }
}

#[cfg(test)]
mod xu_protocol_tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn invalid_driver_menu_default_is_replaced_by_a_supported_entry() {
        let controls = parse_controls(
            "power_line_frequency (menu) : min=0 max=2 step=1 default=3 value=2\n\t0: Disabled\n\t1: 50 Hz\n\t2: 60 Hz\n"
        );
        assert_eq!(controls[0].default_value, 2);
        assert_eq!(controls[0].value, 2);
    }

    #[test]
    fn candidate_gimbal_position_records_decode_live_float_pair() {
        for signature in [
            [0x04, 0x0a, 0x44, 0x3a, 0x18, 0x00],
            [0x02, 0x0a, 0x42, 0x29, 0x05, 0x00],
        ] {
            let mut data = [0u8; 60];
            data[..6].copy_from_slice(&signature);
            data[20..24].copy_from_slice(&(-42.5f32).to_le_bytes());
            data[24..28].copy_from_slice(&(27.25f32).to_le_bytes());
            assert_eq!(decode_candidate_gimbal_position(&data).unwrap(), (-42.5, 27.25));
        }
        let mut unknown = [0u8; 60];
        assert!(decode_candidate_gimbal_position(&unknown).is_err());
        unknown[..6].copy_from_slice(&[0x04, 0x0a, 0x44, 0x3a, 0x18, 0x00]);
        unknown[20..24].copy_from_slice(&(f32::NAN).to_le_bytes());
        assert!(decode_candidate_gimbal_position(&unknown).is_err());
    }

    #[test]
    fn pan_tilt_absolute_readback_decodes_signed_little_endian_and_clamps() {
        let mut data = [0u8; 8];
        data[0..4].copy_from_slice(&(-160_i32 * 3600).to_le_bytes());
        data[4..8].copy_from_slice(&(91_i32 * 3600).to_le_bytes());
        assert_eq!(decode_pan_tilt_absolute(&data), (-150.0, 90.0));
        data[0..4].copy_from_slice(&(45_i32 * 3600).to_le_bytes());
        data[4..8].copy_from_slice(&(-30_i32 * 3600).to_le_bytes());
        assert_eq!(decode_pan_tilt_absolute(&data), (45.0, -30.0));
    }

    #[test]
    fn mirror_image_payload_matches_captured_on_off_commands() {
        let on = mirror_image_payload(true);
        let off = mirror_image_payload(false);
        assert_eq!(&on[..3], &[0x14, 0x01, 0x01]);
        assert_eq!(&off[..3], &[0x14, 0x01, 0x00]);
        assert!(on[3..].iter().all(|byte| *byte == 0));
        assert!(off[3..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn tracking_frame_matches_lite_capture() {
        let frame = obsbot_frame(
            0x0020,
            [0x0a, 0x04, 0xc4, 0x0c, 0x01, 0x00],
            [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00],
        );
        assert_eq!(&frame[..18], hex("aa2520000c00abcb0a04c40c0100e63f0000").as_slice());
    }

    #[test]
    fn gesture_frames_match_lite_captures() {
        let group = [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00];
        let off = obsbot_frame(0x0008, group, [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00]);
        let on = obsbot_frame(0x0009, group, [0x27, 0xff, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(&off[..18], hex("aa2508000c002b5a0a04c4300100e63f0000").as_slice());
        assert_eq!(&on[..18], hex("aa2509000c007a9f0a04c430010027ff0100").as_slice());
    }

    #[test]
    fn dynamic_zoom_direction_payload_matches_lite_capture() {
        let group = [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00];
        // As the SDK sends it: on 01, off 00, like the other gestures.
        assert_eq!(gesture_command(group, true), [0x27, 0xff, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(gesture_command(group, false), [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00]);

        // The capture's sequence values advance across commands, including
        // the separate Dynamic Zoom disable packet: 0x0f, 0x10, 0x11, 0x12.
        let sequence = AtomicU16::new(0x000e);
        assert_eq!(next_xu_sequence(&sequence), 0x000f);
        assert_eq!(next_xu_sequence(&sequence), 0x0010);
        assert_eq!(next_xu_sequence(&sequence), 0x0011);
        assert_eq!(next_xu_sequence(&sequence), 0x0012);
    }

    #[test]
    fn exposure_compensation_labels_match_verified_checkpoints() {
        assert_eq!(exposure_compensation_label(0), "-3.0");
        assert_eq!(exposure_compensation_label(9), "0.0");
        assert_eq!(exposure_compensation_label(15), "+2.0");
        assert_eq!(exposure_compensation_label(16), "+2.3");
        assert_eq!(exposure_compensation_label(18), "+3.0");
    }

    #[test]
    fn face_focus_setter_values_match_camera_readback_mapping() {
        let group = [0x0a, 0x02, 0x02, 0x36, 0x04, 0x00];
        assert_eq!(FaceFocusPriority::Face.payload(), hex("bffb0100").as_slice());
        assert_eq!(FaceFocusPriority::Global.payload(), hex("be070000").as_slice());

        // The frame carries the number 1 (Face) or 0 (Global) at offset 16, and its own CRC before it.
        // Written out from byte 14 these read bf fb 01 00 00 00 and be 07 00 00 00 00, the same CRCs as in
        // the sleep and wake frames, which is where the older notes' four bytes come from.
        let mut face_command = [0u8; 6];
        face_command[2..6].copy_from_slice(&FaceFocusPriority::Face.value());
        let face = obsbot_frame(0x0040, group, face_command);
        assert_eq!(&face[8..14], hex("0a0202360400").as_slice());
        assert_eq!(&face[14..20], hex("bffb01000000").as_slice());

        let mut global_command = [0u8; 6];
        global_command[2..6].copy_from_slice(&FaceFocusPriority::Global.value());
        let global = obsbot_frame(0x0041, group, global_command);
        assert_eq!(&global[8..14], hex("0a0202360400").as_slice());
        assert_eq!(&global[14..20], hex("be0700000000").as_slice());
    }

    #[test]
    fn a_reply_laid_out_like_a_command_frame_decodes_by_its_number() {
        for (value, expected) in [(1u32, FaceFocusPriority::Face), (0u32, FaceFocusPriority::Global)] {
            let mut reply = [0u8; 60];
            reply[..6].copy_from_slice(&hex("aa2916000c00"));
            reply[8..14].copy_from_slice(&hex("020ac2350400"));
            reply[16..20].copy_from_slice(&value.to_le_bytes());
            let payload_crc = crc16_usb(&reply[12..20]);
            reply[14..16].copy_from_slice(&payload_crc.to_le_bytes());
            let crc = crc16_usb(&reply[..12]);
            reply[6..8].copy_from_slice(&crc.to_le_bytes());
            assert_eq!(decode_face_focus_reply(&reply, 0x16).unwrap(), Some(expected));
            // The CRC in front of the number is the pair of bytes the earlier notes took for the value.
            assert_eq!(&reply[14..18], expected.payload().as_slice());
        }
    }

    #[test]
    fn face_focus_get_request_uses_header_only_get_opcode_and_full_header_crc() {
        let frame = face_focus_get_frame(0x0016);
        assert_eq!(&frame[..12], hex("aa0116000c00d6680a02c235").as_slice());
        let mut header = frame[..12].to_vec();
        header[6..8].fill(0);
        assert_eq!(u16::from_le_bytes([frame[6], frame[7]]), crc16_usb(&header));
    }

    #[test]
    fn face_focus_get_reply_decodes_both_camera_states() {
        for (payload, expected) in [
            (hex("bffb0100"), FaceFocusPriority::Face),
            (hex("be070000"), FaceFocusPriority::Global),
        ] {
            let mut reply = [0u8; 60];
            reply[..6].copy_from_slice(&hex("aa2916000c00"));
            reply[8..14].copy_from_slice(&hex("020ac2350400"));
            reply[14..18].copy_from_slice(&payload);
            let crc = crc16_usb(&reply[..12]);
            reply[6..8].copy_from_slice(&crc.to_le_bytes());
            assert_eq!(decode_face_focus_reply(&reply, 0x0016).unwrap(), Some(expected));
        }
    }

    #[test]
    fn face_focus_get_reply_rejects_stale_bad_crc_and_unknown_values() {
        let mut reply = [0u8; 60];
        reply[..6].copy_from_slice(&hex("aa2916000c00"));
        reply[8..14].copy_from_slice(&hex("020ac2350400"));
        reply[14..18].copy_from_slice(&hex("bffb0100"));
        let crc = crc16_usb(&reply[..12]);
        reply[6..8].copy_from_slice(&crc.to_le_bytes());

        assert_eq!(decode_face_focus_reply(&reply, 0x0017).unwrap(), None);
        reply[6] ^= 0x01;
        assert!(decode_face_focus_reply(&reply, 0x0016).is_err());
        reply[6] ^= 0x01;
        reply[14..18].copy_from_slice(&hex("01020304"));
        assert!(decode_face_focus_reply(&reply, 0x0016).is_err());
    }

    #[test]
    fn gesture_zoom_ratio_frame_matches_lite_capture() {
        let group = [0x0a, 0x04, 0x44, 0x32, 0x04, 0x00];
        let mut command = [0u8; 6];
        command[2..6].copy_from_slice(&1.0f32.to_le_bytes());
        let frame = obsbot_frame(0x0069, group, command);
        assert_eq!(&frame[..20], hex("aa2569000c0099200a04443204009fd70000803f").as_slice());
        assert!(set_gesture_zoom_ratio_payload_is_valid(1.0));
        assert!(set_gesture_zoom_ratio_payload_is_valid(4.0));
        assert!(!set_gesture_zoom_ratio_payload_is_valid(0.99));
        assert!(!set_gesture_zoom_ratio_payload_is_valid(4.01));
    }

    #[test]
    fn sleep_and_wake_frames_are_unchanged() {
        let group = [0x0a, 0x02, 0xc2, 0xa0, 0x04, 0x00];
        let wake = obsbot_frame(0x00a5, group, [0, 0, 0, 0, 0, 0]);
        let sleep = obsbot_frame(0x0042, group, [0, 0, 1, 0, 0, 0]);
        assert_eq!(&wake[..20], hex("aa25a5000c005fef0a02c2a00400be0700000000").as_slice());
        assert_eq!(&sleep[..20], hex("aa2542000c00ea630a02c2a00400bffb01000000").as_slice());
        assert!(wake[20..].iter().all(|byte| *byte == 0));
        assert!(sleep[20..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn auto_exposure_priority_payloads_match_the_sdk() {
        let face = auto_exposure_mode_payload(true);
        let global = auto_exposure_mode_payload(false);
        assert_eq!(face.len(), 60);
        assert_eq!(global.len(), 60);
        // OBSBOT's SDK (2026-11-24): cameraSetFaceAER(1) sends 03 01 01. The 2026-11-20 capture had them the
        // other way round.
        assert_eq!(&face[..3], &[0x03, 0x01, 0x01]);
        assert_eq!(&global[..3], &[0x03, 0x01, 0x00]);
        assert!(face[3..].iter().all(|byte| *byte == 0));
        assert!(global[3..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn auto_exposure_priority_readback_uses_response_byte_seven() {
        let mut status = [0u8; 60];
        status[2] = 0x01; // SET_CUR value is not the GET_CUR read-back field.
        status[7] = 0x01;
        assert_eq!(auto_exposure_mode_from_status(&status), Some(true));
        status[7] = 0x00;
        assert_eq!(auto_exposure_mode_from_status(&status), Some(false));
        // And the command, as the SDK sends it: 03 01 01 face, 03 01 00 global.
        assert_eq!(&auto_exposure_mode_payload(true)[..3], &[0x03, 0x01, 0x01]);
        assert_eq!(&auto_exposure_mode_payload(false)[..3], &[0x03, 0x01, 0x00]);
        status[7] = 0x02;
        assert_eq!(auto_exposure_mode_from_status(&status), None);
    }

    #[test]
    fn microphone_during_sleep_status_mapping_uses_byte_16() {
        let mut status = [0u8; 60];
        status[XU_AUDIO_MICROPHONE_DURING_SLEEP_OFFSET] = 0;
        assert_eq!(microphone_during_sleep_state(&status), Some(false));
        status[XU_AUDIO_MICROPHONE_DURING_SLEEP_OFFSET] = 1;
        assert_eq!(microphone_during_sleep_state(&status), Some(true));
        status[XU_AUDIO_MICROPHONE_DURING_SLEEP_OFFSET] = 2;
        assert_eq!(microphone_during_sleep_state(&status), None);
    }

    #[test]
    fn microphone_during_sleep_write_uses_captured_command_not_status_block() {
        for (enabled, expected) in [(false, [0x13, 0x01, 0x00]), (true, [0x13, 0x01, 0x01])] {
            let payload = microphone_during_sleep_payload(enabled);
            assert_eq!(&payload[..3], &expected);
        }
    }

    #[test]
    fn microphone_disable_mapping_matches_tiny_2_lite_capture() {
        assert_eq!(XU_AUDIO_MICROPHONE_DISABLED_OFFSET, 2);
        assert_eq!(microphone_disabled_wire_value(true), 0x00);
        assert_eq!(microphone_disabled_wire_value(false), 0x01);
    }

    #[test]
    fn radio_distance_payload_matches_tiny_2_lite_capture() {
        for (distance, expected) in [
            (0, [0x1b, 0x01, 0x00]),
            (1, [0x1b, 0x01, 0x01]),
            (2, [0x1b, 0x01, 0x02]),
        ] {
            let payload = radio_distance_payload(distance).unwrap();
            assert_eq!(&payload[..3], &expected);
            assert!(payload[3..].iter().all(|byte| *byte == 0));
        }
        assert_eq!(radio_distance_label(0), "Close");
        assert_eq!(radio_distance_label(1), "Standard");
        assert_eq!(radio_distance_label(2), "Far");
        assert!(radio_distance_payload(3).is_err());
    }

    #[test]
    fn audio_settings_use_capture_verified_status_bytes() {
        assert_eq!(XU_AUDIO_NOISE_REDUCTION_OFFSET, 8);
        assert_eq!(XU_AUDIO_AUTO_GAIN_OFFSET, 25);
        assert_eq!(audio_noise_reduction_label(0), "Off");
        assert_eq!(audio_noise_reduction_label(1), "Weak");
        assert_eq!(audio_noise_reduction_label(2), "Medium");
        assert_eq!(audio_noise_reduction_label(3), "Strong");
        assert_eq!(audio_noise_reduction_label(4), "Unknown");
    }

    #[test]
    fn audio_setting_patch_preserves_every_other_status_byte() {
        let original = [0x5a; 60];
        let patched = patch_xu_status_byte(original, XU_AUDIO_NOISE_REDUCTION_OFFSET, 3).unwrap();
        assert_eq!(patched[8], 3);
        for index in 0..60 {
            if index != XU_AUDIO_NOISE_REDUCTION_OFFSET {
                assert_eq!(patched[index], original[index], "byte {index} changed unexpectedly");
            }
        }
        assert!(patch_xu_status_byte(original, 60, 1).is_err());
    }

    #[test]
    fn hdr_payload_and_status_mapping_match_tiny_2_lite_capture() {
        assert_eq!(hdr_payload(true), [0x01, 0x01, 0x01]);
        assert_eq!(hdr_payload(false), [0x01, 0x01, 0x00]);

        let mut status = [0u8; 60];
        status[0x06] = 0x00;
        assert_eq!(hdr_state_from_status(&status), Some(false));
        status[0x06] = 0x01;
        assert_eq!(hdr_state_from_status(&status), Some(true));
        status[0x06] = 0xff;
        assert_eq!(hdr_state_from_status(&status), None);
    }

    #[test]
    fn led_setting_payload_matches_off_and_three_brightness_levels() {
        assert_eq!(led_brightness_payload(0).unwrap(), [0x1a, 0x01, 0x00]);
        assert_eq!(led_brightness_payload(1).unwrap(), [0x1a, 0x01, 0x01]);
        assert_eq!(led_brightness_payload(2).unwrap(), [0x1a, 0x01, 0x02]);
        assert_eq!(led_brightness_payload(3).unwrap(), [0x1a, 0x01, 0x03]);
        assert!(led_brightness_payload(4).is_err());
    }

    #[test]
    fn framed_query_and_live_pose_decode_match_wire_layout() {
        let frame = obsbot_query_frame(0x03, 0x0043, 0x1234);
        assert_eq!(&frame[..12], &[
            0xaa, 0x01, 0x34, 0x12, 0x0c, 0x00,
            frame[6], frame[7], 0x0a, 0x03, 0x43, 0x00,
        ]);
        let mut header = frame[..12].to_vec();
        header[6..8].fill(0);
        assert_eq!(&frame[6..8], &crc16_usb(&header).to_le_bytes());
        let mut reply = [0u8; 12];
        reply[8..10].copy_from_slice(&(1250i16).to_le_bytes());
        reply[10..12].copy_from_slice(&(-3400i16).to_le_bytes());
        assert_eq!(decode_live_gimbal_position(&reply).unwrap(), (-34.0, -12.5));
        assert!(decode_live_gimbal_position(&reply[..10]).is_err());
    }

    #[test]
    fn initial_pose_payload_matches_startup_command_layout_and_limits() {
        let payload = initial_pose_payload(3, 12.5, -8.0, 1.5).unwrap();
        assert_eq!(payload.len(), 24);
        assert_eq!(u32::from_le_bytes(payload[..4].try_into().unwrap()), 3);
        let values: Vec<f32> = payload[4..].chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(values, vec![12.5, 8.0, 0.0, 1.5, 0.0]);
        assert!(initial_pose_payload(3, 181.0, 0.0, 1.0).is_err());
        assert!(initial_pose_payload(3, 0.0, -181.0, 1.0).is_err());
        assert!(initial_pose_payload(3, 0.0, -103.4, 1.0).is_ok());
        assert!(initial_pose_payload(3, 0.0, 0.0, 4.1).is_err());
    }

    #[test]
    fn ai_mode_wire_payload_is_four_bytes() {
        let mut payload = [0u8; 4];
        payload[0] = 0x16;
        payload[1] = 0x02;
        payload[2] = 0x02;
        payload[3] = 0x01;
        assert_eq!(payload, [0x16, 0x02, 0x02, 0x01]);
    }
}

#[cfg(test)]
mod sim_gimbal_tests {
    use super::sim;

    // The simulated gimbal is process-wide, so one test walks through all the cases in order.
    #[test]
    fn the_pretend_gimbal_follows_absolute_moves_and_speeds_within_the_camera_limits() {
        // Absolute moves are in driver units: 3600 per degree.
        sim::apply_set_ctrl("--set-ctrl=pan_absolute=7200");
        sim::apply_set_ctrl("--set-ctrl=tilt_absolute=-3600");
        let (pan, tilt) = sim::position();
        assert!((pan - 2.0).abs() < 1e-6 && (tilt + 1.0).abs() < 1e-6, "{pan} {tilt}");

        // Moves beyond the range stop at the limit.
        sim::apply_set_ctrl("--set-ctrl=pan_absolute=99999999,tilt_absolute=-99999999");
        let (pan, tilt) = sim::position();
        assert_eq!((pan, tilt), (crate::backend::angles::PAN_RANGE.1, crate::backend::angles::TILT_RANGE.0));

        // A speed turns it steadily until a stop arrives, then it holds still.
        sim::apply_set_ctrl("--set-ctrl=pan_absolute=0,tilt_absolute=0");
        sim::apply_set_ctrl("--set-ctrl=pan_speed=80,tilt_speed=-40");
        std::thread::sleep(std::time::Duration::from_millis(120));
        let (moving_pan, moving_tilt) = sim::position();
        assert!(moving_pan > 1.0 && moving_tilt < -0.5, "{moving_pan} {moving_tilt}");
        sim::apply_set_ctrl("--set-ctrl=pan_speed=0,tilt_speed=0");
        let held = sim::position();
        std::thread::sleep(std::time::Duration::from_millis(60));
        let later = sim::position();
        assert!((held.0 - later.0).abs() < 1e-9 && (held.1 - later.1).abs() < 1e-9, "it kept moving after the stop");

        // Other options are ignored.
        sim::apply_set_ctrl("--get-ctrl=pan_absolute");
        assert_eq!(sim::position(), later);
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_command_that_never_answers_is_stopped() {
        let started = Instant::now();
        let result = run_with_timeout(Command::new("sleep").arg("30"), Duration::from_millis(300));
        assert!(result.unwrap_err().contains("did not answer"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_normal_command_returns_its_output_and_status() {
        let (ok, out, _) = run_with_timeout(Command::new("echo").arg("hello"), Duration::from_secs(5)).unwrap();
        assert!(ok && out.trim() == "hello");
        let (ok, _, _) = run_with_timeout(&mut Command::new("false"), Duration::from_secs(5)).unwrap();
        assert!(!ok);
        // Lots of output never blocks it.
        let (ok, out, _) = run_with_timeout(Command::new("sh").args(["-c", "yes x | head -c 300000"]), Duration::from_secs(5)).unwrap();
        assert!(ok && out.len() >= 300_000);
    }

    #[test]
    fn changing_the_capture_mode_is_given_longer_than_a_control() {
        assert!(v4l2_timeout("--set-fmt-video=width=1280,height=720,pixelformat=MJPG") > v4l2_timeout("--set-ctrl=brightness=1"));
        assert!(v4l2_timeout("--set-parm=30.000") > v4l2_timeout("--get-ctrl=pan_absolute"));
    }
}

#[cfg(test)]
mod settle_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_late_readback_is_waited_for_not_reported_as_a_failure() {
        let mut readings = vec!["Global", "Global", "Global", "Face"].into_iter();
        let result = settle_readback_with("Face", || Ok(readings.next().unwrap()), Duration::ZERO);
        assert_eq!(result, Ok("Face"));
    }

    #[test]
    fn a_camera_that_really_keeps_the_old_value_is_reported_as_it_is() {
        let mut calls = 0;
        let result = settle_readback_with(1, || { calls += 1; Ok(0) }, Duration::ZERO);
        assert_eq!(result, Ok(0));
        assert_eq!(calls, 7);
    }

    #[test]
    fn a_failed_readback_is_an_error_at_once() {
        let mut calls = 0;
        let result: Result<i32, String> = settle_readback_with(1, || { calls += 1; Err("no answer".into()) }, Duration::ZERO);
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    /// Frames captured from OBSBOT's SDK on a Tiny 2 Lite (2026-11-24):
    /// our frame builder must reproduce each one exactly (sequence, command and payload as captured).
    #[test]
    fn sdk_probe_frames_are_reproduced_exactly() {
        let cases: [(u16, u16, &[u8], &str); 8] = [
            (0x10, 0x0a44, &[1], "aa 25 10 00 0c 00 4a f6 0a 04 44 0a 01 00 27 ff 01"),
            (0x11, 0x0a44, &[0], "aa 25 11 00 0c 00 1b 33 0a 04 44 0a 01 00 e6 3f 00"),
            (0x12, 0x3b84, &[1], "aa 25 12 00 0c 00 7a e8 0a 04 84 3b 01 00 27 ff 01"),
            (0x1c, 0x5084, &[1], "aa 25 1c 00 0c 00 5a f2 0a 04 84 50 01 00 27 ff 01"),
            (0x14, 0x4c04, &(-30.0f32).to_le_bytes(), "aa 25 14 00 0c 00 bb 11 0a 04 04 4c 04 00 3b 97 00 00 f0 c1"),
            (0x16, 0x4c84, &30.0f32.to_le_bytes(), "aa 25 16 00 0c 00 7b 1b 0a 04 84 4c 04 00 3a 37 00 00 f0 41"),
            (0x18, 0x4d04, &(-10.0f32).to_le_bytes(), "aa 25 18 00 0c 00 ba ee 0a 04 04 4d 04 00 66 57 00 00 20 c1"),
            (0x1a, 0x4d84, &10.0f32.to_le_bytes(), "aa 25 1a 00 0c 00 7a e4 0a 04 84 4d 04 00 67 f7 00 00 20 41"),
        ];
        for (sequence, command, payload, captured) in cases {
            let expected: Vec<u8> = captured.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
            let frame = obsbot_frame_with_payload(sequence, command, payload).unwrap();
            assert_eq!(&frame[..expected.len()], &expected[..], "command 0x{command:04x}");
            assert!(frame[expected.len()..].iter().all(|b| *b == 0), "the rest of the frame is zeros");
        }
    }

    /// Round 3 of the SDK captures (undeclared functions): every frame reproduced byte for byte.
    #[test]
    fn round_three_frames_are_reproduced_exactly() {
        let cases: Vec<(u8, u16, u16, Vec<u8>, &str)> = vec![
            (0x02, 0x08, MIC_VOLUME_SET, 49i16.to_le_bytes().to_vec(), "aa 25 08 00 0c 00 58 8f 0a 02 02 81 02 00 eb 96 31 00"),
            (0x02, 0x05, CAMERA_SLEEP_SET, 600u32.to_le_bytes().to_vec(), "aa 25 05 00 0c 00 98 ad 0a 02 c2 a1 04 00 0c a7 58 02 00 00"),
            (0x12, 0x1c, LED_ENABLED_SET, vec![1], "aa 25 1c 00 0c 00 4d 7d 0a 12 cd 75 01 00 27 ff 01"),
            (0x02, 0x36, FOCUS_POINT_SET, focus_point_payload(0.5, 0.5).to_vec(),
                "aa 25 36 00 0c 00 58 32 0a 02 82 35 14 00 51 7b 00 00 00 3f 00 00 00 3f 00 00 00 00 00 00 00 00 64 00 00 00"),
            (0x04, 0x38, SELECT_TARGET_SET, select_target_payload(0.5, 0.5).to_vec(),
                "aa 25 38 00 0c 00 7a 72 0a 04 04 04 09 00 f5 22 00 00 00 3f 00 00 00 3f 00"),
        ];
        for (module, sequence, command, payload, captured) in cases {
            let expected: Vec<u8> = captured.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
            let frame = obsbot_frame_to(module, sequence, command, &payload).unwrap();
            assert_eq!(&frame[..expected.len()], &expected[..], "command 0x{command:04x}");
            assert!(frame[expected.len()..].iter().all(|b| *b == 0));
        }
    }

    /// The SDK's bytes (2026-11-24 capture): auto gain is the tagged `17 01 xx` command, and vertical tracking
    /// is command 0x0cc4 with 0 / 1 / 2.
    #[test]
    fn auto_gain_and_vertical_tracking_follow_the_sdk() {
        assert_eq!(&auto_gain_payload(true)[..3], &[0x17, 0x01, 0x01]);
        assert_eq!(&auto_gain_payload(false)[..3], &[0x17, 0x01, 0x00]);
        assert!(auto_gain_payload(true)[3..].iter().all(|b| *b == 0));
        // Captured: aa 25 .. 0a 04 c4 0c 01 00 <crc> 01 for Headroom (crc 27 ff for payload 01).
        let frame = obsbot_frame_with_payload(0x30, 0x0cc4, &[1]).unwrap();
        assert_eq!(&frame[8..17], &[0x0a, 0x04, 0xc4, 0x0c, 0x01, 0x00, 0x27, 0xff, 0x01]);
        assert_eq!(vertical_tracking_from_status(0x21), Some(2));
        assert_eq!(vertical_tracking_from_name("sport"), Some(2));
        assert_eq!(vertical_tracking_name(1), "headroom");
    }

    /// The tracking area's commands as captured from the SDK (2026-11-24), and our tilt convention turned into
    /// the camera's "down positive" pitch.
    #[test]
    fn tracking_area_frames_and_conventions() {
        // Pan -30..30 and tilt -10..10: yaw -30..30, pitch -10..10 (tilt negated and swapped).
        assert_eq!(tracking_area_to_camera((-30.0, 30.0), (-10.0, 10.0)), [-30.0, 30.0, -10.0, 10.0]);
        // Tilt -30..70 (down 30, up 70) is pitch -70..30.
        assert_eq!(tracking_area_to_camera((-140.0, 140.0), (-30.0, 70.0)), [-120.0, 120.0, -70.0, 30.0]);
        assert_eq!(tracking_area_from_camera([-100.0, 100.0, -70.0, 30.0]), ((-100.0, 100.0), (-30.0, 70.0)));
        // The frames, byte for byte, as the SDK sent them.
        let yaw_min = obsbot_frame_with_payload(0x14, ZONE_YAW_MIN_SET, &(-30.0f32).to_le_bytes()).unwrap();
        assert_eq!(&yaw_min[..20], &[0xaa, 0x25, 0x14, 0x00, 0x0c, 0x00, 0xbb, 0x11, 0x0a, 0x04, 0x04, 0x4c, 0x04, 0x00, 0x3b, 0x97, 0x00, 0x00, 0xf0, 0xc1]);
        // The return position written back as read (centre, zoom 1), byte for byte (sequence 0x21).
        let home = obsbot_frame_with_payload(0x21, ZONE_HOME_SET, &zone_home_payload(0.0, 0.0, 1.0)).unwrap();
        let captured = "aa 25 21 00 0c 00 4b ff 0a 04 84 4e 18 00 65 10 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 80 3f 00 00 00 00";
        let expected: Vec<u8> = captured.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
        assert_eq!(&home[..expected.len()], &expected[..]);
        // The start-up position reply the SDK decoded as yaw 27.64, pitch -22.45, zoom 1 (same layout).
        let boot = [0x03, 0, 0, 0, 0xb8, 0x1e, 0xdd, 0x41, 0x9a, 0x99, 0xb3, 0xc1, 0, 0, 0, 0, 0, 0, 0x80, 0x3f, 0, 0, 0, 0];
        let (pan, tilt, zoom) = zone_home_from_reply(&boot).unwrap();
        assert!((pan - 27.64).abs() < 0.01 && (tilt - 22.45).abs() < 0.01 && zoom == 1.0);
        assert_eq!(zone_home_from_reply(&zone_home_payload(-12.5, 20.0, 1.5)).unwrap(), (-12.5, 20.0, 1.5));
        // Start-up position and saved positions, byte for byte as the SDK wrote them (probe round 5).
        let boot = obsbot_frame_with_payload(0x11, BOOT_POSITION_SET, &initial_pose_payload(3, 27.64, 22.45, 1.0).unwrap()).unwrap();
        let captured = "aa 25 11 00 0c 00 9a e6 0a 04 44 38 18 00 5c e6 03 00 00 00 b8 1e dd 41 9a 99 b3 c1 00 00 00 00 00 00 80 3f 00 00 00 00";
        let expected: Vec<u8> = captured.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
        assert_eq!(&boot[..expected.len()], &expected[..]);
        let delete = obsbot_frame_with_payload(0x1f, PRESET_DELETE, &1u32.to_le_bytes()).unwrap();
        assert_eq!(&delete[..20], &[0xaa, 0x25, 0x1f, 0x00, 0x0c, 0x00, 0x6a, 0xd3, 0x0a, 0x04, 0x84, 0x39, 0x04, 0x00, 0xbf, 0xfb, 0x01, 0x00, 0x00, 0x00]);
        let mut add = 1u32.to_le_bytes().to_vec();
        // The stored floats exactly (yaw -17.3, pitch -31.4 as the camera holds them), then 0, zoom 1, 0.
        for value in [f32::from_bits(0xc18a6667), f32::from_bits(0xc1fb3333), 0.0, 1.0, 0.0] { add.extend_from_slice(&value.to_le_bytes()); }
        let add = obsbot_frame_with_payload(0x20, PRESET_ADD, &add).unwrap();
        let captured = "aa 25 20 00 0c 00 0a 1c 0a 04 44 39 18 00 c7 cf 01 00 00 00 67 66 8a c1 33 33 fb c1 00 00 00 00 00 00 80 3f 00 00 00 00";
        let expected: Vec<u8> = captured.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
        assert_eq!(&add[..expected.len()], &expected[..]);
        assert_eq!(PRESET_UPDATE, 0x3a04);
        // Gimbal auto-tracking off while the zone is set, as the SDK sent it (sequence 0x34).
        let gimbal = obsbot_frame_with_payload(0x34, ZONE_TRACK_GIMBAL_SET, &[0]).unwrap();
        assert_eq!(&gimbal[..17], &[0xaa, 0x25, 0x34, 0x00, 0x0c, 0x00, 0x8b, 0x88, 0x0a, 0x04, 0x44, 0x08, 0x01, 0x00, 0xe6, 0x3f, 0x00]);
        // Zone tracking on, byte for byte as the SDK sent it (sequence 0x51).
        let zone = obsbot_frame_with_payload(0x51, ZONE_TRACK_STATE_SET, &[1]).unwrap();
        assert_eq!(&zone[..17], &[0xaa, 0x25, 0x51, 0x00, 0x0c, 0x00, 0xf9, 0xe6, 0x0a, 0x04, 0xc4, 0x08, 0x01, 0x00, 0x27, 0xff, 0x01]);
        // Hand side, as the SDK sent it for Left: 04 2244, payload 01.
        let left = obsbot_frame_with_payload(0x30, HAND_SIDE_SET, &[1]).unwrap();
        assert_eq!(&left[8..17], &[0x0a, 0x04, 0x44, 0x22, 0x01, 0x00, 0x27, 0xff, 0x01]);
        // The hand-tracking area's yaw min, as the SDK sent it (sequence 0x22, -30°).
        let hand = obsbot_frame_with_payload(0x22, HAND_YAW_MIN_SET, &(-30.0f32).to_le_bytes()).unwrap();
        assert_eq!(&hand[..20], &[0xaa, 0x25, 0x22, 0x00, 0x0c, 0x00, 0x6a, 0x1c, 0x0a, 0x04, 0x44, 0x20, 0x04, 0x00, 0x3b, 0x97, 0x00, 0x00, 0xf0, 0xc1]);
        let on = obsbot_frame_with_payload(0x1c, ZONE_ENABLED_SET, &[1]).unwrap();
        assert_eq!(&on[..17], &[0xaa, 0x25, 0x1c, 0x00, 0x0c, 0x00, 0x5a, 0xf2, 0x0a, 0x04, 0x84, 0x50, 0x01, 0x00, 0x27, 0xff, 0x01]);
    }
}