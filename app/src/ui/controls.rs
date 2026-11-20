use gtk::prelude::*;
use std::collections::HashMap;
use std::time::Duration;
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
    if control.value < control.min || control.value > control.max {
        blockers.push(format!(
            "The driver reports current value {} outside its declared range {}–{}",
            control.value, control.min, control.max
        ));
    }
    if control.disabled && blockers.is_empty() {
        blockers.push(if control.kind.contains("read-only") {
            "The V4L2 driver marks this control read-only".to_string()
        } else {
            "The V4L2 driver reports this control as inactive or disabled".to_string()
        });
    }
    blockers
}

fn control_display_name(name: &str) -> String {
    match name {
        "exposure_time_absolute" => "Shutter speed".to_string(),
        "iso_sensitivity" => "ISO sensitivity".to_string(),
        "iso_sensitivity_auto" => "Auto ISO".to_string(),
        "gain" => "Sensor gain".to_string(),
        _ => name.replace('_', " "),
    }
}

fn control_display_value(name: &str, value: i64) -> String {
    match name {
        // V4L2_CID_EXPOSURE_ABSOLUTE is expressed in 100-microsecond units.
        "exposure_time_absolute" if value > 0 => {
            let denominator = (10_000.0 / value as f64).round() as i64;
            if denominator >= 2 {
                format!("1/{denominator} s")
            } else {
                format!("{:.2} s", value as f64 / 10_000.0)
            }
        }
        "iso_sensitivity" => format!("ISO {value}"),
        "gain" => format!("Gain {value}"),
        _ => value.to_string(),
    }
}

fn v4l2_control_group(name: &str) -> &'static str {
    match name {
        "white_balance_automatic" | "red_balance" | "blue_balance" | "white_balance_temperature" => "White balance",
        "auto_exposure" | "exposure_time_absolute" | "power_line_frequency" | "gain" => "Exposure",
        "focus_absolute" => "Focus",
        "zoom_absolute" | "zoom_continuous" => "Zoom",
        _ => "Image",
    }
}

fn add_gimbal_position_feedback(container: &gtk::Box, device: &str) {
    let frame = gtk::Frame::new(Some("Gimbal position"));
    ui::layout::mark_compact_item(frame.upcast_ref(), "camera:gimbal_position");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("adaptive-row");
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(10);
    row.set_margin_end(10);

    let pan_column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let pan_title = gtk::Label::new(Some("Pan"));
    pan_title.add_css_class("dim-label");
    let pan_value = gtk::Label::new(Some("Reading…"));
    pan_value.add_css_class("title-2");
    pan_value.set_xalign(0.0);
    pan_column.append(&pan_title);
    pan_column.append(&pan_value);

    let tilt_column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let tilt_title = gtk::Label::new(Some("Tilt"));
    tilt_title.add_css_class("dim-label");
    let tilt_value = gtk::Label::new(Some("Reading…"));
    tilt_value.add_css_class("title-2");
    tilt_value.set_xalign(0.0);
    tilt_column.append(&tilt_title);
    tilt_column.append(&tilt_value);

    pan_column.set_hexpand(true);
    tilt_column.set_hexpand(true);
    row.append(&pan_column);
    row.append(&tilt_column);
    frame.set_child(Some(&row));
    container.append(&frame);

    let device = device.to_string();
    // Initialize immediately, then keep the readout live. This selector-0x02
    // GET_CUR candidate is based on the documented position-like float pair.
    match get_gimbal_position(&device) {
        Ok((pan, tilt)) => {
            pan_value.set_text(&format!("{pan:+.1}°"));
            tilt_value.set_text(&format!("{tilt:+.1}°"));
        }
        Err(error) => {
            pan_value.set_text("Unavailable");
            tilt_value.set_text("Unavailable");
            pan_value.set_tooltip_text(Some(&error));
            tilt_value.set_tooltip_text(Some(&error));
        }
    }
    let pan_weak = pan_value.downgrade();
    let tilt_weak = tilt_value.downgrade();
    gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
        let (Some(pan_label), Some(tilt_label)) = (pan_weak.upgrade(), tilt_weak.upgrade()) else {
            return gtk::glib::ControlFlow::Break;
        };
        match get_gimbal_position(&device) {
            Ok((pan, tilt)) => {
                pan_label.set_text(&format!("{pan:+.1}°"));
                tilt_label.set_text(&format!("{tilt:+.1}°"));
                pan_label.set_tooltip_text(None);
                tilt_label.set_tooltip_text(None);
            }
            Err(error) => {
                pan_label.set_text("Unavailable");
                tilt_label.set_text("Unavailable");
                pan_label.set_tooltip_text(Some(&error));
                tilt_label.set_tooltip_text(Some(&error));
            }
        }
        gtk::glib::ControlFlow::Continue
    });
}

