//! Cards that are not generated from the registry: presets, capture mode,
//! appearance and developer tools.

use super::preview::PreviewHub;
use super::store::Store;
use super::style::Style;
use super::theme::{self, ThemeMode, UiScale};
use super::widgets::{self, Segmented};
use crate::backend::registry::Opt;
use crate::backend::{self, current_config, update_config_checked, CameraPresetEntry};
use crate::{config, presets, video};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn card(title: &str, glyph: &str, subtitle: &str) -> (gtk::Box, gtk::Box) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    head.add_css_class("group-head");
    head.append(&widgets::icon(glyph));
    head.append(&widgets::text(title, &["group-title"]));
    card.append(&head);
    if !subtitle.is_empty() {
        let sub = widgets::text(subtitle, &["setting-summary"]);
        sub.set_margin_top(2);
        card.append(&sub);
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(12);
    card.append(&body);
    (card, body)
}

const fn opt(value: i64, label: &'static str) -> Opt { Opt { value, label, hint: "", confirm: None } }

// ── Presets page ──────────────────────────────────────────────────────────

pub fn presets_page(store: &Store) -> gtk::ScrolledWindow {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 16);
    column.set_margin_end(6);
    column.set_margin_bottom(20);
    column.append(&camera_memory_card(store));
    column.append(&app_presets_card(store));
    column.append(&startup_pose_card(store));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&column));
    scroll
}

struct Slot {
    name: gtk::Entry,
    detail: gtk::Label,
    go: gtk::Button,
    save: gtk::Button,
    delete: gtk::Button,
    entry: RefCell<Option<CameraPresetEntry>>,
}

