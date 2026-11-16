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
    let pad_update = pad.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let width = pad_update.width() as f64;
        let height = pad_update.height() as f64;
        let cx = width / 2.0;
        let cy = height / 2.0;
        let radius = 70.0_f64.min(cx - 8.0).min(cy - 8.0).max(1.0);
        let (start_x, start_y) = *start_update.borrow();
        let x = ((start_x + dx - cx) / radius).clamp(-1.0, 1.0);
        let y = (-(start_y + dy - cy) / radius).clamp(-1.0, 1.0);
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

    let total = controls.len();
    for control in controls {
        if (control.disabled && control.name != "zoom_absolute") || (control.kind == "button" && control.name != "zoom_absolute") || control.name == "pan_absolute" || control.name == "tilt_absolute" || (joystick_enabled && (control.name == "pan_speed" || control.name == "tilt_speed")) { continue; }
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
            toggle.connect_toggled(move |button| {
                let value = if button.is_active() { 1 } else { 0 };
                let result = set_control(&dev, &name, value);
                status_label.set_text(&result.map(|_| format!("{name}: {value}"))
                    .unwrap_or_else(|e| format!("Could not set {name}: {e}")));
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
            scale.add_mark(control.default_value as f64, gtk::PositionType::Bottom,
                Some(&format!("Default: {}", control.default_value)));

            let reset = gtk::Button::with_label("Reset");
            reset.set_sensitive(control.value != control.default_value);
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

fn main() {
    let app = gtk::Application::builder().application_id("io.github.hypesthyper.Tiny4LiteControl").build();
    app.connect_activate(build_ui);
    app.run();
}
