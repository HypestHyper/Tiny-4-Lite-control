use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::fs;
use std::path::Path;
use std::process::Command;

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

fn xu_send_frame(device: &str, sequence: u16, function_group: [u8; 6], command: [u8; 6]) -> Result<(), String> {
    let mut frame = obsbot_frame(sequence, function_group, command);
    xu_query(device, 0x02, UVC_SET_CUR, &mut frame)
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

fn set_tracking_speed(device: &str, sport: bool) -> Result<(), String> {
    let group = [0x0a, 0x04, 0xc4, 0x0c, 0x01, 0x00];
    let (sequence, command) = if sport {
        (0x0021, [0x67, 0xfe, 0x02, 0x00, 0x00, 0x00])
    } else {
        (0x0020, [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00])
    };
    xu_send_frame(device, sequence, group, command)
}

fn set_gesture(device: &str, group: [u8; 6], enabled: bool) -> Result<(), String> {
    let (sequence, command) = if enabled {
        (match group[3] { 0x30 => 0x0009, 0x31 => 0x000b, 0x33 if group[2] == 0x44 => 0x000d, _ => 0x000f },
         [0x27, 0xff, 0x01, 0x00, 0x00, 0x00])
    } else {
        (match group[3] { 0x30 => 0x0008, 0x31 => 0x000a, 0x33 if group[2] == 0x44 => 0x000c, _ => 0x000e },
         [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00])
    };
    xu_send_frame(device, sequence, group, command)
}

fn ai_mode_label(status: &[u8; 60]) -> &'static str {
    match (status[0x18], status[0x1c]) {
        (0x00, 0x00) => "Off",
        (0x02, 0x00) => "Normal tracking",
        (0x02, 0x01) => "Upper body",
        (0x02, 0x02) => "Close-up",
        (0x02, 0x03) => "Headless",
        (0x02, 0x04) => "Lower body",
        (0x05, 0x00) => "Desk mode",
        (0x04, 0x00) => "Whiteboard",
        (0x03, 0x00) | (0x06, 0x00) => "Hand",
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
    let mode_label = gtk::Label::new(Some("Tracking mode"));
    mode_label.set_width_chars(22);
    mode_label.set_xalign(0.0);
    let mode = gtk::ComboBoxText::new();
    mode.set_hexpand(true);
    let modes: [(&str, &str, u8, u8); 10] = [
        ("off", "Off", 0x00, 0x00),
        ("normal", "Normal tracking", 0x02, 0x00),
        ("upper", "Upper body", 0x02, 0x01),
        ("close", "Close-up", 0x02, 0x02),
        ("headless", "Headless", 0x02, 0x03),
        ("lower", "Lower body", 0x02, 0x04),
        ("group", "Group", 0x01, 0x00),
        ("desk", "Desk mode", 0x05, 0x00),
        ("whiteboard", "Whiteboard", 0x04, 0x00),
        ("hand", "Hand", 0x03, 0x00),
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

    let gestures: [(&str, [u8; 6]); 4] = [
        ("Target selection gesture", [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00]),
        ("Zoom (L-pose) gesture", [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00]),
        ("Dynamic zoom gesture", [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00]),
        ("Dynamic zoom direction", [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00]),
    ];
    for (label, group) in gestures {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Label::new(Some(label));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        row.append(&name);
        for (caption, enabled) in [("Enable", true), ("Disable", false)] {
            let button = gtk::Button::with_label(caption);
            let dev = device.to_string();
            let status_gesture = status.clone();
            button.connect_clicked(move |_| {
                let result = set_gesture(&dev, group, enabled);
                status_gesture.set_text(&result.map(|_| format!("{label}: {} command sent.", if enabled { "enable" } else { "disable" }))
                    .unwrap_or_else(|e| format!("{label} failed: {e}")));
            });
            row.append(&button);
        }
        layout.append(&row);
    }

    let refresh_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
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

    let note = gtk::Label::new(Some("AI commands use OBSBOT's vendor XU. Sport's legacy firmware value is not auto-selected; Hand mode uses the Lite-tested 0x03 tuple. Gesture buttons send explicit enable/disable commands."));
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
    for control in controls {
        if (control.disabled && control.name != "zoom_absolute" && control.name != "focus_absolute") || (control.kind == "button" && control.name != "zoom_absolute") || control.name == "pan_absolute" || control.name == "tilt_absolute" || (joystick_enabled && (control.name == "pan_speed" || control.name == "tilt_speed")) { continue; }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
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

fn build_ui(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder().application(app)
        .title("Tiny 2 Lite Control").default_width(760).default_height(760).build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(18); root.set_margin_bottom(18); root.set_margin_start(18); root.set_margin_end(18);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = gtk::Label::new(Some("OBSBOT Tiny 2 Lite"));
    title.add_css_class("title-1"); title.set_hexpand(true); title.set_xalign(0.0);
    let refresh = gtk::Button::with_label("Refresh");
    header.append(&title); header.append(&refresh); root.append(&header);

    let device_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    device_row.append(&gtk::Label::new(Some("Camera node")));
    let devices = gtk::ComboBoxText::new(); devices.set_hexpand(true);
    device_row.append(&devices); root.append(&device_row);

    let status = gtk::Label::new(Some("Searching for USB device 3564:fef9…"));
    status.set_xalign(0.0); status.set_wrap(true); status.add_css_class("dim-label");
    root.append(&status);

    let scroll = gtk::ScrolledWindow::builder().vexpand(true).hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never).build();
    let controls_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    scroll.set_child(Some(&controls_box)); root.append(&scroll);
    let footer = gtk::Label::new(Some("Pan/tilt buttons move the physical gimbal by 1° immediately."));
    footer.set_xalign(0.0); footer.set_wrap(true); footer.add_css_class("dim-label");
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
    let win_refresh = window.clone();
    let status_refresh = status.clone();
    refresh.connect_clicked(move |_| {
        devices_refresh.remove_all();
        let nodes = discover_tiny4lite();
        for node in &nodes { devices_refresh.append(Some(node), node); }
        if let Some(first) = nodes.first() {
            devices_refresh.set_active_id(Some(first));
        } else {
            clear_box(&controls_refresh);
            status_refresh.set_text("Tiny 2 Lite not found. Connect it and press Refresh.");
        }
    });

    if nodes.is_empty() {
        status.set_text("Tiny 2 Lite not found. Connect it and press Refresh. Expected USB ID: 3564:fef9.");
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
        assert_eq!(&frame[..18], hex("aa2520000c00abcb0a04c40c0100e63f00").as_slice());
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