fn camera_memory_card(store: &Store) -> gtk::Box {
    let (card, body) = card("Camera memory", "bookmarks",
        "Three positions stored in the camera itself. They stay when you unplug it or remove this app.");
    let slots: Rc<Vec<Slot>> = Rc::new((0..3u8).map(|index| {
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("Name this position"));
        name.set_hexpand(true);
        name.set_max_length(32);
        widgets::label_accessible(&name, &format!("Name of camera position {}", index + 1));
        let detail = widgets::text("Reading…", &["small", "dim"]);
        let go = widgets::icon_button("play_arrow", "Move the camera to this position", true);
        go.add_css_class("filled");
        let save = widgets::icon_button("save", "Save the camera's current position here", true);
        let delete = widgets::icon_button("delete", "Delete this position", true);
        delete.add_css_class("danger");
        Slot { name, detail, go, save, delete, entry: RefCell::new(None) }
    }).collect());

    for (index, slot) in slots.iter().enumerate() {
        let item = gtk::Box::new(gtk::Orientation::Vertical, 8);
        item.add_css_class("list-item");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let number = widgets::text(&format!("{}", index + 1), &["heading", "accent"]);
        number.set_width_chars(2);
        top.append(&number);
        top.append(&slot.name);
        top.append(&slot.go);
        top.append(&slot.save);
        top.append(&slot.delete);
        item.append(&top);
        item.append(&slot.detail);
        body.append(&item);
    }

    let reading = Rc::new(Cell::new(false));
    let reload: Rc<dyn Fn()> = {
        let slots = slots.clone();
        let store = store.clone();
        let reading = reading.clone();
        Rc::new(move || {
            if reading.replace(true) { return; }
            for slot in slots.iter() { slot.detail.set_text("Reading…"); }
            let slots = slots.clone();
            let reading = reading.clone();
            let store2 = store.clone();
            store.call(Some("camera-presets"), |device| backend::read_camera_presets(device), move |result| {
                reading.set(false);
                match result {
                    Ok(entries) => {
                        for (index, slot) in slots.iter().enumerate() {
                            let found = entries.iter().find(|e| usize::from(e.slot) == index).cloned();
                            match &found {
                                Some(e) => {
                                    slot.name.set_text(&e.name);
                                    slot.detail.set_text(&format!("Pan {} · Tilt {} · Zoom {:.2}×", backend::angles::format(e.pan_degrees), backend::angles::format(e.tilt_degrees), e.zoom));
                                }
                                None => { slot.name.set_text(""); slot.detail.set_text("Empty. Point the camera, name it, and save."); }
                            }
                            slot.go.set_sensitive(found.is_some());
                            slot.delete.set_sensitive(found.is_some());
                            *slot.entry.borrow_mut() = found;
                        }
                    }
                    Err(error) => {
                        for slot in slots.iter() { slot.detail.set_text("Could not read this position."); }
                        store2.error(&format!("Could not read the camera's saved positions: {error}"));
                    }
                }
            });
        })
    };

    for (index, slot) in slots.iter().enumerate() {
        let slot_number = index as u8;
        let store_go = store.clone();
        let slots_go = slots.clone();
        slot.go.connect_clicked(move |_| {
            let label = slots_go[index].name.text().to_string();
            let store = store_go.clone();
            store_go.confirm("open_with", "Move the camera?", &format!("The gimbal will turn to position {} {}. Clear the space around the camera first.", index + 1, if label.is_empty() { String::new() } else { format!("({label})") }), "Move camera", move || {
                let store2 = store.clone();
                store.call(None, move |d| backend::recall_camera_preset(d, slot_number), move |r| match r {
                    Ok(()) => store2.toast(&format!("Moving to position {}", slot_number + 1)),
                    Err(e) => store2.error(&format!("Could not move to position {}: {e}", slot_number + 1)),
                });
            });
        });

        let (store_save, slots_save, reload_save) = (store.clone(), slots.clone(), reload.clone());
        slot.save.connect_clicked(move |_| {
            let name = slots_save[index].name.text().trim().to_string();
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
                store_save.error("Give the position a name of up to 32 letters, numbers or symbols (no accents or emoji).");
                return;
            }
            let update = slots_save[index].entry.borrow().is_some();
            let (store, reload, verify) = (store_save.clone(), reload_save.clone(), name.clone());
            store_save.call(None, move |d| {
                backend::save_camera_preset(d, slot_number, &name, update)?;
                let entries = backend::read_camera_presets(d)?;
                Ok::<bool, String>(entries.iter().any(|e| e.slot == slot_number && e.name == verify))
            }, move |r| {
                match r {
                    Ok(true) => store.toast(&format!("Saved position {}", slot_number + 1)),
                    Ok(false) => store.error(&format!("The camera accepted the command, but position {} did not read back as saved.", slot_number + 1)),
                    Err(e) => store.error(&format!("Could not save position {}: {e}", slot_number + 1)),
                }
                reload();
            });
        });

        let (store_del, slots_del, reload_del) = (store.clone(), slots.clone(), reload.clone());
        slot.delete.connect_clicked(move |_| {
            let label = slots_del[index].name.text().to_string();
            let (store, reload) = (store_del.clone(), reload_del.clone());
            store_del.confirm("delete", &format!("Delete position {}?", index + 1), &format!("{} will be removed from the camera. This cannot be undone.", if label.is_empty() { "The position".to_string() } else { format!("“{label}”") }), "Delete", move || {
                let (store2, reload2) = (store.clone(), reload.clone());
                store.call(None, move |d| backend::delete_camera_preset(d, slot_number), move |r| {
                    match r {
                        Ok(()) => store2.toast(&format!("Deleted position {}", slot_number + 1)),
                        Err(e) => store2.error(&format!("Could not delete position {}: {e}", slot_number + 1)),
                    }
                    reload2();
                });
            });
        });

        // Renaming an existing position: press Enter in its name field.
        let (store_rename, slots_rename, reload_rename) = (store.clone(), slots.clone(), reload.clone());
        slot.name.connect_activate(move |entry| {
            let Some(current) = slots_rename[index].entry.borrow().clone() else { return };
            let name = entry.text().trim().to_string();
            if name == current.name { return; }
            let (store, reload) = (store_rename.clone(), reload_rename.clone());
            store_rename.call(None, move |d| backend::rename_camera_preset(d, slot_number, &name), move |r| {
                match r { Ok(()) => store.toast(&format!("Renamed position {}", slot_number + 1)), Err(e) => store.error(&format!("Could not rename: {e}")) }
                reload();
            });
        });
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let refresh = widgets::icon_text_button("refresh", "Read from camera");
    refresh.add_css_class("small");
    let reload_click = reload.clone();
    refresh.connect_clicked(move |_| reload_click());
    actions.append(&refresh);
    actions.append(&widgets::info_button("Camera memory", &[
        "Saving stores where the camera is pointing and its zoom. Moving to a position turns the gimbal, so the app asks first.".into(),
        "Press Enter in a name field to rename a position.".into(),
    ]));
    body.append(&actions);

    // Read when the card first appears, not at start-up.
    let loaded = Rc::new(Cell::new(false));
    card.connect_map(move |_| { if !loaded.replace(true) { reload(); } });
    card
}

