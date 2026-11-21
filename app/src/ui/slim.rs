//! The slim window: a narrow panel with just the controls you choose.

use super::app::Ctx;
use super::full::{connection_banner, StatusChip};
use super::joystick::Joystick;
use super::preview::PreviewView;
use super::settings::Rows;
use super::store::{Dialogs, Toaster};
use super::widgets::{self, Density, SettingRow};
use super::windowing;
use crate::backend::registry::{self, Group};
use crate::backend::{current_config, update_config};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub const ITEM_PREVIEW: &str = "camera:preview";
pub const ITEM_JOYSTICK: &str = "camera:joystick";

pub struct SlimWindow {
    pub window: gtk::ApplicationWindow,
    pub joystick: Joystick,
    _state: Rc<SlimState>,
}

struct SlimState {
    ctx: Rc<Ctx>,
    list: gtk::Box,
    rows: RefCell<Rows>,
    preview: PreviewView,
    preview_wrap: gtk::Revealer,
    joystick_card: gtk::Box,
    empty: gtk::Box,
    stack: gtk::Stack,
    customise_rows: RefCell<Vec<gtk::Switch>>,
}

fn selected() -> Vec<String> { current_config().compact_controls }

fn is_selected(id: &str) -> bool { selected().iter().any(|s| s == id) }

fn set_selected(id: &str, on: bool) {
    update_config(|c| {
        c.compact_controls.retain(|s| s != id);
        if on { c.compact_controls.push(id.to_string()); }
    });
}