pub(crate) fn populate_controls(device: &str, container: &gtk::Box, window: &gtk::ApplicationWindow, status: &gtk::Label) {
    let scroll_position = ui::layout::capture_scroll_position(container.upcast_ref());
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
    if has_pan && has_tilt {
        add_gimbal_position_feedback(container, device);
    }
    if has_pan || has_tilt {
        let frame = gtk::Frame::new(Some("Gimbal movement"));
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
        add_focus_mode_control(container, device, status, autofocus_enabled);
    }

    // Auto Exposure priority is independent of autofocus. Keep both controls
    // visible, but disable them unless the V4L2 exposure mode is Auto (value 0).
    let auto_exposure_enabled = controls.iter()
        .find(|c| c.name == "auto_exposure")
        .is_some_and(|c| c.value == 0);
    if controls.iter().any(|c| c.name == "auto_exposure") {
        add_auto_exposure_mode_control(container, device, status, auto_exposure_enabled);
        add_exposure_compensation_control(container, device, status, auto_exposure_enabled);
    }

    let mut ordered_controls = controls.clone();
    ordered_controls.sort_by_key(|control| match v4l2_control_group(&control.name) {
        "Exposure" => 0,
        "White balance" => 1,
        "Focus" => 2,
        "Image" => 3,
        _ => 4,
    });
    let mut control_groups: HashMap<&'static str, gtk::Box> = HashMap::new();
    for control in ordered_controls {
        if control.name == "focus_automatic_continuous" { continue; }
        if control.kind == "button" && control.name != "zoom_absolute" || control.name == "pan_absolute" || control.name == "tilt_absolute" || (joystick_enabled && (control.name == "pan_speed" || control.name == "tilt_speed")) { continue; }
        let blockers = control_blockers(&control, &controls);
        let unavailable = control.disabled || !blockers.is_empty();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("adaptive-row");
        row.set_margin_top(3);
        row.set_margin_bottom(3);
        let title = gtk::Label::new(Some(&control_display_name(&control.name)));
        title.set_width_chars(22);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        row.append(&title);

        if control.kind == "bool" || (!control.kind.contains("menu") && control.min == 0 && control.max == 1) {
            let toggle = gtk::CheckButton::new();
            toggle.set_active(control.value != 0);
            toggle.set_sensitive(!unavailable);
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
                if !succeeded {
                    button.set_active(value == 0);
                }
                if succeeded && matches!(name.as_str(), "white_balance_automatic" | "focus_automatic_continuous") {
                    populate_controls(&refresh_device, &controls_refresh, &window_refresh, &status_label);
                }
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
                let result = set_control(&dev, &name, value);
                match result {
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
                control.step as f64,
            );
            scale.set_value(control.value.clamp(control.min, control.max.max(control.min)) as f64);
            scale.set_draw_value(false);
            scale.set_digits(0);
            scale.set_hexpand(true);
            scale.set_sensitive(!unavailable);
            scale.set_tooltip_text(Some(&format!(
                "Allowed range: {} to {} · step {} · default {}",
                control.min, control.max, control.step, control.default_value
            )));
            title.set_tooltip_text(Some(&format!(
                "Range: {} to {} · step {} · default {}",
                control.min, control.max, control.step, control.default_value
            )));
            scale.add_mark(control.default_value as f64, gtk::PositionType::Bottom,
                Some(&format!("Default: {}", control_display_value(&control.name, control.default_value))));

            let value_label = gtk::Label::new(Some(&control_display_value(&control.name, control.value)));
            value_label.set_width_chars(9);
            value_label.set_xalign(1.0);
            value_label.add_css_class("dim-label");

            let reset = gtk::Button::with_label("Reset");
            reset.set_sensitive(!unavailable && control.value != control.default_value);
            let reset_scale = scale.clone();
            reset.connect_clicked(move |_| {
                reset_scale.set_value(control.default_value as f64);
            });

            let dev = device.to_string();
            let name = control.name.clone();
            let status_label = status.clone();
            let reset_button = reset.clone();
            let value_display = value_label.clone();
            let display_name = name.clone();
            let default_value = control.default_value;
            let last_value = Rc::new(Cell::new(control.value));
            let reverting = Rc::new(Cell::new(false));
            let last_value_cb = last_value.clone();
            let reverting_cb = reverting.clone();
            scale.connect_value_changed(move |slider| {
                if reverting_cb.get() { return; }
                let requested = slider.value().round() as i64;
                value_display.set_text(&control_display_value(&display_name, requested));
                let result = set_control(&dev, &name, requested);
                match result {
                    Ok(()) => {
                        last_value_cb.set(requested);
                        status_label.set_text(&format!("{} set to {}", control_display_name(&name), control_display_value(&name, requested)));
                        reset_button.set_sensitive(requested != default_value);
                    }
                    Err(error) => {
                        reverting_cb.set(true);
                        slider.set_value(last_value_cb.get() as f64);
                        value_display.set_text(&control_display_value(&display_name, last_value_cb.get()));
                        reverting_cb.set(false);
                        reset_button.set_sensitive(last_value_cb.get() != default_value);
                        status_label.set_text(&format!("Could not set {name}: {error}"));
                    }
                }
            });
            row.append(&scale);
            row.append(&value_label);
            row.append(&reset);
        }
        let group_name = v4l2_control_group(&control.name);
        let group = control_groups.entry(group_name).or_insert_with(|| {
            let frame = gtk::Frame::new(Some(group_name));
            let content = gtk::Box::new(gtk::Orientation::Vertical, 5);
            content.set_margin_top(6);
            content.set_margin_bottom(6);
            content.set_margin_start(8);
            content.set_margin_end(8);
            frame.add_css_class("compact-group");
            frame.set_child(Some(&content));
            container.append(&frame);
            content
        });
        ui::layout::mark_compact_item(row.upcast_ref(), &format!("v4l2:{}", control.name));
        group.append(&row);
        if unavailable {
            let reason = if blockers.is_empty() {
                "The camera driver reports this setting as unavailable.".to_string()
            } else {
                blockers.join("; ")
            };
            let note = gtk::Label::new(Some(&format!("Unavailable: {reason}.")));
            note.set_wrap(true);
            note.set_xalign(0.0);
            note.add_css_class("dim-label");
            note.set_margin_start(8);
            note.set_margin_bottom(4);
            group.append(&note);
        }
        group.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }
    // Keep related camera settings together. The standard controls above are
    // ordered Exposure → White balance → Focus → Image → Zoom. Zoom presets
    // follow immediately after the V4L2 zoom controls in Camera settings.
    add_camera_settings(container, device, window, status);
    add_ai_controls(container, device, status);
    add_audio_controls(container, device, status);
    ui::presets::append(container, device, window, status);
    ui::video::append(container, device, status);
    add_sleep_controls(container, device, status);

    status.set_text(&format!("Loaded {total} V4L2 controls from {device}. Sliders apply changes immediately."));
    // The test app intentionally exposes the complete control set. Ignore
    // any compact-mode preference persisted by older builds, otherwise it
    // silently hides controls with no remaining UI to restore them.
    ui::layout::apply_compact_visibility(container.upcast_ref(), false, &[]);
    ui::layout::disable_scroll_wheel_controls(container.upcast_ref());
    ui::layout::restore_scroll_position(scroll_position);
}