fn app_presets_card(store: &Store) -> gtk::Box {
    let (card, body) = card("Presets", "tune",
        "Save the current picture, tracking and gimbal position under a name, and bring them back later. Applying a preset turns the gimbal to its saved position.");
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Preset name, for example Desk"));
    entry.set_hexpand(true);
    let save = gtk::Button::with_label("Save current");
    save.add_css_class("filled");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&entry);
    row.append(&save);
    body.append(&row);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&list);

    let render: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
    let rebuild: Rc<dyn Fn()> = {
        let (list, store, render) = (list.clone(), store.clone(), render.clone());
        Rc::new(move || {
            while let Some(child) = list.first_child() { list.remove(&child); }
            let saved = current_config().presets;
            if saved.is_empty() {
                let empty = widgets::text("No presets yet. Set the camera up the way you like, then save it here.", &["small", "dim"]);
                empty.add_css_class("list-item");
                empty.add_css_class("empty");
                list.append(&empty);
            }
            for (index, preset) in saved.iter().enumerate() {
                let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                item.add_css_class("list-item");
                let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
                titles.set_hexpand(true);
                titles.append(&widgets::text(&preset.name, &["setting-label"]));
                titles.append(&widgets::text(&format!("{} settings", preset.settings.len()), &["small", "dim"]));
                let apply = gtk::Button::with_label("Apply");
                apply.add_css_class("small");
                let delete = widgets::icon_button("delete", &format!("Delete {}", preset.name), true);
                delete.add_css_class("danger");
                item.append(&titles);
                item.append(&apply);
                item.append(&delete);
                list.append(&item);

                let (store_apply, preset_apply) = (store.clone(), preset.clone());
                apply.connect_clicked(move |_| {
                    let (store, preset) = (store_apply.clone(), preset_apply.clone());
                    let run = move || {
                        let (store, preset) = (store.clone(), preset.clone());
                        let name = preset.name.clone();
                        store.clone().call(None, move |d| presets::apply(d, &preset), move |r| {
                            match r {
                                Ok(message) => store.toast(&if message.is_empty() { format!("Applied {name}") } else { message }),
                                Err(e) => store.error(&format!("Some settings in {name} were not applied: {e}")),
                            }
                            store.refresh_soon();
                        });
                    };
                    // A preset with a saved position turns the gimbal, so ask first.
                    if presets::moves_gimbal(&preset_apply) {
                        store_apply.confirm("open_with", &format!("Apply {}?", preset_apply.name),
                            "This preset includes a gimbal position, so the camera will turn. Clear the space around it first.", "Apply preset", run);
                    } else {
                        run();
                    }
                });
                let (store_delete, render_delete) = (store.clone(), render.clone());
                let name = preset.name.clone();
                delete.connect_clicked(move |_| {
                    let removed = update_config_checked(|c| { if index < c.presets.len() { c.presets.remove(index); } });
                    match removed {
                        Ok(()) => store_delete.toast(&format!("Deleted {name}")),
                        Err(e) => store_delete.error(&format!("Could not delete {name}: {e}")),
                    }
                    if let Some(render) = render_delete.borrow().as_ref() { render(); }
                });
            }
        })
    };
    *render.borrow_mut() = Some(rebuild.clone());
    rebuild();

    let (store_save, rebuild_save, entry_save) = (store.clone(), rebuild.clone(), entry.clone());
    save.connect_clicked(move |_| {
        let name = entry_save.text().trim().to_string();
        if name.is_empty() { store_save.error("Name the preset first."); return; }
        let (store, rebuild, entry) = (store_save.clone(), rebuild_save.clone(), entry_save.clone());
        let captured_name = name.clone();
        store_save.call(None, move |d| presets::capture(d, captured_name), move |r| {
            match r {
                Ok(preset) => {
                    let saved = update_config_checked(|c| {
                        if let Some(existing) = c.presets.iter_mut().find(|p| p.name.eq_ignore_ascii_case(&name)) { *existing = preset.clone(); }
                        else if c.presets.len() < 10 { c.presets.push(preset.clone()); }
                    });
                    let full = current_config().presets.len() >= 10 && !current_config().presets.iter().any(|p| p.name.eq_ignore_ascii_case(&name));
                    match saved {
                        Ok(()) if full => store.error("You can keep up to 10 presets. Delete one first."),
                        Ok(()) => { store.toast(&format!("Saved {name}")); entry.set_text(""); }
                        Err(e) => store.error(&format!("Could not save {name}: {e}")),
                    }
                    rebuild();
                }
                Err(e) => store.error(&format!("Could not read the camera's settings: {e}")),
            }
        });
    });
    card
}

