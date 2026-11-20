use gtk::prelude::*;
use crate::{current_config, presets, update_config_checked};

fn refresh(combo: &gtk::ComboBoxText) {
    combo.remove_all();
    for (index, preset) in current_config().presets.iter().enumerate() {
        combo.append(Some(&index.to_string()), &preset.name);
    }
    if !current_config().presets.is_empty() {
        combo.set_active(Some(0));
    }
}

pub(crate) fn append(
    container: &gtk::Box,
    device: &str,
    window: &gtk::ApplicationWindow,
    status: &gtk::Label,
) {
    let frame = gtk::Frame::new(Some("Camera presets"));
    crate::ui::layout::mark_compact_item(frame.upcast_ref(), "camera:presets");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let description = gtk::Label::new(Some(
        "Save and recall up to 10 camera configurations. Presets include readable V4L2 controls, absolute gimbal positions, and OBSBOT settings whose state is known."
    ));
    description.set_wrap(true);
    description.set_xalign(0.0);
    description.add_css_class("dim-label");
    layout.append(&description);

    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    name_row.add_css_class("adaptive-row");
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Preset name"));
    name.set_text("My preset");
    name.set_hexpand(true);
    let save = gtk::Button::with_label("Save current");
    save.add_css_class("suggested-action");
    name_row.append(&name);
    name_row.append(&save);
    layout.append(&name_row);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("adaptive-row");
    let combo = gtk::ComboBoxText::new();
    combo.set_hexpand(true);
    refresh(&combo);
    let apply = gtk::Button::with_label("Apply");
    let delete = gtk::Button::with_label("Delete");
    actions.append(&combo);
    actions.append(&apply);
    actions.append(&delete);
    layout.append(&actions);

    let name_from_selection = name.clone();
    combo.connect_changed(move |selection| {
        let Some(index) = selection.active_id().and_then(|id| id.parse::<usize>().ok()) else { return; };
        if let Some(preset) = current_config().presets.get(index) {
            name_from_selection.set_text(&preset.name);
        }
    });

    let device_save = device.to_string();
    let name_save = name.clone();
    let combo_save = combo.clone();
    let status_save = status.clone();
    save.connect_clicked(move |_| {
        let preset_name = name_save.text().trim().to_string();
        if preset_name.is_empty() {
            status_save.set_text("Enter a name for this preset.");
            return;
        }
        let mut config = current_config();
        let preset = match presets::capture(&device_save, preset_name.clone()) {
            Ok(value) => value,
            Err(error) => {
                status_save.set_text(&format!("Could not read camera settings for preset: {error}"));
                return;
            }
        };
        if let Some(existing) = config.presets.iter_mut()
            .find(|item| item.name.eq_ignore_ascii_case(&preset_name)) {
            *existing = preset;
        } else if config.presets.len() < 10 {
            config.presets.push(preset);
        } else {
            status_save.set_text("Preset limit reached. Delete one of the 10 presets before saving another.");
            return;
        }
        match update_config_checked(|current| *current = config) {
            Ok(()) => {
                refresh(&combo_save);
                status_save.set_text(&format!("Preset '{preset_name}' saved."));
            }
            Err(error) => status_save.set_text(&format!("Could not save preset: {error}")),
        }
    });

    let device_apply = device.to_string();
    let combo_apply = combo.clone();
    let container_apply = container.clone();
    let window_apply = window.clone();
    let status_apply = status.clone();
    apply.connect_clicked(move |_| {
        let Some(index) = combo_apply.active_id().and_then(|id| id.parse::<usize>().ok()) else {
            status_apply.set_text("Select a camera preset first.");
            return;
        };
        let config = current_config();
        let Some(preset) = config.presets.get(index) else {
            status_apply.set_text("The selected preset no longer exists. Refresh the camera controls.");
            return;
        };
        let message = presets::apply(&device_apply, preset)
            .unwrap_or_else(|error| format!("Preset submission reported errors: {error}"));
        crate::ui::controls::populate_controls(
            &device_apply, &container_apply, &window_apply, &status_apply,
        );
        status_apply.set_text(&message);
    });

    let combo_delete = combo.clone();
    let status_delete = status.clone();
    delete.connect_clicked(move |_| {
        let Some(index) = combo_delete.active_id().and_then(|id| id.parse::<usize>().ok()) else {
            status_delete.set_text("Select a camera preset to delete.");
            return;
        };
        let mut config = current_config();
        if index >= config.presets.len() {
            status_delete.set_text("The selected preset no longer exists.");
            return;
        }
        let removed = config.presets.remove(index).name;
        match update_config_checked(|current| *current = config) {
            Ok(()) => {
                refresh(&combo_delete);
                status_delete.set_text(&format!("Preset '{removed}' deleted."));
            }
            Err(error) => status_delete.set_text(&format!("Could not delete preset: {error}")),
        }
    });

    frame.set_child(Some(&layout));
    container.append(&frame);
}


