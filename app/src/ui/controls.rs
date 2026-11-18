use gtk::prelude::*;
use std::collections::HashMap;
use crate::*;

fn control_blockers(control: &Control, controls: &[Control]) -> Vec<String> {
    let value = |name: &str| controls.iter().find(|item| item.name == name).map(|item| item.value);
    let mut blockers = Vec::new();
    match control.name.as_str() {
        "red_balance" | "blue_balance" | "white_balance_temperature" => {
            if value("white_balance_automatic").unwrap_or(0) != 0 {
                blockers.push("Auto white balance is enabled".to_string());
            }
        }
        "exposure_time_absolute" => {
            if value("auto_exposure").is_some_and(|mode| mode != 1) {
                blockers.push("Exposure mode is not Manual".to_string());
            }
        }
        "focus_absolute" => {
            if value("focus_automatic_continuous").unwrap_or(0) != 0 {
                blockers.push("Autofocus is enabled".to_string());
            }
        }
        _ => {}
    }
    if control.disabled {
        blockers.push(if control.kind.contains("read-only") {
            "The V4L2 driver marks this control read-only".to_string()
        } else {
            "The V4L2 driver reports this control as inactive or disabled".to_string()
        });
    }
    blockers
}

fn v4l2_control_group(name: &str) -> &'static str {
    match name {
        "white_balance_automatic" | "red_balance" | "blue_balance" | "white_balance_temperature" => "White balance",
        "auto_exposure" | "exposure_time_absolute" | "power_line_frequency" => "Exposure",
        "focus_absolute" => "Focus",
        "zoom_absolute" => "Zoom",
        _ => "Image",
    }
}

