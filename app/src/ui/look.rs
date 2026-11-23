//! The Look bubble beside the search button: change the Studio camera's LUT and its strength from any tab,
//! without the preview. Favourites come first, then the other LUTs.

use super::store::Store;
use super::widgets;
use crate::backend::{current_config, update_config};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// The round button, shown while the Studio camera is on and has LUTs; lit while a LUT is applied.
pub fn bubble(store: &Store) -> gtk::Button {
    let button = widgets::icon_text_button("photo_filter", "LUTs");
    button.add_css_class("look-bubble");
    button.set_valign(gtk::Align::Center);
    let popover = gtk::Popover::new();
    popover.add_css_class("quick-chooser");
    popover.set_has_arrow(false);
    popover.set_parent(&button);
    {
        let popover = popover.clone();
        button.connect_destroy(move |_| popover.unparent());
    }
    {
        let (popover, store) = (popover.clone(), store.clone());
        button.connect_clicked(move |_| {
            popover.set_child(Some(&contents(&popover, &store)));
            popover.popup();
        });
    }
    // Follows the LUT however it is changed (Device tab, the preview, a preset, another window).
    let show = {
        let weak = button.downgrade();
        let seen: Rc<RefCell<Option<(bool, Option<String>, bool)>>> = Rc::default();
        move || {
            let Some(button) = weak.upgrade() else { return false };
            let studio = current_config().studio;
            let now = (studio.enabled && !crate::lut::list().is_empty(), studio.lut.clone(), studio.lut_on);
            if seen.borrow().as_ref() != Some(&now) {
                button.set_visible(now.0);
                let words = match (&now.1, now.2) {
                    (Some(name), true) => format!("Look: {} on the Studio camera", crate::lut::label(name)),
                    _ => "Look: no LUT on the Studio camera".to_string(),
                };
                button.set_tooltip_text(Some(&words));
                widgets::label_accessible(&button, &words);
                if now.2 && now.1.is_some() { button.add_css_class("lit"); } else { button.remove_css_class("lit"); }
                *seen.borrow_mut() = Some(now);
            }
            true
        }
    };
    show();
    glib::timeout_add_local(Duration::from_secs(1), move || if show() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break });
    button
}

/// What the bubble opens: No LUT, the favourites, the other LUTs, and the strength.
fn contents(popover: &gtk::Popover, store: &Store) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.set_margin_top(6); list.set_margin_bottom(6); list.set_margin_start(6); list.set_margin_end(6);
    list.set_size_request(220, -1);
    list.append(&widgets::line("Look", &["setting-label"]));
    list.append(&widgets::line("The Studio camera's LUT, for every app using it", &["small", "dim"]));
    let studio = current_config().studio;
    let on = studio.lut.clone().filter(|_| studio.lut_on);
    let favourites = crate::lut::favourites();
    let others: Vec<String> = crate::lut::list().into_iter().filter(|n| !favourites.contains(n)).collect();
    let choice = |label: String, pick: Option<String>| {
        let item = gtk::Button::with_label(&label);
        item.add_css_class("flat-item");
        if pick == on { item.add_css_class("chosen"); }
        if let Some(text) = item.child().and_then(|c| c.downcast::<gtk::Label>().ok()) { text.set_xalign(0.0); }
        let (popover, store) = (popover.clone(), store.clone());
        item.connect_clicked(move |_| {
            match &pick {
                None => { update_config(|c| c.studio.lut_on = false); store.toast("No LUT"); }
                Some(name) => {
                    update_config(|c| c.studio.pick_lut(&name));
                    store.toast(&format!("Look: {}", crate::lut::label(name)));
                }
            }
            popover.popdown();
        });
        item
    };
    list.append(&choice("No LUT".into(), None));
    if !favourites.is_empty() {
        list.append(&widgets::line("Favourites", &["small", "dim", "chooser-head"]));
        for name in &favourites { list.append(&choice(format!("\u{2605} {}", crate::lut::label(name)), Some(name.clone()))); }
    }
    if !others.is_empty() {
        list.append(&widgets::line(if favourites.is_empty() { "LUTs" } else { "Others" }, &["small", "dim", "chooser-head"]));
        for name in &others { list.append(&choice(crate::lut::label(name), Some(name.clone()))); }
    }
    // The strength, saved once the slider settles (each strength is a new table, so the picture restarts).
    list.append(&widgets::line("Strength", &["small", "dim", "chooser-head"]));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let strength = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    strength.set_draw_value(false);
    strength.set_hexpand(true);
    strength.set_value(f64::from(studio.lut_strength));
    strength.set_sensitive(on.is_some());
    widgets::lock_wheel(&strength);
    widgets::label_accessible(&strength, "LUT strength");
    let amount = widgets::line(&format!("{}%", studio.lut_strength), &["setting-value", "tnum"]);
    amount.set_width_chars(4);
    {
        let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let amount = amount.clone();
        strength.connect_value_changed(move |s| {
            let value = s.value().round() as u8;
            amount.set_text(&format!("{value}%"));
            if let Some(id) = timer.borrow_mut().take() { id.remove(); }
            let slot = timer.clone();
            *timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(600), move || {
                slot.borrow_mut().take();
                update_config(|c| c.studio.set_lut_strength(value));
            }));
        });
    }
    row.append(&strength);
    row.append(&amount);
    list.append(&row);
    list
}
