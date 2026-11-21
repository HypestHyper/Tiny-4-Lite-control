//! The full window: stage (preview and gimbal) beside the settings tabs.

use super::app::Ctx;
use super::joystick::Joystick;
use super::preview::PreviewView;
use super::settings::{self, Rows};
use super::store::{Connection, Dialogs, Toaster};
use super::{panels, widgets, windowing};
use crate::backend::registry::Tab;
use crate::backend::{current_config, update_config};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page { Tab(Tab), Presets }

/// Icons used by the tab bar, for the icon coverage test.
#[cfg(test)]
pub(crate) fn page_icons() -> Vec<&'static str> { PAGES.iter().map(|p| p.2).collect() }

const PAGES: [(Page, &str, &str); 3] = [
    (Page::Tab(Tab::Camera), "Camera", "photo_camera"),
    (Page::Presets, "Presets", "bookmarks"),
    (Page::Tab(Tab::Device), "Device", "tune"),
];

fn page_name(page: Page) -> &'static str {
    match page {
        Page::Tab(Tab::Camera) => "camera", Page::Tab(Tab::Device) => "device", Page::Presets => "presets",
    }
}

pub struct FullWindow {
    pub window: gtk::ApplicationWindow,
    pub joystick: Joystick,
    _keep: Rc<FullState>,
}

struct FullState {
    /// Keeps the rows alive: their signal handlers hold only weak references.
    _rows: Rows,
    search_rows: RefCell<Rows>,
    stack: gtk::Stack,
    tabs: RefCell<Vec<(Page, gtk::ToggleButton)>>,
    last_page: Cell<usize>,
    search: gtk::SearchEntry,
    search_timer: RefCell<Option<glib::SourceId>>,
    popped: gtk::Stack,
    /// The preview view must outlive the constructor, or the hub loses it.
    _view: PreviewView,
}

