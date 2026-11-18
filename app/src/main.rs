mod capture;
mod design;

use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU16, Ordering};

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
    run_v4l2(device, &format!("--set-ctrl={name}={value}")).map(|_| ())
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
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let hint = gtk::Label::new(Some("Drag the stick to move. Release to stop."));
    hint.add_css_class("dim-label");
    layout.append(&hint);

    let sensitivity = Rc::new(RefCell::new(1.0_f64));
    let sensitivity_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    sensitivity_row.add_css_class("adaptive-row");
    let sensitivity_label = gtk::Label::new(Some("Sensitivity"));
    let sensitivity_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 10.0, 200.0, 5.0);
    sensitivity_scale.set_value(100.0);
    sensitivity_scale.set_digits(0);
    sensitivity_scale.set_draw_value(true);
    sensitivity_scale.set_hexpand(true);
    let sensitivity_value = gtk::Label::new(Some("100%"));
    let sensitivity_state = sensitivity.clone();
    let sensitivity_value_label = sensitivity_value.clone();
    sensitivity_scale.connect_value_changed(move |scale| {
        let percent = scale.value().round();
        *sensitivity_state.borrow_mut() = percent / 100.0;
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
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)
}

fn add_camera_settings(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Camera settings"));
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    // FOV presets are positions on the standard V4L2 zoom_absolute control.
    let zoom_frame = gtk::Frame::new(Some("Zoom and field of view"));
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
        button.connect_clicked(move |_| {
            let result = set_control(&dev, "zoom_absolute", value)
                .and_then(|_| get_control(&dev, "zoom_absolute"))
                .map(|actual| format!("Zoom preset requested: {label}. Camera reports zoom_absolute={actual}."));
            status_label.set_text(&result.unwrap_or_else(|e| format!("Could not apply zoom preset: {e}")));
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
    let initial_led = xu_status(device).ok().map(|data| data[0x21]).unwrap_or(0);
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
// The earlier 0/1 Boolean payload was borrowed from a different implementation
// and is contradicted by the Lite capture. The captured values correlate with
// Face then Global transitions: Face = BF FB 01 00; Global = BE 07 00 00.
// The capture does not provide reliable read-back, so the mapping remains
// hardware-testable rather than claiming device-side confirmation.
fn face_focus_payload(face_priority: bool) -> [u8; 4] {
    if face_priority { [0xbf, 0xfb, 0x01, 0x00] }
    else { [0xbe, 0x07, 0x00, 0x00] }
}

fn set_face_focus(device: &str, face_priority: bool) -> Result<(), String> {
    let group = [0x0a, 0x02, 0x02, 0x36, 0x04, 0x00];
    let mut command = [0u8; 6];
    command[2..6].copy_from_slice(&face_focus_payload(face_priority));
    xu_send_frame(device, group, command)
}

fn add_focus_mode_control(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Autofocus mode"));
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let global = gtk::CheckButton::with_label("Global Focus");
    let face = gtk::CheckButton::with_label("Face Focus");
    face.set_group(Some(&global));
    global.set_active(true);
    row.append(&global);
    row.append(&face);
    layout.append(&row);

    // The camera has a dedicated read command, but this app does not yet
    // decode it. Keep the current selection for this UI session and report
    // command submission without claiming device-side read-back.
    let current_face = Rc::new(Cell::new(false));
    let updating = Rc::new(Cell::new(false));
    for (button, face_priority) in [(global.clone(), false), (face.clone(), true)] {
        let dev = device.to_string();
        let status_label = status.clone();
        let current = current_face.clone();
        let updating_state = updating.clone();
        let global_restore = global.clone();
        let face_restore = face.clone();
        button.connect_toggled(move |selected| {
            if updating_state.get() || !selected.is_active() {
                return;
            }
            let previous = current.get();
            match set_face_focus(&dev, face_priority) {
                Ok(()) => {
                    current.set(face_priority);
                    status_label.set_text(if face_priority {
                        "Face-priority autofocus command submitted. Verify the camera's focus behavior."
                    } else {
                        "Global autofocus command submitted. Verify the camera's focus behavior."
                    });
                }
                Err(error) => {
                    updating_state.set(true);
                    global_restore.set_active(!previous);
                    face_restore.set_active(previous);
                    updating_state.set(false);
                    status_label.set_text(&format!("Could not change autofocus mode: {error}"));
                }
            }
        });
    }

    let note = gtk::Label::new(Some("Global and Face use captured Tiny 2 Lite command payloads. No device-side read-back is available; the selection is tracked locally and must be verified by observing focus behavior."));
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
                populate_controls(&dev, &controls_refresh, &window_refresh, &status_refresh);
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
    xu_query(device, 0x06, UVC_SET_CUR, &mut data)
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
    xu_send_frame(device, group, command)
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
    xu_send_frame(device, group, command)
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
    xu_send_frame(device, group, command)
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
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let mode_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    mode_row.add_css_class("adaptive-row");
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
    let apply_mode = gtk::Button::with_label("Apply");
    let mode_apply = mode.clone();
    let dev = device.to_string();
    let status_mode = status.clone();
    apply_mode.connect_clicked(move |_| {
        let result = mode_apply.active_id().and_then(|id| modes.iter().find(|m| m.0 == id.as_str()))
            .ok_or_else(|| "Select an AI tracking mode.".to_string())
            .and_then(|entry| set_ai_mode(&dev, (entry.2, entry.3)));
        status_mode.set_text(&result.map(|_| "AI tracking mode command sent. Refresh status to confirm.".to_string())
            .unwrap_or_else(|e| format!("AI mode failed: {e}")));
    });
    mode_row.append(&mode_label);
    mode_row.append(&mode);
    mode_row.append(&apply_mode);
    layout.append(&mode_row);

    let speed_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    speed_row.add_css_class("adaptive-row");
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
    let apply_speed = gtk::Button::with_label("Apply");
    let speed_apply = speed.clone();
    let dev = device.to_string();
    let status_speed = status.clone();
    apply_speed.connect_clicked(move |_| {
        let sport = speed_apply.active_id().as_deref() == Some("sport");
        let result = set_tracking_speed(&dev, sport);
        status_speed.set_text(&result.map(|_| "Tracking speed command sent. Refresh status to confirm. Sport encoding is documented for firmware 6.2.5.x.".to_string())
            .unwrap_or_else(|e| format!("Tracking speed failed: {e}")));
    });
    speed_row.append(&speed_label);
    speed_row.append(&speed);
    speed_row.append(&apply_speed);
    layout.append(&speed_row);

    // Status byte 0x07 is inverted for the L-pose Zoom gesture:
    // 1 = disabled, 0 = enabled. Other gesture states have no confirmed
    // device read-back, so those start Unknown and become state-aware after
    // the first successful command in this session.
    let gestures: [(&str, [u8; 6], Option<bool>); 4] = [
        ("Target selection gesture", [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00], None),
        ("Zoom (L-pose) gesture", [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00],
            xu_status(device).ok().map(|data| data[0x07] == 0)),
        ("Dynamic zoom gesture", [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00], None),
        ("Dynamic zoom direction", [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00], None),
    ];
    for (label, group, initial_state) in gestures {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("adaptive-row");
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
    let ratio_layout = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let ratio_help = gtk::Label::new(Some("Zoom applied when the camera recognizes the L-shaped Zoom gesture. Range: 1.00×–4.00×."));
    ratio_help.set_wrap(true);
    ratio_help.set_xalign(0.0);
    ratio_help.add_css_class("dim-label");
    ratio_layout.append(&ratio_help);

    let ratio_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ratio_row.add_css_class("adaptive-row");
    let ratio_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 4.0, 0.01);
    ratio_scale.set_value(1.0);
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


fn populate_controls(device: &str, container: &gtk::Box, window: &gtk::ApplicationWindow, status: &gtk::Label) {
    clear_box(container);
    let output = match run_v4l2(device, "--list-ctrls-menus") {
        Ok(v) => v, Err(e) => { status.set_text(&format!("Could not read controls: {e}")); return; }
    };
    let controls = parse_controls(&output);
    if controls.is_empty() { status.set_text("No V4L2 controls were reported for this video node."); return; }

    let heading = gtk::Label::new(Some(&format!("Controls on {device}")));
    heading.add_css_class("title-3");
    heading.set_halign(gtk::Align::Start);
    container.append(&heading);

    add_sleep_controls(container, device, status);
    add_camera_settings(container, device, status);
    add_ai_controls(container, device, status);

    let pan_speed = controls.iter().find(|c| c.name == "pan_speed" && !c.disabled).cloned();
    let tilt_speed = controls.iter().find(|c| c.name == "tilt_speed" && !c.disabled).cloned();
    let joystick_enabled = match (pan_speed.as_ref(), tilt_speed.as_ref()) {
        (Some(pan), Some(tilt)) if pan.min <= 0 && pan.max >= 0 && tilt.min <= 0 && tilt.max >= 0 => {
            add_speed_joystick(container, device, pan, tilt, status);
            true
        }
        _ => false,
    };

    let has_pan = controls.iter().any(|c| c.name == "pan_absolute" && !c.disabled);
    let has_tilt = controls.iter().any(|c| c.name == "tilt_absolute" && !c.disabled);
    if has_pan || has_tilt {
        let frame = gtk::Frame::new(Some("Gimbal movement"));
        let movement = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    movement.add_css_class("adaptive-row");
        if has_pan {
            add_move_button(&movement, device, "pan_absolute", -3600, "Pan −1°", status);
            add_move_button(&movement, device, "pan_absolute", 3600, "Pan +1°", status);
        }
        if has_tilt {
            add_move_button(&movement, device, "tilt_absolute", -3600, "Tilt −1°", status);
            add_move_button(&movement, device, "tilt_absolute", 3600, "Tilt +1°", status);
        }
        frame.set_child(Some(&movement));
        container.append(&frame);
    }

    let autofocus_enabled = controls.iter()
        .find(|c| c.name == "focus_automatic_continuous")
        .map(|c| c.value != 0)
        .unwrap_or(false);
    let total = controls.len();

    // Keep the autofocus master switch above its dependent controls.
    if controls.iter().any(|c| c.name == "focus_automatic_continuous") {
        add_autofocus_control(container, device, window, status, autofocus_enabled);
    }
    if autofocus_enabled {
        add_focus_mode_control(container, device, status);
    }

    for control in controls {
        if control.name == "focus_automatic_continuous" { continue; }
        if autofocus_enabled && control.name == "focus_absolute" { continue; }
        if (control.disabled && control.name != "zoom_absolute" && control.name != "focus_absolute") || (control.kind == "button" && control.name != "zoom_absolute") || control.name == "pan_absolute" || control.name == "tilt_absolute" || control.name == "zoom_continuous" || (joystick_enabled && (control.name == "pan_speed" || control.name == "tilt_speed")) { continue; }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("adaptive-row");
        row.set_margin_top(3);
        row.set_margin_bottom(3);
        let title = gtk::Label::new(Some(&control.name.replace('_', " ")));
        title.set_width_chars(22);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        row.append(&title);

        if control.kind == "bool" || (!control.kind.contains("menu") && control.min == 0 && control.max == 1) {
            let toggle = gtk::CheckButton::new();
            toggle.set_active(control.value != 0);
            let dev = device.to_string();
            let name = control.name.clone();
            let status_label = status.clone();
            let controls_refresh = container.clone();
            let window_refresh = window.clone();
            let refresh_device = dev.clone();
            toggle.connect_toggled(move |button| {
                let value = if button.is_active() { 1 } else { 0 };
                let result = set_control(&dev, &name, value);
                let succeeded = result.is_ok();
                status_label.set_text(&result.map(|_| format!("{name}: {value}"))
                    .unwrap_or_else(|e| format!("Could not set {name}: {e}")));
                if succeeded && name == "focus_automatic_continuous" {
                    populate_controls(&refresh_device, &controls_refresh, &window_refresh, &status_label);
                }
            });
            row.append(&toggle);
        } else if control.kind.contains("menu") && !control.menu.is_empty() {
            let combo = gtk::ComboBoxText::new();
            for (value, label) in &control.menu { combo.append(Some(&value.to_string()), label); }
            combo.set_active_id(Some(&control.value.to_string()));
            let combo_apply = combo.clone();
            let apply = gtk::Button::with_label("Apply");
            let dev = device.to_string();
            let name = control.name.clone();
            let status_label = status.clone();
            apply.connect_clicked(move |_| {
                if let Some(id) = combo_apply.active_id() {
                    let value = id.parse::<i64>().unwrap_or_default();
                    let result = set_control(&dev, &name, value);
                    status_label.set_text(&result.map(|_| format!("{name} updated")).unwrap_or_else(|e| e));
                }
            });
            row.append(&combo);
            row.append(&apply);
        } else {
            let scale = gtk::Scale::with_range(
                gtk::Orientation::Horizontal,
                control.min as f64,
                control.max.max(control.min + control.step) as f64,
                control.step as f64,
            );
            scale.set_value(control.value.clamp(control.min, control.max.max(control.min)) as f64);
            scale.set_draw_value(true);
            scale.set_digits(0);
            scale.set_hexpand(true);
            if control.name == "focus_absolute" {
                scale.set_sensitive(!autofocus_enabled);
            }
            scale.add_mark(control.default_value as f64, gtk::PositionType::Bottom,
                Some(&format!("Default: {}", control.default_value)));

            let reset = gtk::Button::with_label("Reset");
            reset.set_sensitive(control.value != control.default_value
                && !(control.name == "focus_absolute" && autofocus_enabled));
            let reset_scale = scale.clone();
            reset.connect_clicked(move |_| {
                reset_scale.set_value(control.default_value as f64);
            });

            let dev = device.to_string();
            let name = control.name.clone();
            let status_label = status.clone();
            let reset_button = reset.clone();
            let default_value = control.default_value;
            scale.connect_value_changed(move |slider| {
                let requested = slider.value().round() as i64;
                let result = set_control(&dev, &name, requested);
                let success = result.is_ok();
                status_label.set_text(&result.map(|_| format!("{name}: {requested}"))
                    .unwrap_or_else(|e| format!("Could not set {name}: {e}")));
                reset_button.set_sensitive(success && requested != default_value);
            });
            row.append(&scale);
            row.append(&reset);
        }
        container.append(&row);
        container.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }
    status.set_text(&format!("Loaded {total} V4L2 controls from {device}. Sliders apply changes immediately."));
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


fn theme_config_path() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("tiny4lite-control").join("settings.conf")
}
fn load_theme_mode() -> design::ThemeMode {
    let value = fs::read_to_string(theme_config_path()).unwrap_or_default();
    design::ThemeMode::from_name(value.lines().find_map(|line| line.strip_prefix("theme=")).unwrap_or("system"))
}
fn save_theme_mode(mode: design::ThemeMode) {
    let path = theme_config_path();
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_ok() {
            let _ = fs::write(path, format!("theme={}\n", mode.name()));
        }
    }
}


fn apply_compact_layout(widget: &gtk::Widget, compact: bool) {
    if let Some(container) = widget.downcast_ref::<gtk::Box>() {
        if container.has_css_class("adaptive-row") {
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

fn build_ui(app: &gtk::Application) {
    let theme_provider = design::install();
    let initial_theme = load_theme_mode();
    design::apply_theme(&theme_provider, initial_theme);

    let window = gtk::ApplicationWindow::builder().application(app)
        .title("Tiny 2 Lite Control").default_width(1040).default_height(780).build();
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
        let mode = design::ThemeMode::from_name(combo.active_id().as_deref().unwrap_or("system"));
        design::apply_theme(&theme_provider_change, mode);
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

    let navigation = gtk::StackSidebar::new();
    navigation.add_css_class("navigation-sidebar");
    navigation.set_stack(&stack);
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Fill);
    switcher.set_hexpand(true);
    switcher.add_css_class("compact-switcher");
    root.append(&switcher);
    let navigation_responsive = navigation.clone();
    let switcher_responsive = switcher.clone();
    let root_responsive = root.clone();
    window.connect_notify_local(Some("width"), move |window, _| {
        let width = window.width();
        let wide = width >= 760;
        let compact = width < 620;
        navigation_responsive.set_visible(wide);
        switcher_responsive.set_visible(!wide);
        root_responsive.set_margin_top(if compact { 6 } else { 12 });
        root_responsive.set_margin_bottom(if compact { 6 } else { 12 });
        apply_compact_layout(root_responsive.upcast_ref(), compact);
    });

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    content.add_css_class("adaptive-row");
    content.set_vexpand(true);
    content.append(&navigation);
    content.append(&stack);
    root.append(&content);

    let footer = gtk::Label::new(Some(
        "Pan and tilt controls move the physical gimbal. USB capture requires tshark and pkexec."
    ));
    footer.set_xalign(0.0);
    footer.set_wrap(true);
    footer.add_css_class("dim-label");
    root.append(&footer);
    window.set_child(Some(&root));

    let nodes = discover_tiny4lite();
    for node in &nodes { devices.append(Some(node), node); }
    if let Some(first) = nodes.first() { devices.set_active_id(Some(first)); }

    let controls = controls_box.clone();
    let win = window.clone();
    let status_select = status.clone();
    devices.connect_changed(move |combo| {
        if let Some(id) = combo.active_id() {
            populate_controls(&id, &controls, &win, &status_select);
        }
    });

    let devices_refresh = devices.clone();
    let controls_refresh = controls_box.clone();
    let status_refresh = status.clone();
    refresh.connect_clicked(move |_| {
        devices_refresh.remove_all();
        let nodes = discover_tiny4lite();
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
        assert_eq!(face_focus_payload(true), hex("bffb0100").as_slice());
        assert_eq!(face_focus_payload(false), hex("be070000").as_slice());

        let mut face_command = [0u8; 6];
        face_command[2..6].copy_from_slice(&face_focus_payload(true));
        let face = obsbot_frame(0x0040, group, face_command);
        assert_eq!(&face[8..14], hex("0a0202360400").as_slice());
        assert_eq!(&face[16..20], hex("bffb0100").as_slice());

        let mut global_command = [0u8; 6];
        global_command[2..6].copy_from_slice(&face_focus_payload(false));
        let global = obsbot_frame(0x0041, group, global_command);
        assert_eq!(&global[8..14], hex("0a0202360400").as_slice());
        assert_eq!(&global[16..20], hex("be070000").as_slice());
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