fn startup_pose_card(store: &Store) -> gtk::Box {
    let (card, body) = card("Start-up position", "power",
        "Where the camera points each time it powers on.");
    let current = widgets::text("Reading…", &["body"]);
    body.append(&current);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let refresh = widgets::icon_text_button("refresh", "Read");
    refresh.add_css_class("small");
    let set = gtk::Button::with_label("Use current position");
    set.add_css_class("small");
    actions.append(&refresh);
    actions.append(&set);
    actions.append(&widgets::info_button("Start-up position", &[
        "This is stored in the camera, separate from your saved positions. Using the current position keeps the camera's other start-up picture settings as they are.".into(),
    ]));
    body.append(&actions);

    let read: Rc<dyn Fn()> = {
        let (store, current) = (store.clone(), current.clone());
        Rc::new(move || {
            let current = current.clone();
            store.call(Some("startup-pose"), |d| backend::read_initial_camera_state(d), move |r| match r {
                Ok(state) => {
                    let source = match state.source { 0..=2 => format!("Position {}", state.source + 1), 3 => "Custom position".into(), other => format!("Unknown ({other})") };
                    current.set_text(&format!("{source} · Pan {} · Tilt {} · Zoom {:.2}×", backend::angles::format(state.pan), backend::angles::format(state.tilt), state.zoom));
                }
                Err(_) => current.set_text("Not available right now."),
            });
        })
    };
    let read_click = read.clone();
    refresh.connect_clicked(move |_| read_click());
    let (store_set, read_set) = (store.clone(), read.clone());
    set.connect_clicked(move |_| {
        let (store, read) = (store_set.clone(), read_set.clone());
        store_set.confirm("power", "Change the start-up position?", "From now on the camera will point where it is pointing now each time it powers on.", "Use current position", move || {
            let (store2, read2) = (store.clone(), read.clone());
            store.call(None, |d| backend::set_initial_camera_state(d), move |r| {
                match r { Ok(_) => store2.toast("Start-up position saved"), Err(e) => store2.error(&format!("Could not save the start-up position: {e}")) }
                read2();
            });
        });
    });
    let loaded = Rc::new(Cell::new(false));
    card.connect_map(move |_| { if !loaded.replace(true) { read(); } });
    card
}

// ── Device page ───────────────────────────────────────────────────────────

