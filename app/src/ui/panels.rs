//! Cards that are not generated from the registry: presets, capture mode,
//! appearance and developer tools.

use super::preview::PreviewHub;
use super::store::Store;
use super::style::Style;
use super::theme::{self, ThemeMode, UiScale};
use super::widgets::{self, Segmented};
use crate::backend::registry::Opt;
use crate::backend::{self, current_config, update_config, update_config_checked, CameraPresetEntry};
use crate::{config, presets, video};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

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
    let scroll = widgets::scroller();
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
            let _ = &slots_go;
            let store = store_go.clone();
            let store2 = store.clone();
            let store_wake = store.clone();
            super::joystick::after_waking(&store_wake, move || store.call_now(move |d| backend::recall_camera_preset(d, slot_number), move |r| match r {
                Ok(()) => store2.toast(&format!("Moving to position {}", slot_number + 1)),
                Err(e) => store2.error(&format!("Could not move to position {}: {e}", slot_number + 1)),
            }));
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
        "Saving stores where the camera is pointing and its zoom. Moving to a position turns the gimbal.".into(),
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
        "Save the current picture, tracking and gimbal position under a name, and bring them back later.");
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
                    let _ = (&store_apply, &preset_apply);
                    // A sleeping camera is woken first, or the preset's position would be ignored.
                    super::joystick::after_waking(&store_apply, run);
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

