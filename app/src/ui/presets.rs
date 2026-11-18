use gtk::prelude::*;
use crate::{config, current_config, presets, update_config_checked};

fn refresh(combo: &gtk::ComboBoxText) {
    combo.remove_all();
    for (index, preset) in current_config().presets.iter().enumerate() {
        combo.append(Some(&index.to_string()), &preset.name);
    }
    if !current_config().presets.is_empty() {
        combo.set_active(Some(0));
    }
}

pub(crate) fn append(container: &gtk::Box, device: &str, status: &gtk::Label) {
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
    let selected_presets = combo.clone();
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
        status_apply.set_text(&presets::apply(&device_apply, preset)
            .unwrap_or_else(|error| format!("Preset applied with errors: {error}")));
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