pub fn capture_mode_card(store: &Store, hub: &PreviewHub) -> gtk::Box {
    let (card, body) = card("Capture mode", "videocam",
        "The format, size and frame rate the camera sends to apps.");
    let loaded = Rc::new(Cell::new(false));
    let formats = gtk::DropDown::from_strings(&[]);
    let sizes = gtk::DropDown::from_strings(&[]);
    let rates = gtk::DropDown::from_strings(&[]);
    for (name, drop) in [("Format", &formats), ("Size", &sizes), ("Frame rate", &rates)] {
        widgets::lock_wheel(drop);
        widgets::label_accessible(drop, name);
    }
    let grid = gtk::Grid::new();
    grid.set_row_spacing(10);
    grid.set_column_spacing(12);
    for (row, (name, drop)) in [("Format", &formats), ("Size", &sizes), ("Frame rate", &rates)].into_iter().enumerate() {
        let label = widgets::text(name, &["setting-label"]);
        label.set_valign(gtk::Align::Center);
        grid.attach(&label, 0, row as i32, 1, 1);
        drop.set_hexpand(true);
        grid.attach(drop, 1, row as i32, 1, 1);
    }
    body.append(&grid);
    let status = widgets::text("Changing this briefly pauses the preview.", &["small", "faint"]);
    body.append(&status);

    struct Model { caps: Vec<video::VideoCapability>, current: Option<video::VideoMode> }
    let model: Rc<RefCell<Model>> = Rc::new(RefCell::new(Model { caps: Vec::new(), current: None }));
    let suppress = Rc::new(Cell::new(false));
    // True while a change is being applied, so a second one cannot start on top of it.
    let busy = Rc::new(Cell::new(false));
    // Each choice restarts a short wait, so picking a size and then a rate applies once.
    let latest_choice = Rc::new(Cell::new(0_u32));

    let fill: Rc<dyn Fn()> = {
        let (model, formats, sizes, rates, suppress) = (model.clone(), formats.clone(), sizes.clone(), rates.clone(), suppress.clone());
        Rc::new(move || {
            suppress.set(true);
            let model = model.borrow();
            let mut names: Vec<String> = model.caps.iter().map(|c| c.pixel_format.clone()).collect();
            names.sort();
            names.dedup();
            let current = model.current.as_ref();
            let format = current.map(|m| m.pixel_format.clone()).filter(|f| names.contains(f)).or_else(|| names.first().cloned()).unwrap_or_default();
            formats.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
            formats.set_selected(names.iter().position(|n| *n == format).unwrap_or(0) as u32);
            let mut size_list: Vec<(u32, u32)> = model.caps.iter().filter(|c| c.pixel_format == format).map(|c| (c.width, c.height)).collect();
            size_list.sort_by(|a, b| (b.0 * b.1).cmp(&(a.0 * a.1)));
            size_list.dedup();
            let size_names: Vec<String> = size_list.iter().map(|(w, h)| format!("{w} × {h}")).collect();
            sizes.set_model(Some(&gtk::StringList::new(&size_names.iter().map(String::as_str).collect::<Vec<_>>())));
            let size = current.map(|m| (m.width, m.height)).filter(|s| size_list.contains(s)).or_else(|| size_list.first().copied());
            sizes.set_selected(size.and_then(|s| size_list.iter().position(|x| *x == s)).unwrap_or(0) as u32);
            let mut rate_list: Vec<u32> = model.caps.iter().filter(|c| c.pixel_format == format && Some((c.width, c.height)) == size).flat_map(|c| c.frame_rates_milli.clone()).collect();
            rate_list.sort_by(|a, b| b.cmp(a));
            rate_list.dedup();
            let rate_names: Vec<String> = rate_list.iter().map(|r| format!("{:.0} fps", f64::from(*r) / 1000.0)).collect();
            rates.set_model(Some(&gtk::StringList::new(&rate_names.iter().map(String::as_str).collect::<Vec<_>>())));
            let rate = current.map(|m| m.frame_rate_milli).filter(|r| rate_list.contains(r)).or_else(|| rate_list.first().copied());
            rates.set_selected(rate.and_then(|r| rate_list.iter().position(|x| *x == r)).unwrap_or(0) as u32);
            suppress.set(false);
        })
    };

    let load: Rc<dyn Fn()> = {
        let (store, model, fill) = (store.clone(), model.clone(), fill.clone());
        Rc::new(move || {
            let (model, fill, store_err) = (model.clone(), fill.clone(), store.clone());
            store.call(Some("video-modes"), |d| (video::capabilities(d), video::current_mode(d)), move |(caps, current)| {
                match caps {
                    Ok(caps) => { *model.borrow_mut() = Model { caps, current }; fill(); }
                    Err(e) => store_err.error(&format!("Could not read the camera's capture modes: {e}")),
                }
            });
        })
    };
    let load_map = load.clone();
    card.connect_map(move |_| { if !loaded.replace(true) { load_map(); } });

    // Choosing a format or size changes what the next dropdowns offer.
    let apply: Rc<dyn Fn()> = {
        let (store, hub, model, formats, sizes, rates, suppress, load, status, busy) =
            (store.clone(), hub.clone(), model.clone(), formats.clone(), sizes.clone(), rates.clone(), suppress.clone(), load.clone(), status.clone(), busy.clone());
        Rc::new(move || {
            if suppress.get() || busy.get() { return; }
            let selected = |drop: &gtk::DropDown| drop.selected_item().and_then(|o| o.downcast::<gtk::StringObject>().ok()).map(|s| s.string().to_string());
            let (Some(format), Some(size), Some(rate)) = (selected(&formats), selected(&sizes), selected(&rates)) else { return };
            let mut parts = size.split(" × ");
            let (Some(width), Some(height)) = (parts.next().and_then(|v| v.parse::<u32>().ok()), parts.next().and_then(|v| v.parse::<u32>().ok())) else { return };
            let Some(fps) = rate.split(' ').next().and_then(|v| v.parse::<f64>().ok()) else { return };
            let mode = video::VideoMode { width, height, pixel_format: format, frame_rate_milli: (fps * 1000.0).round() as u32 };
            if model.borrow().current.as_ref() == Some(&mode) { return; }
            let previous = model.borrow().current.clone();
            let (store2, load2, mode_for_job) = (store.clone(), load.clone(), mode.clone());
            // Show that something is happening, and take the lists out of play until it is over.
            busy.set(true);
            for drop in [&formats, &sizes, &rates] { drop.set_sensitive(false); }
            status.set_text("Applying… the picture pauses for a moment.");
            let (formats_done, sizes_done, rates_done, status_done, busy_done) = (formats.clone(), sizes.clone(), rates.clone(), status.clone(), busy.clone());
            hub.while_stopped(move |d| {
                match video::set_mode(d, &mode_for_job) {
                    Ok(()) => Ok(()),
                    Err(e) => { if let Some(p) = previous { let _ = video::set_mode(d, &p); } Err(e) }
                }
            }, move |r| {
                busy_done.set(false);
                for drop in [&formats_done, &sizes_done, &rates_done] { drop.set_sensitive(true); }
                status_done.set_text("Changing this briefly pauses the preview.");
                match r {
                    Ok(()) => {
                        let saved = config::VideoModeConfig { width: mode.width, height: mode.height, pixel_format: mode.pixel_format.clone(), frame_rate_milli: mode.frame_rate_milli };
                        let _ = update_config_checked(|c| c.video_mode = Some(saved));
                        store2.toast(&format!("Capture mode {} × {} at {:.0} fps", mode.width, mode.height, f64::from(mode.frame_rate_milli) / 1000.0));
                    }
                    Err(e) => store2.error(&format!("The camera did not accept that mode: {e}")),
                }
                load2();
            });
        })
    };
    for (drop, rebuild_first) in [(&formats, true), (&sizes, true), (&rates, false)] {
        let (apply, model, fill, suppress, formats, sizes, latest_choice) = (apply.clone(), model.clone(), fill.clone(), suppress.clone(), formats.clone(), sizes.clone(), latest_choice.clone());
        drop.connect_selected_notify(move |_| {
            if suppress.get() { return; }
            if rebuild_first {
                // Keep the other lists valid for the new choice, then apply the first match.
                let selected = |d: &gtk::DropDown| d.selected_item().and_then(|o| o.downcast::<gtk::StringObject>().ok()).map(|s| s.string().to_string());
                let mut guess = model.borrow().current.clone();
                if let (Some(format), Some(size)) = (selected(&formats), selected(&sizes)) {
                    if let Some(g) = guess.as_mut() {
                        g.pixel_format = format.clone();
                        let mut parts = size.split(" × ");
                        if let (Some(w), Some(h)) = (parts.next().and_then(|v| v.parse().ok()), parts.next().and_then(|v| v.parse().ok())) { g.width = w; g.height = h; }
                        let valid = model.borrow().caps.iter().any(|c| c.pixel_format == g.pixel_format && c.width == g.width && c.height == g.height);
                        if !valid {
                            if let Some(c) = model.borrow().caps.iter().find(|c| c.pixel_format == g.pixel_format) { g.width = c.width; g.height = c.height; }
                        }
                        let rates = model.borrow().caps.iter().find(|c| c.pixel_format == g.pixel_format && c.width == g.width && c.height == g.height).map(|c| c.frame_rates_milli.clone()).unwrap_or_default();
                        if !rates.contains(&g.frame_rate_milli) { if let Some(r) = rates.first() { g.frame_rate_milli = *r; } }
                    }
                }
                let before = model.borrow().current.clone();
                model.borrow_mut().current = guess;
                fill();
                let after = model.borrow().current.clone();
                model.borrow_mut().current = before;
                let _ = after;
            }
            let choice = latest_choice.get().wrapping_add(1);
            latest_choice.set(choice);
            let (apply, latest_choice) = (apply.clone(), latest_choice.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if latest_choice.get() == choice { apply(); }
            });
        });
    }
    card
}