pub fn startup_pose_card(store: &Store) -> gtk::Box {
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
        "The size, frame rate and format the camera sends to apps.");
    let loaded = Rc::new(Cell::new(false));
    // One list of every mode the camera offers, so a size and a frame rate that do not go together can
    // never be chosen.
    let modes = gtk::DropDown::from_strings(&[]);
    widgets::lock_wheel(&modes);
    widgets::label_accessible(&modes, "Capture mode");
    modes.set_hexpand(true);
    body.append(&modes);
    let status = widgets::text("Changing this briefly pauses the preview.", &["small", "faint"]);
    body.append(&status);

    struct Model { modes: Vec<video::VideoMode>, current: Option<video::VideoMode> }
    let model: Rc<RefCell<Model>> = Rc::new(RefCell::new(Model { modes: Vec::new(), current: None }));
    let suppress = Rc::new(Cell::new(false));
    // True while a change is being applied, so a second one cannot start on top of it.
    let busy = Rc::new(Cell::new(false));

    let describe = |m: &video::VideoMode| {
        let name = match (m.width, m.height) { (3840, 2160) => " (4K)", (1920, 1080) => " (1080p)", (1280, 720) => " (720p)", _ => "" };
        format!("{} × {}{} · {:.0} fps · {}", m.width, m.height, name, f64::from(m.frame_rate_milli) / 1000.0, m.pixel_format)
    };
    let fill: Rc<dyn Fn()> = {
        let (model, modes, suppress) = (model.clone(), modes.clone(), suppress.clone());
        Rc::new(move || {
            suppress.set(true);
            let model = model.borrow();
            let names: Vec<String> = model.modes.iter().map(describe).collect();
            modes.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
            let at = model.current.as_ref().and_then(|c| model.modes.iter().position(|m| m.width == c.width && m.height == c.height
                && m.pixel_format == c.pixel_format && m.frame_rate_milli.abs_diff(c.frame_rate_milli) <= 500));
            modes.set_selected(at.map(|i| i as u32).unwrap_or(gtk::INVALID_LIST_POSITION));
            suppress.set(false);
        })
    };

    let load: Rc<dyn Fn()> = {
        let (store, model, fill) = (store.clone(), model.clone(), fill.clone());
        Rc::new(move || {
            let (model, fill, store_err) = (model.clone(), fill.clone(), store.clone());
            store.call(Some("video-modes"), |d| (video::capabilities(d), video::current_mode(d)), move |(caps, current)| {
                match caps {
                    Ok(caps) => {
                        // Largest first; within a size, the fastest first; compressed before raw.
                        let mut all: Vec<video::VideoMode> = caps.iter().flat_map(|c| c.frame_rates_milli.iter().map(move |r| video::VideoMode {
                            width: c.width, height: c.height, pixel_format: c.pixel_format.clone(), frame_rate_milli: *r })).collect();
                        all.sort_by(|a, b| (b.width * b.height).cmp(&(a.width * a.height))
                            .then(b.frame_rate_milli.cmp(&a.frame_rate_milli))
                            .then((a.pixel_format != "MJPG").cmp(&(b.pixel_format != "MJPG"))));
                        all.dedup();
                        *model.borrow_mut() = Model { modes: all, current };
                        fill();
                    }
                    Err(e) => store_err.error(&format!("Could not read the camera's capture modes: {e}")),
                }
            });
        })
    };
    let load_map = load.clone();
    card.connect_map(move |_| { if !loaded.replace(true) { load_map(); } });
    // The preview knows what the camera is really sending. If that is not what the list shows (after the
    // camera was unplugged and plugged in, say), the list follows it.
    {
        let (model, fill, busy, load) = (model.clone(), fill.clone(), busy.clone(), load.clone());
        hub.on_mode(move |mode| {
            if busy.get() { return; }
            if model.borrow().modes.is_empty() { load(); return; }
            let differs = model.borrow().current.as_ref() != Some(mode);
            if differs {
                model.borrow_mut().current = Some(mode.clone());
                fill();
            }
        });
    }

    let apply: Rc<dyn Fn(video::VideoMode)> = {
        let (store, hub, model, modes, load, status, busy) =
            (store.clone(), hub.clone(), model.clone(), modes.clone(), load.clone(), status.clone(), busy.clone());
        Rc::new(move |mode: video::VideoMode| {
            if busy.get() { return; }
            if model.borrow().current.as_ref() == Some(&mode) { return; }
            // Changing the mode breaks the picture of any other program that is using the camera, and the
            // camera refuses anyway while a stream is open. So the choice waits: the background service
            // applies it the moment nothing has the camera open.
            if !backend::ui_test_mode() {
                let others = backend::usage::holder_names(&store.device());
                if !others.is_empty() {
                    let wanted = config::VideoModeConfig { width: mode.width, height: mode.height, pixel_format: mode.pixel_format.clone(), frame_rate_milli: mode.frame_rate_milli };
                    let _ = update_config_checked(|c| c.pending_video_mode = Some(wanted));
                    let who = format!("{} {}", others.join(", "), if others.len() == 1 { "is" } else { "are" });
                    if crate::service::running_status().is_some() {
                        store.toast(&format!("{who} using the camera, so the mode changes as soon as it is free."));
                    } else {
                        store.error(&format!("{who} using the camera. The change waits for it to be free, but the background service is not running to do it: start it on the Device tab, or close the program and choose again."));
                    }
                    load();
                    return;
                }
            }
            let previous = model.borrow().current.clone();
            let (store2, load2, mode_for_job) = (store.clone(), load.clone(), mode.clone());
            // Show that something is happening, and take the list out of play until it is over.
            busy.set(true);
            modes.set_sensitive(false);
            status.set_text("Applying… the picture pauses for a moment.");
            let (modes_done, status_done, busy_done) = (modes.clone(), status.clone(), busy.clone());
            hub.while_stopped(move |d| {
                match video::set_mode(d, &mode_for_job) {
                    Ok(()) => Ok(()),
                    Err(e) => { if let Some(p) = previous { let _ = video::set_mode(d, &p); } Err(e) }
                }
            }, move |r| {
                busy_done.set(false);
                modes_done.set_sensitive(true);
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
    {
        let (apply, model, suppress) = (apply.clone(), model.clone(), suppress.clone());
        modes.connect_selected_notify(move |drop| {
            if suppress.get() { return; }
            let chosen = model.borrow().modes.get(drop.selected() as usize).cloned();
            if let Some(mode) = chosen { apply(mode); }
        });
    }
    card
}

/// The sleep timer: when the camera is put to sleep so it does not stay on. The background service does it
/// and reads these settings as soon as they change.
pub fn sleep_timer_card() -> gtk::Box {
    let (card, body) = card("Sleep timer", "bedtime",
        "Puts the camera to sleep when it is not needed, so it does not stay on. All of these need the background service.");
    body.append(&service_needed_notice("The background service is not running, so none of these happen. Install or start it under Background service, further down this tab."));
    let switch_row = |title: &str, summary: &str, active: bool, on_change: Box<dyn Fn(bool)>| -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);
        titles.append(&widgets::text(title, &["setting-label"]));
        titles.append(&widgets::text(summary, &["setting-summary"]));
        let switch = gtk::Switch::new();
        switch.set_valign(gtk::Align::Center);
        switch.set_active(active);
        widgets::label_accessible(&switch, title);
        switch.connect_active_notify(move |sw| on_change(sw.is_active()));
        row.append(&titles);
        row.append(&switch);
        row
    };
    let config = current_config();

    // A box to type minutes in, with arrows to step it.
    let minutes = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let caption = widgets::line("Sleep after", &["setting-label"]);
    caption.set_hexpand(true);
    minutes.append(&caption);
    let spin = gtk::SpinButton::with_range(1.0, 1440.0, 1.0);
    spin.set_numeric(true);
    spin.set_width_chars(5);
    spin.set_value(f64::from(config.service.auto_sleep_timeout_minutes.clamp(1, 1440)));
    widgets::lock_wheel(&spin);
    widgets::label_accessible(&spin, "Minutes without use before the camera sleeps");
    minutes.append(&spin);
    minutes.append(&widgets::line("minutes", &["small", "dim"]));
    minutes.set_sensitive(config.service.auto_sleep_enabled);
    let save_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    spin.connect_value_changed(move |spin| {
        let minutes = spin.value().round() as u32;
        // Saved once typing or stepping settles, not on every keystroke.
        if let Some(id) = save_timer.borrow_mut().take() { id.remove(); }
        let timer = save_timer.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(600), move || {
            timer.borrow_mut().take();
            update_config(|c| c.service.auto_sleep_timeout_minutes = minutes);
        });
        *save_timer.borrow_mut() = Some(id);
    });

    body.append(&switch_row("Sleep when nothing is using the camera",
        "After the time below with no app using the camera or the Studio camera (a paused preview does not count) and no control used in this window.",
        config.service.auto_sleep_enabled,
        Box::new({ let minutes = minutes.clone(); move |on| { minutes.set_sensitive(on); update_config(|c| c.service.auto_sleep_enabled = on); } })));
    body.append(&minutes);
    body.append(&switch_row("Sleep when the computer shuts down",
        "Also when you log out, so the camera is not left on.",
        config.service.sleep_on_shutdown,
        Box::new(|on| update_config(|c| c.service.sleep_on_shutdown = on))));
    body.append(&switch_row("Sleep when the computer suspends",
        "Needs dbus-monitor and systemd-inhibit (or elogind-inhibit). The camera is put to sleep just before the suspend.",
        config.service.sleep_on_suspend,
        Box::new(|on| update_config(|c| c.service.sleep_on_suspend = on))));
    card
}

/// The app itself: whether it is installed (AppImage in ~/Applications, the tiny4lite-control command, the
/// app-menu entry), with Install, Update and Remove.
pub fn app_card(store: &Store) -> gtk::Box {
    let (card, body) = card("App", "open_in_full",
        "Install keeps the AppImage in ~/Applications and adds the tiny4lite-control command and an app menu entry. The background service uses the installed copy.");
    let status = widgets::text("", &["setting-summary"]);
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.set_selectable(true);
    body.append(&status);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let install = gtk::Button::with_label("Install");
    install.add_css_class("filled");
    let remove = gtk::Button::with_label("Remove");
    for button in [&install, &remove] { button.add_css_class("small"); buttons.append(button); }
    body.append(&buttons);
    let refresh: Rc<dyn Fn()> = {
        let (status, install, remove) = (status.clone(), install.clone(), remove.clone());
        Rc::new(move || {
            use crate::install as i;
            let from = i::running_from().map(|p| p.display().to_string()).unwrap_or_default();
            let mut lines = Vec::new();
            match i::installed_target() {
                Some(target) => {
                    lines.push(format!("Installed: {}", target.display()));
                    if i::update_available() {
                        lines.push(format!("This window runs another file: {from}. Install it to use it from now on (the command, the menu entry and the service move to it)."));
                    }
                    if !i::bin_on_path() { lines.push(format!("To use the tiny4lite-control command, add ~/.local/bin to your PATH: {}", i::path_hint())); }
                }
                None => lines.push(format!("Not installed. Running from {from}.")),
            }
            status.set_text(&lines.join("\n"));
            install.set_visible(i::installed_target().is_none() || i::update_available());
            install.set_label(if i::installed_target().is_some() { "Install this version" } else { "Install" });
            remove.set_visible(i::installed_target().is_some());
        })
    };
    refresh();
    let act = |store: Store, refresh: Rc<dyn Fn()>, work: fn() -> Result<String, String>| move |_: &gtk::Button| {
        match work() { Ok(message) => store.toast(&message), Err(error) => store.error(&error) }
        refresh();
    };
    install.connect_clicked(act(store.clone(), refresh.clone(), crate::install::install));
    remove.connect_clicked(act(store.clone(), refresh.clone(), crate::install::uninstall));
    card
}

/// The Studio camera: a second camera with filters (denoise first), run by the background service. Its
/// switch, each filter with its strength beside it, the graphics-chip choice, what it needs, and what it
/// is doing and costing right now.
pub fn denoise_card(store: &Store) -> gtk::Box {
    let (card, body) = card("Studio camera", "blur_on",
        "Adds a second camera, \u{201c}Tiny 2 Lite Studio\u{201d}, with the filters below applied. Choose it in your video app. It only uses the camera while an app has it open.");
    body.append(&service_needed_notice("The background service is not running, so the Studio camera is not either. Install or start it under Background service, further down this tab."));
    let config = current_config().denoise;
    body.append(&plain_switch_row("Studio camera",
        "Make \u{201c}Tiny 2 Lite Studio\u{201d} available to apps.",
        config.enabled, |on| update_config(|c| c.denoise.enabled = on)));
    body.append(&widgets::text("Filters", &["small", "dim", "chooser-head"]));

    // The switch, and the strength right beside it while it is on.
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&widgets::text("Denoise", &["setting-label"]));
    titles.append(&widgets::text("Less grain. Strength: to the right, smoother but softer.", &["setting-summary"]));
    let strength = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 100.0, 1.0);
    strength.set_draw_value(false);
    strength.set_size_request(140, -1);
    strength.set_value(f64::from(config.strength));
    strength.set_valign(gtk::Align::Center);
    widgets::lock_wheel(&strength);
    widgets::label_accessible(&strength, "Denoise strength");
    let amount = widgets::line(&config.strength.to_string(), &["setting-value", "tnum"]);
    amount.set_width_chars(3);
    amount.set_valign(gtk::Align::Center);
    let beside = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    beside.append(&strength);
    beside.append(&amount);
    let reveal = gtk::Revealer::new();
    reveal.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
    reveal.set_child(Some(&beside));
    reveal.set_reveal_child(config.denoise);
    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    switch.set_active(config.denoise);
    widgets::label_accessible(&switch, "Denoise");
    {
        let reveal = reveal.clone();
        switch.connect_active_notify(move |sw| {
            let on = sw.is_active();
            update_config(|c| c.denoise.denoise = on);
            reveal.set_reveal_child(on);
        });
    }
    {
        // Saved once the slider settles: a change restarts the denoising for a moment.
        let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let amount = amount.clone();
        strength.connect_value_changed(move |s| {
            let value = s.value().round() as u8;
            amount.set_text(&value.to_string());
            if let Some(id) = timer.borrow_mut().take() { id.remove(); }
            let slot = timer.clone();
            *timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(500), move || {
                slot.borrow_mut().take();
                update_config(|c| c.denoise.strength = value);
            }));
        });
    }
    row.append(&titles);
    row.append(&reveal);
    row.append(&switch);
    body.append(&row);

    body.append(&plain_switch_row("Use the graphics chip",
        "Denoise on the GPU when it can: VA-API (Intel) or Vulkan (AMD, Intel, NVIDIA), with the best-looking filter and almost no processor time. Off, or without one, the processor does it.",
        config.use_gpu, |on| update_config(|c| c.denoise.use_gpu = on)));
    body.append(&plain_switch_row("Limit to 30 fps",
        "A camera set to 60 fps gives the Studio camera 30: half the work, and plenty for calls.",
        config.limit_fps, |on| update_config(|c| c.denoise.limit_fps = on)));

    // What it needs, with a way to set it up.
    let needs = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let needs_text = widgets::text("", &["small", "dim"]);
    needs_text.set_hexpand(true);
    let setup = gtk::Button::with_label("Set up…");
    setup.add_css_class("small");
    setup.set_valign(gtk::Align::Center);
    setup.set_tooltip_text(Some("Opens a terminal that installs ffmpeg and v4l2loopback and makes the Studio camera (asks for your password)"));
    {
        let store = store.clone();
        setup.connect_clicked(move |_| { if let Err(error) = crate::setup::run_in_terminal(&["studio", "setup"]) { store.error(&error); } });
    }
    needs.append(&needs_text);
    needs.append(&setup);
    body.append(&needs);

    // What it is doing, and what it costs.
    let state = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    state.add_css_class("denoise-state");
    let glyph = widgets::icon("memory");
    glyph.set_valign(gtk::Align::Start);
    let state_text = widgets::text("", &["small"]);
    state_text.set_hexpand(true);
    state_text.set_selectable(true);
    state.append(&glyph);
    state.append(&state_text);
    body.append(&state);

    let refresh = {
        let (needs, needs_text, state, state_text) = (needs.downgrade(), needs_text.downgrade(), state.downgrade(), state_text.downgrade());
        move || -> bool {
            let (Some(needs), Some(needs_text), Some(state), Some(state_text)) = (needs.upgrade(), needs_text.upgrade(), state.upgrade(), state_text.upgrade()) else { return false };
            let mut missing = Vec::new();
            if !crate::denoise::ffmpeg_found() { missing.push("ffmpeg"); }
            if crate::denoise::loopback_node().is_none() { missing.push("the virtual camera (v4l2loopback)"); }
            needs.set_visible(!missing.is_empty());
            needs_text.set_text(&format!("Needs {}.", missing.join(" and ")));
            let report = crate::service::running_status().and_then(|s| s.denoise);
            match report {
                Some(r) if r.state != "off" || !r.detail.is_empty() => {
                    let mut line = match r.state.as_str() {
                        "running" => "In use".to_string(),
                        "waiting" => "Ready".to_string(),
                        "unavailable" => "Not available".to_string(),
                        "error" => "Problem".to_string(),
                        "off" => "Off".to_string(),
                        other => other.to_string(),
                    };
                    if let Some(cpu) = r.cpu_percent { line.push_str(&format!(" · CPU {cpu:.0}%")); }
                    if let Some(mb) = r.memory_mb { line.push_str(&format!(" · {mb:.0} MB")); }
                    if r.gpu { line.push_str(" · graphics chip"); }
                    if let Some(device) = &r.device { line.push_str(&format!(" · {device}")); }
                    if !r.detail.is_empty() { line.push_str(&format!("\n{}", r.detail)); }
                    state_text.set_text(&line);
                    state.set_visible(true);
                }
                _ => state.set_visible(false),
            }
            true
        }
    };
    refresh();
    glib::timeout_add_local(Duration::from_secs(2), move || if refresh() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    card
}

