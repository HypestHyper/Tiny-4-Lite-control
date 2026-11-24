//! Settings pages generated from the registry.

use super::store::Store;
use super::widgets::{self, Density, SettingRow};
use crate::backend::registry::{self, Group, Setting, Tab};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Rows must outlive their widgets' signal handlers, so whoever builds a page keeps them here.
pub type Rows = Rc<RefCell<Vec<SettingRow>>>;

/// Cards flowing into one or two columns, shortest column first.
#[derive(Clone)]
pub struct Masonry {
    root: gtk::Box,
    columns: Rc<RefCell<Vec<gtk::Box>>>,
    cards: Rc<RefCell<Vec<(gtk::Widget, usize)>>>,
    count: Rc<Cell<usize>>,
}

impl Masonry {
    pub fn new() -> Masonry {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.set_homogeneous(true);
        let masonry = Masonry { root, columns: Rc::default(), cards: Rc::default(), count: Rc::new(Cell::new(0)) };
        masonry.set_columns(1);
        masonry
    }

    pub fn widget(&self) -> &gtk::Box { &self.root }

    /// `weight` is roughly how tall the card is, used to balance the columns.
    pub fn add(&self, card: &impl IsA<gtk::Widget>, weight: usize) {
        self.cards.borrow_mut().push((card.clone().upcast(), weight));
        self.layout();
    }

    pub fn set_columns(&self, count: usize) {
        let count = count.max(1);
        if self.count.get() == count && !self.columns.borrow().is_empty() { return; }
        self.count.set(count);
        for (card, _) in self.cards.borrow().iter() {
            if let Some(parent) = card.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) { parent.remove(card); }
        }
        while let Some(child) = self.root.first_child() { self.root.remove(&child); }
        self.columns.borrow_mut().clear();
        for _ in 0..count {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
            column.set_valign(gtk::Align::Start);
            self.root.append(&column);
            self.columns.borrow_mut().push(column);
        }
        self.layout();
    }

    fn layout(&self) {
        let columns = self.columns.borrow();
        let mut heights = vec![0usize; columns.len()];
        for (card, weight) in self.cards.borrow().iter() {
            if let Some(parent) = card.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) { parent.remove(card); }
            let target = heights.iter().enumerate().min_by_key(|(_, h)| **h).map(|(i, _)| i).unwrap_or(0);
            columns[target].append(card);
            heights[target] += *weight;
        }
    }
}

/// A card holding the rows of one group.
pub fn group_card(store: &Store, group: Group, settings: &[&'static Setting], density: Density, keep: &Rows, foldable: bool) -> (gtk::Box, usize) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    card.set_accessible_role(gtk::AccessibleRole::Group);
    widgets::label_accessible(&card, group.title());

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    head.add_css_class("group-head");
    head.set_margin_bottom(4);
    head.append(&widgets::icon(group.icon()));
    head.append(&widgets::text(group.title(), &["group-title"]));
    // Settings on another card that change how this one works, a click away.
    if foldable && group == Group::Tracking { head.append(&widgets::link_button("settings", "Tracking area settings", "Tracking area")); }
    card.append(&head);
    // In a tab the card can be folded away; in search results it always shows.
    let content = if foldable { widgets::collapsible(&card, &head, group.title()) } else { card.clone() };

    let mut first = true;
    for setting in settings {
        if !first {
            let divider = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            divider.add_css_class("divider");
            content.append(&divider);
        }
        first = false;
        // Camera power is two plain buttons, as on the Camera controls card, not a switch.
        if setting.id == "obsbot:sleep" {
            content.append(&super::joystick::power_buttons(store, None));
            continue;
        }
        let row = SettingRow::new(store, setting, density);
        // A gesture shows a picture of the hand pose beside it.
        if let Some(icon) = super::gesture_draw::for_setting(setting.id) {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let picture = super::gesture_draw::picture(icon, if density == Density::Slim { 36 } else { 40 });
            picture.set_valign(gtk::Align::Start);
            picture.set_margin_top(6);
            row.widget().set_hexpand(true);
            line.append(&picture);
            line.append(row.widget());
            content.append(&line);
            keep.borrow_mut().push(row);
            continue;
        }
        content.append(row.widget());
        keep.borrow_mut().push(row);
        if setting.id == "obsbot:mic_disabled" { content.append(&widgets::mic_meter(store)); }
    }
    (card, settings.len() * 2 + 2)
}

/// Settings shown on another card instead of their group's (the camera's own sleep timer is on Sleep timer).
const SHOWN_ELSEWHERE: &[&str] = &["obsbot:camera_sleep"];

fn settings_in(group: Group) -> Vec<&'static Setting> {
    registry::all().iter().filter(|s| s.group == group && !SHOWN_ELSEWHERE.contains(&s.id)).collect()
}

/// Scrollable page for one tab, with cards for each of its groups.
pub struct TabPage {
    pub root: gtk::ScrolledWindow,
    pub masonry: Masonry,
}

pub fn tab_page(store: &Store, tab: Tab, keep: &Rows) -> TabPage {
    let masonry = Masonry::new();
    // Zoom and field of view live in the Camera controls card beside the preview, not in a tab.
    // Camera power first on the Device tab, where it is easy to find.
    let mut groups: Vec<Group> = Group::ALL.iter().copied().filter(|g| g.tab() == tab && *g != Group::Zoom).collect();
    groups.sort_by_key(|g| *g != Group::Power);
    for group in groups {
        let settings = settings_in(group);
        if settings.is_empty() { continue; }
        let (card, weight) = group_card(store, group, &settings, Density::Full, keep, true);
        masonry.add(&card, weight);
    }
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 12);
    holder.set_margin_end(6);
    holder.set_margin_bottom(20);
    holder.append(masonry.widget());
    let root = widgets::scroller();
    root.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    root.set_vexpand(true);
    root.set_child(Some(&holder));
    TabPage { root, masonry }
}

/// Rows matching a search, grouped by their group.
pub fn search_results(store: &Store, query: &str, keep: &Rows) -> gtk::Box {
    let query = query.trim().to_lowercase();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 16);
    column.set_margin_bottom(20);
    column.set_margin_end(6);
    let matches = |s: &Setting| {
        s.label.to_lowercase().contains(&query) || s.summary.to_lowercase().contains(&query)
            || s.group.title().to_lowercase().contains(&query)
    };
    let mut any = false;
    for group in Group::ALL {
        let found: Vec<&'static Setting> = registry::all().iter().filter(|s| s.group == group && matches(s)).collect();
        if found.is_empty() { continue; }
        any = true;
        let (card, _) = group_card(store, group, &found, Density::Full, keep, false);
        column.append(&card);
    }
    if !any {
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
        empty.set_halign(gtk::Align::Center);
        empty.set_margin_top(48);
        empty.append(&widgets::icon("search_off"));
        if let Some(glyph) = empty.first_child() { glyph.add_css_class("big"); }
        empty.append(&widgets::text(&format!("No setting matches “{}”.", query), &["body", "dim"]));
        empty.append(&widgets::text("Try a shorter word, such as “focus” or “mic”.", &["small", "faint"]));
        column.append(&empty);
    }
    column
}

/// Settings not in the registry but reported by the driver, as one extra card.
pub fn discovered_card(store: &Store, keep: &Rows) -> Option<gtk::Box> {
    let snapshot = store.snapshot();
    let extra = registry::discovered(&snapshot);
    if extra.is_empty() { return None; }
    let leaked: Vec<&'static Setting> = extra.into_iter().map(|s| &*Box::leak(Box::new(s))).collect();
    Some(group_card(store, Group::Other, &leaked, Density::Full, keep, true).0)
}