impl FullWindow {
    pub fn new(ctx: &Rc<Ctx>) -> FullWindow {
        let store = ctx.store.clone();
        let window = gtk::ApplicationWindow::builder()
            .application(&ctx.app)
            .title(windowing::TITLE_FULL)
            .default_width(1180)
            .default_height(760)
            .build();
        windowing::set_class(&window, windowing::CLASS_FULL);
        window.set_size_request(340, 480);

        // ── Header ──
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("header");
        let logo = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        logo.add_css_class("logo-tile");
        logo.set_halign(gtk::Align::Start);
        logo.set_valign(gtk::Align::Center);
        let logo_icon = widgets::icon("videocam");
        logo_icon.set_halign(gtk::Align::Center);
        logo_icon.set_hexpand(true);
        logo.set_hexpand(false);
        logo.append(&logo_icon);
        let title = widgets::line("Tiny 2 Lite", &["app-title"]);
        title.set_valign(gtk::Align::Center);
        let chip = StatusChip::new();
        chip.root.set_valign(gtk::Align::Center);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let slim_button = widgets::icon_button("pip", "Switch to the slim window", false);
        let refresh = widgets::icon_button("refresh", "Read the camera again", false);
        let controls = gtk::WindowControls::new(gtk::PackType::End);
        controls.set_visible(ctx.style.window_buttons());
        { let (c, s) = (controls.clone(), ctx.style.clone()); ctx.style.on_change(move || c.set_visible(s.window_buttons())); }
        // Start in the smallest form; the first layout pass gives the words back when there is room.
        title.set_visible(false);
        chip.set_compact(true);
        header.append(&logo);
        header.append(&title);
        header.append(&chip.root);
        header.append(&spacer);
        header.append(&refresh);
        header.append(&slim_button);
        header.append(&controls);
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&header));
        window.set_titlebar(Some(&handle));

        // ── Stage (left) ──
        let hub_view = PreviewView::new(&ctx.hub);
        hub_view.lock_aspect();
        let joystick = Joystick::new(&store);
        let stage = gtk::Box::new(gtk::Orientation::Vertical, 16);
        stage.set_size_request(320, -1);
        // The preview and the gimbal scroll together when the window is short, so the window can
        // always be made smaller than they are.
        let stage_scroller = gtk::ScrolledWindow::new();
        stage_scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        stage_scroller.set_propagate_natural_height(false);
        stage_scroller.set_propagate_natural_width(true);
        stage_scroller.set_min_content_height(120);
        stage_scroller.set_child(Some(&stage));

        let preview_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview_card.add_css_class("stage-card");
        // The preview is as tall as its width and the camera's shape make it; the gimbal takes the rest.
        preview_card.set_vexpand(false);
        preview_card.set_valign(gtk::Align::Start);
        let controls_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let pause = widgets::icon_button("pause", "Pause the preview", true);
        pause.add_css_class("on-stage");
        let hub = ctx.hub.clone();
        pause.connect_clicked(move |button| {
            let enabled = !hub.is_enabled();
            hub.set_enabled(enabled);
            if let Some(glyph) = button.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                widgets::set_icon(&glyph, if enabled { "pause" } else { "play_arrow" });
            }
            button.set_tooltip_text(Some(if enabled { "Pause the preview" } else { "Resume the preview" }));
            widgets::label_accessible(button, if enabled { "Pause the preview" } else { "Resume the preview" });
        });
        let pop = widgets::icon_button("open_in_new", "Open the preview in its own window", true);
        pop.add_css_class("on-stage");
        let weak_ctx = Rc::downgrade(ctx);
        pop.connect_clicked(move |_| { if let Some(ctx) = weak_ctx.upgrade() { ctx.set_popped_out(true); } });
        // The pause button shows the state it was left in.
        if !ctx.hub.is_enabled() {
            if let Some(glyph) = pause.child().and_then(|c| c.downcast::<gtk::Label>().ok()) { widgets::set_icon(&glyph, "play_arrow"); }
            pause.set_tooltip_text(Some("Resume the preview"));
        }
        controls_row.append(&pause);
        controls_row.append(&pop);
        hub_view.add_control(&controls_row);

        // When the preview is in its own window, say so and offer a way back.
        let popped = gtk::Stack::new();
        popped.set_vexpand(false);
        popped.add_named(hub_view.widget(), Some("here"));
        let away = gtk::Box::new(gtk::Orientation::Vertical, 10);
        away.add_css_class("preview-surface");
        away.set_valign(gtk::Align::Fill);
        let note = gtk::Box::new(gtk::Orientation::Vertical, 10);
        note.set_halign(gtk::Align::Center);
        note.set_valign(gtk::Align::Center);
        note.set_vexpand(true);
        let glyph = widgets::icon("picture_in_picture");
        glyph.add_css_class("big");
        note.append(&glyph);
        note.append(&widgets::text("The preview is in its own window.", &["preview-note"]));
        let back = gtk::Button::with_label("Bring it back");
        back.add_css_class("filled");
        back.set_halign(gtk::Align::Center);
        let weak_ctx = Rc::downgrade(ctx);
        back.connect_clicked(move |_| { if let Some(ctx) = weak_ctx.upgrade() { ctx.set_popped_out(false); } });
        note.append(&back);
        away.append(&note);
        popped.add_named(&away, Some("away"));
        popped.set_visible_child_name(if current_config().preview_popped_out { "away" } else { "here" });
        preview_card.append(&popped);

        let gimbal_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        gimbal_card.add_css_class("card");
        gimbal_card.set_vexpand(true);
        gimbal_card.append(&joystick.widget());
        stage.append(&preview_card);
        stage.append(&gimbal_card);

        // ── Settings (right) ──
        let rows: Rows = Rc::default();
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(180);
        stack.set_vexpand(true);
        let mut masonries = Vec::new();
        let page_scrollers: Rc<RefCell<Vec<gtk::ScrolledWindow>>> = Rc::default();
        for (page, _, _) in PAGES {
            match page {
                Page::Tab(tab) => {
                    let built = settings::tab_page(&store, tab, &rows);
                    if tab == Tab::Device {
                        // Extra cards that are not registry settings.
                        built.masonry.add(&panels::capture_mode_card(&store, &ctx.hub), 6);
                        built.masonry.add(&panels::startup_pose_card(&store), 5);
                        built.masonry.add(&panels::sleep_timer_card(), 5);
                        built.masonry.add(&panels::appearance_card(&ctx.style), 8);
                        built.masonry.add(&panels::developer_card(&store), 4);
                    }
                    if tab == Tab::Camera {
                        // Driver controls with no catalogue entry appear once the camera has answered.
                        let masonry = built.masonry.clone();
                        let (st, rows) = (store.clone(), rows.clone());
                        let added = Rc::new(Cell::new(false));
                        store.subscribe(move |snapshot| {
                            if !snapshot.controls.is_empty() && !added.replace(true) {
                                if let Some(card) = settings::discovered_card(&st, &rows) { masonry.add(&card, 6); }
                            }
                            !added.get()
                        });
                    }
                    masonries.push(built.masonry.clone());
                    page_scrollers.borrow_mut().push(built.root.clone());
                    stack.add_named(&built.root, Some(page_name(page)));
                }
                Page::Presets => {
                    let presets = panels::presets_page(&store);
                    page_scrollers.borrow_mut().push(presets.clone());
                    stack.add_named(&presets, Some(page_name(page)));
                }
            }
        }
        let search_holder = gtk::ScrolledWindow::new();
        search_holder.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        page_scrollers.borrow_mut().push(search_holder.clone());
        stack.add_named(&search_holder, Some("search"));

        let search = gtk::SearchEntry::new();
        search.add_css_class("search");
        search.set_placeholder_text(Some("Find a setting"));
        search.set_hexpand(true);
        widgets::label_accessible(&search, "Find a setting");
        let search_row = gtk::Revealer::new();
        search_row.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        search_row.set_child(Some(&search));
        let search_toggle = widgets::icon_button("search", "Find a setting (Ctrl+F)", false);

        let tab_bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tab_bar.add_css_class("tabbar");
        tab_bar.set_halign(gtk::Align::Start);
        let bar_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bar_row.append(&tab_bar);
        let bar_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar_spacer.set_hexpand(true);
        bar_row.append(&bar_spacer);
        bar_row.append(&search_toggle);

        let banner = connection_banner(ctx);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_size_request(340, -1);
        bar_row.set_margin_bottom(12);
        search_row.set_margin_bottom(0);
        banner.set_margin_bottom(0);
        column.append(&bar_row);
        column.append(&search_row);
        column.append(&banner);
        stack.set_margin_top(2);
        column.append(&stack);

        // Detects the settings column width, to pick one or two card columns.
        let sensor = gtk::DrawingArea::new();
        sensor.set_content_height(0);
        sensor.set_hexpand(true);
        sensor.set_valign(gtk::Align::Start);
        sensor.set_can_target(false);
        let right = gtk::Overlay::new();
        right.set_child(Some(&column));
        right.add_overlay(&sensor);

        let state = Rc::new(FullState {
            _rows: rows, search_rows: RefCell::new(Rc::default()), stack: stack.clone(), tabs: RefCell::default(),
            last_page: Cell::new(0), search: search.clone(), search_timer: RefCell::default(), popped: popped.clone(),
            _view: hub_view.clone(),
        });

        for (index, (page, label, glyph)) in PAGES.iter().enumerate() {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.set_halign(gtk::Align::Center);
            content.append(&widgets::icon(glyph));
            let text = widgets::line(label, &["tab-label"]);
            // The words give way to the icons rather than forcing the window wide.
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            content.append(&text);
            let button = gtk::ToggleButton::new();
            button.set_child(Some(&content));
            button.add_css_class("tab");
            button.set_tooltip_text(Some(&format!("{label} (Alt+{})", index + 1)));
            widgets::label_accessible(&button, label);
            tab_bar.append(&button);
            state.tabs.borrow_mut().push((*page, button));
        }
        for (index, (_, button)) in state.tabs.borrow().iter().enumerate() {
            let state2 = state.clone();
            button.connect_clicked(move |_| select_page(&state2, index));
        }
        select_page(&state, 0);

        // Search
        let (state_search, store_search, holder) = (state.clone(), store.clone(), search_holder.clone());
        search.connect_search_changed(move |entry| {
            if let Some(id) = state_search.search_timer.borrow_mut().take() { id.remove(); }
            let (state, store, holder, entry) = (state_search.clone(), store_search.clone(), holder.clone(), entry.clone());
            let id = glib::timeout_add_local_once(Duration::from_millis(140), move || {
                state.search_timer.borrow_mut().take();
                let query = entry.text().to_string();
                if query.trim().is_empty() {
                    let index = state.last_page.get();
                    select_page(&state, index);
                    return;
                }
                let keep: Rows = Rc::default();
                holder.set_child(Some(&settings::search_results(&store, &query, &keep)));
                *state.search_rows.borrow_mut() = keep;
                for (_, button) in state.tabs.borrow().iter() { button.set_active(false); }
                state.stack.set_visible_child_name("search");
            });
            *state_search.search_timer.borrow_mut() = Some(id);
        });
        let hide_row = search_row.clone();
        search.connect_stop_search(move |entry| { entry.set_text(""); hide_row.set_reveal_child(false); });
        let (toggle_row, toggle_entry) = (search_row.clone(), search.clone());
        search_toggle.connect_clicked(move |_| {
            let open = !toggle_row.reveals_child();
            toggle_row.set_reveal_child(open);
            if open { toggle_entry.grab_focus(); } else { toggle_entry.set_text(""); }
        });

        // ── Layout ──
        // Wide: the preview and camera controls on the left, the tabs and their content on the right, with a
        // divider between. Stacked: one page that scrolls as a whole, with the preview and controls on top,
        // the tabs under them and the content under those.
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_wide_handle(false);
        paned.set_resize_start_child(false);
        paned.set_resize_end_child(true);
        paned.set_shrink_start_child(true);
        paned.set_shrink_end_child(true);
        paned.set_margin_start(20);
        paned.set_margin_end(14);
        paned.set_margin_bottom(20);
        paned.set_start_child(Some(&stage_scroller));
        paned.set_end_child(Some(&right));
        let stacked = Rc::new(Cell::new(false));
        paned.set_position(if current_config().split_position >= 320 { current_config().split_position } else { 420 });

        let outer_box = gtk::Box::new(gtk::Orientation::Vertical, 16);
        outer_box.set_margin_start(20);
        outer_box.set_margin_end(14);
        outer_box.set_margin_bottom(20);
        let outer_scroll = gtk::ScrolledWindow::new();
        outer_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        outer_scroll.set_child(Some(&outer_box));
        let layout_stack = gtk::Stack::new();
        layout_stack.add_named(&paned, Some("wide"));
        layout_stack.add_named(&outer_scroll, Some("stacked"));
        layout_stack.set_visible_child_name("wide");

        let dialogs = Dialogs::new();
        dialogs.overlay().set_child(Some(&layout_stack));
        dialogs.install();
        let toaster = Toaster::new();
        dialogs.overlay().add_overlay(toaster.widget());
        store.add_dialogs(dialogs.clone());
        store.add_toaster(toaster);
        window.set_child(Some(dialogs.overlay()));


        // Adapt to the window. The layout follows the room the right-hand panel has, not the window: with the
        // left panel made narrower, the window can get narrower before the page has to stack.
        const RIGHT_MIN: i32 = 470;
        const RIGHT_BACK: i32 = 510;
        const MARGINS: i32 = 34;
        let layout = Rc::new(Cell::new(0_u8));
        let last_width = Rc::new(Cell::new(0_i32));
        let apply_layout = {
            let (paned, stage, stage_scroller, right, preview_card, gimbal_card, stacked, outer_box, layout_stack, stack, scrollers, popped) =
                (paned.clone(), stage.clone(), stage_scroller.clone(), right.clone(), preview_card.clone(), gimbal_card.clone(), stacked.clone(), outer_box.clone(), layout_stack.clone(), stack.clone(), page_scrollers.clone(), popped.clone());
            Rc::new(move |mode: u8, width: i32, first: bool| {
                let now_stacked = mode != 0;
                if first || now_stacked != stacked.get() {
                    if now_stacked {
                        paned.set_start_child(None::<&gtk::Widget>);
                        paned.set_end_child(None::<&gtk::Widget>);
                        stage_scroller.set_child(None::<&gtk::Widget>);
                        if stage.parent().is_none() { outer_box.append(&stage); }
                        if right.parent().is_none() { outer_box.append(&right); }
                        layout_stack.set_visible_child_name("stacked");
                    } else {
                        for widget in [stage.upcast_ref::<gtk::Widget>(), right.upcast_ref()] {
                            if widget.parent().as_ref() == Some(outer_box.upcast_ref()) { outer_box.remove(widget); }
                        }
                        stage_scroller.set_child(Some(&stage));
                        paned.set_start_child(Some(&stage_scroller));
                        paned.set_end_child(Some(&right));
                        let saved = current_config().split_position;
                        paned.set_position(if saved >= 320 { saved } else { 420 });
                        layout_stack.set_visible_child_name("wide");
                    }
                    stacked.set(now_stacked);
                    // Stacked, the page scrolls as one; wide, each tab scrolls inside its own space.
                    for scroller in scrollers.borrow().iter() {
                        if now_stacked {
                            scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
                            scroller.set_propagate_natural_height(true);
                            scroller.set_vexpand(false);
                        } else {
                            scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
                            scroller.set_propagate_natural_height(false);
                            scroller.set_vexpand(true);
                        }
                    }
                    stack.set_vexpand(!now_stacked);
                    right.set_vexpand(!now_stacked);
                }
                popped.set_vexpand(false);
                preview_card.set_vexpand(false);
                preview_card.set_valign(gtk::Align::Start);
                if mode == 0 {
                    stage.set_orientation(gtk::Orientation::Vertical);
                    stage.set_size_request(320, -1);
                    stage.set_halign(gtk::Align::Fill);
                    preview_card.set_hexpand(false);
                    gimbal_card.set_hexpand(false);
                    gimbal_card.set_vexpand(true);
                } else {
                    // One column, centred and not wider than the picture needs to be.
                    stage.set_orientation(gtk::Orientation::Vertical);
                    stage.set_size_request((width - MARGINS).clamp(300, 560), -1);
                    stage.set_halign(gtk::Align::Center);
                    preview_card.set_hexpand(true);
                    gimbal_card.set_hexpand(true);
                    gimbal_card.set_vexpand(false);
                }
            })
        };
        let relayout = {
            let (paned, stacked, layout, last_width, ctx, title, chip, apply_layout) =
                (paned.clone(), stacked.clone(), layout.clone(), last_width.clone(), ctx.clone(), title.clone(), chip.clone(), apply_layout.clone());
            Rc::new(move |width: i32| {
                last_width.set(width);
                ctx.style.report_width(width);
                // The header gives up its words before its buttons.
                title.set_visible(width >= 540);
                chip.set_compact(width < 720);
                // What the right-hand panel would have: the window less the left panel and the margins.
                let split = if stacked.get() {
                    let saved = current_config().split_position;
                    if saved >= 320 { saved } else { 420 }
                } else { paned.position().max(320) };
                let right_width = width - split - MARGINS;
                let was = layout.get();
                let go_stacked = if was == 0 { right_width < RIGHT_MIN } else { right_width < RIGHT_BACK };
                // A little slack each way, so a window sitting on a threshold does not flicker.
                let next = if go_stacked { 1 } else { 0 };
                let first = false;
                // The stacked column follows the window width, so it is set again on every resize.
                if next == was && next == 0 && !first { return; }
                layout.set(next);
                apply_layout(next, width, first);
            })
        };

        // Window width drives the layout.
        let width_sensor = gtk::DrawingArea::new();
        width_sensor.set_content_height(0);
        width_sensor.set_hexpand(true);
        width_sensor.set_vexpand(false);
        width_sensor.set_valign(gtk::Align::Start);
        dialogs.overlay().add_overlay(&width_sensor);
        width_sensor.set_can_target(false);
        let r = relayout.clone();
        width_sensor.connect_resize(move |_, width, _| r(width));

        // Settings column width drives the card columns.
        let two_columns = Rc::new(Cell::new(false));
        let masonries = Rc::new(masonries);
        sensor.connect_resize({
            let (two, masonries, state) = (two_columns.clone(), masonries.clone(), state.clone());
            move |_, width, _| {
                // Tabs shrink to icons when the column is tight.
                for (_, button) in state.tabs.borrow().iter() {
                    let words = width >= 420;
                    if let Some(content) = button.child() {
                        if let Some(label) = content.last_child() { label.set_visible(words); }
                        // With the words gone the gap beside the icon goes too, so the icon sits in the middle.
                        if let Ok(row) = content.downcast::<gtk::Box>() { row.set_spacing(if words { 8 } else { 0 }); }
                    }
                }
                // One column of cards at every width: the stage on the left already uses the width.
                let want_two = false;
                if want_two != two.get() {
                    two.set(want_two);
                    for masonry in masonries.iter() { masonry.set_columns(if want_two { 2 } else { 1 }); }
                }
            }
        });

        // Remember the split the user chose, and never let the right-hand panel get squeezed below what
        // it needs: the divider stops there, and a still narrower window stacks the page instead.
        paned.connect_position_notify({
            let (stacked, last_width) = (stacked.clone(), last_width.clone());
            move |paned| {
                if stacked.get() || paned.orientation() != gtk::Orientation::Horizontal { return; }
                let position = paned.position();
                let width = last_width.get();
                let furthest = width - MARGINS - RIGHT_MIN;
                if width > 0 && furthest >= 320 && position > furthest {
                    paned.set_position(furthest);
                    return;
                }
                if position >= 320 { glib::idle_add_local_once(move || update_config(|c| c.split_position = position)); }
            }
        });

        // Keyboard: Alt+1…5 for tabs, Ctrl+F or typing to search.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let state_keys = state.clone();
        let search_keys = search.clone();
        let search_row_keys = search_row.clone();
        let window_keys = window.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            use gtk::gdk::{Key, ModifierType};
            if modifiers.contains(ModifierType::ALT_MASK) {
                let index = match key { Key::_1 => Some(0), Key::_2 => Some(1), Key::_3 => Some(2), Key::_4 => Some(3), Key::_5 => Some(4), _ => None };
                if let Some(index) = index { select_page(&state_keys, index); return glib::Propagation::Stop; }
            }
            if modifiers.contains(ModifierType::CONTROL_MASK) && key == Key::f {
                search_row_keys.set_reveal_child(true);
                search_keys.grab_focus();
                return glib::Propagation::Stop;
            }
            // Typing anywhere starts a search, unless a text field already has focus.
            let focus_in_text = gtk::prelude::GtkWindowExt::focus(&window_keys).is_some_and(|w| w.is::<gtk::Text>() || w.is::<gtk::Entry>());
            if !focus_in_text && !modifiers.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK) {
                if let Some(c) = key.to_unicode().filter(|c| c.is_alphanumeric()) {
                    search_row_keys.set_reveal_child(true);
                    search_keys.grab_focus();
                    search_keys.set_text(&c.to_string());
                    search_keys.set_position(-1);
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);

        // Header actions
        let weak_ctx = Rc::downgrade(ctx);
        slim_button.connect_clicked(move |_| { if let Some(ctx) = weak_ctx.upgrade() { ctx.show_slim(); } });
        let store_refresh = store.clone();
        refresh.connect_clicked(move |_| { store_refresh.refresh(); store_refresh.toast("Reading the camera"); });

        // Status chip follows the camera.
        let (chip_store, chip2) = (store.clone(), chip.clone());
        store.subscribe(move |snapshot| { chip2.update(chip_store.connection(), snapshot); true });
        chip.update(store.connection(), &store.snapshot());

        // Refresh when the user comes back to the window.
        let refresh_store = store.clone();
        window.connect_is_active_notify(move |w| { if w.is_active() { refresh_store.refresh_if_stale(); } });

        // Closing the window ends the app, with the gimbal stopped.
        let (weak_ctx, js) = (Rc::downgrade(ctx), joystick.clone());
        window.connect_close_request(move |_| {
            js.stop_now_blocking();
            if let Some(ctx) = weak_ctx.upgrade() { ctx.quit(); }
            glib::Propagation::Proceed
        });

        // Hooks for the headless screenshot harness. Only active with the synthetic camera.
        if crate::backend::ui_test_mode() {
            let env = |name: &str| std::env::var(name).ok();
            if let (Some(w), Some(h)) = (env("TINY4LITE_UI_TEST_WIDTH"), env("TINY4LITE_UI_TEST_HEIGHT")) {
                if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) { window.set_default_size(w, h); }
            }
            if let Some(page) = env("TINY4LITE_UI_TEST_PAGE") {
                if let Some(index) = PAGES.iter().position(|(p, _, _)| page_name(*p) == page) { select_page(&state, index); }
            }
            if let Some(query) = env("TINY4LITE_UI_TEST_SEARCH") {
                search_row.set_reveal_child(true);
                search.set_text(&query);
            }
        }

        FullWindow { window, joystick, _keep: state }
    }

    /// Show whether the preview is here or in its own window.
    pub fn set_popped_out(&self, popped: bool) {
        self._keep.popped.set_visible_child_name(if popped { "away" } else { "here" });
    }
}