/// A banner across the top of the window while the background service is not set up: why it is needed,
/// a button that sets it up, and "Not now", which hides it for good (it can still be set up on the Device
/// tab). It goes away by itself once the service is installed or running.
pub fn service_setup_banner(store: &Store) -> gtk::Revealer {
    let revealer = gtk::Revealer::new();
    revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    let banner = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    banner.add_css_class("setup-banner");
    banner.set_margin_start(20);
    banner.set_margin_end(14);
    banner.set_margin_bottom(12);
    let glyph = widgets::icon("bedtime");
    glyph.set_valign(gtk::Align::Start);
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    words.append(&widgets::text("Set up the background service", &["setting-label"]));
    words.append(&widgets::text(
        "Without it the camera stays on after you are done with it: the service puts it to sleep when no app is using it, \
         when the computer shuts down or suspends, and puts your settings back when it is plugged in. It runs as your user and needs no password, except to install a missing program.",
        &["setting-summary"]));
    let setup = gtk::Button::with_label("Set up…");
    setup.add_css_class("filled");
    setup.add_css_class("small");
    setup.set_valign(gtk::Align::Center);
    let later = gtk::Button::with_label("Not now");
    later.add_css_class("small");
    later.set_valign(gtk::Align::Center);
    banner.append(&glyph);
    banner.append(&words);
    banner.append(&setup);
    banner.append(&later);
    revealer.set_child(Some(&banner));

    let wanted = || !current_config().service_setup_dismissed
        && crate::setup::installed().is_none() && crate::service::running_status().is_none()
        // The screenshot tests keep their layout unless they ask for the banner.
        && !(backend::ui_test_mode() && std::env::var_os("TINY4LITE_UI_TEST_BANNER").is_none());
    revealer.set_reveal_child(wanted());
    {
        let weak = revealer.downgrade();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            let Some(revealer) = weak.upgrade() else { return glib::ControlFlow::Break };
            if revealer.reveals_child() && !wanted() { revealer.set_reveal_child(false); }
            glib::ControlFlow::Continue
        });
    }
    {
        let store = store.clone();
        setup.connect_clicked(move |_| {
            if let Err(error) = crate::setup::run_in_terminal(&["service", "setup"]) { store.error(&error); }
        });
    }
    {
        let (store, weak) = (store.clone(), revealer.downgrade());
        later.connect_clicked(move |_| {
            update_config(|c| c.service_setup_dismissed = true);
            if let Some(revealer) = weak.upgrade() { revealer.set_reveal_child(false); }
            store.toast("You can set it up any time on the Device tab, under Background service.");
        });
    }
    revealer
}

