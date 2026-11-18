use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use crate::{update_config_checked, video};

fn fill_resolutions(
    combo: &gtk::ComboBoxText,
    modes: &[video::VideoCapability],
    format: &str,
    preferred: Option<(u32, u32)>,
) {
    combo.remove_all();
    let mut seen = Vec::new();
    for mode in modes.iter().filter(|mode| mode.pixel_format == format) {
        let size = (mode.width, mode.height);
        if !seen.contains(&size) {
            seen.push(size);
            combo.append(Some(&format!("{}x{}", mode.width, mode.height)),
                &format!("{} × {}", mode.width, mode.height));
        }
    }
    if let Some((width, height)) = preferred {
        combo.set_active_id(Some(&format!("{width}x{height}")));
    }
    if combo.active_id().is_none() { combo.set_active(Some(0)); }
}

fn fill_rates(
    combo: &gtk::ComboBoxText,
    modes: &[video::VideoCapability],
    format: &str,
    size: &str,
    preferred: Option<u32>,
) {
    combo.remove_all();
    let Some((width, height)) = size.split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))) else { return; };
    let Some(mode) = modes.iter().find(|mode|
        mode.pixel_format == format && mode.width == width && mode.height == height) else { return; };
    for rate in &mode.frame_rates_milli {
        let label = if rate % 1000 == 0 {
            format!("{} fps", rate / 1000)
        } else {
            format!("{:.3} fps", *rate as f64 / 1000.0)
        };
        combo.append(Some(&rate.to_string()), &label);
    }
    if let Some(rate) = preferred {
        combo.set_active_id(Some(&rate.to_string()));
    }
    if combo.active_id().is_none() { combo.set_active(Some(0)); }
}

fn selected_mode(
    modes: &[video::VideoCapability],
    format: &str,
    size: &str,
    rate: &str,
) -> Option<video::VideoMode> {
    let (width, height) = size.split_once('x')?;
    let width = width.parse().ok()?;
    let height = height.parse().ok()?;
    let frame_rate_milli = rate.parse().ok()?;
    modes.iter().find(|mode|
        mode.pixel_format == format
            && mode.width == width
            && mode.height == height
            && mode.frame_rates_milli.contains(&frame_rate_milli)
    )?;
    Some(video::VideoMode {
        width, height, pixel_format: format.to_string(), frame_rate_milli,
    })
}