fn select_page(state: &Rc<FullState>, index: usize) {
    state.last_page.set(index);
    let tabs = state.tabs.borrow();
    for (i, (_, button)) in tabs.iter().enumerate() { button.set_active(i == index); }
    if let Some((page, _)) = tabs.get(index) { state.stack.set_visible_child_name(page_name(*page)); }
    if !state.search.text().is_empty() { state.search.set_text(""); }
}

// ── Header status ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct StatusChip {
    pub root: gtk::Box,
    icon: gtk::Label,
    text: gtk::Label,
}


impl StatusChip {
    pub fn new() -> StatusChip {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.add_css_class("status-chip");
        let icon = widgets::icon("check_circle");
        icon.add_css_class("small-icon");
        let text = gtk::Label::new(Some("Connecting"));
        root.append(&icon);
        root.append(&text);
        StatusChip { root, icon, text }
    }

    pub fn update(&self, connection: Connection, snapshot: &crate::backend::registry::Snapshot) {
        for class in ["ok", "warn", "error"] { self.root.remove_css_class(class); }
        let (class, glyph, label) = match connection {
            Connection::Absent => ("warn", "usb_off", "Camera not found"),
            Connection::NoVideoNode => ("warn", "videocam_off", "Video device unavailable"),
            Connection::Connected if snapshot.error.is_some() => ("error", "error", "Can't reach the camera"),
            Connection::Connected if snapshot.byte(0x02) == Some(1) => ("warn", "bedtime", "Asleep"),
            Connection::Connected if snapshot.controls.is_empty() => ("", "hourglass_empty", "Connecting"),
            Connection::Connected => ("ok", "check_circle", "Connected"),
        };
        if !class.is_empty() { self.root.add_css_class(class); }
        widgets::set_icon(&self.icon, glyph);
        self.text.set_text(label);
        widgets::label_accessible(&self.root, label);
        self.root.set_tooltip_text(Some(label));
    }