/// A warning that shows only while the background service is not running, for the settings that rely
/// on it. It looks again every couple of seconds.
pub fn service_needed_notice(message: &str) -> gtk::Box {
    let notice = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    notice.add_css_class("service-notice");
    let glyph = widgets::icon("info");
    glyph.set_valign(gtk::Align::Start);
    let text = widgets::text(message, &["small"]);
    text.set_hexpand(true);
    notice.append(&glyph);
    notice.append(&text);
    let refresh = { let notice = notice.downgrade(); move || match notice.upgrade() {
        Some(notice) => { notice.set_visible(crate::service::running_status().is_none()); true }
        None => false,
    } };
    refresh();
    glib::timeout_add_local(Duration::from_secs(2), move || if refresh() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    notice
}

/// How long ago, for people: 40 s, 12 min, 3 h 5 min.
fn how_long(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds} s"),
        60..=3599 => format!("{} min", seconds / 60),
        _ => format!("{} h {} min", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// The background service: its status, installing it, and starting, stopping and removing it. The status
/// is watched and refreshed every couple of seconds.
pub fn service_card(store: &Store) -> gtk::Box {
    use crate::setup::{self, InstalledAs};
    let (card, body) = card("Background service", "terminal",
        "Watches the camera and does the sleep timer, shutdown and suspend sleep, restoring settings and waiting capture changes.");
    let status = widgets::text("", &["setting-summary"]);
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.set_selectable(true);
    body.append(&status);
    let needs = widgets::text("", &["small", "dim"]);
    needs.set_wrap(true);
    needs.set_xalign(0.0);
    body.append(&needs);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let install = gtk::Button::with_label("Install…");
    install.add_css_class("filled");
    install.set_tooltip_text(Some("Opens a terminal that checks what is needed, installs it (asking for your password) and installs and starts the service"));
    let start = gtk::Button::with_label("Start");
    let stop = gtk::Button::with_label("Stop");
    let remove = gtk::Button::with_label("Remove");
    remove.add_css_class("danger");
    let install_missing = gtk::Button::with_label("Install missing programs…");
    for button in [&install, &start, &stop, &remove, &install_missing] { button.add_css_class("small"); buttons.append(button); }
    body.append(&buttons);

    let refresh: Rc<dyn Fn()> = {
        let (status, needs, install, start, stop, remove, install_missing) =
            (status.downgrade(), needs.downgrade(), install.downgrade(), start.downgrade(), stop.downgrade(), remove.downgrade(), install_missing.downgrade());
        Rc::new(move || {
            let (Some(status), Some(needs), Some(install), Some(start), Some(stop), Some(remove), Some(install_missing)) =
                (status.upgrade(), needs.upgrade(), install.upgrade(), start.upgrade(), stop.upgrade(), remove.upgrade(), install_missing.upgrade()) else { return };
            let running = crate::service::running_status();
            let installed = setup::installed();
            let mut lines = Vec::new();
            match &running {
                Some(s) => {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    lines.push(format!("Running · process {} · up {}", s.pid, how_long(now.saturating_sub(s.started))));
                    lines.push(format!("Camera: {}", s.camera.as_deref().unwrap_or("not connected")));
                    lines.push(format!("Last: {} ({} ago)", s.last_event, how_long(now.saturating_sub(s.last_event_at))));
                }
                None => lines.push("Not running".to_string()),
            }
            lines.push(match installed {
                Some(InstalledAs::Systemd) => "Starts with your session: systemd user service".to_string(),
                Some(InstalledAs::Autostart) => "Starts with your session: desktop autostart entry".to_string(),
                None => "Not installed: it does not start with your session".to_string(),
            });
            status.set_text(&lines.join("\n"));
            install.set_visible(installed.is_none());
            start.set_visible(installed.is_some());
            // One started by hand can be stopped here too.
            stop.set_visible(installed.is_some() || running.is_some());
            remove.set_visible(installed.is_some());
            start.set_sensitive(running.is_none());
            stop.set_sensitive(running.is_some());
            let missing = setup::missing();
            if missing.is_empty() {
                needs.set_visible(false);
                install_missing.set_visible(false);
            } else {
                let names: Vec<String> = missing.iter().map(|r| format!("{} ({})", r.name, if r.required { "needed" } else { "for sleeping on suspend" })).collect();
                needs.set_text(&format!("Missing: {}", names.join(", ")));
                needs.set_visible(true);
                install_missing.set_visible(setup::manager().is_some());
            }
        })
    };
    refresh();
    {
        let refresh = refresh.clone();
        let weak = status.downgrade();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            if weak.upgrade().is_none() { return glib::ControlFlow::Break; }
            refresh();
            glib::ControlFlow::Continue
        });
    }

    // Start, stop and remove run off the window's thread, then report.
    let act = {
        let (store, refresh) = (store.clone(), refresh.clone());
        Rc::new(move |work: fn() -> Result<String, String>| {
            let (sender, receiver) = async_channel::bounded::<Result<String, String>>(1);
            std::thread::spawn(move || { let _ = sender.send_blocking(work()); });
            let (store, refresh) = (store.clone(), refresh.clone());
            glib::spawn_future_local(async move {
                if let Ok(result) = receiver.recv().await {
                    match result { Ok(message) => store.toast(&message), Err(error) => store.error(&error) }
                    refresh();
                }
            });
        })
    };
    { let act = act.clone(); start.connect_clicked(move |_| act(setup::start)); }
    { let act = act.clone(); stop.connect_clicked(move |_| act(setup::stop)); }
    { let act = act.clone(); remove.connect_clicked(move |_| act(setup::remove)); }
    let terminal = |store: Store, arguments: &'static [&'static str]| move |_: &gtk::Button| {
        if let Err(error) = setup::run_in_terminal(arguments) { store.error(&error); }
    };
    install.connect_clicked(terminal(store.clone(), &["service", "setup"]));
    install_missing.connect_clicked(terminal(store.clone(), &["requirements", "--install", "--wait"]));

    // How it starts on this system, and every other way, for systems and window managers that Install
    // does not cover.
    let systemd = setup::systemd_user_available();
    let route = widgets::text(if systemd {
        "This system has a systemd user manager, so Install sets it up as a systemd user service: it starts with your session and systemd restarts it if it fails."
    } else {
        "This system has no systemd user manager (for example runit, OpenRC, s6 or dinit), so Install adds a desktop autostart entry and starts the service now. Desktop environments (GNOME, KDE, Xfce, Cinnamon, MATE, LXQt) read that entry when you log in. A window manager on its own usually does not: add the line for yours below."
    }, &["small", "dim"]);
    route.set_wrap(true);
    route.set_xalign(0.0);
    body.append(&route);
    body.append(&other_ways_to_start(store, systemd));
    card
}