pub(crate) fn append(container: &gtk::Box, device: &str, status: &gtk::Label) {
    let frame = gtk::Frame::new(Some("Video capture"));
    crate::ui::layout::mark_compact_item(frame.upcast_ref(), "camera:video_mode");
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 8);
    layout.set_margin_top(8);
    layout.set_margin_bottom(8);
    layout.set_margin_start(10);
    layout.set_margin_end(10);

    let note = gtk::Label::new(Some("Resolution, pixel format and frame rate are read from this video node's advertised V4L2 capabilities. Changes apply immediately."));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    layout.append(&note);

    let modes = match video::capabilities(device) {
        Ok(modes) => modes,
        Err(error) => {
            let unavailable = gtk::Label::new(Some(&format!("Video mode controls unavailable: {error}")));
            unavailable.set_wrap(true);
            unavailable.set_xalign(0.0);
            unavailable.add_css_class("dim-label");
            layout.append(&unavailable);
            frame.set_child(Some(&layout));
            container.append(&frame);
            return;
        }
    };

    let current = video::current_mode(device);
    let format_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    format_row.add_css_class("adaptive-row");
    let format_label = gtk::Label::new(Some("Pixel format"));
    format_label.set_width_chars(16);
    format_label.set_xalign(0.0);
    let format = gtk::ComboBoxText::new();
    format.set_hexpand(true);
    let mut formats: Vec<String> = modes.iter().map(|mode| mode.pixel_format.clone()).collect();
    formats.sort();
    formats.dedup();
    for item in &formats { format.append(Some(item), item); }
    let preferred_format = current.as_ref().map(|mode| mode.pixel_format.as_str())
        .filter(|value| formats.iter().any(|item| item == value))
        .or_else(|| formats.first().map(String::as_str));
    if let Some(value) = preferred_format { format.set_active_id(Some(value)); }
    format_row.append(&format_label);
    format_row.append(&format);
    layout.append(&format_row);

    let size_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    size_row.add_css_class("adaptive-row");
    let size_label = gtk::Label::new(Some("Resolution"));
    size_label.set_width_chars(16);
    size_label.set_xalign(0.0);
    let size = gtk::ComboBoxText::new();
    size.set_hexpand(true);
    fill_resolutions(&size, &modes, preferred_format.unwrap_or(""), current.as_ref().map(|m| (m.width, m.height)));
    size_row.append(&size_label);
    size_row.append(&size);
    layout.append(&size_row);

    let rate_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    rate_row.add_css_class("adaptive-row");
    let rate_label = gtk::Label::new(Some("Frame rate"));
    rate_label.set_width_chars(16);
    rate_label.set_xalign(0.0);
    let rate = gtk::ComboBoxText::new();
    rate.set_hexpand(true);
    fill_rates(
        &rate, &modes, preferred_format.unwrap_or(""),
        size.active_id().as_deref().unwrap_or(""),
        current.as_ref().map(|m| m.frame_rate_milli),
    );
    rate_row.append(&rate_label);
    rate_row.append(&rate);
    layout.append(&rate_row);

    let updating = Rc::new(Cell::new(false));
    let last_mode = Rc::new(RefCell::new(current.clone()));
    let device_name = device.to_string();
    let format_apply = format.clone();
    let size_apply = size.clone();
    let rate_apply = rate.clone();
    let status_apply = status.clone();
    let modes_apply = modes.clone();
    let updating_apply = updating.clone();
    let last_apply = last_mode.clone();
    let format_restore = format.clone();
    let size_restore = size.clone();
    let rate_restore = rate.clone();
    let apply = Rc::new(move || {
        if updating_apply.get() { return; }
        let Some(selected) = selected_mode(
            &modes_apply,
            format_apply.active_id().as_deref().unwrap_or(""),
            size_apply.active_id().as_deref().unwrap_or(""),
            rate_apply.active_id().as_deref().unwrap_or(""),
        ) else { return; };
        if last_apply.borrow().as_ref() == Some(&selected) { return; }
        match video::set_mode(&device_name, &selected) {
            Ok(()) => {
                *last_apply.borrow_mut() = Some(selected.clone());
                crate::restart_preview(&device_name, Some(selected.clone()));
                let persist = crate::config::VideoModeConfig {
                    width: selected.width,
                    height: selected.height,
                    pixel_format: selected.pixel_format.clone(),
                    frame_rate_milli: selected.frame_rate_milli,
                };
                if let Err(error) = update_config_checked(|config| config.video_mode = Some(persist)) {
                    status_apply.set_text(&format!("Video mode changed, but could not save the selection: {error}"));
                } else {
                    status_apply.set_text(&format!(
                        "Capture mode set to {} × {} · {} · {:.3} fps.",
                        selected.width, selected.height, selected.pixel_format,
                        selected.frame_rate_milli as f64 / 1000.0
                    ));
                }
            }
            Err(error) => {
                status_apply.set_text(&format!("Could not apply video mode: {error}"));
                if let Some(previous) = last_apply.borrow().clone() {
                    // A failed frame-rate change can occur after the format
                    // has already changed. Best-effort restore the full mode.
                    let _ = video::set_mode(&device_name, &previous);
                    updating_apply.set(true);
                    format_restore.set_active_id(Some(&previous.pixel_format));
                    fill_resolutions(&size_restore, &modes_apply, &previous.pixel_format, Some((previous.width, previous.height)));
                    fill_rates(&rate_restore, &modes_apply, &previous.pixel_format,
                        &format!("{}x{}", previous.width, previous.height), Some(previous.frame_rate_milli));
                    updating_apply.set(false);
                }
            }
        }
    });

    let apply_format = apply.clone();
    let updating_format = updating.clone();
    let modes_format = modes.clone();
    let size_format = size.clone();
    let rate_format = rate.clone();
    let current_format = current.clone();
    format.connect_changed(move |combo| {
        if updating_format.get() { return; }
        updating_format.set(true);
        let selected_format = combo.active_id().map(|id| id.to_string()).unwrap_or_default();
        fill_resolutions(&size_format, &modes_format, &selected_format,
            current_format.as_ref().filter(|mode| mode.pixel_format == selected_format).map(|m| (m.width, m.height)));
        fill_rates(&rate_format, &modes_format, &selected_format,
            size_format.active_id().as_deref().unwrap_or(""),
            current_format.as_ref().filter(|mode| mode.pixel_format == selected_format).map(|m| m.frame_rate_milli));
        updating_format.set(false);
        apply_format();
    });

    let apply_size = apply.clone();
    let updating_size = updating.clone();
    let modes_size = modes.clone();
    let format_size = format.clone();
    let rate_size = rate.clone();
    let current_size = current.clone();
    size.connect_changed(move |combo| {
        if updating_size.get() { return; }
        updating_size.set(true);
        let selected_size = combo.active_id().map(|id| id.to_string()).unwrap_or_default();
        fill_rates(&rate_size, &modes_size,
            format_size.active_id().as_deref().unwrap_or(""),
            &selected_size,
            current_size.as_ref().filter(|mode|
                format_size.active_id().as_deref() == Some(mode.pixel_format.as_str())
                && selected_size == format!("{}x{}", mode.width, mode.height)
            ).map(|m| m.frame_rate_milli));
        updating_size.set(false);
        apply_size();
    });

    rate.connect_changed(move |_| apply());

    frame.set_child(Some(&layout));
    container.append(&frame);
}