fn refresh_camera_native(combo: &gtk::ComboBoxText, device: &str, status: &gtk::Label) -> Vec<crate::CameraPresetEntry> {
    combo.remove_all();
    match crate::read_camera_presets(device) {
        Ok(entries) => {
            for entry in &entries {
                combo.append(Some(&entry.slot.to_string()), &format!(
                    "Slot {} · {} · Pan {:+.1}° / Tilt {:+.1}° / Zoom {:.2}×",
                    entry.slot + 1, entry.name, entry.pan_degrees, entry.tilt_degrees, entry.zoom
                ));
            }
            if !entries.is_empty() { combo.set_active(Some(0)); }
            entries
        }
        Err(error) => {
            status.set_text(&format!("Could not read camera-native presets: {error}"));
            Vec::new()
        }
    }
}

/// Camera-resident preset management. Unlike the local configuration
/// presets above, these slots are stored by the camera and survive app removal.
pub(crate) fn append_camera_native(
    container: &gtk::Box,
    device: &str,
    status: &gtk::Label,
) {
    let frame = gtk::Frame::new(Some("Camera-native presets"));
    crate::ui::layout::mark_compact_item(frame.upcast_ref(), "camera:native-presets");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let description = gtk::Label::new(Some(
        "Stored in the Tiny 2 Lite itself. Saving captures the camera's live gimbal pose and zoom. Recall physically moves the gimbal."
    ));
    description.set_wrap(true);
    description.set_xalign(0.0);
    description.add_css_class("dim-label");
    layout.append(&description);

    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    name_row.add_css_class("adaptive-row");
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Camera preset name"));
    name.set_text("Initial state");
    name.set_hexpand(true);
    let save = gtk::Button::with_label("Save current");
    save.add_css_class("suggested-action");
    name_row.append(&name);
    name_row.append(&save);
    layout.append(&name_row);

    let select_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    select_row.add_css_class("adaptive-row");
    let combo = gtk::ComboBoxText::new();
    combo.set_hexpand(true);
    let refresh_button = gtk::Button::with_label("Refresh");
    select_row.append(&combo);
    select_row.append(&refresh_button);
    layout.append(&select_row);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("adaptive-row");
    let recall = gtk::Button::with_label("Recall");
    let rename = gtk::Button::with_label("Rename");
    let delete = gtk::Button::with_label("Delete");
    actions.append(&recall);
    actions.append(&rename);
    actions.append(&delete);
    layout.append(&actions);

    let initial_entries = refresh_camera_native(&combo, device, status);
    if let Some(first) = initial_entries.first() { name.set_text(&first.name); }

    let device_refresh = device.to_string();
    let combo_refresh = combo.clone();
    let status_refresh = status.clone();
    refresh_button.connect_clicked(move |_| {
        let entries = refresh_camera_native(&combo_refresh, &device_refresh, &status_refresh);
        status_refresh.set_text(&format!("Read {} camera-native preset(s).", entries.len()));
    });

    let device_save = device.to_string();
    let combo_save = combo.clone();
    let name_save = name.clone();
    let status_save = status.clone();
    save.connect_clicked(move |_| {
        let preset_name = name_save.text().trim().to_string();
        if preset_name.is_empty() || !preset_name.is_ascii() || preset_name.len() > 40 {
            status_save.set_text("Use a preset name containing 1–40 ASCII bytes.");
            return;
        }
        let entries = match crate::read_camera_presets(&device_save) {
            Ok(entries) => entries,
            Err(error) => {
                status_save.set_text(&format!("Could not inspect camera preset slots: {error}"));
                return;
            }
        };
        let selected = combo_save.active_id().and_then(|id| id.parse::<u8>().ok());
        let (slot, update) = if let Some(slot) = selected.filter(|slot| entries.iter().any(|entry| entry.slot == *slot)) {
            (slot, true)
        } else if let Some(slot) = (0u8..3).find(|slot| !entries.iter().any(|entry| entry.slot == *slot)) {
            (slot, false)
        } else {
            status_save.set_text("All three camera preset slots are occupied. Select one to update it.");
            return;
        };
        match crate::save_camera_preset(&device_save, slot, &preset_name, update) {
            Ok(()) => {
                let refreshed = refresh_camera_native(&combo_save, &device_save, &status_save);
                combo_save.set_active_id(Some(&slot.to_string()));
                status_save.set_text(&format!("Camera preset '{}' {} in slot {}.", preset_name, if update { "updated" } else { "created" }, slot + 1));
                let _ = refreshed;
            }
            Err(error) => status_save.set_text(&format!("Could not save camera preset: {error}")),
        }
    });

    let device_recall = device.to_string();
    let combo_recall = combo.clone();
    let status_recall = status.clone();
    recall.connect_clicked(move |_| {
        let Some(slot) = combo_recall.active_id().and_then(|id| id.parse::<u8>().ok()) else {
            status_recall.set_text("Select a camera-native preset first.");
            return;
        };
        match crate::recall_camera_preset(&device_recall, slot) {
            Ok(()) => status_recall.set_text(&format!("Recall command submitted for camera preset slot {}. The gimbal should move.", slot + 1)),
            Err(error) => status_recall.set_text(&format!("Could not recall camera preset: {error}")),
        }
    });

    let device_rename = device.to_string();
    let combo_rename = combo.clone();
    let name_rename = name.clone();
    let status_rename = status.clone();
    rename.connect_clicked(move |_| {
        let Some(slot) = combo_rename.active_id().and_then(|id| id.parse::<u8>().ok()) else {
            status_rename.set_text("Select a camera-native preset to rename.");
            return;
        };
        let preset_name = name_rename.text().trim().to_string();
        match crate::rename_camera_preset(&device_rename, slot, &preset_name) {
            Ok(()) => {
                refresh_camera_native(&combo_rename, &device_rename, &status_rename);
                combo_rename.set_active_id(Some(&slot.to_string()));
                status_rename.set_text(&format!("Camera preset slot {} renamed to '{}'.", slot + 1, preset_name));
            }
            Err(error) => status_rename.set_text(&format!("Could not rename camera preset: {error}")),
        }
    });

    let device_delete = device.to_string();
    let combo_delete = combo.clone();
    let status_delete = status.clone();
    delete.connect_clicked(move |_| {
        let Some(slot) = combo_delete.active_id().and_then(|id| id.parse::<u8>().ok()) else {
            status_delete.set_text("Select a camera-native preset to delete.");
            return;
        };
        match crate::delete_camera_preset(&device_delete, slot) {
            Ok(()) => {
                let entries = refresh_camera_native(&combo_delete, &device_delete, &status_delete);
                if entries.iter().any(|entry| entry.slot == slot) {
                    status_delete.set_text("The camera acknowledged deletion, but the slot is still present. It was not removed.");
                } else {
                    status_delete.set_text(&format!("Camera preset slot {} deleted and verified absent.", slot + 1));
                }
            }
            Err(error) => status_delete.set_text(&format!("Could not delete camera preset: {error}")),
        }
    });

    frame.set_child(Some(&layout));
    container.append(&frame);
}


