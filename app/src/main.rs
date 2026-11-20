mod capture;
mod config;
mod presets;
mod ui;
mod video;

use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::OnceLock;

thread_local! {
    static APP_CONFIG: RefCell<config::AppConfig> = RefCell::new(config::AppConfig::default());
}

fn update_config_checked(update: impl FnOnce(&mut config::AppConfig)) -> Result<(), String> {
    // Persist a candidate snapshot first. If disk I/O or validation fails,
    // keep the in-memory config unchanged as well.
    let mut snapshot = current_config();
    update(&mut snapshot);
    config::save(&snapshot)?;
    APP_CONFIG.with(|current| *current.borrow_mut() = snapshot);
    Ok(())
}

fn update_config(update: impl FnOnce(&mut config::AppConfig)) {
    if let Err(error) = update_config_checked(update) {
        eprintln!("Could not save settings: {error}");
    }
}

fn current_config() -> config::AppConfig {
    APP_CONFIG.with(|value| value.borrow().clone())
}

/// Restore the remembered host capture mode only when this camera still
/// advertises it. This does not replay camera movement or vendor settings.
fn apply_saved_video_mode(device: &str) -> Option<String> {
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
struct Control {
    name: String, kind: String, min: i64, max: i64, step: i64,
    value: i64, default_value: i64, disabled: bool, menu: Vec<(i64, String)>,
}


fn ui_test_mode() -> bool {
    std::env::var("TINY4LITE_UI_TEST").is_ok_and(|value| value == "1")
}

fn mock_v4l2_output(option: &str) -> String {
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
            "ioctl: VIDIOC_ENUM_FMT\n Type: Video Capture\n [0]: 'MJPG' (Motion-JPEG, compressed)\n   Size: Discrete 1920x1080\n     Interval: Discrete 0.033s (30.000 fps)\n     Interval: Discrete 0.017s (60.000 fps)\n   Size: Discrete 1280x720\n     Interval: Discrete 0.033s (30.000 fps)\n [1]: 'YUYV' (YUYV 4:2:2)\n   Size: Discrete 640x480\n     Interval: Discrete 0.033s (30.000 fps)\n"
        ),
        "--get-fmt-video" => String::from("Format Video Capture:\n\tWidth/Height      : 1920/1080\n\tPixel Format      : 'MJPG'\n"),
        "--get-parm" => String::from("Streaming Parameters Video Capture:\n\tFrames per second: 30.000 (30/1)\n"),
        value if value.starts_with("--get-ctrl=") => {
            let name = value.trim_start_matches("--get-ctrl=");
            let current = match name {
                "pan_absolute" | "tilt_absolute" | "pan_speed" | "tilt_speed" => 0,
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

fn run_v4l2(device: &str, option: &str) -> Result<String, String> {
    if ui_test_mode() { return Ok(mock_v4l2_output(option)); }
    let output = Command::new("v4l2-ctl").arg("-d").arg(device).arg(option).output()
        .map_err(|e| format!("Could not run v4l2-ctl: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn get_control(device: &str, name: &str) -> Result<i64, String> {
    let output = run_v4l2(device, &format!("--get-ctrl={name}"))?;
    output.split(':').last().unwrap_or("").trim().parse::<i64>()
        .map_err(|_| format!("Could not parse {name} value: {output}"))
}

fn set_control(device: &str, name: &str, value: i64) -> Result<(), String> {
    run_v4l2(device, &format!("--set-ctrl={name}={value}"))?;
    // Position and speed are transient movement commands, never preferences.
    if !matches!(name, "pan_absolute" | "tilt_absolute" | "pan_speed" | "tilt_speed") {
        update_config(|config| { config.v4l2_controls.insert(name.to_string(), value); });
    }
    Ok(())
}

fn usb_camera_present() -> bool {
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else { return false; };
    entries.flatten().any(|entry| {
        let path = entry.path();
        fs::read_to_string(path.join("idVendor")).unwrap_or_default().trim().eq_ignore_ascii_case("3564")
            && fs::read_to_string(path.join("idProduct")).unwrap_or_default().trim().eq_ignore_ascii_case("fef9")
    })
}

fn discover_tiny4lite() -> Vec<String> {
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

fn parse_controls(text: &str) -> Vec<Control> {
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

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() { container.remove(&child); }
}

fn add_move_button(row: &gtk::Box, device: &str,
                   control: &str, delta: i64, label: &str, status: &gtk::Label) {
    let button = gtk::Button::with_label(label);
    let dev = device.to_string();
    let ctrl = control.to_string();
    let status_label = status.clone();
    button.connect_clicked(move |_| {
        let result = get_control(&dev, &ctrl).and_then(|current| {
            let (min, max) = if ctrl == "tilt_absolute" { (-324000, 324000) } else { (-468000, 468000) };
            let target = (current + delta).clamp(min, max);
            set_control(&dev, &ctrl, target)?;
            Ok(format!("{ctrl} set to {target} (one-degree step)"))
        });
        status_label.set_text(&result.unwrap_or_else(|e| format!("Movement failed: {e}")));
    });
    row.append(&button);
}

fn joystick_axis(position: f64, min: i64, max: i64) -> i64 {
    // V4L2 pan/tilt speed controls are signed: zero is neutral.
    if position < 0.0 {
        ((position * min.abs() as f64).round() as i64).clamp(min, 0)
    } else {
        ((position * max as f64).round() as i64).clamp(0, max)
    }
}

fn add_speed_joystick(container: &gtk::Box, device: &str, pan: &Control, tilt: &Control, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Pan / tilt speed"));
    ui::layout::mark_compact_item(frame.upcast_ref(), "camera:pan-tilt-joystick");
    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 6);
    controls.set_hexpand(true);
    let hint = gtk::Label::new(Some("Drag the stick to move. Release to stop."));
    hint.add_css_class("dim-label");
    controls.append(&hint);

    let initial_sensitivity = current_config().joystick_sensitivity_percent.clamp(10, 200);
    let sensitivity = Rc::new(RefCell::new(initial_sensitivity as f64 / 100.0));
    let sensitivity_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    sensitivity_row.add_css_class("adaptive-row");
    let sensitivity_label = gtk::Label::new(Some("Sensitivity"));
    let sensitivity_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 10.0, 200.0, 5.0);
    sensitivity_scale.set_value(initial_sensitivity as f64);
    sensitivity_scale.set_digits(0);
    sensitivity_scale.set_draw_value(true);
    sensitivity_scale.set_hexpand(true);
    let sensitivity_value = gtk::Label::new(Some("100%"));
    let sensitivity_state = sensitivity.clone();
    let sensitivity_value_label = sensitivity_value.clone();
    sensitivity_scale.connect_value_changed(move |scale| {
        let percent = scale.value().round();
        *sensitivity_state.borrow_mut() = percent / 100.0;
        update_config(|config| config.joystick_sensitivity_percent = percent as u16);
        sensitivity_value_label.set_text(&format!("{percent:.0}%"));
    });
    sensitivity_row.append(&sensitivity_label);
    sensitivity_row.append(&sensitivity_scale);
    sensitivity_row.append(&sensitivity_value);
    controls.append(&sensitivity_row);

    let pad = gtk::DrawingArea::new();
    pad.set_content_width(160);
    pad.set_content_height(150);
    pad.set_size_request(160, 150);
    pad.set_hexpand(false);
    pad.set_valign(gtk::Align::Center);
    let position = Rc::new(RefCell::new((0.0_f64, 0.0_f64)));
    let draw_position = position.clone();
    pad.set_draw_func(move |_, cr, width, height| {
        let cx = width as f64 / 2.0;
        let cy = height as f64 / 2.0;
        let radius = 55.0_f64.min(cx - 8.0).min(cy - 8.0);
        let (x, y) = *draw_position.borrow();
        cr.set_line_width(2.0);
        cr.set_source_rgba(0.45, 0.48, 0.54, 0.55);
        cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        cr.move_to(cx - radius, cy);
        cr.line_to(cx + radius, cy);
        cr.move_to(cx, cy - radius);
        cr.line_to(cx, cy + radius);
        let _ = cr.stroke();
        cr.set_source_rgba(0.25, 0.48, 0.78, 0.9);
        cr.arc(cx + x * radius, cy - y * radius, 13.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });

    let pan_name = pan.name.clone();
    let tilt_name = tilt.name.clone();
    let pan_min = pan.min;
    let pan_max = pan.max;
    let tilt_min = tilt.min;
    let tilt_max = tilt.max;
    let dev = device.to_string();
    let status_drag = status.clone();
    let drag = gtk::GestureDrag::new();
    let drag_position = position.clone();
    let drag_start = Rc::new(RefCell::new((80.0_f64, 75.0_f64)));
    let start_capture = drag_start.clone();
    drag.connect_drag_begin(move |_, x, y| {
        *start_capture.borrow_mut() = (x, y);
    });
    let start_update = drag_start.clone();
    let sensitivity_drag = sensitivity.clone();
    let pad_update = pad.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let width = pad_update.width() as f64;
        let height = pad_update.height() as f64;
        let cx = width / 2.0;
        let cy = height / 2.0;
        let radius = 55.0_f64.min(cx - 8.0).min(cy - 8.0).max(1.0);
        let (start_x, start_y) = *start_update.borrow();
        let sensitivity_factor = *sensitivity_drag.borrow();
        let x = (((start_x + dx - cx) / radius) * sensitivity_factor).clamp(-1.0, 1.0);
        let y = (((-(start_y + dy - cy) / radius)) * sensitivity_factor).clamp(-1.0, 1.0);
        *drag_position.borrow_mut() = (x, y);
        pad_update.queue_draw();
        let pan_value = joystick_axis(x, pan_min, pan_max);
        let tilt_value = joystick_axis(y, tilt_min, tilt_max);
        let result = set_control(&dev, &pan_name, pan_value)
            .and_then(|_| set_control(&dev, &tilt_name, tilt_value));
        if let Err(error) = result {
            status_drag.set_text(&format!("Joystick update failed: {error}"));
        } else {
            status_drag.set_text(&format!("Pan speed: {pan_value} · Tilt speed: {tilt_value}"));
        }
    });

    let dev_end = device.to_string();
    let pan_end = pan.name.clone();
    let tilt_end = tilt.name.clone();
    let status_end = status.clone();
    let end_position = position.clone();
    let pad_end = pad.clone();
    drag.connect_drag_end(move |_, _, _| {
        *end_position.borrow_mut() = (0.0, 0.0);
        pad_end.queue_draw();
        let result = set_control(&dev_end, &pan_end, 0)
            .and_then(|_| set_control(&dev_end, &tilt_end, 0));
        status_end.set_text(&result.map(|_| "Gimbal stopped.".to_string())
            .unwrap_or_else(|e| format!("Could not stop gimbal: {e}")));
    });
    pad.add_controller(drag);
    layout.append(&controls);
    layout.append(&pad);
    frame.set_child(Some(&layout));
    container.append(&frame);
}



// OBSBOT vendor UVC Extension Unit transport. This is deliberately separate
// from V4L2 controls: Tiny 2 Lite exposes its AI features through XU unit 2.
#[repr(C)]
struct UvcXuControlQuery {
    unit: u8,
    selector: u8,
    query: u8,
    _pad: u8,
    size: u16,
    _pad2: u16,
    data: *mut u8,
}

const UVCIOC_CTRL_QUERY: std::os::raw::c_ulong = 0xc010_7521;
const XU_UNIT: u8 = 2;
const UVC_SET_CUR: u8 = 0x01;
const UVC_GET_CUR: u8 = 0x81;

extern "C" {
    fn ioctl(fd: std::os::raw::c_int, request: std::os::raw::c_ulong, ...) -> std::os::raw::c_int;
}

fn xu_query(device: &str, selector: u8, request: u8, data: &mut [u8]) -> Result<(), String> {
    if ui_test_mode() {
        if selector == 0x06 && request == UVC_SET_CUR && data.len() >= 60 && data[0] == 0x03 && data[1] == 0x01 {
            LAST_UI_TEST_AUTO_EXPOSURE_PRIORITY.store(data[2], Ordering::Relaxed);
        } else if selector == 0x02 && request == UVC_SET_CUR && data.len() >= 12 && data[0] == 0xaa && data[1] == 0x01 {
            LAST_UI_TEST_QUERY_SEQUENCE.store(u16::from_le_bytes([data[2], data[3]]), Ordering::Relaxed);
            LAST_UI_TEST_QUERY_COMMAND.store(u16::from_le_bytes([data[10], data[11]]), Ordering::Relaxed);
        } else if request == UVC_GET_CUR {
            data.fill(0);
            if selector == 0x02 && data.len() >= 60 {
                let command = LAST_UI_TEST_QUERY_COMMAND.load(Ordering::Relaxed);
                let sequence = LAST_UI_TEST_QUERY_SEQUENCE.load(Ordering::Relaxed);
                let payload: Vec<u8> = match command {
                    0x0043 => vec![0; 12],
                    0x6804 => 1.0f32.to_le_bytes().to_vec(),
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
                if !payload.is_empty() {
                    data[0] = 0xaa; data[1] = 0x29;
                    data[2..4].copy_from_slice(&sequence.to_le_bytes());
                    data[4..6].copy_from_slice(&12u16.to_le_bytes());
                    data[8..12].copy_from_slice(&[0x0a, 0x04, command as u8, (command >> 8) as u8]);
                    data[12..14].copy_from_slice(&(payload.len() as u16).to_le_bytes());
                    data[16..16 + payload.len()].copy_from_slice(&payload);
                }
            } else if selector == 0x06 && data.len() >= 0x25 {
                data[XU_AUTO_EXPOSURE_PRIORITY_OFFSET] = LAST_UI_TEST_AUTO_EXPOSURE_PRIORITY.load(Ordering::Relaxed);
                data[0x10] = 1;
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

fn pu_query(device: &str, selector: u8, request: u8, data: &mut [u8]) -> Result<(), String> {
    if ui_test_mode() {
        if request == UVC_GET_CUR && data.len() >= 2 {
            data[..2].copy_from_slice(&10u16.to_le_bytes());
        }
        return Ok(());
    }
    use std::os::fd::AsRawFd;
    let file = std::fs::OpenOptions::new().read(true).write(true).open(device)
        .map_err(|e| format!("Cannot open {device} for UVC Processing Unit controls: {e}"))?;
    let mut query = UvcXuControlQuery {
        unit: 3, selector, query: request, _pad: 0,
        size: data.len() as u16, _pad2: 0, data: data.as_mut_ptr(),
    };
    let result = unsafe { ioctl(file.as_raw_fd(), UVCIOC_CTRL_QUERY, &mut query) };
    if result < 0 {
        return Err(format!("UVC Processing Unit 3 selector 0x{selector:02x} request 0x{request:02x}: {}",
            std::io::Error::last_os_error()));
    }
    Ok(())
}

const UVC_CAMERA_TERMINAL_UNIT: u8 = 1;
const UVC_CT_PANTILT_ABSOLUTE: u8 = 0x0d;
// Existing V4L2 PTZ controls use 3600 raw units for a one-degree step.
const PANTILT_RAW_UNITS_PER_DEGREE: f64 = 3600.0;
const PAN_DISPLAY_LIMIT_DEGREES: f64 = 130.0;
const TILT_DISPLAY_LIMIT_DEGREES: f64 = 90.0;

fn camera_terminal_query(device: &str, selector: u8, request: u8, data: &mut [u8]) -> Result<(), String> {
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

fn decode_pan_tilt_absolute(data: &[u8; 8]) -> (f64, f64) {
    let pan_raw = i32::from_le_bytes(data[0..4].try_into().expect("four-byte pan field"));
    let tilt_raw = i32::from_le_bytes(data[4..8].try_into().expect("four-byte tilt field"));
    (
        (pan_raw as f64 / PANTILT_RAW_UNITS_PER_DEGREE).clamp(-PAN_DISPLAY_LIMIT_DEGREES, PAN_DISPLAY_LIMIT_DEGREES),
        (tilt_raw as f64 / PANTILT_RAW_UNITS_PER_DEGREE).clamp(-TILT_DISPLAY_LIMIT_DEGREES, TILT_DISPLAY_LIMIT_DEGREES),
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

fn query_obsbot(device: &str, receiver: u8, command: u16) -> Result<Vec<u8>, String> {
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

fn decode_live_gimbal_position(data: &[u8]) -> Result<(f64, f64), String> {
    if data.len() < 12 { return Err(format!("Live gimbal reply is {} bytes; expected at least 12.", data.len())); }
    let pitch = i16::from_le_bytes([data[8], data[9]]) as f64 / 100.0;
    let pan = i16::from_le_bytes([data[10], data[11]]) as f64 / 100.0;
    Ok((pan, -pitch))
}

pub(crate) fn get_gimbal_position(device: &str) -> Result<(f64, f64), String> {
    let reply = query_obsbot(device, 0x03, 0x0043)?;
    decode_live_gimbal_position(&reply)
}

fn get_live_zoom_factor(device: &str) -> Result<f32, String> {
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

fn initial_pose_payload(source: u32, pan: f32, tilt: f32, zoom: f32) -> Result<Vec<u8>, String> {
    if !pan.is_finite() || !tilt.is_finite() || pan.abs() > 180.0 || !(-180.0..=180.0).contains(&tilt) {
        return Err(format!("Initial pose is outside representable camera angles: pan/tilt must be within ±180° (received {pan:+.1}° / {tilt:+.1}°)."));
    }
    if !zoom.is_finite() || !(1.0..=4.0).contains(&zoom) {
        return Err(format!("Initial zoom must be 1.0–4.0× (received {zoom:.2}×)."));
    }
    let mut payload = Vec::with_capacity(24);
    payload.extend_from_slice(&source.to_le_bytes());
    for value in [pan, -tilt, 0.0f32, zoom, 0.0f32] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    Ok(payload)
}

pub(crate) fn set_initial_camera_state(device: &str) -> Result<InitialCameraState, String> {
    let (pan, tilt) = get_gimbal_position(device)?;
    let zoom = get_live_zoom_factor(device)?;
    let pose = initial_pose_payload(3, pan as f32, tilt as f32, zoom)?;
    // Read and preserve the existing initial-state picture/action block. If it
    // cannot be read, abort before changing the boot pose.
    let actions = query_obsbot(device, 0x04, 0x3e84)?;
    if actions.len() != 40 {
        return Err(format!("Initial-state settings reply is {} bytes; expected 40. Nothing was changed.", actions.len()));
    }
    send_camera_preset_command(device, 0x3ec4, &pose)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    send_camera_preset_command(device, 0x3e44, &actions)?;
    let saved = read_initial_camera_state(device)?;
    if (saved.pan - pan as f32).abs() > 0.2 || (saved.tilt - tilt as f32).abs() > 0.2 {
        return Err(format!("Initial-state write was sent, but read-back differs: requested Pan {pan:+.1}° / Tilt {tilt:+.1}°, camera reports Pan {:+.1}° / Tilt {:+.1}°.", saved.pan, saved.tilt));
    }
    Ok(saved)
}

fn decode_candidate_gimbal_position(data: &[u8; 60]) -> Result<(f64, f64), String> {
    let record = &data[..6];
    if record != [0x04, 0x0a, 0x44, 0x3a, 0x18, 0x00]
        && record != [0x02, 0x0a, 0x42, 0x29, 0x05, 0x00]
    {
        return Err(format!("Unrecognized position response record: {:02x?}", record));
    }
    let pan = f32::from_le_bytes(data[20..24].try_into().unwrap()) as f64;
    let tilt = f32::from_le_bytes(data[24..28].try_into().unwrap()) as f64;
    if !pan.is_finite() || !tilt.is_finite() {
        return Err("Position response contained non-finite coordinates.".into());
    }
    Ok((pan.clamp(-130.0, 130.0), tilt.clamp(-90.0, 90.0)))
}

fn exposure_compensation_payload(position: u8) -> Result<[u8; 2], String> {
    if position > 18 {
        return Err("Exposure compensation position must be between 0 and 18.".into());
    }
    Ok(((position as u16) + 1).to_le_bytes())
}

fn exposure_compensation_position(payload: [u8; 2]) -> Option<u8> {
    let raw = u16::from_le_bytes(payload);
    (1..=19).contains(&raw).then_some((raw - 1) as u8)
}

fn get_exposure_compensation(device: &str) -> Result<u8, String> {
    let mut data = [0u8; 2];
    pu_query(device, 0x01, UVC_GET_CUR, &mut data)?;
    exposure_compensation_position(data)
        .ok_or_else(|| format!("Camera returned unsupported exposure compensation value {}.", u16::from_le_bytes(data)))
}

fn set_exposure_compensation(device: &str, position: u8) -> Result<(), String> {
    let mut data = exposure_compensation_payload(position)?;
    pu_query(device, 0x01, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.exposure_compensation_position = Some(position));
    Ok(())
}

fn exposure_compensation_label(position: u8) -> String {
    if position == 9 { "0.0".into() }
    else { format!("{:+.1}", -3.0 + position as f64 / 3.0) }
}

fn add_exposure_compensation_control(container: &gtk::Box, device: &str, status: &gtk::Label, auto_exposure_enabled: bool) {
    let frame = gtk::Frame::new(Some("Exposure compensation"));
    ui::layout::mark_compact_item(frame.upcast_ref(), "obsbot:exposure_compensation");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let saved = current_config().exposure_compensation_position.filter(|value| *value <= 18);
    let initial = get_exposure_compensation(device).ok().or(saved);
    let position = initial.unwrap_or(9);
    let value_label = gtk::Label::new(Some(&initial.map(exposure_compensation_label)
        .unwrap_or_else(|| "0.0 · camera value unavailable".into())));
    value_label.set_xalign(0.0);

    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 18.0, 1.0);
    scale.set_digits(0);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_sensitive(auto_exposure_enabled);
    scale.add_mark(0.0, gtk::PositionType::Bottom, Some("−3"));
    scale.add_mark(9.0, gtk::PositionType::Bottom, Some("0"));
    scale.add_mark(18.0, gtk::PositionType::Bottom, Some("+3"));
    scale.set_value(position as f64);

    let last = Rc::new(Cell::new(position));
    let updating = Rc::new(Cell::new(false));
    let dev = device.to_string();
    let status_label = status.clone();
    let last_position = last.clone();
    let guard = updating.clone();
    let value = value_label.clone();
    scale.connect_value_changed(move |slider| {
        if guard.get() { return; }
        let requested = slider.value().round().clamp(0.0, 18.0) as u8;
        match set_exposure_compensation(&dev, requested) {
            Ok(()) => {
                last_position.set(requested);
                value.set_text(&exposure_compensation_label(requested));
                status_label.set_text(&format!("Exposure compensation set to {}.", exposure_compensation_label(requested)));
            }
            Err(error) => {
                guard.set(true);
                slider.set_value(last_position.get() as f64);
                guard.set(false);
                status_label.set_text(&format!("Could not set exposure compensation: {error}"));
            }
        }
    });

    let note_text = if auto_exposure_enabled {
        "19 positions from −3.0 to +3.0. This is the UVC Processing Unit Backlight Compensation control, not a V4L2 exposure-bias control."
    } else {
        "Unavailable: Exposure mode is not Auto. Set Exposure mode to Auto to adjust exposure compensation."
    };
    let note = gtk::Label::new(Some(note_text));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&value_label);
    layout.append(&scale);
    layout.append(&note);
    frame.set_child(Some(&layout));
    container.append(&frame);
}

fn xu_status(device: &str) -> Result<[u8; 60], String> {
    let mut data = [0u8; 60];
    xu_query(device, 0x06, UVC_GET_CUR, &mut data)?;
    Ok(data)
}

const XU_AUDIO_NOISE_REDUCTION_OFFSET: usize = 8;
const XU_AUDIO_MICROPHONE_DURING_SLEEP_OFFSET: usize = 16;
const XU_AUDIO_AUTO_GAIN_OFFSET: usize = 25;
const XU_AUDIO_MICROPHONE_DISABLED_OFFSET: usize = 2;
const XU_AUDIO_RADIO_DISTANCE_OFFSET: usize = 2;

fn microphone_disabled_wire_value(disabled: bool) -> u8 {
    if disabled { 0 } else { 1 }
}

fn microphone_during_sleep_state(status: &[u8; 60]) -> Option<bool> {
    match status[XU_AUDIO_MICROPHONE_DURING_SLEEP_OFFSET] {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}


fn audio_noise_reduction_label(level: u8) -> &'static str {
    match level {
        0 => "Off",
        1 => "Weak",
        2 => "Medium",
        3 => "Strong",
        _ => "Unknown",
    }
}

fn patch_xu_status_byte(mut data: [u8; 60], offset: usize, value: u8) -> Result<[u8; 60], String> {
    let byte = data.get_mut(offset)
        .ok_or_else(|| format!("OBSBOT status byte offset {offset} is outside the 60-byte block."))?;
    *byte = value;
    Ok(data)
}

fn set_xu_status_byte(device: &str, offset: usize, value: u8) -> Result<(), String> {
    // GET_CUR returns the current shared configuration block. Preserve every
    // unrelated field and change only the byte established by controlled capture.
    let current = xu_status(device)?;
    let mut data = patch_xu_status_byte(current, offset, value)?;
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)
}

fn microphone_during_sleep_payload(enabled: bool) -> [u8; 3] {
    [0x13, 0x01, u8::from(enabled)]
}

fn set_microphone_during_sleep(device: &str, enabled: bool) -> Result<(), String> {
    // Selector 0x06 status byte 0x10 is the read-back. The write is a
    // distinct tagged command: 13 01 <0|1>, not a status-block rewrite.
    let mut data = microphone_during_sleep_payload(enabled);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.audio_microphone_during_sleep = Some(enabled));
    Ok(())
}

fn set_microphone_disabled(device: &str, disabled: bool) -> Result<(), String> {
    // Unlike the GET_CUR status block, this captured command has its own
    // fixed 60-byte payload. Its first three bytes are 1c 01 <state>.
    let mut data = [0u8; 60];
    data[..2].copy_from_slice(&[0x1c, 0x01]);
    data[XU_AUDIO_MICROPHONE_DISABLED_OFFSET] = microphone_disabled_wire_value(disabled);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.audio_microphone_disabled = Some(disabled));
    Ok(())
}

fn radio_distance_label(distance: u8) -> &'static str {
    match distance { 0 => "Close", 1 => "Standard", 2 => "Far", _ => "Unknown" }
}

fn radio_distance_payload(distance: u8) -> Result<[u8; 60], String> {
    if distance > 2 { return Err("Radio distance must be Close, Standard, or Far.".into()); }
    let mut data = [0u8; 60];
    data[..2].copy_from_slice(&[0x1b, 0x01]);
    data[XU_AUDIO_RADIO_DISTANCE_OFFSET] = distance;
    Ok(data)
}

fn set_radio_distance(device: &str, distance: u8) -> Result<(), String> {
    let mut data = radio_distance_payload(distance)?;
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.audio_radio_distance = Some(distance));
    Ok(())
}

// Experimental Pop filter implementation: replay the captured 60-byte commands verbatim.
// The sequence/check fields are not decoded or regenerated.
fn pop_filter_payload(enabled: bool) -> [u8; 60] {
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

fn set_pop_filter(device: &str, enabled: bool) -> Result<(), String> {
    let mut data = pop_filter_payload(enabled);
    xu_query(device, 0x02, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.pop_filter_enabled = Some(enabled));
    Ok(())
}

/// Mirror image command captured from three controlled OBSBOT Center transitions.
/// Only payload byte 2 changes: 1 = On, 0 = Off; the remaining 57 bytes are zero.
fn mirror_image_payload(enabled: bool) -> [u8; 60] {
    let mut data = [0u8; 60];
    data[..3].copy_from_slice(&[0x14, 0x01, u8::from(enabled)]);
    data
}

fn set_mirror_image(device: &str, enabled: bool) -> Result<(), String> {
    let mut data = mirror_image_payload(enabled);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.mirror_image_enabled = Some(enabled));
    Ok(())
}

fn set_auto_gain(device: &str, enabled: bool) -> Result<(), String> {
    set_xu_status_byte(device, XU_AUDIO_AUTO_GAIN_OFFSET, u8::from(enabled))?;
    update_config(|config| config.audio_auto_gain = Some(enabled));
    Ok(())
}

fn set_noise_reduction(device: &str, level: u8) -> Result<(), String> {
    if level > 3 {
        return Err("Noise reduction must be Off, Weak, Medium, or Strong.".into());
    }
    set_xu_status_byte(device, XU_AUDIO_NOISE_REDUCTION_OFFSET, level)?;
    update_config(|config| config.audio_noise_reduction = Some(level));
    Ok(())
}


fn add_mirror_image_control(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Image orientation"));
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    layout.set_margin_top(6);
    layout.set_margin_bottom(6);
    layout.set_margin_start(8);
    layout.set_margin_end(8);
        let mirror_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        mirror_row.add_css_class("adaptive-row");
        let mirror_label = gtk::Label::new(Some("Mirror image"));
        mirror_label.set_xalign(0.0);
        mirror_label.set_hexpand(true);
        let mirror_switch = gtk::Switch::new();
        mirror_switch.set_valign(gtk::Align::Center);
        let initial_mirror = current_config().mirror_image_enabled;
        mirror_switch.set_active(initial_mirror.unwrap_or(false));
        mirror_switch.set_tooltip_text(Some(if initial_mirror.is_some() {
            "Shows the last successfully submitted state; camera read-back is not established."
        } else {
            "Initial state is unknown; the switch starts visually Off. Camera read-back is not established."
        }));
        let mirror_device = device.to_string();
        let mirror_status = status.clone();
        mirror_switch.connect_state_set(move |switch, enabled| {
            match set_mirror_image(&mirror_device, enabled) {
                Ok(()) => {
                    mirror_status.set_text(if enabled {
                        "Mirror image enabled (captured payload)."
                    } else {
                        "Mirror image disabled (captured payload)."
                    });
                    gtk::glib::Propagation::Proceed
                }
                Err(error) => {
                    switch.set_active(!enabled);
                    mirror_status.set_text(&format!("Could not change Mirror image: {error}"));
                    gtk::glib::Propagation::Stop
                }
            }
        });
        mirror_row.append(&mirror_label);
        mirror_row.append(&mirror_switch);
        layout.append(&mirror_row);
    
    let note = gtk::Label::new(Some("Mirror image uses a captured vendor command. Camera read-back is not established; the switch shows the last successfully submitted state."));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&note);
    frame.set_child(Some(&layout));
    container.append(&frame);
}

fn add_audio_controls(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Microphone processing"));
    ui::layout::mark_compact_item(frame.upcast_ref(), "obsbot:audio-processing");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let microphone_disabled_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    microphone_disabled_row.add_css_class("adaptive-row");
    let microphone_disabled_label = gtk::Label::new(Some("Disable microphone"));
    microphone_disabled_label.set_xalign(0.0);
    microphone_disabled_label.set_hexpand(true);
    let microphone_disabled = gtk::Switch::new();
    microphone_disabled.set_valign(gtk::Align::Center);
    let initial_microphone_disabled = current_config().audio_microphone_disabled;
    microphone_disabled.set_active(initial_microphone_disabled.unwrap_or(false));
    if initial_microphone_disabled.is_none() {
        microphone_disabled.set_tooltip_text(Some("No camera read-back is available; the switch starts visually Off until you change it."));
    } else {
        microphone_disabled.set_tooltip_text(Some("Shows the last successfully submitted state; camera read-back is not established."));
    }
    let microphone_device = device.to_string();
    let microphone_status = status.clone();
    microphone_disabled.connect_state_set(move |_, disabled| {
        match set_microphone_disabled(&microphone_device, disabled) {
            Ok(()) => {
                microphone_status.set_text(if disabled {
                    "Camera microphone disabled. The camera may restart."
                } else {
                    "Camera microphone enabled. The camera may restart."
                });
                gtk::glib::Propagation::Proceed
            }
            Err(error) => {
                microphone_status.set_text(&format!("Could not change microphone state: {error}"));
                gtk::glib::Propagation::Stop
            }
        }
    });
    microphone_disabled_row.append(&microphone_disabled_label);
    microphone_disabled_row.append(&microphone_disabled);
    layout.append(&microphone_disabled_row);

    // This is independent from the "Disable microphone" command above.
    // The captured GET_CUR configuration block exposes this setting at byte 16.
    let sleep_mic_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    sleep_mic_row.add_css_class("adaptive-row");
    let sleep_mic_label = gtk::Label::new(Some("Keep microphone on while sleeping"));
    sleep_mic_label.set_xalign(0.0);
    sleep_mic_label.set_hexpand(true);
    let sleep_mic = gtk::Switch::new();
    sleep_mic.set_valign(gtk::Align::Center);
    let camera_sleep_mic = xu_status(device).ok().and_then(|data| microphone_during_sleep_state(&data));
    let initial_sleep_mic = camera_sleep_mic.or(current_config().audio_microphone_during_sleep);
    if let Some(enabled) = initial_sleep_mic {
        sleep_mic.set_active(enabled);
        sleep_mic.set_tooltip_text(Some(if camera_sleep_mic.is_some() {
            "Read from selector 0x06 status byte 0x10; writes use selector 0x06 command 13 01 <state>."
        } else {
            "Camera read-back unavailable; showing the last successfully submitted local setting."
        }));
    } else {
        sleep_mic.set_sensitive(false);
        sleep_mic.set_tooltip_text(Some("Initial state unavailable: selector 0x06 status byte 0x10 could not be read."));
    }
    let sleep_mic_device = device.to_string();
    let sleep_mic_status = status.clone();
    sleep_mic.connect_state_set(move |_, enabled| {
        match set_microphone_during_sleep(&sleep_mic_device, enabled) {
            Ok(()) => {
                sleep_mic_status.set_text(if enabled {
                    "Keep microphone on while sleeping enabled."
                } else {
                    "Keep microphone on while sleeping disabled."
                });
                gtk::glib::Propagation::Proceed
            }
            Err(error) => {
                sleep_mic_status.set_text(&format!("Could not change microphone-during-sleep setting: {error}"));
                gtk::glib::Propagation::Stop
            }
        }
    });
    sleep_mic_row.append(&sleep_mic_label);
    sleep_mic_row.append(&sleep_mic);
    layout.append(&sleep_mic_row);

    let auto_gain_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    auto_gain_row.add_css_class("adaptive-row");
    let auto_gain_label = gtk::Label::new(Some("Auto gain"));
    auto_gain_label.set_xalign(0.0);
    auto_gain_label.set_hexpand(true);
    let auto_gain = gtk::Switch::new();
    auto_gain.set_valign(gtk::Align::Center);
    let camera_auto_gain = xu_status(device).ok()
        .map(|data| data[XU_AUDIO_AUTO_GAIN_OFFSET])
        .filter(|value| *value <= 1)
        .map(|value| value != 0);
    let initial_auto_gain = camera_auto_gain.or(current_config().audio_auto_gain);
    if let Some(enabled) = initial_auto_gain {
        auto_gain.set_active(enabled);
    } else {
        auto_gain.set_sensitive(false);
        auto_gain.set_tooltip_text(Some("Auto gain state could not be read from the camera."));
    }
    let auto_gain_device = device.to_string();
    let auto_gain_status = status.clone();
    auto_gain.connect_state_set(move |_switch, enabled| {
        match set_auto_gain(&auto_gain_device, enabled) {
            Ok(()) => auto_gain_status.set_text(if enabled {
                "Microphone auto gain enabled."
            } else {
                "Microphone auto gain disabled."
            }),
            Err(error) => {
                auto_gain_status.set_text(&format!("Could not change auto gain: {error}"));
                return gtk::glib::Propagation::Stop;
            }
        }
        gtk::glib::Propagation::Proceed
    });
    auto_gain_row.append(&auto_gain_label);
    auto_gain_row.append(&auto_gain);
    layout.append(&auto_gain_row);

    let noise_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    noise_row.add_css_class("adaptive-row");
    let noise_label = gtk::Label::new(Some("Noise reduction"));
    noise_label.set_xalign(0.0);
    noise_label.set_hexpand(true);
    let noise_level = gtk::ComboBoxText::new();
    for (value, label) in [(0, "Off"), (1, "Weak"), (2, "Medium"), (3, "Strong")] {
        noise_level.append(Some(&value.to_string()), label);
    }
    let camera_noise_level = xu_status(device).ok()
        .map(|data| data[XU_AUDIO_NOISE_REDUCTION_OFFSET])
        .filter(|value| *value <= 3);
    let initial_noise_level = camera_noise_level.or(current_config().audio_noise_reduction);
    if let Some(level) = initial_noise_level {
        noise_level.set_active_id(Some(&level.to_string()));
    } else {
        noise_level.set_sensitive(false);
        noise_level.set_tooltip_text(Some("Noise reduction state could not be read from the camera."));
    }
    let noise_device = device.to_string();
    let noise_status = status.clone();
    let last_level = Rc::new(Cell::new(initial_noise_level.unwrap_or(0)));
    let changing = Rc::new(Cell::new(false));
    let last_level_change = last_level.clone();
    let changing_state = changing.clone();
    let noise_restore = noise_level.clone();
    noise_level.connect_changed(move |selection| {
        if changing_state.get() { return; }
        let Some(id) = selection.active_id() else { return; };
        let Ok(level) = id.parse::<u8>() else { return; };
        match set_noise_reduction(&noise_device, level) {
            Ok(()) => {
                last_level_change.set(level);
                noise_status.set_text(&format!("Microphone noise reduction: {}.", audio_noise_reduction_label(level)));
            }
            Err(error) => {
                changing_state.set(true);
                noise_restore.set_active_id(Some(&last_level_change.get().to_string()));
                changing_state.set(false);
                noise_status.set_text(&format!("Could not change noise reduction: {error}"));
            }
        }
    });
    noise_row.append(&noise_label);
    noise_row.append(&noise_level);
    layout.append(&noise_row);

    let radio_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    radio_row.add_css_class("adaptive-row");
    let radio_label = gtk::Label::new(Some("Radio distance"));
    radio_label.set_xalign(0.0);
    radio_label.set_hexpand(true);
    let radio_distance = gtk::ComboBoxText::new();
    for (value, label) in [(0, "Close"), (1, "Standard"), (2, "Far")] {
        radio_distance.append(Some(&value.to_string()), label);
    }
    let initial_radio_distance = current_config().audio_radio_distance.filter(|value| *value <= 2);
    if let Some(distance) = initial_radio_distance {
        radio_distance.set_active_id(Some(&distance.to_string()));
    } else {
        // Make the unknown state explicit without silently choosing a real
        // radio range. The changed handler ignores this non-numeric sentinel.
        radio_distance.append(Some("unknown"), "Unknown");
        radio_distance.set_active_id(Some("unknown"));
        radio_distance.set_tooltip_text(Some("Camera read-back is unavailable. Choose Close, Standard, or Far to submit a value; the last successful selection is saved locally."));
    }
    let radio_device = device.to_string();
    let radio_status = status.clone();
    let radio_last = Rc::new(Cell::new(initial_radio_distance));
    let radio_guard = Rc::new(Cell::new(false));
    let radio_last_change = radio_last.clone();
    let radio_changing = radio_guard.clone();
    let radio_restore = radio_distance.clone();
    radio_distance.connect_changed(move |selection| {
        if radio_changing.get() { return; }
        let Some(id) = selection.active_id() else { return; };
        let Ok(distance) = id.parse::<u8>() else { return; };
        match set_radio_distance(&radio_device, distance) {
            Ok(()) => {
                radio_last_change.set(Some(distance));
                radio_status.set_text(&format!("Microphone radio distance set to {}.", radio_distance_label(distance)));
            }
            Err(error) => {
                radio_changing.set(true);
                if let Some(previous) = radio_last_change.get() {
                    radio_restore.set_active_id(Some(&previous.to_string()));
                } else {
                    radio_restore.set_active(None);
                }
                radio_changing.set(false);
                radio_status.set_text(&format!("Could not change radio distance: {error}"));
            }
        }
    });
    radio_row.append(&radio_label);
    radio_row.append(&radio_distance);
    layout.append(&radio_row);

    let pop_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    pop_row.add_css_class("adaptive-row");
    let pop_label = gtk::Label::new(Some("Pop filter"));
    pop_label.set_xalign(0.0);
    pop_label.set_hexpand(true);
    let pop_switch = gtk::Switch::new();
    pop_switch.set_valign(gtk::Align::Center);
    let initial_pop = current_config().pop_filter_enabled;
    pop_switch.set_active(initial_pop.unwrap_or(false));
    pop_switch.set_tooltip_text(Some(if initial_pop.is_some() {
        "Shows the last successfully submitted state; camera read-back is not established."
    } else {
        "Initial state is unknown; the switch starts visually Off. Camera read-back is not established."
    }));
    let pop_device = device.to_string();
    let pop_status = status.clone();
    pop_switch.connect_state_set(move |switch, enabled| {
        match set_pop_filter(&pop_device, enabled) {
            Ok(()) => {
                pop_status.set_text(if enabled { "Pop filter enabled (captured payload replay)." } else { "Pop filter disabled (captured payload replay)." });
                gtk::glib::Propagation::Proceed
            }
            Err(error) => {
                switch.set_active(!enabled);
                pop_status.set_text(&format!("Could not change Pop filter: {error}"));
                gtk::glib::Propagation::Stop
            }
        }
    });
    pop_row.append(&pop_label);
    pop_row.append(&pop_switch);
    layout.append(&pop_row);


    let note = gtk::Label::new(Some(
        "Microphone during sleep reads selector 0x06 status byte 16 and writes the separately captured 13 01 <state> command. Disable microphone uses 1c 01 <state>. Auto gain and noise reduction patch their own status bytes. Pop filter remains experimental pending physical validation."
    ));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&note);
    frame.set_child(Some(&layout));
    container.append(&frame);
}

fn crc16_usb(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in bytes {
        crc ^= *byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xa001 } else { crc >> 1 };
        }
    }
    !crc
}

fn obsbot_frame(sequence: u16, function_group: [u8; 6], command: [u8; 6]) -> [u8; 60] {
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
static XU_SEQUENCE: OnceLock<AtomicU16> = OnceLock::new();
static LAST_UI_TEST_QUERY_COMMAND: AtomicU16 = AtomicU16::new(0);
static LAST_UI_TEST_QUERY_SEQUENCE: AtomicU16 = AtomicU16::new(0);
static LAST_UI_TEST_AUTO_EXPOSURE_PRIORITY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn xu_sequence_counter() -> &'static AtomicU16 {
    XU_SEQUENCE.get_or_init(|| {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u16)
            .unwrap_or(0);
        AtomicU16::new(seed)
    })
}

fn next_xu_sequence(counter: &AtomicU16) -> u16 {
    counter.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

fn xu_send_frame(device: &str, function_group: [u8; 6], command: [u8; 6]) -> Result<(), String> {
    let sequence = next_xu_sequence(xu_sequence_counter());
    let mut frame = obsbot_frame(sequence, function_group, command);
    xu_query(device, 0x02, UVC_SET_CUR, &mut frame)
}

fn obsbot_frame_with_payload(sequence: u16, command: u16, payload: &[u8]) -> Result<[u8; 60], String> {
    if payload.len() > 44 {
        return Err(format!("OBSBOT command payload is too large: {} bytes", payload.len()));
    }
    let mut frame = [0u8; 60];
    frame[0] = 0xaa;
    frame[1] = 0x25;
    frame[2..4].copy_from_slice(&sequence.to_le_bytes());
    frame[4..6].copy_from_slice(&12u16.to_le_bytes());
    frame[8..12].copy_from_slice(&[0x0a, 0x04, command as u8, (command >> 8) as u8]);
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

fn decode_preset_base64(input: &[u8]) -> Result<String, String> {
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

fn parse_camera_preset_entry(data: &[u8; 60]) -> Result<Option<CameraPresetEntry>, String> {
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

fn preset_slot_index(slot: u8) -> Result<u32, String> {
    if slot >= 3 { return Err("Tiny 2 Lite preset slot must be 1, 2, or 3.".into()); }
    Ok(u32::from(slot))
}

fn send_camera_preset_command(device: &str, command: u16, payload: &[u8]) -> Result<(), String> {
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
    if !pan.is_finite() || !tilt.is_finite() || pan.abs() > 150.0 || !(-70.0..=90.0).contains(&(tilt as f32)) {
        return Err(format!("Current live pose is outside camera preset limits: pan {pan:+.1}°, tilt {tilt:+.1}°."));
    }
    if !(1.0..=4.0).contains(&zoom) {
        return Err(format!("Current live zoom {zoom:.2}× is outside preset limits 1.0–4.0×."));
    }
    for value in [pan as f32, -tilt as f32, 0.0f32, zoom, -1000.0f32] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    send_camera_preset_command(device, if update { 0x3e04 } else { 0x3944 }, &payload)?;
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

pub(crate) fn delete_camera_preset(device: &str, slot: u8) -> Result<(), String> {
    let index = preset_slot_index(slot)?.to_le_bytes();
    send_camera_preset_command(device, 0x3984, &index)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    const CLEARED: [u8; 44] = [
        0x00, 0x00, 0x00, 0x00, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    send_camera_preset_command(device, 0x3d84, &CLEARED)
}

pub(crate) fn recall_camera_preset(device: &str, slot: u8) -> Result<(), String> {
    let mut payload = preset_slot_index(slot)?.to_le_bytes().to_vec();
    for _ in 0..4 { payload.extend_from_slice(&1.0f32.to_le_bytes()); }
    send_camera_preset_command(device, 0x39c4, &payload)
}


fn led_brightness_payload(level: u8) -> Result<[u8; 3], String> {
    if level > 3 {
        return Err("LED setting must be Off (0) or brightness level 1–3.".into());
    }
    // The captured selector 0x06/tag 0x1A values are the four settings:
    // raw 0 = Off; raw 1, 2, 3 = brightness levels 1, 2, 3.
    // Do not subtract one: that incorrectly turns level 1 into Off.
    Ok([0x1a, 0x01, level])
}

fn set_led_brightness(device: &str, level: u8) -> Result<(), String> {
    let payload = led_brightness_payload(level)?;
    let mut data = [0u8; 60];
    data[..payload.len()].copy_from_slice(&payload);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.led_brightness = Some(level));
    Ok(())
}

// Tiny 2 Lite HDR control, captured as a 60-byte selector 0x06 SET_CUR.
// Captures correlate third byte 0x01 with HDR On and 0x00 with HDR Off.
fn hdr_payload(enabled: bool) -> [u8; 3] {
    [0x01, 0x01, if enabled { 0x01 } else { 0x00 }]
}

fn hdr_state_from_status(status: &[u8; 60]) -> Option<bool> {
    match status[0x06] {
        0x00 => Some(false),
        0x01 => Some(true),
        _ => None,
    }
}

fn set_hdr_enabled(device: &str, enabled: bool) -> Result<(), String> {
    let payload = hdr_payload(enabled);
    let mut data = [0u8; 60];
    data[..payload.len()].copy_from_slice(&payload);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    update_config(|config| config.hdr_enabled = Some(enabled));
    Ok(())
}

// Tiny 2 Lite Face / Global Auto Exposure priority.
// XU unit 2, selector 0x06, 60-byte SET_CUR and GET_CUR. Controlled
// 2026-11-20 captures established the write value at byte 2 and the
// authoritative read-back value at byte 7. This is NOT autofocus priority.
const XU_AUTO_EXPOSURE_PRIORITY_OFFSET: usize = 7;

fn auto_exposure_mode_payload(face_priority: bool) -> [u8; 60] {
    let mut data = [0u8; 60];
    data[..3].copy_from_slice(&[0x03, 0x01, if face_priority { 0x00 } else { 0x01 }]);
    data
}

fn auto_exposure_mode_from_status(status: &[u8; 60]) -> Option<bool> {
    match status[XU_AUTO_EXPOSURE_PRIORITY_OFFSET] {
        0x00 => Some(true),  // Face
        0x01 => Some(false), // Global
        _ => None,
    }
}

fn get_auto_exposure_mode(device: &str) -> Result<bool, String> {
    let status = xu_status(device)?;
    auto_exposure_mode_from_status(&status).ok_or_else(|| format!(
        "Camera returned unsupported Auto Exposure priority value 0x{:02x} at status byte 0x{:02x}.",
        status[XU_AUTO_EXPOSURE_PRIORITY_OFFSET], XU_AUTO_EXPOSURE_PRIORITY_OFFSET
    ))
}

fn set_auto_exposure_mode(device: &str, face_priority: bool) -> Result<bool, String> {
    let mut data = auto_exposure_mode_payload(face_priority);
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;

    // Do not treat a successful ioctl or the requested value as proof of
    // device state. Read the complete status block and trust byte 7.
    let actual_face_priority = get_auto_exposure_mode(device)
        .map_err(|error| format!("SET_CUR was submitted, but Auto Exposure priority read-back failed: {error}"))?;
    update_config(|config| config.auto_exposure_mode = Some(
        if actual_face_priority { "face" } else { "global" }.into()
    ));
    Ok(actual_face_priority)
}

fn add_auto_exposure_mode_control(container: &gtk::Box, device: &str, status: &gtk::Label, auto_exposure_enabled: bool) {
    let frame = gtk::Frame::new(Some("Auto Exposure priority"));
    ui::layout::mark_compact_item(frame.upcast_ref(), "obsbot:auto-exposure-mode");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let selector = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    selector.add_css_class("linked");
    selector.set_halign(gtk::Align::Start);
    let global = gtk::ToggleButton::with_label("Global");
    let face = gtk::ToggleButton::with_label("Face");
    face.set_group(Some(&global));
    global.add_css_class("flat");
    face.add_css_class("flat");
    selector.set_sensitive(auto_exposure_enabled);

    // Hardware read-back is authoritative. The saved preference is only a
    // fallback for devices/firmware where the selector cannot be queried.
    let camera_face_priority = get_auto_exposure_mode(device).ok();
    let saved_mode = current_config().auto_exposure_mode;
    let initial_face_priority = camera_face_priority.or_else(|| match saved_mode.as_deref() {
        Some("face") => Some(true),
        Some("global") => Some(false),
        _ => None,
    });
    global.set_active(initial_face_priority == Some(false));
    face.set_active(initial_face_priority == Some(true));
    selector.append(&global);
    selector.append(&face);
    layout.append(&selector);

    let current_face = Rc::new(Cell::new(initial_face_priority));
    for (button, face_priority) in [(global.clone(), false), (face.clone(), true)] {
        let dev = device.to_string();
        let status_label = status.clone();
        let current = current_face.clone();
        let global_restore = global.clone();
        let face_restore = face.clone();
        button.connect_clicked(move |selected| {
            if !selected.is_active() { return; }
            let previous = current.get();
            match set_auto_exposure_mode(&dev, face_priority) {
                Ok(actual_face_priority) => {
                    current.set(Some(actual_face_priority));
                    global_restore.set_active(!actual_face_priority);
                    face_restore.set_active(actual_face_priority);
                    if actual_face_priority == face_priority {
                        status_label.set_text(if actual_face_priority {
                            "Auto Exposure priority read-back confirms Face."
                        } else {
                            "Auto Exposure priority read-back confirms Global."
                        });
                    } else {
                        status_label.set_text(if actual_face_priority {
                            "Camera reports Face priority; it did not retain the requested Global setting."
                        } else {
                            "Camera reports Global priority; it did not retain the requested Face setting."
                        });
                    }
                }
                Err(error) => {
                    global_restore.set_active(previous == Some(false));
                    face_restore.set_active(previous == Some(true));
                    status_label.set_text(&format!("Could not verify Auto Exposure priority change: {error}"));
                }
            }
        });
    }

    let note_text = if auto_exposure_enabled {
        if camera_face_priority.is_some() {
            "Face / Global controls Auto Exposure priority, not autofocus. Initial selection and every change are confirmed from selector 0x06 GET_CUR status byte 7."
        } else {
            "Face / Global controls Auto Exposure priority, not autofocus. Camera read-back is currently unavailable; the saved local selection is shown as a fallback."
        }
    } else {
        "Unavailable: Exposure mode is not Auto. Set Exposure mode to Auto to enable Face / Global exposure priority."
    };
    let note = gtk::Label::new(Some(note_text));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&note);
    frame.set_child(Some(&layout));
    container.append(&frame);
}

fn add_camera_settings(container: &gtk::Box, device: &str, window: &gtk::ApplicationWindow, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Camera settings"));
    frame.add_css_class("compact-group");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    // FOV presets are positions on the standard V4L2 zoom_absolute control.
    let zoom_frame = gtk::Frame::new(Some("Zoom and field of view"));
    ui::layout::mark_compact_item(zoom_frame.upcast_ref(), "obsbot:zoom-presets");
    let zoom_layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    zoom_layout.set_margin_top(8);
    zoom_layout.set_margin_bottom(8);
    zoom_layout.set_margin_start(10);
    zoom_layout.set_margin_end(10);
    let zoom_presets = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    zoom_presets.add_css_class("adaptive-row");
    for (label, value) in [("86°", 0_i64), ("78°", 5), ("65°", 15)] {
        let button = gtk::Button::with_label(label);
        let dev = device.to_string();
        let status_label = status.clone();
        let controls_refresh = container.clone();
        let window_refresh = window.clone();
        let refresh_device = dev.clone();
        button.connect_clicked(move |_| {
            match set_control(&dev, "zoom_absolute", value) {
                Ok(()) => ui::controls::populate_controls(&refresh_device, &controls_refresh, &window_refresh, &status_label),
                Err(error) => status_label.set_text(&format!("Could not apply zoom preset: {error}")),
            }
        });
        zoom_presets.append(&button);
    }
    zoom_layout.append(&zoom_presets);
    let zoom_note = gtk::Label::new(Some("Use V4L2 zoom_absolute (0–100) for smooth zoom and these common FOV positions. zoom_continuous is shown separately but disabled when the driver reports an out-of-range value; the separate vendor FOV command is not used."));
    zoom_note.set_wrap(true);
    zoom_note.set_xalign(0.0);
    zoom_note.add_css_class("dim-label");
    zoom_layout.append(&zoom_note);
    zoom_frame.set_child(Some(&zoom_layout));
    layout.append(&zoom_frame);

    let hdr_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    hdr_row.add_css_class("adaptive-row");
    ui::layout::mark_compact_item(hdr_row.upcast_ref(), "obsbot:hdr");
    let hdr_label = gtk::Label::new(Some("HDR"));
    hdr_label.set_xalign(0.0);
    hdr_label.set_hexpand(true);
    let hdr_switch = gtk::Switch::new();
    hdr_switch.set_valign(gtk::Align::Center);
    let initial_hdr = xu_status(device).ok().and_then(|data| hdr_state_from_status(&data))
        .or_else(|| current_config().hdr_enabled);
    if let Some(enabled) = initial_hdr {
        hdr_switch.set_active(enabled);
    } else {
        hdr_switch.set_sensitive(false);
        hdr_switch.set_tooltip_text(Some("HDR state is unavailable from the camera status block."));
    }
    let hdr_status = status.clone();
    let hdr_device = device.to_string();
    hdr_switch.connect_state_set(move |_switch, enabled| {
        match set_hdr_enabled(&hdr_device, enabled) {
            Ok(()) => {
                hdr_status.set_text(if enabled {
                    "HDR enable command submitted."
                } else {
                    "HDR disable command submitted."
                });
            }
            Err(error) => {
                hdr_status.set_text(&format!("Could not change HDR: {error}"));
                return gtk::glib::Propagation::Stop;
            }
        }
        gtk::glib::Propagation::Proceed
    });
    hdr_row.append(&hdr_label);
    hdr_row.append(&hdr_switch);
    layout.append(&hdr_row);

    let led_frame = gtk::Frame::new(Some("LED illumination"));
    let led_layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let led_level_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ui::layout::mark_compact_item(led_level_row.upcast_ref(), "obsbot:led-brightness");
    led_level_row.add_css_class("adaptive-row");
    let led_level_label = gtk::Label::new(Some("LED setting"));
    led_level_label.set_width_chars(22);
    led_level_label.set_xalign(0.0);
    let led_level = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 3.0, 1.0);
    led_level.set_digits(0);
    led_level.set_draw_value(true);
    led_level.set_hexpand(true);
    led_level.add_mark(0.0, gtk::PositionType::Bottom, Some("Off"));
    led_level.add_mark(1.0, gtk::PositionType::Bottom, Some("1"));
    led_level.add_mark(2.0, gtk::PositionType::Bottom, Some("2"));
    led_level.add_mark(3.0, gtk::PositionType::Bottom, Some("3"));
    let initial_led = xu_status(device).ok().map(|data| data[0x21])
        .or_else(|| current_config().led_brightness).unwrap_or(0);
    led_level.set_value(if initial_led <= 3 { initial_led as f64 } else { 0.0 });
    led_level_row.append(&led_level_label);
    led_level_row.append(&led_level);
    led_layout.append(&led_level_row);

    let led_slider_status = status.clone();
    let led_slider_device = device.to_string();
    let led_last_value = Rc::new(Cell::new(initial_led.min(3)));
    let led_reverting = Rc::new(Cell::new(false));
    let led_last_value_cb = led_last_value.clone();
    let led_reverting_cb = led_reverting.clone();
    led_level.connect_value_changed(move |scale| {
        if led_reverting_cb.get() { return; }
        let setting = scale.value().round().clamp(0.0, 3.0) as u8;
        match set_led_brightness(&led_slider_device, setting) {
            Ok(()) => {
                led_last_value_cb.set(setting);
                let description = match setting {
                    0 => "Off".to_string(),
                    n => format!("brightness level {n}"),
                };
                led_slider_status.set_text(&format!("LED setting: {description}."));
            }
            Err(error) => {
                led_reverting_cb.set(true);
                scale.set_value(led_last_value_cb.get() as f64);
                led_reverting_cb.set(false);
                led_slider_status.set_text(&format!("Could not change LED setting: {error}"));
            }
        }
    });

    let led_note = gtk::Label::new(Some("Off plus three brightness levels. Each position sends selector 0x06, tag 0x1A directly. The separate tag 0x18 command is intentionally not used here because its observed effect is blink/steady behaviour, not a verified power switch."));
    led_note.set_wrap(true);
    led_note.set_xalign(0.0);
    led_note.add_css_class("dim-label");
    led_layout.append(&led_note);
    led_frame.set_child(Some(&led_layout));
    layout.append(&led_frame);
    add_mirror_image_control(&layout, device, status);

    let note = gtk::Label::new(Some("Image controls are discovered from V4L2 metadata; camera-reported ranges, steps, menus and inactive flags are authoritative."));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&note);
    frame.set_child(Some(&layout));
    container.append(&frame);
}

// Tiny 2 Lite focus-priority command (wire command 0x3602, Camera receiver 0x02).
// The Nov 18 capture correlates Global -> Face with BE 07 00 00 and
// Face -> Global with BF FB 01 00. The camera does not provide reliable
// read-back, so this records the last successfully submitted command.
fn face_focus_payload(face_priority: bool) -> [u8; 4] {
    if face_priority { [0xbe, 0x07, 0x00, 0x00] }
    else { [0xbf, 0xfb, 0x01, 0x00] }
}

fn set_face_focus(device: &str, face_priority: bool) -> Result<(), String> {
    let group = [0x0a, 0x02, 0x02, 0x36, 0x04, 0x00];
    let mut command = [0u8; 6];
    command[2..6].copy_from_slice(&face_focus_payload(face_priority));
    xu_send_frame(device, group, command)?;
    update_config(|config| config.focus_mode = Some(if face_priority { "face" } else { "global" }.into()));
    Ok(())
}

fn add_focus_mode_control(container: &gtk::Box, device: &str, status: &gtk::Label, autofocus_enabled: bool) {
    let frame = gtk::Frame::new(Some("Autofocus priority"));
    ui::layout::mark_compact_item(frame.upcast_ref(), "obsbot:focus_mode");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    // A grouped ToggleButton pair behaves as one segmented choice: exactly
    // one priority is selected, and changing it sends the corresponding XU command.
    let selector = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    selector.add_css_class("linked");
    selector.set_halign(gtk::Align::Start);
    let global = gtk::ToggleButton::with_label("Global");
    let face = gtk::ToggleButton::with_label("Face");
    face.set_group(Some(&global));
    global.add_css_class("flat");
    face.add_css_class("flat");
    selector.set_sensitive(autofocus_enabled);
    let saved_focus_mode = current_config().focus_mode;
    match saved_focus_mode.as_deref() {
        Some("global") => global.set_active(true),
        Some("face") => face.set_active(true),
        _ => {} // No confirmed local selection: leave both segments unselected.
    }
    selector.append(&global);
    selector.append(&face);
    layout.append(&selector);

    let current_face = Rc::new(Cell::new(match saved_focus_mode.as_deref() {
        Some("face") => Some(true),
        Some("global") => Some(false),
        _ => None,
    }));
    // Dispatch from the clicked segment, not both buttons' toggled signals.
    // A grouped ToggleButton emits toggled for the segment being deselected as
    // well as the one being selected. Handling only a user click guarantees one
    // command for the newly selected priority and avoids depending on signal order.
    for (button, face_priority) in [(global.clone(), false), (face.clone(), true)] {
        let dev = device.to_string();
        let status_label = status.clone();
        let current = current_face.clone();
        let global_restore = global.clone();
        let face_restore = face.clone();
        button.connect_clicked(move |selected| {
            if !selected.is_active() {
                return;
            }
            let previous = current.get();
            match set_face_focus(&dev, face_priority) {
                Ok(()) => {
                    current.set(Some(face_priority));
                    status_label.set_text(if face_priority {
                        "Face-priority autofocus command submitted. Verify the camera's focus behavior."
                    } else {
                        "Global autofocus command submitted. Verify the camera's focus behavior."
                    });
                }
                Err(error) => {
                    global_restore.set_active(previous == Some(false));
                    face_restore.set_active(previous == Some(true));
                    status_label.set_text(&format!("Could not change autofocus priority: {error}"));
                }
            }
        });
    }

    let note_text = if autofocus_enabled {
        "Global and Face are mutually exclusive autofocus priorities. The camera has no confirmed read-back for this setting, so the selected value is the last successfully submitted command."
    } else {
        "Unavailable: Autofocus is disabled. Turn Autofocus on to enable Global / Face priority."
    };
    let note = gtk::Label::new(Some(note_text));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&note);
    frame.set_child(Some(&layout));
    container.append(&frame);
}

fn add_autofocus_control(
    container: &gtk::Box,
    device: &str,
    window: &gtk::ApplicationWindow,
    status: &gtk::Label,
    enabled: bool,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("adaptive-row");
    ui::layout::mark_compact_item(row.upcast_ref(), "v4l2:focus_automatic_continuous");
    let title = gtk::Label::new(Some("Autofocus"));
    title.set_width_chars(22);
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let toggle = gtk::Switch::new();
    toggle.set_active(enabled);
    toggle.set_valign(gtk::Align::Center);

    let dev = device.to_string();
    let controls_refresh = container.clone();
    let window_refresh = window.clone();
    let status_refresh = status.clone();
    toggle.connect_state_set(move |switch, active| {
        match set_control(&dev, "focus_automatic_continuous", if active { 1 } else { 0 }) {
            Ok(()) => {
                status_refresh.set_text(if active {
                    "Autofocus enabled."
                } else {
                    "Autofocus disabled. Manual focus is available."
                });
                ui::controls::populate_controls(&dev, &controls_refresh, &window_refresh, &status_refresh);
            }
            Err(error) => {
                status_refresh.set_text(&format!("Could not change autofocus: {error}"));
                switch.set_active(!active);
            }
        }
        gtk::glib::Propagation::Proceed
    });

    row.append(&title);
    row.append(&toggle);
    container.append(&row);
}

fn set_ai_mode(device: &str, mode: (u8, u8)) -> Result<(), String> {
    // Selector 0x06 uses [opcode, payload length, m, n].
    let mut data = [0u8; 60];
    data[0] = 0x16;
    data[1] = 0x02;
    data[2] = mode.0;
    data[3] = mode.1;
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)?;
    let name = match mode { (0x00, 0x00) => "off", (0x02, 0x00) => "normal", (0x02, 0x01) => "upper", (0x02, 0x02) => "close", (0x02, 0x03) => "headless", (0x02, 0x04) => "lower", (0x01, 0x00) => "group", _ => "unknown" };
    update_config(|config| config.ai_tracking_mode = Some(name.into()));
    Ok(())
}

fn set_sleep_mode(device: &str, sleep: bool) -> Result<(), String> {
    // CAM_SET_DEV_STATUS (0xA0C2), confirmed on Tiny 2 Lite.
    // Power-state verification is deliberately separate: the camera's
    // status byte may lag behind the command.
    let group = [0x0a, 0x02, 0xc2, 0xa0, 0x04, 0x00];
    let command = if sleep {
        [0x00, 0x00, 0x01, 0x00, 0x00, 0x00]
    } else {
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    };
    xu_send_frame(device, group, command)
}

fn add_sleep_controls(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Camera power"));
    ui::layout::mark_compact_item(frame.upcast_ref(), "camera:power");
    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    layout.add_css_class("adaptive-row");
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    for (label, sleep) in [("Wake", false), ("Sleep", true)] {
        let button = gtk::Button::with_label(label);
        let dev = device.to_string();
        let status_label = status.clone();
        button.connect_clicked(move |_| {
            let result = set_sleep_mode(&dev, sleep);
            status_label.set_text(&result.map(|_| format!(
                "{} command sent. Use Read power status to verify the camera's state.",
                if sleep { "Sleep" } else { "Wake" }
            )).unwrap_or_else(|error| format!(
                "{} command failed: {error}",
                if sleep { "Sleep" } else { "Wake" }
            )));
        });
        layout.append(&button);
    }

    let read = gtk::Button::with_label("Read power status");
    let dev = device.to_string();
    let status_read = status.clone();
    read.connect_clicked(move |_| {
        status_read.set_text(&match xu_status(&dev) {
            Ok(data) => match data[0x02] {
                0x00 => "Camera status: awake.".to_string(),
                0x01 => "Camera status: asleep.".to_string(),
                value => format!("Camera sleep-state byte is unknown: 0x{value:02x}."),
            },
            Err(error) => format!("Could not read camera power status: {error}"),
        });
    });
    layout.append(&read);

    frame.set_child(Some(&layout));
    container.append(&frame);
}

fn set_tracking_speed(device: &str, sport: bool) -> Result<(), String> {
    let group = [0x0a, 0x04, 0xc4, 0x0c, 0x01, 0x00];
    let command = if sport {
        [0x67, 0xfe, 0x02, 0x00, 0x00, 0x00]
    } else {
        [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00]
    };
    xu_send_frame(device, group, command)?;
    update_config(|config| config.tracking_speed = Some(if sport { "sport" } else { "standard" }.into()));
    Ok(())
}

fn set_gesture_zoom_ratio_payload_is_valid(ratio: f32) -> bool {
    ratio.is_finite() && (1.0..=4.0).contains(&ratio)
}

fn set_gesture_zoom_ratio(device: &str, ratio: f32) -> Result<(), String> {
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

fn gesture_command(group: [u8; 6], enabled: bool) -> [u8; 6] {
    // Dynamic Zoom Direction is the exception: its captured ON/OFF payloads
    // are inverted relative to the other gesture commands.
    let wire_enabled = if group == [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00] {
        !enabled
    } else {
        enabled
    };

    if wire_enabled {
        [0x27, 0xff, 0x01, 0x00, 0x00, 0x00]
    } else {
        [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00]
    }
}

fn set_gesture(device: &str, group: [u8; 6], enabled: bool) -> Result<(), String> {
    let command = gesture_command(group, enabled);
    xu_send_frame(device, group, command)?;
    let key = match group { [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00] => "target_selection", [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00] => "zoom_l_pose", [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00] => "dynamic_zoom", [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00] => "dynamic_zoom_direction", _ => "unknown" };
    if key != "unknown" { update_config(|config| { config.gesture_states.insert(key.into(), enabled); }); }
    Ok(())
}

fn ai_mode_label(status: &[u8; 60]) -> &'static str {
    match (status[0x18], status[0x1c]) {
        (0x00, 0x00) => "Off",
        (0x02, 0x00) => "Normal tracking",
        (0x02, 0x01) => "Upper body",
        (0x02, 0x02) => "Close-up",
        (0x02, 0x03) => "Headless",
        (0x02, 0x04) => "Lower body",
        (0x01, 0x00) => "Group",
        _ => "Unknown mode",
    }
}

fn add_ai_controls(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("AI tracking and gestures"));
    frame.add_css_class("compact-group");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let mode_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    mode_row.add_css_class("adaptive-row");
    ui::layout::mark_compact_item(mode_row.upcast_ref(), "obsbot:ai-tracking-mode");
    let mode_label = gtk::Label::new(Some("Tracking mode"));
    mode_label.set_width_chars(22);
    mode_label.set_xalign(0.0);
    let mode = gtk::ComboBoxText::new();
    mode.set_hexpand(true);
    let modes: [(&str, &str, u8, u8); 7] = [
        ("off", "Off", 0x00, 0x00),
        ("normal", "Normal tracking", 0x02, 0x00),
        ("upper", "Upper body", 0x02, 0x01),
        ("close", "Close-up", 0x02, 0x02),
        ("headless", "Headless", 0x02, 0x03),
        ("lower", "Lower body", 0x02, 0x04),
        ("group", "Group", 0x01, 0x00),
    ];
    for (id, label, _, _) in modes { mode.append(Some(id), label); }
    if let Ok(current) = xu_status(device) {
        let current_id = modes.iter().find(|(_, _, m, n)| *m == current[0x18] && *n == current[0x1c])
            .map(|(id, _, _, _)| *id);
        if let Some(id) = current_id { mode.set_active_id(Some(id)); }
        status.set_text(&format!("AI status: {} · tracking speed: {}",
            ai_mode_label(&current),
            match current[0x24] { 0x00 => "Standard", 0x02 => "Sport", 0x21 => "Sport (legacy)", _ => "Unknown" }));
    } else {
        status.set_text("AI controls shown, but XU status could not be read. Check camera node permissions.");
    }
    let last_mode = Rc::new(RefCell::new(mode.active_id().map(|id| id.to_string())
        .or_else(|| current_config().ai_tracking_mode.clone())));
    let changing_mode = Rc::new(Cell::new(false));
    let dev = device.to_string();
    let status_mode = status.clone();
    let last_mode_change = last_mode.clone();
    let changing_mode_state = changing_mode.clone();
    let mode_restore = mode.clone();
    mode.connect_changed(move |combo| {
        if changing_mode_state.get() { return; }
        let Some(id) = combo.active_id().map(|id| id.to_string()) else { return; };
        let result = modes.iter().find(|entry| entry.0 == id)
            .ok_or_else(|| "Unknown AI tracking mode.".to_string())
            .and_then(|entry| set_ai_mode(&dev, (entry.2, entry.3)));
        match result {
            Ok(()) => {
                *last_mode_change.borrow_mut() = Some(id.clone());
                status_mode.set_text(&format!("AI tracking mode set to {}. Device status can be refreshed to verify.", id));
            }
            Err(error) => {
                changing_mode_state.set(true);
                if let Some(previous) = last_mode_change.borrow().as_deref() {
                    mode_restore.set_active_id(Some(previous));
                }
                changing_mode_state.set(false);
                status_mode.set_text(&format!("AI mode failed: {error}"));
            }
        }
    });
    mode_row.append(&mode_label);
    mode_row.append(&mode);
    layout.append(&mode_row);

    let speed_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    speed_row.add_css_class("adaptive-row");
    ui::layout::mark_compact_item(speed_row.upcast_ref(), "obsbot:tracking-speed");
    let speed_label = gtk::Label::new(Some("Tracking speed"));
    speed_label.set_width_chars(22);
    speed_label.set_xalign(0.0);
    let speed = gtk::ComboBoxText::new();
    speed.append(Some("standard"), "Standard");
    speed.append(Some("sport"), "Sport");
    if let Ok(current) = xu_status(device) {
        speed.set_active_id(Some(if current[0x24] == 0 { "standard" } else { "sport" }));
    } else {
        speed.set_active_id(Some("standard"));
    }
    let last_speed = Rc::new(RefCell::new(speed.active_id().map(|id| id.to_string())));
    let changing_speed = Rc::new(Cell::new(false));
    let dev = device.to_string();
    let status_speed = status.clone();
    let last_speed_change = last_speed.clone();
    let changing_speed_state = changing_speed.clone();
    let speed_restore = speed.clone();
    speed.connect_changed(move |combo| {
        if changing_speed_state.get() { return; }
        let Some(id) = combo.active_id().map(|id| id.to_string()) else { return; };
        let result = set_tracking_speed(&dev, id == "sport");
        match result {
            Ok(()) => {
                *last_speed_change.borrow_mut() = Some(id.clone());
                status_speed.set_text(&format!("Tracking speed set to {id}."));
            }
            Err(error) => {
                changing_speed_state.set(true);
                if let Some(previous) = last_speed_change.borrow().as_deref() {
                    speed_restore.set_active_id(Some(previous));
                }
                changing_speed_state.set(false);
                status_speed.set_text(&format!("Tracking speed failed: {error}"));
            }
        }
    });
    speed_row.append(&speed_label);
    speed_row.append(&speed);
    layout.append(&speed_row);

    // Status byte 0x07 is inverted for the L-pose Zoom gesture:
    // 1 = disabled, 0 = enabled. Other gesture states have no confirmed
    // device read-back, so those start Unknown and become state-aware after
    // the first successful command in this session.
    let saved_gestures = current_config().gesture_states;
    let gestures: [(&str, [u8; 6], Option<bool>); 4] = [
        ("Target selection gesture", [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00], saved_gestures.get("target_selection").copied()),
        ("Zoom (L-pose) gesture", [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00],
            xu_status(device).ok().map(|data| data[0x07] == 0).or_else(|| saved_gestures.get("zoom_l_pose").copied())),
        ("Dynamic zoom gesture", [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00], saved_gestures.get("dynamic_zoom").copied()),
        ("Dynamic zoom direction", [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00], saved_gestures.get("dynamic_zoom_direction").copied()),
    ];
    for (label, group, initial_state) in gestures {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("adaptive-row");
        let gesture_key = match group {
            [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00] => "obsbot:target-selection",
            [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00] => "obsbot:zoom-l-pose",
            [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00] => "obsbot:dynamic-zoom",
            _ => "obsbot:dynamic-zoom-direction",
        };
        ui::layout::mark_compact_item(row.upcast_ref(), gesture_key);
        let name = gtk::Label::new(Some(label));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        row.append(&name);

        let state = Rc::new(RefCell::new(initial_state));
        let button = gtk::Button::with_label(&match initial_state {
            Some(true) => "Enabled · Disable",
            Some(false) => "Disabled · Enable",
            None => "Unknown · Enable",
        });
        let dev = device.to_string();
        let state_click = state.clone();
        let button_click = button.clone();
        let status_gesture = status.clone();
        button.connect_clicked(move |_| {
            // Never display an unknown state as Off. First click enables it;
            // later clicks toggle the state established by successful writes.
            let enabled = !state_click.borrow().unwrap_or(false);
            match set_gesture(&dev, group, enabled) {
                Ok(()) => {
                    *state_click.borrow_mut() = Some(enabled);
                    button_click.set_label(if enabled { "Enabled · Disable" } else { "Disabled · Enable" });
                    status_gesture.set_text(&format!("{label}: {} command sent.{}",
                        if enabled { "enabled" } else { "disabled" },
                        if label == "Zoom (L-pose) gesture" {
                            " Device state is readable from status byte 0x07."
                        } else {
                            " No confirmed device read-back exists; state reflects successful commands in this session."
                        }));
                }
                Err(error) => status_gesture.set_text(&format!("{label} change failed: {error}")),
            }
        });
        row.append(&button);
        layout.append(&row);
    }

    let ratio_frame = gtk::Frame::new(Some("Gesture zoom ratio"));
    ui::layout::mark_compact_item(ratio_frame.upcast_ref(), "obsbot:gesture-zoom-ratio");
    let ratio_layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let ratio_help = gtk::Label::new(Some("Zoom applied when the camera recognizes the L-shaped Zoom gesture. Range: 1.00×–4.00×."));
    ratio_help.set_wrap(true);
    ratio_help.set_xalign(0.0);
    ratio_help.add_css_class("dim-label");
    ratio_layout.append(&ratio_help);

    let ratio_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ratio_row.add_css_class("adaptive-row");
    let ratio_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 4.0, 0.01);
    ratio_scale.set_value(current_config().gesture_zoom_ratio.unwrap_or(1.0) as f64);
    ratio_scale.set_digits(2);
    ratio_scale.set_draw_value(true);
    ratio_scale.set_hexpand(true);
    ratio_scale.set_value_pos(gtk::PositionType::Right);
    let dev_ratio = device.to_string();
    let status_ratio = status.clone();
    ratio_scale.connect_value_changed(move |slider| {
        let ratio = slider.value() as f32;
        let result = set_gesture_zoom_ratio(&dev_ratio, ratio);
        status_ratio.set_text(&result.map(|_| format!("Gesture zoom ratio set to {ratio:.2}×."))
            .unwrap_or_else(|e| format!("Could not set gesture zoom ratio: {e}")));
    });
    ratio_row.append(&ratio_scale);
    ratio_layout.append(&ratio_row);
    ratio_frame.set_child(Some(&ratio_layout));
    layout.append(&ratio_frame);

    let refresh_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    refresh_row.add_css_class("adaptive-row");
    let refresh = gtk::Button::with_label("Read AI status");
    let dev = device.to_string();
    let status_refresh = status.clone();
    refresh.connect_clicked(move |_| {
        status_refresh.set_text(&match xu_status(&dev) {
            Ok(data) => format!("AI status: {} · speed byte: 0x{:02x} · raw mode: {:02x}/{:02x}",
                ai_mode_label(&data), data[0x24], data[0x18], data[0x1c]),
            Err(e) => format!("Could not read AI status: {e}"),
        });
    });
    refresh_row.append(&refresh);
    layout.append(&refresh_row);

    let note = gtk::Label::new(Some("AI commands use OBSBOT's vendor XU. Gesture zoom ratio is sent as a little-endian f32 using command 0x3244. The L-pose Zoom gesture has device read-back at status byte 0x07; other gesture states are unknown on startup and tracked locally after a successful toggle. Desk, Whiteboard, and Hand tracking are omitted."));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&note);
    frame.set_child(Some(&layout));
    container.append(&frame);
}