/// The line to add for each window manager and init system, with where it goes and a Copy button.
fn other_ways_to_start(store: &Store, systemd: bool) -> gtk::Expander {
    let expander = gtk::Expander::new(Some("Other ways to start it"));
    expander.add_css_class("other-ways");
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inner.set_margin_top(8);
    let names: Vec<&str> = crate::service::TARGETS.iter().map(|(name, _)| *name).collect();
    let list = gtk::StringList::new(&names);
    let drop = gtk::DropDown::new(Some(list), gtk::Expression::NONE);
    widgets::label_accessible(&drop, "Window manager or init system");
    // Start on what this session looks like, so the right line is usually already showing.
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let guess = ["hyprland", "niri", "sway", "i3", "river", "labwc", "openbox", "bspwm", "mango"].into_iter().find(|n| desktop.contains(n))
        .unwrap_or(if systemd { "systemd" } else { "xdg" });
    drop.set_selected(names.iter().position(|n| *n == guess).unwrap_or(0) as u32);
    drop.set_halign(gtk::Align::Start);
    let what = widgets::text("", &["setting-summary"]);
    let place = widgets::text("", &["small", "dim"]);
    place.set_selectable(true);
    let code = gtk::Label::new(None);
    code.add_css_class("code");
    code.set_selectable(true);
    code.set_wrap(true);
    code.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    code.set_xalign(0.0);
    let copy = gtk::Button::with_label("Copy");
    copy.add_css_class("small");
    copy.set_halign(gtk::Align::Start);
    let show = {
        let (place, code, names, what) = (place.clone(), code.clone(), names.clone(), what.clone());
        move |index: u32| {
            let Some(name) = names.get(index as usize) else { return };
            if let Some((_, description)) = crate::service::TARGETS.get(index as usize) { what.set_text(description); }
            if let Some((text, where_)) = crate::service::snippet(name, &crate::service::program_path()) {
                code.set_text(text.trim_end());
                place.set_text(&format!("Put this in {where_}. This app never edits these files for you."));
            }
        }
    };
    show(drop.selected());
    drop.connect_selected_notify(move |d| show(d.selected()));
    {
        let (code, store) = (code.clone(), store.clone());
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&code.text());
            store.toast("Copied.");
        });
    }
    inner.append(&drop);
    inner.append(&what);
    inner.append(&place);
    inner.append(&code);
    inner.append(&copy);
    expander.set_child(Some(&inner));
    expander
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

    // Wraps onto a second line in a narrow panel instead of holding the window wide.
    let swatches = gtk::FlowBox::new();
    swatches.set_selection_mode(gtk::SelectionMode::None);
    swatches.set_column_spacing(10);
    swatches.set_row_spacing(10);
    swatches.set_max_children_per_line(6);
    swatches.set_min_children_per_line(1);
    swatches.set_homogeneous(true);
    swatches.set_halign(gtk::Align::Start);
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
        swatches.insert(&button, -1);
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

    // With Mirror image on the picture is flipped, so the stick turns the other way to match it.
    let mirror = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&widgets::text("Flip the joystick with the mirror", &["setting-label"]));
    titles.append(&widgets::text("While Mirror image is on, pushing the stick right moves the picture you see to the right.", &["setting-summary"]));
    let follow = gtk::Switch::new();
    follow.set_valign(gtk::Align::Center);
    follow.set_active(current_config().joystick_follows_mirror);
    widgets::label_accessible(&follow, "Flip the joystick with the mirror");
    follow.connect_active_notify(|sw| super::joystick::set_follow_mirror(sw.is_active()));
    mirror.append(&titles);
    mirror.append(&follow);
    body.append(&mirror);

    body.append(&plain_switch_row("Start the preview when the app opens",
        "Off, the preview waits for its play button, so opening the app does not turn the camera on.",
        current_config().preview_on_start,
        |on| update_config(|c| c.preview_on_start = on)));
    card
}