impl SlimWindow {
    pub fn new(ctx: &Rc<Ctx>) -> SlimWindow {
        let store = ctx.store.clone();
        let window = gtk::ApplicationWindow::builder()
            .application(&ctx.app)
            .title(windowing::TITLE_SLIM)
            .default_width(380)
            .default_height(620)
            .build();
        windowing::set_class(&window, windowing::CLASS_SLIM);
        window.set_size_request(320, 260);

        // Header
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("header");
        header.set_size_request(-1, 56);
        let title = widgets::line("Quick controls", &["heading"]);
        title.set_valign(gtk::Align::Center);
        title.set_hexpand(true);
        let chip = StatusChip::new();
        chip.root.set_valign(gtk::Align::Center);
        chip.set_compact(true);
        let customise = widgets::icon_button("tune", "Choose what this window shows", true);
        let full = widgets::icon_button("open_in_full", "Switch to the full window", true);
        let controls = gtk::WindowControls::new(gtk::PackType::End);
        controls.set_visible(ctx.style.window_buttons());
        { let (c, s) = (controls.clone(), ctx.style.clone()); ctx.style.on_change(move || c.set_visible(s.window_buttons())); }
        header.append(&title);
        header.append(&chip.root);
        header.append(&customise);
        header.append(&full);
        header.append(&controls);
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&header));
        window.set_titlebar(Some(&handle));

        // Content
        let preview = PreviewView::new(&ctx.hub);
        let pause_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let pop = widgets::icon_button("open_in_new", "Open the preview in its own window", true);
        pop.add_css_class("on-stage");
        let weak_ctx = Rc::downgrade(ctx);
        pop.connect_clicked(move |_| { if let Some(ctx) = weak_ctx.upgrade() { ctx.set_popped_out(true); } });
        pause_row.append(&pop);
        preview.add_control(&pause_row);
        let preview_wrap = gtk::Revealer::new();
        preview_wrap.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        preview_wrap.set_child(Some(preview.widget()));

        let joystick = Joystick::new(&store);
        let joystick_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        joystick_card.add_css_class("card");
        joystick_card.add_css_class("tight");
        joystick_card.append(&joystick.widget_for(super::widgets::Density::Slim));
        joystick_card.set_size_request(-1, -1);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("card");
        list.add_css_class("tight");

        let empty = gtk::Box::new(gtk::Orientation::Vertical, 10);
        empty.set_halign(gtk::Align::Center);
        empty.set_margin_top(24);
        let glyph = widgets::icon("tune");
        glyph.add_css_class("big");
        empty.append(&glyph);
        empty.append(&widgets::text("Nothing to show yet.", &["body"]));
        empty.append(&widgets::text("Choose the controls you use most.", &["small", "dim"]));
        let choose = gtk::Button::with_label("Choose controls");
        choose.add_css_class("filled");
        empty.append(&choose);

        let banner = connection_banner(ctx);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.add_css_class("slim-root");
        content.append(&banner);
        content.append(&preview_wrap);
        content.append(&joystick_card);
        content.append(&list);
        content.append(&empty);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&content));

        // Customise sheet
        let sheet = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(220);
        stack.add_named(&scroll, Some("view"));
        stack.add_named(&sheet, Some("customise"));

        let state = Rc::new(SlimState {
            ctx: ctx.clone(), list, rows: RefCell::default(), preview, preview_wrap, joystick_card, empty,
            stack: stack.clone(), customise_rows: RefCell::default(),
        });
        build_sheet(&state, &sheet);
        rebuild(&state);

        let dialogs = Dialogs::new();
        dialogs.overlay().set_child(Some(&stack));
        dialogs.install();
        let toaster = Toaster::new();
        dialogs.overlay().add_overlay(toaster.widget());
        store.add_dialogs(dialogs.clone());
        store.add_toaster(toaster);
        window.set_child(Some(dialogs.overlay()));

        let s = state.clone();
        customise.connect_clicked(move |_| show_sheet(&s, true));
        let s = state.clone();
        choose.connect_clicked(move |_| show_sheet(&s, true));
        let weak_ctx = Rc::downgrade(ctx);
        full.connect_clicked(move |_| { if let Some(ctx) = weak_ctx.upgrade() { ctx.show_full(); } });

        let (chip_store, chip2) = (store.clone(), chip.clone());
        store.subscribe(move |snapshot| { chip2.update(chip_store.connection(), snapshot); true });
        chip.update(store.connection(), &store.snapshot());

        let refresh_store = store.clone();
        window.connect_is_active_notify(move |w| { if w.is_active() { refresh_store.refresh_if_stale(); } });

        // Narrow window: keep the geometry compact.
        let sensor = gtk::DrawingArea::new();
        sensor.set_content_height(0);
        sensor.set_hexpand(true);
        sensor.set_valign(gtk::Align::Start);
        sensor.set_can_target(false);
        dialogs.overlay().add_overlay(&sensor);
        let weak_ctx = Rc::downgrade(ctx);
        let preview_state = state.clone();
        sensor.connect_resize(move |_, width, _| {
            if let Some(ctx) = weak_ctx.upgrade() { ctx.style.report_width(width); }
            // Keep the preview 16:9 as the window is resized (the slim root has 14 px margins).
            let height = (f64::from(width - 28) * 9.0 / 16.0).round().clamp(110.0, 420.0) as i32;
            preview_state.preview.widget().set_size_request(-1, height);
        });

        let keys = gtk::EventControllerKey::new();
        let s = state.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape && s.stack.visible_child_name().as_deref() == Some("customise") {
                show_sheet(&s, false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);

        let (weak_ctx, js) = (Rc::downgrade(ctx), joystick.clone());
        window.connect_close_request(move |_| {
            js.stop_now_blocking();
            if let Some(ctx) = weak_ctx.upgrade() { ctx.quit(); }
            glib::Propagation::Proceed
        });
        if crate::backend::ui_test_mode() {
            if let (Ok(w), Ok(h)) = (std::env::var("TINY4LITE_UI_TEST_WIDTH"), std::env::var("TINY4LITE_UI_TEST_HEIGHT")) {
                if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) { window.set_default_size(w, h); }
            }
            if std::env::var("TINY4LITE_UI_TEST_PAGE").is_ok_and(|p| p == "customise") { show_sheet(&state, true); }
        }
        SlimWindow { window, joystick, _state: state }
    }
}

fn show_sheet(state: &Rc<SlimState>, show: bool) {
    if show {
        for (switch, id) in state.customise_rows.borrow().iter().zip(all_item_ids()) { switch.set_active(is_selected(&id)); }
    }
    state.stack.set_visible_child_name(if show { "customise" } else { "view" });
}

fn all_item_ids() -> Vec<String> {
    // Same order as the sheet is built in: extras first, then group by group.
    let mut ids = vec![ITEM_PREVIEW.to_string(), ITEM_JOYSTICK.to_string()];
    for group in Group::ALL {
        ids.extend(registry::all().iter().filter(|s| s.quick && s.group == group).map(|s| s.id.to_string()));
    }
    ids
}

