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

fn run_v4l2(device: &str, option: &str) -> Result<String, String> {
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
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let hint = gtk::Label::new(Some("Drag the stick to move. Release to stop."));
    hint.add_css_class("dim-label");
    layout.append(&hint);

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
    layout.append(&sensitivity_row);

    let pad = gtk::DrawingArea::new();
    pad.set_content_width(240);
    pad.set_content_height(190);
    pad.set_hexpand(true);
    let position = Rc::new(RefCell::new((0.0_f64, 0.0_f64)));
    let draw_position = position.clone();
    pad.set_draw_func(move |_, cr, width, height| {
        let cx = width as f64 / 2.0;
        let cy = height as f64 / 2.0;
        let radius = 70.0_f64.min(cx - 8.0).min(cy - 8.0);
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
    let drag_start = Rc::new(RefCell::new((120.0_f64, 95.0_f64)));
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
        let radius = 70.0_f64.min(cx - 8.0).min(cy - 8.0).max(1.0);
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

fn xu_status(device: &str) -> Result<[u8; 60], String> {
    let mut data = [0u8; 60];
    xu_query(device, 0x06, UVC_GET_CUR, &mut data)?;
    Ok(data)
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
static XU_SEQUENCE: AtomicU16 = AtomicU16::new(0);

fn next_xu_sequence(counter: &AtomicU16) -> u16 {
    counter.fetch_add(1, Ordering::Relaxed)
}

fn xu_send_frame(device: &str, function_group: [u8; 6], command: [u8; 6]) -> Result<(), String> {
    let sequence = next_xu_sequence(&XU_SEQUENCE);
    let mut frame = obsbot_frame(sequence, function_group, command);
    xu_query(device, 0x02, UVC_SET_CUR, &mut frame)
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
    for (label, value) in [("Wide · 1.00×", 0_i64), ("Medium · 1.15×", 5), ("Narrow · 1.47×", 15)] {
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
    let zoom_note = gtk::Label::new(Some("All zoom uses V4L2 zoom_absolute (0–100). The general slider below provides smooth adjustment; these buttons set common FOV positions. zoom_continuous and the separate vendor FOV command are not used."));
    zoom_note.set_wrap(true);
    zoom_note.set_xalign(0.0);
    zoom_note.add_css_class("dim-label");
    zoom_layout.append(&zoom_note);
    zoom_frame.set_child(Some(&zoom_layout));
    layout.append(&zoom_frame);
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
    led_level.connect_value_changed(move |scale| {
        let setting = scale.value().round() as u8;
        let result = set_led_brightness(&led_slider_device, setting);
        let description = match setting {
            0 => "Off".to_string(),
            n => format!("brightness level {n}"),
        };
        led_slider_status.set_text(&result.map(|_| format!("LED setting: {description}."))
            .unwrap_or_else(|e| format!("Could not change LED setting: {e}")));
    });

    let led_note = gtk::Label::new(Some("Off plus three brightness levels. Each position sends selector 0x06, tag 0x1A directly. The separate tag 0x18 command is intentionally not used here because its observed effect is blink/steady behaviour, not a verified power switch."));
    led_note.set_wrap(true);
    led_note.set_xalign(0.0);
    led_note.add_css_class("dim-label");
    led_layout.append(&led_note);
    led_frame.set_child(Some(&led_layout));
    layout.append(&led_frame);

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

fn add_focus_mode_control(container: &gtk::Box, device: &str, status: &gtk::Label) {
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

    let note = gtk::Label::new(Some("Global and Face are mutually exclusive autofocus priorities. The camera has no confirmed read-back for this setting, so the selected value is the last successfully submitted command."));
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
    device_info.set_size_request(-1, 105);
    if let Ok(info) = usb_descriptor_info() { device_info.buffer().set_text(&info); }
    else { device_info.buffer().set_text("USB descriptor information unavailable until the camera is visible to Linux."); }
    let info_scroll = gtk::ScrolledWindow::new();
    info_scroll.set_min_content_height(105);
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

    let window = gtk::ApplicationWindow::builder().application(app)
        .title("Tiny 2 Lite Control")
        .default_width(900)
        .default_height(680)
        .build();
    window.set_size_request(620, 480);
    window.add_css_class("tiny4-window");

    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.add_css_class("tiny4-root");
    root.set_margin_top(8);
    root.set_margin_bottom(8);
    root.set_margin_start(8);
    root.set_margin_end(8);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.add_css_class("adaptive-row");
    header.add_css_class("app-header");
    let app_icon = gtk::Label::new(Some("videocam"));
    app_icon.add_css_class("material-icon");
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
    let controls_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
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

    let footer = gtk::Label::new(Some(
        "Pan and tilt controls move the physical gimbal. USB capture requires tshark and pkexec."
    ));
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
        let sequence = AtomicU16::new(0x000f);
        assert_eq!(next_xu_sequence(&sequence), 0x000f);
        assert_eq!(next_xu_sequence(&sequence), 0x0010);
        assert_eq!(next_xu_sequence(&sequence), 0x0011);
        assert_eq!(next_xu_sequence(&sequence), 0x0012);
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
    fn led_setting_payload_matches_off_and_three_brightness_levels() {
        assert_eq!(led_brightness_payload(0).unwrap(), [0x1a, 0x01, 0x00]);
        assert_eq!(led_brightness_payload(1).unwrap(), [0x1a, 0x01, 0x01]);
        assert_eq!(led_brightness_payload(2).unwrap(), [0x1a, 0x01, 0x02]);
        assert_eq!(led_brightness_payload(3).unwrap(), [0x1a, 0x01, 0x03]);
        assert!(led_brightness_payload(4).is_err());
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