/// A switch row with a title and a line under it.
fn plain_switch_row(title: &str, summary: &str, active: bool, changed: impl Fn(bool) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&widgets::text(title, &["setting-label"]));
    let line = widgets::text(summary, &["setting-summary"]);
    line.set_wrap(true);
    line.set_xalign(0.0);
    titles.append(&line);
    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    switch.set_active(active);
    widgets::label_accessible(&switch, title);
    switch.connect_active_notify(move |sw| changed(sw.is_active()));
    row.append(&titles);
    row.append(&switch);
    row
}

/// One axis of the gimbal limits: what it is called, its ends, and where its settings live.
struct AxisLimits {
    name: &'static str,
    low_name: &'static str,
    high_name: &'static str,
    summary: &'static str,
    published: (f64, f64),
    mechanical: f64,
    read: fn(&config::AppConfig) -> (bool, f64, f64),
    write_on: fn(&mut config::AppConfig, bool),
    write_range: fn(&mut config::AppConfig, f64, f64),
    /// Pan or tilt out of a live position (pan, tilt).
    live: fn((f64, f64)) -> f64,
}

const AXES: [AxisLimits; 2] = [
    AxisLimits {
        name: "Pan", low_name: "Left", high_name: "Right",
        summary: "OBSBOT's range is 140° each way. Off, it can turn to the ends of the mechanism, 150°, where the camera stops itself.",
        published: (-140.0, 140.0), mechanical: 150.0,
        read: |c| (c.pan_limit, c.pan_limit_low, c.pan_limit_high),
        write_on: |c, on| c.pan_limit = on,
        write_range: |c, low, high| { c.pan_limit_low = low; c.pan_limit_high = high; },
        live: |(pan, _)| pan,
    },
    AxisLimits {
        name: "Tilt", low_name: "Down", high_name: "Up",
        summary: "OBSBOT's range is 30° down to 70° up. Off, it can tilt to the ends of the mechanism, 90°, where the camera stops itself.",
        published: (-30.0, 70.0), mechanical: 90.0,
        read: |c| (c.tilt_limit, c.tilt_limit_low, c.tilt_limit_high),
        write_on: |c, on| c.tilt_limit = on,
        write_range: |c, low, high| { c.tilt_limit_low = low; c.tilt_limit_high = high; },
        live: |(_, tilt)| tilt,
    },
];