#[cfg(test)]
mod tests {
    use super::{control_blockers, control_display_value, v4l2_control_group};
    use crate::Control;

    fn control(name: &str, value: i64, disabled: bool) -> Control {
        Control {
            name: name.into(), kind: "int".into(), min: 0, max: 10_000, step: 1,
            value, default_value: 50, disabled, menu: Vec::new(),
        }
    }

    #[test]
    fn white_balance_manual_controls_explain_auto_white_balance_blocker() {
        let controls = vec![control("white_balance_automatic", 1, false), control("red_balance", 1043, true)];
        assert_eq!(control_blockers(&controls[1], &controls), vec!["Auto white balance is enabled".to_string()]);
    }

    #[test]
    fn shutter_speed_requires_manual_exposure() {
        let controls = vec![control("auto_exposure", 3, false), control("exposure_time_absolute", 330, true)];
        assert_eq!(control_blockers(&controls[1], &controls), vec!["Exposure mode is not Manual".to_string()]);
    }

    #[test]
    fn manual_focus_requires_autofocus_to_be_disabled() {
        let controls = vec![control("focus_automatic_continuous", 1, false), control("focus_absolute", 100, true)];
        assert_eq!(control_blockers(&controls[1], &controls), vec!["Autofocus is enabled".to_string()]);
    }

    #[test]
    fn out_of_range_driver_value_is_explained_and_blocked() {
        let mut invalid = control("zoom_continuous", 245, true);
        invalid.min = -100;
        invalid.max = 100;
        assert_eq!(
            control_blockers(&invalid, &[invalid.clone()]),
            vec!["The driver reports current value 245 outside its declared range -100–100".to_string()]
        );
    }

    #[test]
    fn standard_control_groups_follow_camera_workflow() {
        assert_eq!(v4l2_control_group("auto_exposure"), "Exposure");
        assert_eq!(v4l2_control_group("white_balance_automatic"), "White balance");
        assert_eq!(v4l2_control_group("focus_absolute"), "Focus");
        assert_eq!(v4l2_control_group("zoom_absolute"), "Zoom");
        assert_eq!(v4l2_control_group("brightness"), "Image");
    }

    #[test]
    fn exposure_values_display_as_shutter_speeds() {
        assert_eq!(control_display_value("exposure_time_absolute", 330), "1/30 s");
        assert_eq!(control_display_value("exposure_time_absolute", 167), "1/60 s");
        assert_eq!(control_display_value("exposure_time_absolute", 2500), "1/4 s");
    }

    #[test]
    fn iso_and_gain_keep_their_actual_v4l2_meaning() {
        assert_eq!(control_display_value("iso_sensitivity", 400), "ISO 400");
        assert_eq!(control_display_value("gain", 18), "Gain 18");
    }
}