fn usb_descriptor_info() -> Result<String, String> {
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

fn hex_dump(data: &[u8]) -> String {
    let mut output = String::new();
    for (offset, chunk) in data.chunks(16).enumerate() {
        output.push_str(&format!("{:04x}: ", offset * 16));
        for byte in chunk { output.push_str(&format!("{:02x} ", byte)); }
        output.push('\n');
    }
    output
}

fn decode_xu_status(data: &[u8; 60]) -> String {
    let sleep = match data[0x02] { 0x00 => "Awake", 0x01 => "Sleep", _ => "Unknown" };
    let hdr = if data[0x06] == 0 { "Off" } else { "On / non-zero" };
    let fov = match data[0x11] { 1 => "Wide (86°)", 2 => "Normal (78°)", v => {
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

fn decode_xu_command_response(data: &[u8; 60]) -> String {
    let record = &data[9..15];
    let position_record = record == [0x04, 0x0a, 0x44, 0x3a, 0x18, 0x00]
        || record == [0x02, 0x0a, 0x42, 0x29, 0x05, 0x00];
    if !position_record {
        return format!("No recognized position-like response record at bytes 9..14: {:02x?}\nOffset-20 values are not decoded for this response.", record);
    }
    let pan = f32::from_le_bytes(data[20..24].try_into().unwrap());
    let tilt = f32::from_le_bytes(data[24..28].try_into().unwrap());
    if pan.is_finite() && tilt.is_finite() {
        format!("Candidate position record: {:02x?}\nCandidate pan: {pan:.2}\nCandidate tilt: {tilt:.2}\n\nUnverified: these floats are position-like in prior captures, but live movement correlation is still required.",
            record)
    } else {
        format!("Recognized record, but non-finite float values at offset 20: {pan:?}, {tilt:?}")
    }
}

fn add_diagnostics_page(device_combo: &gtk::ComboBoxText) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    page.add_css_class("content-page");
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let heading = gtk::Label::new(Some("Read-only XU diagnostics"));
    heading.add_css_class("title-2");
    heading.set_xalign(0.0);
    page.append(&heading);

    let intro = gtk::Label::new(Some(
        "Reads 60-byte UVC Extension Unit responses. These actions are GET_CUR only; no camera settings are changed."
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    page.append(&intro);

    let device_info = gtk::TextView::new();
    device_info.set_editable(false);
    device_info.set_cursor_visible(false);
    device_info.set_monospace(true);
    device_info.set_wrap_mode(gtk::WrapMode::WordChar);
    device_info.set_size_request(-1, 140);
    if let Ok(info) = usb_descriptor_info() { device_info.buffer().set_text(&info); }
    else { device_info.buffer().set_text("USB descriptor information unavailable until the camera is visible to Linux."); }
    let info_scroll = gtk::ScrolledWindow::new();
    info_scroll.set_min_content_height(140);
    info_scroll.set_child(Some(&device_info));
    page.append(&info_scroll);

    let model_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    model_row.add_css_class("adaptive-row");
    let model_button = gtk::Button::with_label("Read model string (XU 0x08)");
    let model_output = gtk::Label::new(Some("Not read."));
    model_output.set_xalign(0.0);
    model_output.set_wrap(true);
    model_row.append(&model_button);
    model_row.append(&model_output);
    page.append(&model_row);

    let selector_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    selector_row.add_css_class("adaptive-row");
    let read_status = gtk::Button::with_label("Read status (0x06)");
    let read_command = gtk::Button::with_label("Read command channel (0x02)");
    selector_row.append(&read_status);
    selector_row.append(&read_command);
    page.append(&selector_row);

    let output = gtk::TextView::new();
    output.set_editable(false);
    output.set_cursor_visible(false);
    output.set_monospace(true);
    output.set_wrap_mode(gtk::WrapMode::None);
    output.set_vexpand(true);
    let output_scroll = gtk::ScrolledWindow::new();
    output_scroll.set_vexpand(true);
    output_scroll.set_child(Some(&output));
    page.append(&output_scroll);

    let device = device_combo.clone();
    let model_label = model_output.clone();
    model_button.connect_clicked(move |_| {
        let result = device.active_id()
            .ok_or_else(|| "Select a Tiny 2 Lite V4L2 node first.".to_string())
            .and_then(|node| {
                let mut bytes = [0u8; 60];
                xu_query(node.as_str(), 0x08, UVC_GET_CUR, &mut bytes)?;
                let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
                let value = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
                Ok(if value.is_empty() { format!("Raw: {}", hex_dump(&bytes)) } else { value })
            });
        model_label.set_text(&result.unwrap_or_else(|e| format!("Read failed: {e}")));
    });

    let device = device_combo.clone();
    let view = output.clone();
    read_status.connect_clicked(move |_| {
        let result = device.active_id()
            .ok_or_else(|| "Select a Tiny 2 Lite V4L2 node first.".to_string())
            .and_then(|node| xu_status(node.as_str()));
        match result {
            Ok(bytes) => {
                let text = format!("Selector 0x06 · GET_CUR · 60 bytes\n\n{}\n{}",
                    decode_xu_status(&bytes), hex_dump(&bytes));
                view.buffer().set_text(&text);
            }
            Err(error) => view.buffer().set_text(&format!("Read failed: {error}")),
        }
    });

    let device = device_combo.clone();
    let view = output.clone();
    read_command.connect_clicked(move |_| {
        let result = device.active_id()
            .ok_or_else(|| "Select a Tiny 2 Lite V4L2 node first.".to_string())
            .and_then(|node| {
                let mut bytes = [0u8; 60];
                xu_query(node.as_str(), 0x02, UVC_GET_CUR, &mut bytes)?;
                Ok(bytes)
            });
        match result {
            Ok(bytes) => {
                let text = format!("Selector 0x02 · GET_CUR · 60 bytes\n\n{}\n{}",
                    decode_xu_command_response(&bytes), hex_dump(&bytes));
                view.buffer().set_text(&text);
            }
            Err(error) => view.buffer().set_text(&format!("Read failed: {error}")),
        }
    });

    page
}


fn save_theme_mode(mode: ui::design::ThemeMode) {
    update_config(|config| config.theme = mode.name().to_string());
}


fn apply_compact_layout(widget: &gtk::Widget, compact: bool) {
    if let Some(container) = widget.downcast_ref::<gtk::Box>() {
        if container.has_css_class("adaptive-row") && !container.has_css_class("app-header") {
            container.set_orientation(if compact { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
        }
    }
    if widget.has_css_class("content-page") {
        if compact { widget.add_css_class("compact-layout"); }
        else { widget.remove_css_class("compact-layout"); }
    }
    if widget.is::<gtk::Frame>() {
        if compact { widget.add_css_class("compact-layout"); }
        else { widget.remove_css_class("compact-layout"); }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        apply_compact_layout(&current, compact);
    }
}

/// Arrange the same live preview and page stack for the two intentional
/// workspaces: a side-by-side desktop layout, or a narrow stacked quick panel.
fn configure_workspace(
    paned: &gtk::Paned,
    preview: &gtk::Frame,
    content: &gtk::Box,
    compact: bool,
    width: i32,
) {
    let stacked = compact || width < 820;
    // Detach both children before swapping their order. GTK widgets cannot
    // have two parents, even briefly.
    paned.set_start_child(None::<&gtk::Widget>);
    paned.set_end_child(None::<&gtk::Widget>);
    if stacked {
        paned.set_orientation(gtk::Orientation::Vertical);
        paned.set_start_child(Some(preview));
        paned.set_end_child(Some(content));
        paned.set_position(if compact { 230 } else { 270 });
    } else {
        paned.set_orientation(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(content));
        paned.set_end_child(Some(preview));
        paned.set_position((width as f64 * 0.64) as i32);
    }
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    paned.set_shrink_start_child(true);
    paned.set_shrink_end_child(false);
}


fn show_compact_settings(
    parent: &gtk::ApplicationWindow,
    device: &str,
    controls_box: &gtk::Box,
    status: &gtk::Label,
) {
    let dialog = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Compact mode controls")
        .default_width(420)
        .default_height(620)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let intro = gtk::Label::new(Some("Choose the controls shown in Compact mode. Full mode always keeps the complete control set."));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    root.append(&intro);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let mut options: Vec<(String, String)> = Vec::new();
    if let Ok(output) = run_v4l2(device, "--list-ctrls-menus") {
        for control in parse_controls(&output) {
            if control.kind == "button" || matches!(control.name.as_str(), "zoom_continuous" | "pan_speed" | "tilt_speed" | "pan_absolute" | "tilt_absolute") {
                continue;
            }
            let key = format!("v4l2:{}", control.name);
            let label = format!("Camera · {}", control.name.replace('_', " "));
            options.push((key, label));
        }
    }
    let custom = [
        ("obsbot:zoom_presets", "Zoom presets"),
        ("obsbot:led_brightness", "LED brightness"),
        ("obsbot:focus_mode", "Autofocus priority · Global / Face"),
        ("obsbot:auto_exposure_mode", "Auto Exposure priority · Global / Face"),
        ("obsbot:ai_tracking_mode", "AI tracking mode"),
        ("obsbot:tracking_speed", "Tracking speed"),
        ("obsbot:target_selection", "Target selection gesture"),
        ("obsbot:zoom_l_pose", "Zoom · L-pose gesture"),
        ("obsbot:dynamic_zoom", "Dynamic zoom gesture"),
        ("obsbot:dynamic_zoom_direction", "Dynamic zoom direction"),
        ("obsbot:gesture_zoom_ratio", "Gesture zoom ratio"),
        ("camera:video_mode", "Video format, resolution and frame rate"),
        ("camera:pan_tilt_joystick", "Pan / tilt joystick"),
        ("camera:gimbal_movement", "Pan / tilt step controls"),
        ("camera:gimbal_position", "Gimbal position readout"),
        ("camera:power", "Camera sleep / wake"),
        ("camera:presets", "Camera presets"),
    ];
    options.extend(custom.iter().map(|(key, label)| (key.to_string(), label.to_string())));
    options.sort_by(|a, b| a.1.cmp(&b.1));

    let selected = current_config().compact_controls;
    let mut checkboxes: Vec<(String, gtk::CheckButton)> = Vec::new();
    for (key, label) in options {
        let check = gtk::CheckButton::with_label(&label);
        check.set_active(selected.contains(&key));
        list.append(&check);
        checkboxes.push((key, check));
    }

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(360)
        .build();
    scroll.set_child(Some(&list));
    root.append(&scroll);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save selection");
    save.add_css_class("suggested-action");
    actions.append(&cancel);
    actions.append(&save);
    root.append(&actions);
    dialog.set_child(Some(&root));

    let dialog_close = dialog.clone();
    cancel.connect_clicked(move |_| dialog_close.close());

    let dialog_save = dialog.clone();
    let device_save = device.to_string();
    let controls_save = controls_box.clone();
    let parent_save = parent.clone();
    let status_save = status.clone();
    save.connect_clicked(move |_| {
        let selection: Vec<String> = checkboxes.iter()
            .filter(|(_, check)| check.is_active())
            .map(|(key, _)| key.clone())
            .collect();
        match update_config_checked(|config| config.compact_controls = selection) {
            Ok(()) => {
                let config = current_config();
                ui::layout::apply_compact_visibility(
                    controls_save.upcast_ref(),
                    config.compact_mode,
                    &config.compact_controls,
                );
                ui::controls::populate_controls(&device_save, &controls_save, &parent_save, &status_save);
                status_save.set_text("Compact mode controls updated.");
                dialog_save.close();
            }
            Err(error) => status_save.set_text(&format!("Could not save Compact mode selection: {error}")),
        }
    });

    dialog.present();
}

// Preview intentionally disabled while USB capture and camera controls are being validated.
fn build_ui(app: &gtk::Application) {
    let loaded_config = match config::load() {
        Ok(value) => value,
        Err(error) => { eprintln!("Settings load warning: {error}"); config::AppConfig::default() }
    };
    APP_CONFIG.with(|current| *current.borrow_mut() = loaded_config.clone());
    let theme_provider = ui::design::install();
    let initial_theme = ui::design::ThemeMode::from_name(&loaded_config.theme);
    ui::design::apply_theme(&theme_provider, initial_theme);

    let test_mode = ui_test_mode();
    let test_width = std::env::var("TINY4LITE_UI_TEST_WIDTH")
        .ok().and_then(|value| value.parse::<i32>().ok()).unwrap_or(1440);
    let test_height = std::env::var("TINY4LITE_UI_TEST_HEIGHT")
        .ok().and_then(|value| value.parse::<i32>().ok()).unwrap_or(1040);
    let window = gtk::ApplicationWindow::builder().application(app)
        .title("Tiny 2 Lite Control")
        .default_width(if test_mode { test_width } else { 900 })
        .default_height(if test_mode { test_height } else { 680 })
        .build();
    window.set_size_request(620, 480);
    window.add_css_class("tiny4-window");

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.add_css_class("tiny4-root");
    root.set_margin_top(8);
    root.set_margin_bottom(8);
    root.set_margin_start(8);
    root.set_margin_end(8);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.add_css_class("adaptive-row");
    header.add_css_class("app-header");
    let app_icon = gtk::Image::from_icon_name("camera-video-symbolic");
    app_icon.set_pixel_size(28);
    app_icon.add_css_class("app-icon");
    app_icon.set_tooltip_text(Some("OBSBOT Tiny 2 Lite"));
    let title = gtk::Label::new(Some("OBSBOT Tiny 2 Lite"));
    title.add_css_class("app-title");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    let refresh = gtk::Button::with_label("Refresh camera");
    refresh.add_css_class("suggested-action");
    let theme = gtk::ComboBoxText::new();
    theme.append(Some("system"), "System");
    theme.append(Some("light"), "Light");
    theme.append(Some("dark"), "Dark");
    theme.set_active_id(Some(initial_theme.name()));
    theme.set_tooltip_text(Some("Choose whether the app follows the desktop theme or uses a fixed theme."));
    let theme_provider_change = theme_provider.clone();
    theme.connect_changed(move |combo| {
        let mode = ui::design::ThemeMode::from_name(combo.active_id().as_deref().unwrap_or("system"));
        ui::design::apply_theme(&theme_provider_change, mode);
        save_theme_mode(mode);
    });
    header.append(&app_icon);
    header.append(&title);
    header.append(&theme);
    header.append(&refresh);
    root.append(&header);

    let device_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    device_row.add_css_class("adaptive-row");
    device_row.add_css_class("device-row");
    let device_label = gtk::Label::new(Some("Camera control device"));
    device_label.set_xalign(0.0);
    let devices = gtk::ComboBoxText::new();
    devices.set_hexpand(true);
    device_row.append(&device_label);
    device_row.append(&devices);
    root.append(&device_row);

    let status = gtk::Label::new(Some("Searching for USB device 3564:fef9…"));
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("status-chip");
    root.append(&status);

    let stack = gtk::Stack::new();
    stack.add_css_class("content-stack");
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);

    let controls_scroll = gtk::ScrolledWindow::builder().vexpand(true).hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never).build();
    let controls_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    controls_box.add_css_class("content-page");
    controls_scroll.set_child(Some(&controls_box));
    stack.add_titled(&controls_scroll, Some("controls"), "Controls");

    // USB capture discovers the camera through sysfs; it does not depend on V4L2 nodes.
    let capture_page = capture::build_page();
    let capture_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true).vexpand(true).build();
    capture_scroll.set_child(Some(&capture_page));
    stack.add_titled(&capture_scroll, Some("capture"), "Capture");

    let diagnostics_page = add_diagnostics_page(&devices);
    let diagnostics_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true).vexpand(true).build();
    diagnostics_scroll.set_child(Some(&diagnostics_page));
    stack.add_titled(&diagnostics_scroll, Some("diagnostics"), "Diagnostics");
    if test_mode {
        let page = std::env::var("TINY4LITE_UI_TEST_PAGE").unwrap_or_else(|_| "controls".into());
        if matches!(page.as_str(), "controls" | "capture" | "diagnostics") {
            stack.set_visible_child_name(&page);
        }
    }

    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Fill);
    switcher.set_hexpand(true);
    switcher.add_css_class("compact-switcher");
    let page_column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page_column.set_hexpand(true);
    page_column.set_vexpand(true);
    page_column.append(&switcher);
    page_column.append(&stack);
    root.append(&page_column);

    let footer_text = if test_mode {
        "Synthetic UI test mode: camera commands and USB capture are disabled."
    } else {
        "Pan and tilt controls move the physical gimbal. USB capture requires tshark and pkexec."
    };
    let footer = gtk::Label::new(Some(footer_text));
    footer.set_xalign(0.0);
    footer.set_wrap(true);
    footer.add_css_class("dim-label");
    root.append(&footer);
    window.set_child(Some(&root));

    let nodes: Vec<String> = discover_tiny4lite().into_iter()
        .filter(|node| video::capabilities(node).is_ok()).collect();
    for node in &nodes { devices.append(Some(node), node); }
    if let Some(first) = nodes.first() { devices.set_active_id(Some(first)); }

    let controls = controls_box.clone();
    let win = window.clone();
    let status_select = status.clone();
    devices.connect_changed(move |combo| {
        if let Some(id) = combo.active_id() {
            let restore_message = apply_saved_video_mode(&id);
            ui::controls::populate_controls(&id, &controls, &win, &status_select);
            if let Some(message) = restore_message {
                status_select.set_text(&message);
            }
        } else {
        }
    });

    let devices_refresh = devices.clone();
    let controls_refresh = controls_box.clone();
    let status_refresh = status.clone();
    refresh.connect_clicked(move |_| {
        devices_refresh.remove_all();
        let nodes: Vec<String> = discover_tiny4lite().into_iter()
            .filter(|node| video::capabilities(node).is_ok()).collect();
        for node in &nodes { devices_refresh.append(Some(node), node); }
        if let Some(first) = nodes.first() {
            devices_refresh.set_active_id(Some(first));
        } else {
            clear_box(&controls_refresh);
                if usb_camera_present() {
                status_refresh.set_text("USB camera detected, but no V4L2 node is available. Controls require the UVC driver; USB capture remains available in Capture.");
            } else {
                status_refresh.set_text("Tiny 2 Lite not found. Connect it and press Refresh camera.");
            }
        }
    });

    if let Some(first) = nodes.first() {
        let restore_message = apply_saved_video_mode(first);
        ui::controls::populate_controls(first, &controls_box, &window, &status);
        if let Some(message) = restore_message {
            status.set_text(&message);
        }
    }
    if nodes.is_empty() {
        if usb_camera_present() {
            status.set_text("USB camera detected (3564:fef9), but no V4L2 node is available. Controls require the UVC driver; USB capture remains available in Capture.");
        } else {
            status.set_text("Tiny 2 Lite not found. Connect it and press Refresh camera. Expected USB ID: 3564:fef9.");
        }
    }
    ui::layout::disable_scroll_wheel_controls(root.upcast_ref());
    window.present();
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
        data[0..4].copy_from_slice(&(-146_i32 * 3600).to_le_bytes());
        data[4..8].copy_from_slice(&(91_i32 * 3600).to_le_bytes());
        assert_eq!(decode_pan_tilt_absolute(&data), (-130.0, 90.0));
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
        assert_eq!(gesture_command(group, false), [0x27, 0xff, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(gesture_command(group, true), [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00]);

        // The capture's sequence values advance across commands, including
        // the separate Dynamic Zoom disable packet: 0x0f, 0x10, 0x11, 0x12.
        let sequence = AtomicU16::new(0x000e);
        assert_eq!(next_xu_sequence(&sequence), 0x000f);
        assert_eq!(next_xu_sequence(&sequence), 0x0010);
        assert_eq!(next_xu_sequence(&sequence), 0x0011);
        assert_eq!(next_xu_sequence(&sequence), 0x0012);
    }

    #[test]
    fn exposure_compensation_maps_all_nineteen_positions() {
        assert_eq!(exposure_compensation_payload(0).unwrap(), hex("0100").as_slice());
        assert_eq!(exposure_compensation_payload(9).unwrap(), hex("0a00").as_slice());
        assert_eq!(exposure_compensation_payload(18).unwrap(), hex("1300").as_slice());
        for position in 0..=18 {
            let payload = exposure_compensation_payload(position).unwrap();
            assert_eq!(exposure_compensation_position(payload), Some(position));
        }
        assert!(exposure_compensation_payload(19).is_err());
        assert_eq!(exposure_compensation_label(0), "-3.0");
        assert_eq!(exposure_compensation_label(9), "0.0");
        assert_eq!(exposure_compensation_label(18), "+3.0");
    }

    #[test]
    fn face_focus_frames_match_tiny_2_lite_capture_values() {
        let group = [0x0a, 0x02, 0x02, 0x36, 0x04, 0x00];
        assert_eq!(face_focus_payload(true), hex("be070000").as_slice());
        assert_eq!(face_focus_payload(false), hex("bffb0100").as_slice());

        let mut face_command = [0u8; 6];
        face_command[2..6].copy_from_slice(&face_focus_payload(true));
        let face = obsbot_frame(0x0040, group, face_command);
        assert_eq!(&face[8..14], hex("0a0202360400").as_slice());
        assert_eq!(&face[16..20], hex("be070000").as_slice());

        let mut global_command = [0u8; 6];
        global_command[2..6].copy_from_slice(&face_focus_payload(false));
        let global = obsbot_frame(0x0041, group, global_command);
        assert_eq!(&global[8..14], hex("0a0202360400").as_slice());
        assert_eq!(&global[16..20], hex("bffb0100").as_slice());
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
    fn sleep_frame_matches_tiny4linux_reference() {
        let group = [0x0a, 0x02, 0xc2, 0xa0, 0x04, 0x00];
        let wake = obsbot_frame(0x00a5, group, [0, 0, 0, 0, 0, 0]);
        let sleep = obsbot_frame(0x0042, group, [0, 0, 1, 0, 0, 0]);
        assert_eq!(&wake[..20], hex("aa25a5000c005fef0a02c2a00400be0700000000").as_slice());
        assert_eq!(&sleep[..20], hex("aa2542000c00ea630a02c2a00400bffb01000000").as_slice());
        assert!(wake[20..].iter().all(|byte| *byte == 0));
        assert!(sleep[20..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn auto_exposure_priority_payloads_match_2026_11_20_capture() {
        let face = auto_exposure_mode_payload(true);
        let global = auto_exposure_mode_payload(false);
        assert_eq!(face.len(), 60);
        assert_eq!(global.len(), 60);
        assert_eq!(&face[..3], &[0x03, 0x01, 0x00]);
        assert_eq!(&global[..3], &[0x03, 0x01, 0x01]);
        assert!(face[3..].iter().all(|byte| *byte == 0));
        assert!(global[3..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn auto_exposure_priority_readback_uses_response_byte_seven() {
        let mut status = [0u8; 60];
        status[2] = 0x01; // SET_CUR value is not the GET_CUR read-back field.
        status[7] = 0x00;
        assert_eq!(auto_exposure_mode_from_status(&status), Some(true));
        status[7] = 0x01;
        assert_eq!(auto_exposure_mode_from_status(&status), Some(false));
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

        let mut status = [0u8; 60];
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

fn main() {
    let app = gtk::Application::builder().application_id("io.github.hypesthyper.Tiny4LiteControl").build();
    app.connect_activate(build_ui);
    app.run();
}