/// Limits on how far the stick, the 1° steps and presets may turn the camera: per axis, on or off, with
/// its two ends typed in or taken from where the camera points now.
pub fn gimbal_limits_card(store: &Store) -> gtk::Box {
    let (card, body) = card("Gimbal limits", "open_with",
        "How far the stick, the 1° steps and presets may turn the camera, in degrees. The stick slows near an end and stops just short of it.");
    // Where the camera points now, for the buttons that take it.
    let position: Rc<Cell<Option<(f64, f64)>>> = Rc::default();
    {
        let position = Rc::downgrade(&position);
        store.subscribe_gimbal(move |live| match position.upgrade() { Some(cell) => { cell.set(live); true } None => false });
    }
    for (index, axis) in AXES.iter().enumerate() {
        if index > 0 {
            let divider = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            divider.add_css_class("divider");
            body.append(&divider);
        }
        body.append(&axis_limits(store, axis, &position));
    }
    card
}

fn axis_limits(store: &Store, axis: &'static AxisLimits, position: &Rc<Cell<Option<(f64, f64)>>>) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    section.add_css_class("limit-axis");
    let (on, low_now, high_now) = (axis.read)(&current_config());
    let note = widgets::text("", &["small", "warning-text"]);
    note.set_visible(false);

    // The two ends side by side, or one above the other in a narrow panel; the reset button under them.
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 8);
    fields.add_css_class("limit-fields");
    let ends = gtk::FlowBox::new();
    ends.set_selection_mode(gtk::SelectionMode::None);
    ends.set_homogeneous(true);
    ends.set_max_children_per_line(2);
    ends.set_min_children_per_line(1);
    ends.set_column_spacing(10);
    ends.set_row_spacing(8);
    let field = |caption: &str, value: f64, low: f64, high: f64| {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.set_hexpand(true);
        column.append(&widgets::text(caption, &["small", "dim"]));
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.add_css_class("linked");
        line.add_css_class("limit-entry");
        let spin = gtk::SpinButton::with_range(low, high, 1.0);
        spin.set_digits(0);
        spin.set_numeric(true);
        spin.set_value(value);
        spin.set_hexpand(true);
        spin.set_width_chars(4);
        widgets::lock_wheel(&spin);
        widgets::label_accessible(&spin, &format!("{} {} limit, degrees", axis.name, caption.to_lowercase()));
        let take = widgets::icon_button("center_focus_strong", &format!("Use where the camera points now as the {} limit", caption.to_lowercase()), true);
        take.add_css_class("take-current");
        line.append(&spin);
        line.append(&take);
        column.append(&line);
        (column, spin, take)
    };
    let (low_box, low, take_low) = field(axis.low_name, low_now, -axis.mechanical, 0.0);
    let (high_box, high, take_high) = field(axis.high_name, high_now, 0.0, axis.mechanical);
    let reset = gtk::Button::with_label("OBSBOT range");
    reset.add_css_class("small");
    reset.set_halign(gtk::Align::Start);
    reset.set_tooltip_text(Some("Put both ends back to OBSBOT's published range"));
    ends.insert(&low_box, -1);
    ends.insert(&high_box, -1);
    fields.append(&ends);
    fields.append(&reset);

    let save = {
        let (low, high, note) = (low.clone(), high.clone(), note.clone());
        move || {
            let (l, h) = (low.value(), high.value());
            if crate::backend::angles::valid_limits(l, h, axis.mechanical) {
                note.set_visible(false);
                update_config(|c| (axis.write_range)(c, l, h));
            } else {
                note.set_text("Leave at least 10° between the two ends.");
                note.set_visible(true);
            }
        }
    };
    { let save = save.clone(); low.connect_value_changed(move |_| save()); }
    { let save = save.clone(); high.connect_value_changed(move |_| save()); }
    {
        let (low, high) = (low.clone(), high.clone());
        reset.connect_clicked(move |_| { low.set_value(axis.published.0); high.set_value(axis.published.1); });
    }
    // Take where the camera points now; an end on the wrong side of centre cannot be a limit.
    for (button, spin, low_end) in [(&take_low, &low, true), (&take_high, &high, false)] {
        let (spin, position, store) = (spin.clone(), position.clone(), store.clone());
        button.connect_clicked(move |_| {
            let Some(now) = position.get().map(axis.live) else {
                store.error("The camera has not said where it points yet.");
                return;
            };
            let now = now.round();
            if (low_end && now > 0.0) || (!low_end && now < 0.0) {
                store.error(&format!("The camera points {} now, which is the other side of centre for this end.", crate::backend::angles::format(now)));
                return;
            }
            spin.set_value(now);
        });
    }

    fields.set_sensitive(on);
    let fields_weak = fields.downgrade();
    section.append(&plain_switch_row(&format!("Limit {}", axis.name.to_lowercase()), axis.summary, on, move |on| {
        update_config(|c| (axis.write_on)(c, on));
        if let Some(fields) = fields_weak.upgrade() { fields.set_sensitive(on); }
    }));
    section.append(&fields);
    section.append(&note);
    section
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
    let version = widgets::text(&format!("Version {}", crate::cli::build_label()), &["small", "dim"]);
    version.set_selectable(true);
    body.append(&version);
    card
}

#[allow(dead_code)]
fn unused(_: &glib::SourceId) {}