    /// Icon only, for narrow windows. The words stay in the tooltip.
    pub fn set_compact(&self, compact: bool) { self.text.set_visible(!compact); }
}

/// A banner that explains a missing camera and offers to look again.
pub fn connection_banner(ctx: &Rc<Ctx>) -> gtk::Revealer {
    let revealer = gtk::Revealer::new();
    revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    card.add_css_class("card");
    card.add_css_class("tight");
    let glyph = widgets::icon("usb_off");
    let text = widgets::text("", &["body"]);
    text.set_hexpand(true);
    let again = gtk::Button::with_label("Check again");
    again.add_css_class("small");
    again.set_valign(gtk::Align::Center);
    card.append(&glyph);
    card.append(&text);
    card.append(&again);
    revealer.set_child(Some(&card));
    let weak_ctx = Rc::downgrade(ctx);
    again.connect_clicked(move |_| { if let Some(ctx) = weak_ctx.upgrade() { ctx.reconnect(true); } });
    let store = ctx.store.clone();
    let (revealer2, text2) = (revealer.clone(), text.clone());
    let update = move |connection: Connection| {
        match connection {
            Connection::Connected => revealer2.set_reveal_child(false),
            Connection::Absent => { text2.set_text("No Tiny 2 Lite found. Plug it in with a USB cable and it will appear here."); revealer2.set_reveal_child(true); }
            Connection::NoVideoNode => { text2.set_text("The camera is plugged in, but Linux has no video device for it. Another program may be using it."); revealer2.set_reveal_child(true); }
        }
    };
    update(store.connection());
    let store2 = store.clone();
    store.subscribe(move |_| { update(store2.connection()); true });
    revealer
}