fn refresh_initial_state(label: &gtk::Label, device: &str, status: &gtk::Label) {
    match crate::read_initial_camera_state(device) {
        Ok(state) => {
            let source = match state.source {
                0..=2 => format!("Preset {}", state.source + 1),
                3 => "Custom pose".to_string(),
                other => format!("Unknown source {other}"),
            };
            label.set_text(&format!(
                "{source} · Pan {:+.1}° · Tilt {:+.1}° · Zoom {:.2}×",
                state.pan, state.tilt, state.zoom
            ));
            status.set_text("Read the camera's stored power-on initial state.");
        }
        Err(error) => {
            label.set_text("Initial state unavailable");
            status.set_text(&format!("Could not read camera initial state: {error}"));
        }
    }
}

/// The camera's power-on/startup pose is separate from ordinary user presets.
pub(crate) fn append_initial_state(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Power-on initial state"));
    crate::ui::layout::mark_compact_item(frame.upcast_ref(), "camera:initial-state");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let description = gtk::Label::new(Some(
        "The camera's own startup pose, not a local preset. Set it from the current live gimbal position. Existing startup picture settings are preserved."
    ));
    description.set_wrap(true);
    description.set_xalign(0.0);
    description.add_css_class("dim-label");
    layout.append(&description);

    let current = gtk::Label::new(Some("Reading camera state…"));
    current.set_xalign(0.0);
    current.set_wrap(true);
    layout.append(&current);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("adaptive-row");
    let refresh = gtk::Button::with_label("Refresh");
    let set_initial = gtk::Button::with_label("Set current position as initial state");
    set_initial.add_css_class("suggested-action");
    actions.append(&refresh);
    actions.append(&set_initial);
    layout.append(&actions);

    refresh_initial_state(&current, device, status);

    let device_refresh = device.to_string();
    let current_refresh = current.clone();
    let status_refresh = status.clone();
    refresh.connect_clicked(move |_| refresh_initial_state(&current_refresh, &device_refresh, &status_refresh));

    let device_set = device.to_string();
    let current_set = current.clone();
    let status_set = status.clone();
    set_initial.connect_clicked(move |_| {
        set_initial.set_sensitive(false);
        match crate::set_initial_camera_state(&device_set) {
            Ok(state) => {
                let source = if state.source == 3 { "Custom pose".to_string() } else { format!("Preset {}", state.source + 1) };
                current_set.set_text(&format!(
                    "{source} · Pan {:+.1}° · Tilt {:+.1}° · Zoom {:.2}×",
                    state.pan, state.tilt, state.zoom
                ));
                status_set.set_text("Camera initial state saved and verified by read-back.");
            }
            Err(error) => {
                status_set.set_text(&format!("Could not set camera initial state: {error}"));
                refresh_initial_state(&current_set, &device_set, &status_set);
            }
        }
        set_initial.set_sensitive(true);
    });

    frame.set_child(Some(&layout));
    container.append(&frame);
}