pub(crate) fn populate_controls(device: &str, container: &gtk::Box, window: &gtk::ApplicationWindow, status: &gtk::Label) {
    let scroll_position = ui::layout::capture_scroll_position(container.upcast_ref());
    clear_box(container);

    let output = match run_v4l2(device, "--list-ctrls-menus") {
        Ok(value) => value,
        Err(error) => { status.set_text(&format!("Could not read controls: {error}")); return; }
    };
    let controls = parse_controls(&output);
    if controls.is_empty() {
        status.set_text("No V4L2 controls were reported for this video node.");
        return;
    }

    // Each category is a real page, not another heading in one endless form.
    let categories: [(&str, &str, &str); 10] = [
        ("camera", "Camera", "Camera power, status and device controls"),
        ("image", "Image", "Brightness, contrast, saturation and image detail"),
        ("white-balance", "White balance", "Automatic and manual colour temperature"),
        ("exposure", "Exposure", "Exposure mode, shutter and anti-flicker"),
        ("focus", "Focus", "Autofocus, focus priority and manual focus"),
        ("zoom", "Zoom", "Field of view and optical zoom controls"),
        ("movement", "Movement", "Pan, tilt and joystick movement"),
        ("tracking", "Tracking", "AI tracking modes and gesture controls"),
        ("video", "Video", "Capture format, resolution and frame rate"),
        ("presets", "Presets", "Save and recall camera configurations"),
    ];
    let page_stack = gtk::Stack::new();
    page_stack.add_css_class("settings-stack");
    page_stack.set_hexpand(true);
    page_stack.set_vexpand(true);
    page_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    page_stack.set_transition_duration(140);

    let mut pages: HashMap<&'static str, gtk::Box> = HashMap::new();
    for (name, title, description) in categories {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        page.add_css_class("settings-page");
        page.set_margin_top(8);
        page.set_margin_bottom(8);
        page.set_margin_start(8);
        page.set_margin_end(8);

        let intro = gtk::Box::new(gtk::Orientation::Vertical, 3);
        intro.add_css_class("section-intro");
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("title-2");
        heading.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(description));
        subtitle.add_css_class("dim-label");
        subtitle.set_xalign(0.0);
        subtitle.set_wrap(true);
        intro.append(&heading);
        intro.append(&subtitle);
        page.append(&intro);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.add_css_class("settings-section-list");
        page.append(&body);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true).vexpand(true).build();
        scroll.set_child(Some(&page));
        page_stack.add_titled(&scroll, Some(name), title);
        pages.insert(name, body);
    }

    let category_nav = gtk::StackSidebar::new();
    category_nav.add_css_class("category-sidebar");
    category_nav.set_stack(&page_stack);
    let category_switcher = gtk::StackSwitcher::new();
    category_switcher.add_css_class("category-switcher");
    category_switcher.set_stack(Some(&page_stack));
    category_switcher.set_hexpand(true);

    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    heading.add_css_class("controls-toolbar");
    let device_title = gtk::Label::new(Some("CAMERA SETTINGS"));
    device_title.add_css_class("eyebrow");
    device_title.set_xalign(0.0);
    device_title.set_hexpand(true);
    let device_name = gtk::Label::new(Some(device));
    device_name.add_css_class("dim-label");
    device_name.add_css_class("device-caption");
    heading.append(&device_title);
    heading.append(&device_name);
    container.append(&heading);
    container.append(&category_switcher);

    let workspace = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    workspace.add_css_class("category-workspace");
    workspace.set_hexpand(true);
    workspace.set_vexpand(true);
    workspace.append(&category_nav);
    workspace.append(&page_stack);
    container.append(&workspace);

    let page = |key: &str| -> gtk::Box {
        pages.get(key).expect("settings category exists").clone()
    };
    add_sleep_controls(&page("camera"), device, status);
    add_camera_settings(&page("zoom"), device, window, status);
    add_ai_controls(&page("tracking"), device, status);
    ui::presets::append(&page("presets"), device, window, status);
    ui::video::append(&page("video"), device, status);

    let pan_speed = controls.iter().find(|control| control.name == "pan_speed" && !control.disabled).cloned();
    let tilt_speed = controls.iter().find(|control| control.name == "tilt_speed" && !control.disabled).cloned();
    let joystick_enabled = match (pan_speed.as_ref(), tilt_speed.as_ref()) {
        (Some(pan), Some(tilt)) if pan.min <= 0 && pan.max >= 0 && tilt.min <= 0 && tilt.max >= 0 => {
            add_speed_joystick(&page("movement"), device, pan, tilt, status);
            true
        }
        _ => false,
    };

    let has_pan = controls.iter().any(|control| control.name == "pan_absolute" && !control.disabled);
    let has_tilt = controls.iter().any(|control| control.name == "tilt_absolute" && !control.disabled);
    if has_pan || has_tilt {
        let frame = gtk::Frame::new(Some("Gimbal movement"));
        frame.add_css_class("control-card");
        ui::layout::mark_compact_item(frame.upcast_ref(), "camera:gimbal-movement");
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
        page("movement").append(&frame);
    }

    let autofocus_enabled = controls.iter()
        .find(|control| control.name == "focus_automatic_continuous")
        .map(|control| control.value != 0)
        .unwrap_or(false);
    if controls.iter().any(|control| control.name == "focus_automatic_continuous") {
        add_autofocus_control(&page("focus"), device, window, status, autofocus_enabled);
    }
    if autofocus_enabled {
        add_focus_mode_control(&page("focus"), device, status);
    }

    let mut control_groups: HashMap<&'static str, gtk::Box> = HashMap::new();
    for control in controls.iter().cloned() {
        if control.name == "focus_automatic_continuous" { continue; }
        if (control.kind == "button" && control.name != "zoom_absolute")
            || control.name == "pan_absolute"
            || control.name == "tilt_absolute"
            || control.name == "zoom_continuous"
            || (joystick_enabled && (control.name == "pan_speed" || control.name == "tilt_speed")) {
            continue;
        }

        let blockers = control_blockers(&control, &controls);
        let unavailable = !blockers.is_empty();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("control-row");
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        let title = gtk::Label::new(Some(&control.name.replace('_', " ")));
        title.add_css_class("control-label");
        title.set_width_chars(20);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        row.append(&title);

        if control.kind == "bool" || (!control.kind.contains("menu") && control.min == 0 && control.max == 1) {
            let toggle = gtk::Switch::new();
            toggle.set_active(control.value != 0);
            toggle.set_sensitive(!unavailable);
            let dev = device.to_string();
            let name = control.name.clone();
            let status_label = status.clone();
            let controls_refresh = container.clone();
            let window_refresh = window.clone();
            let refresh_device = dev.clone();
            toggle.connect_state_set(move |switch, active| {
                let result = set_control(&dev, &name, if active { 1 } else { 0 });
                let succeeded = result.is_ok();
                status_label.set_text(&result.map(|_| format!("{name}: {}", if active { "On" } else { "Off" }))
                    .unwrap_or_else(|error| format!("Could not set {name}: {error}")));
                if succeeded && matches!(name.as_str(), "white_balance_automatic" | "focus_automatic_continuous") {
                    populate_controls(&refresh_device, &controls_refresh, &window_refresh, &status_label);
                }
                switch.set_state(active);
                glib::Propagation::Stop
            });
            row.append(&toggle);
        } else if control.kind.contains("menu") && !control.menu.is_empty() {
            let combo = gtk::ComboBoxText::new();
            for (value, label) in &control.menu { combo.append(Some(&value.to_string()), label); }
            combo.set_active_id(Some(&control.value.to_string()));
            combo.set_sensitive(!unavailable);
            let dev = device.to_string();
            let name = control.name.clone();
            let status_label = status.clone();
            let controls_refresh = container.clone();
            let window_refresh = window.clone();
            let refresh_device = dev.clone();
            let last_value = Rc::new(Cell::new(control.value));
            let changing = Rc::new(Cell::new(false));
            let last_value_change = last_value.clone();
            let changing_state = changing.clone();
            let combo_restore = combo.clone();
            combo.connect_changed(move |selection| {
                if changing_state.get() { return; }
                let Some(id) = selection.active_id() else { return; };
                let Ok(value) = id.parse::<i64>() else { return; };
                match set_control(&dev, &name, value) {
                    Ok(()) => {
                        last_value_change.set(value);
                        status_label.set_text(&format!("{name} set to {value}."));
                        if name == "auto_exposure" {
                            populate_controls(&refresh_device, &controls_refresh, &window_refresh, &status_label);
                        }
                    }
                    Err(error) => {
                        changing_state.set(true);
                        combo_restore.set_active_id(Some(&last_value_change.get().to_string()));
                        changing_state.set(false);
                        status_label.set_text(&format!("Could not set {name}: {error}"));
                    }
                }
            });
            row.append(&combo);
        } else {
            let scale = gtk::Scale::with_range(
                gtk::Orientation::Horizontal,
                control.min as f64,
                control.max.max(control.min) as f64,
                control.step.max(1) as f64,
            );
            scale.set_value(control.value.clamp(control.min, control.max.max(control.min)) as f64);
            scale.set_draw_value(true);
            scale.set_digits(0);
            scale.set_hexpand(true);
            scale.set_sensitive(!unavailable);
            scale.add_mark(control.default_value as f64, gtk::PositionType::Bottom,
                Some(&format!("Default: {}", control.default_value)));

            let reset = gtk::Button::with_label("Reset");
            reset.add_css_class("control-reset");
            reset.set_sensitive(!unavailable && control.value != control.default_value);
            let reset_scale = scale.clone();
            reset.connect_clicked(move |_| reset_scale.set_value(control.default_value as f64));

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
                    .unwrap_or_else(|error| format!("Could not set {name}: {error}")));
                reset_button.set_sensitive(success && requested != default_value);
            });
            row.append(&scale);
            row.append(&reset);
        }

        let group_name = v4l2_control_group(&control.name);
        let group = control_groups.entry(group_name).or_insert_with(|| {
            let frame = gtk::Frame::new(Some(group_name));
            frame.add_css_class("control-card");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            content.set_margin_top(8);
            content.set_margin_bottom(8);
            content.set_margin_start(10);
            content.set_margin_end(10);
            frame.set_child(Some(&content));
            let target = match group_name {
                "White balance" => page("white-balance"),
                "Exposure" => page("exposure"),
                "Focus" => page("focus"),
                "Zoom" => page("zoom"),
                _ => page("image"),
            };
            target.append(&frame);
            content
        });
        ui::layout::mark_compact_item(row.upcast_ref(), &format!("v4l2:{}", control.name));
        group.append(&row);
        if unavailable {
            let note = gtk::Label::new(Some(&format!("Unavailable: {}.", blockers.join("; "))));
            note.set_wrap(true);
            note.set_xalign(0.0);
            note.add_css_class("dim-label");
            note.add_css_class("availability-note");
            note.set_margin_start(8);
            note.set_margin_bottom(4);
            group.append(&note);
        }
    }

    let total = controls.len();
    status.set_text(&format!("Loaded {total} V4L2 controls from {device}. Changes apply immediately."));
    let config = current_config();
    ui::layout::apply_compact_visibility(container.upcast_ref(), config.compact_mode, &config.compact_controls);
    ui::layout::restore_scroll_position(scroll_position);
}