pub fn appearance_card(style: &Rc<Style>) -> gtk::Box {
    let (card, body) = card("Appearance", "palette", "How the app looks. Applies straight away.");
    let section = |title: &str, control: &gtk::Widget| {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
        row.append(&widgets::text(title, &["setting-label"]));
        row.append(control);
        row
    };

    let mode = Segmented::new();
    mode.set_options(&[opt(0, "System"), opt(1, "Light"), opt(2, "Dark")]);
    mode.select(Some(match style.mode() { ThemeMode::System => 0, ThemeMode::Light => 1, ThemeMode::Dark => 2 }));
    let s = style.clone();
    mode.on_change(move |v| s.set_mode(match v { 1 => ThemeMode::Light, 2 => ThemeMode::Dark, _ => ThemeMode::System }));
    body.append(&section("Theme", mode.widget().upcast_ref()));

    let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let buttons: Rc<RefCell<Vec<(String, gtk::ToggleButton)>>> = Rc::default();
    for (id, name, _) in theme::ACCENTS {
        let button = gtk::ToggleButton::new();
        button.add_css_class("swatch");
        button.add_css_class(&format!("swatch-{id}"));
        button.set_tooltip_text(Some(name));
        widgets::label_accessible(&button, &format!("{name} accent"));
        button.set_active(style.accent() == id);
        let (s, group) = (style.clone(), buttons.clone());
        button.connect_clicked(move |clicked| {
            for (other, b) in group.borrow().iter() { b.set_active(other == id); }
            let _ = clicked;
            s.set_accent(id);
        });
        swatches.append(&button);
        buttons.borrow_mut().push((id.to_string(), button));
    }
    body.append(&section("Accent colour", swatches.upcast_ref()));

    let scale = Segmented::new();
    scale.set_options(&[opt(0, "Auto"), opt(1, "Compact"), opt(2, "Default"), opt(3, "Large")]);
    scale.select(Some(match style.scale() { UiScale::Auto => 0, UiScale::Compact => 1, UiScale::Default => 2, UiScale::Large => 3 }));
    let s = style.clone();
    scale.on_change(move |v| s.set_scale(match v { 1 => UiScale::Compact, 2 => UiScale::Default, 3 => UiScale::Large, _ => UiScale::Auto }));
    body.append(&section("Size", scale.widget().upcast_ref()));

    let buttons_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&widgets::text("Window buttons", &["setting-label"]));
    titles.append(&widgets::text("Show minimise, maximise and close in the header. Turn off if your window manager draws its own.", &["setting-summary"]));
    let buttons_switch = gtk::Switch::new();
    buttons_switch.set_valign(gtk::Align::Center);
    buttons_switch.set_active(style.window_buttons());
    widgets::label_accessible(&buttons_switch, "Show window buttons");
    let s = style.clone();
    buttons_switch.connect_active_notify(move |sw| s.set_window_buttons(sw.is_active()));
    buttons_row.append(&titles);
    buttons_row.append(&buttons_switch);
    body.append(&buttons_row);

    let motion = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&widgets::text("Reduce motion", &["setting-label"]));
    titles.append(&widgets::text("Turn off animations and springy movement.", &["setting-summary"]));
    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    switch.set_active(style.reduce_motion());
    widgets::label_accessible(&switch, "Reduce motion");
    let s = style.clone();
    switch.connect_active_notify(move |sw| s.set_reduce_motion(sw.is_active()));
    motion.append(&titles);
    motion.append(&switch);
    body.append(&motion);

    // The wireframe's pan direction, for the few setups where it swings the wrong way.
    let pan = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&widgets::text("Reverse pan on the camera model", &["setting-label"]));
    titles.append(&widgets::text("Turn on if the model swings the wrong way when the camera pans. You only need to set this once.", &["setting-summary"]));
    let reverse = gtk::Switch::new();
    reverse.set_valign(gtk::Align::Center);
    reverse.set_active(current_config().model_pan_reversed);
    widgets::label_accessible(&reverse, "Reverse pan on the camera model");
    reverse.connect_active_notify(|sw| super::gimbal_model::set_pan_reversed(sw.is_active()));
    pan.append(&titles);
    pan.append(&reverse);
    body.append(&pan);
    card
}

pub fn developer_card(store: &Store) -> gtk::Box {
    let (card, body) = card("Developer tools", "terminal", "For checking how the camera talks. Nothing here changes camera settings unless you start a capture.");
    let open = gtk::Button::with_label("Open developer tools");
    open.add_css_class("small");
    open.set_halign(gtk::Align::Start);
    let store = store.clone();
    open.connect_clicked(move |button| {
        if let Some(window) = button.root().and_then(|r| r.downcast::<gtk::Window>().ok()) {
            super::devtools::open(&window, &store.device());
        }
    });
    body.append(&open);
    card
}

#[allow(dead_code)]
fn unused(_: &glib::SourceId) {}