/// Rebuild the visible list from the saved choice.
fn rebuild(state: &Rc<SlimState>) {
    let list = &state.list;
    while let Some(child) = list.first_child() { list.remove(&child); }
    let keep: Rows = Rc::default();
    let chosen = selected();
    let mut count = 0;
    for setting in registry::all().iter().filter(|s| s.quick && chosen.iter().any(|id| id == s.id)) {
        if count > 0 {
            let divider = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            divider.add_css_class("divider");
            list.append(&divider);
        }
        let row = SettingRow::new(&state.ctx.store, setting, Density::Slim);
        list.append(row.widget());
        keep.borrow_mut().push(row);
        count += 1;
    }
    *state.rows.borrow_mut() = keep;
    state.list.set_visible(count > 0);
    state.preview_wrap.set_reveal_child(is_selected(ITEM_PREVIEW));
    state.joystick_card.set_visible(is_selected(ITEM_JOYSTICK));
    let nothing = count == 0 && !is_selected(ITEM_PREVIEW) && !is_selected(ITEM_JOYSTICK);
    state.empty.set_visible(nothing);
    if !is_selected(ITEM_JOYSTICK) { /* the pad releases itself when hidden */ }
    let _ = &state.preview;
}

fn build_sheet(state: &Rc<SlimState>, sheet: &gtk::Box) {
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.set_margin_start(16);
    head.set_margin_end(16);
    head.set_margin_top(4);
    head.set_margin_bottom(8);
    let title = widgets::text("Choose what to show", &["title"]);
    title.set_hexpand(true);
    let done = gtk::Button::with_label("Done");
    done.add_css_class("filled");
    head.append(&title);
    head.append(&done);
    sheet.append(&head);
    let hint = widgets::text("Turn on the controls you want in this window. Changes apply straight away.", &["small", "dim"]);
    hint.set_margin_start(16);
    hint.set_margin_end(16);
    hint.set_margin_bottom(8);
    sheet.append(&hint);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_margin_start(14);
    column.set_margin_end(14);
    column.set_margin_bottom(16);

    let mut switches: Vec<gtk::Switch> = Vec::new();
    let mut add_item = |parent: &gtk::Box, id: &str, label: &str, summary: &str, first: bool| {
        if !first {
            let divider = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            divider.add_css_class("divider");
            parent.append(&divider);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("setting-row");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);
        titles.append(&widgets::text(label, &["setting-label"]));
        if !summary.is_empty() { titles.append(&widgets::text(summary, &["setting-summary"])); }
        let switch = gtk::Switch::new();
        switch.set_valign(gtk::Align::Center);
        widgets::label_accessible(&switch, &format!("Show {label} in the slim window"));
        let (state2, id2) = (state.clone(), id.to_string());
        switch.connect_active_notify(move |sw| {
            if is_selected(&id2) == sw.is_active() { return; }
            set_selected(&id2, sw.is_active());
            rebuild(&state2);
        });
        row.append(&titles);
        row.append(&switch);
        parent.append(&row);
        switches.push(switch);
    };

    let extras = gtk::Box::new(gtk::Orientation::Vertical, 0);
    extras.add_css_class("card");
    extras.add_css_class("tight");
    add_item(&extras, ITEM_PREVIEW, "Live preview", "A small picture from the camera.", true);
    add_item(&extras, ITEM_JOYSTICK, "Gimbal joystick", "Turn the camera by hand.", false);
    column.append(&extras);

    for group in Group::ALL {
        let members: Vec<_> = registry::all().iter().filter(|s| s.quick && s.group == group).collect();
        if members.is_empty() { continue; }
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("card");
        card.add_css_class("tight");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&widgets::icon(group.icon()));
        head.append(&widgets::text(group.title(), &["heading"]));
        card.append(&head);
        for setting in members {
            let mut summary = setting.summary.to_string();
            if !setting.deps.is_empty() {
                let names: Vec<&str> = setting.deps.iter().filter_map(|d| registry::find(d.on)).map(|s| s.label).collect();
                if !names.is_empty() { summary = format!("{summary} Works together with {}.", names.join(" and ")); }
            }
            add_item(&card, setting.id, setting.label, &summary, false);
        }
        column.append(&card);
    }
    *state.customise_rows.borrow_mut() = switches;
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&column));
    sheet.append(&scroll);
    let s = state.clone();
    done.connect_clicked(move |_| show_sheet(&s, false));
}
