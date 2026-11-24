//! Small building blocks in the app's style, and the generic setting row.

use super::store::Store;
use crate::backend::registry::{Kind, Opt, Origin, Readback, Setting, Snapshot, Value};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

// ── Basics ────────────────────────────────────────────────────────────────

/// A Material Symbols glyph. The text is the ligature name.
pub fn icon(name: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    set_icon(&label, name);
    label.add_css_class("icon");
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    label
}

/// Show another icon in a label made by `icon`.
pub fn set_icon(label: &gtk::Label, name: &str) {
    match super::icons::glyph(name) {
        Some(glyph) => label.set_text(&glyph.to_string()),
        None => { debug_assert!(false, "unknown icon {name}"); label.set_text("?"); }
    }
}

/// One line of text that never wraps (titles, values, tab labels).
pub fn line(content: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(Some(content));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    for class in classes { label.add_css_class(class); }
    label
}

pub fn text(content: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(Some(content));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    for class in classes { label.add_css_class(class); }
    label
}

pub fn label_accessible(widget: &impl IsA<gtk::Accessible>, name: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
}

/// A round icon-only button. Always has a tooltip and an accessible name.
pub fn icon_button(glyph: &str, tooltip: &str, small: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&icon(glyph)));
    button.add_css_class("round");
    if small { button.add_css_class("small"); }
    button.set_tooltip_text(Some(tooltip));
    label_accessible(&button, tooltip);
    button.set_valign(gtk::Align::Center);
    button
}

pub fn icon_text_button(glyph: &str, label: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&icon(glyph));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::new();
    button.set_child(Some(&content));
    button
}

/// Stop the mouse wheel changing a slider or menu; scroll the page instead.
/// A scrolled area that holds still: no coasting after a touchpad scroll (a click while it coasts only
/// stops it, so a switch seems not to respond), and no jump to bring a clicked control into view (the
/// row moves under the pointer and the next click misses).
pub fn scroller() -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_kinetic_scrolling(false);
    let hold = |scroll: &gtk::ScrolledWindow| {
        if let Some(viewport) = scroll.child().and_then(|c| c.downcast::<gtk::Viewport>().ok()) { viewport.set_scroll_to_focus(false); }
    };
    scroll.connect_child_notify(move |scroll| hold(scroll));
    scroll
}

/// The marks under a slider: the ones its setting lists that fall inside the range the camera reports,
/// and always one at the middle of that range, so every slider shows where its middle is.
fn mark_slider(scale: &gtk::Scale, spec: &crate::backend::registry::SliderSpec) {
    scale.clear_marks();
    // A slider over stops is marked at the stops nearest its ticks.
    if !spec.stops.is_empty() {
        for (at, label) in &spec.ticks { scale.add_mark(spec.position_of(*at), gtk::PositionType::Bottom, Some(label)); }
        return;
    }
    let (min, max, step) = (spec.min, spec.max.max(spec.min + spec.step), spec.step.max(f64::EPSILON));
    let marks: Vec<(f64, String)> = spec.ticks.iter().filter(|(at, _)| (min..=max).contains(at)).map(|(at, label)| (*at, label.to_string())).collect();
    let middle = min + ((max - min) / 2.0 / step).round() * step;
    let has_middle = marks.iter().any(|(at, _)| (at - middle).abs() <= step / 2.0 + f64::EPSILON);
    for (at, label) in &marks { scale.add_mark(*at, gtk::PositionType::Bottom, Some(label)); }
    if !has_middle { scale.add_mark(middle, gtk::PositionType::Bottom, Some(&spec.fmt.format(middle))); }
}

pub fn lock_wheel(widget: &impl IsA<gtk::Widget>) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let target = widget.clone().upcast::<gtk::Widget>();
    controller.connect_scroll(move |_, _, dy| {
        let mut parent = target.parent();
        while let Some(current) = parent {
            if let Ok(scrolled) = current.clone().downcast::<gtk::ScrolledWindow>() {
                let adjustment = scrolled.vadjustment();
                let step = adjustment.page_size().powf(2.0 / 3.0);
                let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
                adjustment.set_value((adjustment.value() + dy * step).clamp(adjustment.lower(), upper));
                break;
            }
            parent = current.parent();
        }
        glib::Propagation::Stop
    });
    widget.add_controller(controller);
}

/// The app's own icon (the same as in the app menu), at `size` pixels: for the top-left of the windows.
pub fn app_logo(size: i32) -> gtk::Picture {
    static PNG: &[u8] = include_bytes!("../../assets/tiny4lite-control-128.png");
    let picture = gtk::Picture::new();
    if let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(PNG)) { picture.set_paintable(Some(&texture)); }
    picture.set_size_request(size, size);
    picture.set_can_shrink(true);
    picture.set_halign(gtk::Align::Center);
    picture.set_valign(gtk::Align::Center);
    picture.add_css_class("app-logo");
    picture.set_accessible_role(gtk::AccessibleRole::Presentation);
    picture
}

/// The value of a slider in a pill, with a small up and down arrow on its right that move the slider by one
/// step (one stop on a slider over stops). Holding an arrow repeats, slowly at first, then faster.
fn value_stepper(value: &gtk::Label, scale: &gtk::Scale, label: &str) -> gtk::Box {
    let pill = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    pill.add_css_class("value-pill");
    pill.set_valign(gtk::Align::Center);
    pill.append(value);
    let arrows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    arrows.set_valign(gtk::Align::Center);
    for (glyph, direction, words) in [("expand_less", 1.0, "up"), ("expand_more", -1.0, "down")] {
        let arrow = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        arrow.add_css_class("pill-arrow");
        arrow.append(&icon(glyph));
        arrow.set_tooltip_text(Some(&format!("{label} one step {words} (hold to keep going)")));
        arrow.set_accessible_role(gtk::AccessibleRole::Button);
        label_accessible(&arrow, &format!("{label} one step {words}"));
        let scale = scale.clone();
        let nudge = move || {
            let adjustment = scale.adjustment();
            let step = adjustment.step_increment().max(f64::EPSILON);
            let target = (scale.value() + direction * step).clamp(adjustment.lower(), adjustment.upper());
            if (target - scale.value()).abs() > f64::EPSILON / 2.0 { scale.set_value(target); }
        };
        let repeat: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let press = gtk::GestureClick::new();
        {
            let (repeat, nudge) = (repeat.clone(), nudge.clone());
            press.connect_pressed(move |_, _, _, _| {
                nudge();
                if let Some(id) = repeat.borrow_mut().take() { id.remove(); }
                // After a pause, keep stepping while held.
                let (slot, nudge) = (repeat.clone(), nudge.clone());
                *repeat.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(400), move || {
                    let nudge = nudge.clone();
                    let id = glib::timeout_add_local(Duration::from_millis(70), move || { nudge(); glib::ControlFlow::Continue });
                    *slot.borrow_mut() = Some(id);
                }));
            });
        }
        let stop = { let repeat = repeat.clone(); move || { if let Some(id) = repeat.borrow_mut().take() { id.remove(); } } };
        { let stop = stop.clone(); press.connect_released(move |_, _, _, _| stop()); }
        { let stop = stop.clone(); press.connect_cancel(move |_, _| stop()); }
        { let stop = stop.clone(); press.connect_stopped(move |_| stop()); }
        arrow.add_controller(press);
        arrows.append(&arrow);
    }
    pill.append(&arrows);
    pill
}

/// Let a card fold down to its heading: an arrow at the right of the heading hides or shows the rest. The
/// choice is remembered by the card's title. Returns the box the card's content goes in.
// ── Cards that can be opened from elsewhere ──────────────────────────────

/// A foldable card: the card, and how to fold or unfold it.
struct CardEntry { card: glib::WeakRef<gtk::Box>, fold: Rc<dyn Fn(bool)> }
thread_local! {
    static CARDS: RefCell<Vec<(String, CardEntry)>> = const { RefCell::new(Vec::new()) };
    /// Shows a page of the window by its name ("camera", "presets", "device"); set by the full window.
    static PAGE_OPENER: RefCell<Option<Rc<dyn Fn(&str)>>> = const { RefCell::new(None) };
}

pub fn set_page_opener(opener: impl Fn(&str) + 'static) { PAGE_OPENER.with(|o| *o.borrow_mut() = Some(Rc::new(opener))); }

/// Show the card with this title: its tab, unfolded, scrolled into view. False if no window has it.
pub fn open_card(title: &str) -> bool {
    let found = CARDS.with(|cards| {
        cards.borrow_mut().retain(|(_, e)| e.card.upgrade().is_some());
        cards.borrow().iter().filter(|(t, _)| t == title).filter_map(|(_, e)| e.card.upgrade().map(|c| (c, e.fold.clone())))
            .find(|(c, _)| c.root().is_some())
    });
    let Some((card, fold)) = found else { return false };
    fold(false);
    // The page it is on: the child of a stack that holds it.
    let mut widget: Option<gtk::Widget> = Some(card.clone().upcast());
    while let Some(w) = widget {
        if let Some(stack) = w.parent().and_then(|p| p.downcast::<gtk::Stack>().ok()) {
            if let Some(name) = stack.page(&w).name() {
                if let Some(opener) = PAGE_OPENER.with(|o| o.borrow().clone()) { opener(name.as_str()); } else { stack.set_visible_child(&w); }
            }
            break;
        }
        widget = w.parent();
    }
    // Scroll to it once the page is shown and laid out.
    let weak = card.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
        let Some(card) = weak.upgrade() else { return };
        let mut parent = card.parent();
        while let Some(p) = parent {
            if let Ok(scrolled) = p.clone().downcast::<gtk::ScrolledWindow>() {
                if let Some(child) = scrolled.child() {
                    if let Some(point) = card.compute_point(&child, &gtk::graphene::Point::new(0.0, 0.0)) {
                        let adjustment = scrolled.vadjustment();
                        adjustment.set_value((f64::from(point.y()) - 12.0).clamp(adjustment.lower(), adjustment.upper() - adjustment.page_size()));
                    }
                }
                break;
            }
            parent = p.parent();
        }
        card.add_css_class("flash");
        let weak = card.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || { if let Some(card) = weak.upgrade() { card.remove_css_class("flash"); } });
    });
    true
}

/// A small round button that opens the card with this title (on its tab, unfolded).
pub fn link_button(glyph: &str, tooltip: &str, target: &'static str) -> gtk::Button {
    let button = icon_button(glyph, tooltip, true);
    button.add_css_class("flat-icon");
    button.add_css_class("card-link");
    button.connect_clicked(move |b| {
        if !open_card(target) {
            b.set_tooltip_text(Some(&format!("{target} is in the full window")));
        }
    });
    button
}

/// Make a card foldable: an arrow in its heading folds it away and unfolds it; a click anywhere on a folded
/// card unfolds it too. Folded cards stay folded (by title, in the settings).
pub fn collapsible(card: &gtk::Box, head: &gtk::Box, title: &str) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let reveal = gtk::Revealer::new();
    reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    reveal.set_transition_duration(180);
    reveal.set_child(Some(&content));
    card.append(&reveal);
    let folded = crate::backend::current_config().collapsed_cards.iter().any(|t| t == title);
    reveal.set_reveal_child(!folded);
    if folded { card.add_css_class("folded"); }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    head.append(&spacer);
    let arrow = icon_button(if folded { "expand_more" } else { "expand_less" }, "", true);
    arrow.add_css_class("flat-icon");
    arrow.add_css_class("fold");
    let show = |arrow: &gtk::Button, folded: bool, title: &str| {
        if let Some(glyph) = arrow.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            if folded { set_icon(&glyph, "expand_more"); } else { set_icon(&glyph, "expand_less"); }
        }
        let words = if folded { format!("Show {title}") } else { format!("Fold {title} away") };
        arrow.set_tooltip_text(Some(&words));
        label_accessible(arrow, &words);
    };
    show(&arrow, folded, title);
    head.append(&arrow);
    let set: Rc<dyn Fn(bool)> = {
        let (title, card, reveal, arrow) = (title.to_string(), card.downgrade(), reveal.clone(), arrow.downgrade());
        Rc::new(move |fold: bool| {
            if reveal.reveals_child() == !fold { return; }
            reveal.set_reveal_child(!fold);
            if let Some(card) = card.upgrade() { if fold { card.add_css_class("folded"); } else { card.remove_css_class("folded"); } }
            if let Some(arrow) = arrow.upgrade() { show(&arrow, fold, &title); }
            crate::backend::update_config(|c| {
                c.collapsed_cards.retain(|t| t != &title);
                if fold { c.collapsed_cards.push(title.clone()); }
            });
        })
    };
    {
        let (set, reveal) = (set.clone(), reveal.clone());
        arrow.connect_clicked(move |_| set(reveal.reveals_child()));
    }
    // A click anywhere on a folded card opens it (folding is only with the arrow).
    {
        let click = gtk::GestureClick::new();
        let (set, reveal) = (set.clone(), reveal.clone());
        click.connect_released(move |_, _, _, _| { if !reveal.reveals_child() { set(false); } });
        card.add_controller(click);
    }
    CARDS.with(|cards| cards.borrow_mut().push((title.to_string(), CardEntry { card: card.downgrade(), fold: set })));
    content
}

/// An "i" button with a popover. Used instead of long help text in rows.
/// A row's summary, kept short: a summary of up to 80 characters is shown as it is; a longer one shows
/// only its first sentence (if that is short) and goes whole into an info button.
pub fn short_summary(summary: &str) -> (String, bool) {
    const SHORT: usize = 80;
    if summary.chars().count() <= SHORT { return (summary.to_string(), false); }
    let first = summary.split_inclusive(". ").next().unwrap_or("").trim_end();
    (if first.chars().count() <= SHORT && first.len() < summary.len() { first.to_string() } else { String::new() }, true)
}

/// The title and summary of a row, with the long part of the summary behind an info button placed at the
/// end of `row` by the caller (returned so it can go after the row's control).
pub fn titles_with_info(title: &str, summary: &str) -> (gtk::Box, Option<gtk::Button>) {
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&text(title, &["setting-label"]));
    let (short, long) = short_summary(summary);
    if !short.is_empty() {
        let line = text(&short, &["setting-summary"]);
        line.set_wrap(true);
        line.set_xalign(0.0);
        titles.append(&line);
    }
    (titles, long.then(|| info_button(title, &[summary.to_string()])))
}

pub fn info_button(title: &str, paragraphs: &[String]) -> gtk::Button {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.add_css_class("info-pop");
    body.append(&text(title, &["heading"]));
    for paragraph in paragraphs {
        let label = text(paragraph, &["small", "dim"]);
        label.set_max_width_chars(46);
        label.set_selectable(false);
        body.append(&label);
    }
    let popover = gtk::Popover::new();
    popover.set_child(Some(&body));
    popover.set_has_arrow(false);
    let button = gtk::Button::new();
    button.set_child(Some(&icon("info")));
    popover.set_parent(&button);
    let target = popover.clone();
    // No tooltip while the pop-up is open: a tooltip over an open pop-up can bring down a GTK app on Wayland.
    let weak = button.downgrade();
    popover.connect_closed(move |_| if let Some(b) = weak.upgrade() { b.set_has_tooltip(true); });
    button.connect_clicked(move |b| { b.set_has_tooltip(false); target.popup(); });
    button.connect_destroy(move |_| popover.unparent());
    button.add_css_class("round");
    button.add_css_class("small");
    button.add_css_class("flat-icon");
    button.set_valign(gtk::Align::Center);
    let tip = format!("About {}", title.to_lowercase());
    button.set_tooltip_text(Some(&tip));
    label_accessible(&button, &tip);
    button
}

// ── Segmented control ─────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Segmented {
    root: gtk::Box,
    buttons: Rc<RefCell<Vec<(i64, gtk::ToggleButton)>>>,
    suppress: Rc<Cell<bool>>,
    handler: Rc<RefCell<Option<Rc<dyn Fn(i64)>>>>,
}

impl Segmented {
    pub fn new() -> Segmented {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("segmented");
        root.set_homogeneous(true);
        root.set_accessible_role(gtk::AccessibleRole::Group);
        Segmented { root, buttons: Rc::default(), suppress: Rc::default(), handler: Rc::default() }
    }

    pub fn widget(&self) -> &gtk::Box { &self.root }

    pub fn set_options(&self, options: &[Opt]) {
        while let Some(child) = self.root.first_child() { self.root.remove(&child); }
        self.buttons.borrow_mut().clear();
        let count = options.len();
        for (index, option) in options.iter().enumerate() {
            let button = gtk::ToggleButton::with_label(option.label);
            button.add_css_class("segment");
            if index == 0 { button.add_css_class("first"); }
            if index + 1 == count { button.add_css_class("last"); }
            if !option.hint.is_empty() { button.set_tooltip_text(Some(option.hint)); } else { button.set_tooltip_text(Some(option.label)); }
            // In a narrow panel the words shorten ("Stand…") rather than holding the window wide.
            if let Some(text) = button.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            let value = option.value;
            let this = self.clone();
            button.connect_clicked(move |clicked| {
                if this.suppress.get() { return; }
                // Behave as a radio group: a click always selects, never clears.
                this.select(Some(value));
                let handler = this.handler.borrow().clone();
                if let Some(handler) = handler { handler(value); }
                let _ = clicked;
            });
            self.root.append(&button);
            self.buttons.borrow_mut().push((value, button));
        }
    }

    pub fn select(&self, value: Option<i64>) {
        self.suppress.set(true);
        for (v, button) in self.buttons.borrow().iter() { button.set_active(Some(*v) == value); }
        self.suppress.set(false);
    }

    pub fn on_change(&self, handler: impl Fn(i64) + 'static) { *self.handler.borrow_mut() = Some(Rc::new(handler)); }
}

// ── Setting row ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Density { Full, Slim }

enum Control {
    Switch(gtk::Switch),
    Segments(Segmented),
    Menu { drop: gtk::DropDown, values: Rc<RefCell<Vec<i64>>> },
    /// `stepper` is the pill with the value and its fine-step arrows, shown beside the title.
    Slider { scale: gtk::Scale, value: gtk::Label, reset: gtk::Button, stepper: gtk::Box },
}

struct Row {
    setting: &'static Setting,
    store: Store,
    root: gtk::Box,
    control: Control,
    control_box: gtk::Box,
    summary: gtk::Label,
    reason: gtk::Box,
    reason_text: gtk::Label,
    info_slot: gtk::Box,
    unknown: gtk::Label,
    density: Density,
    suppress: Cell<bool>,
    last_edit: Cell<Option<Instant>>,
    /// Resting after a change, for a setting the camera is slow to switch (HDR).
    cooling: Cell<bool>,
    dragging: Cell<bool>,
    /// The slider was pressed and its release has not been seen yet (unlike `dragging`, not cleared by a
    /// timeout): a release lost outside the window leaves this set, and the next pointer move without a
    /// button ends the slider's drag.
    pressed: Cell<bool>,
    current: Cell<Option<Value>>,
    shown_options: RefCell<Vec<(i64, String)>>,
    shown_range: Cell<(f64, f64, f64)>,
    shown_info: RefCell<String>,
}

/// One camera setting, built from its registry entry.
#[derive(Clone)]
pub struct SettingRow(Rc<Row>);

const EDIT_HOLD: Duration = Duration::from_millis(900);

impl SettingRow {
    pub fn new(store: &Store, setting: &'static Setting, density: Density) -> SettingRow {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("setting-row");
        if density == Density::Slim { root.add_css_class("slim"); }
        root.set_accessible_role(gtk::AccessibleRole::Group);
        label_accessible(&root, setting.label);

        let title = text(setting.label, &["setting-label"]);
        title.set_hexpand(true);
        let summary = text(setting.summary, &["setting-summary"]);
        let info_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        info_slot.set_valign(gtk::Align::Center);
        let unknown = line("Unknown", &["badge", "neutral"]);
        unknown.set_valign(gtk::Align::Center);
        unknown.set_visible(false);
        unknown.set_tooltip_text(Some("Not set in this app yet, and the camera can't report it."));

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);
        // In the small window every title line is the same height, the title centred in it, whatever sits beside it.
        if density == Density::Slim {
            head.add_css_class("slim-head");
            titles.set_valign(gtk::Align::Center);
        }
        titles.append(&title);
        if density == Density::Full { titles.append(&summary); }
        head.append(&titles);
        head.append(&unknown);

        let control_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let (control, inline) = Self::make_control(setting, density);
        control_box.append(&control.widget());

        let reason_text = text("", &["reason"]);
        let reason = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        reason.append(&icon("lock"));
        reason.append(&reason_text);
        reason.set_visible(false);

        // Switches and menus sit beside the title; sliders and segments below it.
        if inline {
            head.append(&control_box);
            head.append(&info_slot);
            root.append(&head);
        } else {
            head.append(&info_slot);
            if let Control::Slider { stepper, .. } = &control { head.insert_child_after(stepper, Some(&titles)); }
            root.append(&head);
            root.append(&control_box);
        }
        root.append(&reason);

        let row = Rc::new(Row {
            setting, store: store.clone(), root, control, control_box, summary, reason, reason_text, info_slot, unknown,
            density, suppress: Cell::new(false), last_edit: Cell::new(None), cooling: Cell::new(false), dragging: Cell::new(false), pressed: Cell::new(false),
            current: Cell::new(None), shown_options: RefCell::default(), shown_range: Cell::new((f64::NAN, 0.0, 0.0)),
            shown_info: RefCell::default(),
        });
        let handle = SettingRow(row);
        handle.connect();
        handle.apply(&store.snapshot());
        let weak = Rc::downgrade(&handle.0);
        store.subscribe(move |snapshot| match weak.upgrade() {
            Some(row) => { SettingRow(row).apply(snapshot); row_alive() }
            None => false,
        });
        handle
    }

    pub fn widget(&self) -> &gtk::Box { &self.0.root }

    fn make_control(setting: &Setting, density: Density) -> (Control, bool) {
        match &setting.kind {
            Kind::Toggle => {
                let switch = gtk::Switch::new();
                switch.set_valign(gtk::Align::Center);
                label_accessible(&switch, setting.label);
                (Control::Switch(switch), true)
            }
            Kind::Choice(options) if options.len() <= 4 && density == Density::Full || matches!(&setting.kind, Kind::Choice(o) if o.len() <= 3) => {
                let segments = Segmented::new();
                segments.set_options(options);
                if density == Density::Slim { segments.widget().add_css_class("small"); }
                (Control::Segments(segments), false)
            }
            Kind::Choice(_) | Kind::DriverMenu => {
                let drop = gtk::DropDown::from_strings(&[]);
                drop.set_valign(gtk::Align::Center);
                lock_wheel(&drop);
                label_accessible(&drop, setting.label);
                (Control::Menu { drop, values: Rc::default() }, true)
            }
            Kind::Slider(spec) => {
                let scale = if spec.stops.is_empty() {
                    gtk::Scale::with_range(gtk::Orientation::Horizontal, spec.min, spec.max, spec.step.max(f64::EPSILON))
                } else {
                    gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, (spec.stops.len().max(2) - 1) as f64, 1.0)
                };
                scale.set_draw_value(false);
                scale.set_hexpand(true);
                scale.set_round_digits(if spec.step < 1.0 { 2 } else { 0 });
                mark_slider(&scale, spec);
                // Colour settings show their colours along the bar.
                match setting.id {
                    "v4l2:white_balance_temperature" => scale.add_css_class("grad-temperature"),
                    "v4l2:hue" => scale.add_css_class("grad-hue"),
                    _ => {}
                }
                label_accessible(&scale, setting.label);
                lock_wheel(&scale);
                let value = line("", &["setting-value", "tnum"]);
                value.set_xalign(1.0);
                value.set_valign(gtk::Align::Center);
                let stepper = value_stepper(&value, &scale, setting.label);
                let reset = icon_button("restart_alt", "Reset to default", true);
                reset.add_css_class("flat-icon");
                reset.add_css_class("reset");
                // Always there, so the slider never changes width; dimmed while there is nothing to reset.
                reset.set_sensitive(false);
                (Control::Slider { scale, value, reset, stepper }, false)
            }
        }
    }

    fn connect(&self) {
        let row = &self.0;
        let setting = row.setting;
        match &row.control {
            Control::Switch(switch) => {
                let weak = Rc::downgrade(&self.0);
                switch.connect_active_notify(move |switch| {
                    let Some(row) = weak.upgrade() else { return };
                    if row.suppress.get() { return; }
                    let wanted = switch.is_active();
                    if let Some(confirm) = row.setting.confirm {
                        // Put the switch back until the user agrees. Not from inside this notification: GTK holds
                        // the change back until the handler returns, so it would arrive unguarded, open a second
                        // dialog for the old value and replace this one (the OK then sent the old value).
                        let this = SettingRow(row.clone());
                        {
                            let this = this.clone();
                            let previous = row.current.get();
                            glib::idle_add_local_once(move || this.show(previous));
                        }
                        row.store.confirm("warning", confirm.title, confirm.body, confirm.action, move || {
                            this.show(Some(Value::Bool(wanted)));
                            this.commit(Value::Bool(wanted));
                        });
                        return;
                    }
                    SettingRow(row).commit(Value::Bool(wanted));
                });
            }
            Control::Segments(segments) => {
                let weak = Rc::downgrade(&self.0);
                segments.on_change(move |value| {
                    if let Some(row) = weak.upgrade() { SettingRow(row).choose(value); }
                });
            }
            Control::Menu { drop, values } => {
                let weak = Rc::downgrade(&self.0);
                let values = values.clone();
                drop.connect_selected_notify(move |drop| {
                    let Some(row) = weak.upgrade() else { return };
                    if row.suppress.get() { return; }
                    let Some(value) = values.borrow().get(drop.selected() as usize).copied() else { return };
                    SettingRow(row).choose(value);
                });
            }
            Control::Slider { scale, value, reset, .. } => {
                let weak = Rc::downgrade(&self.0);
                let value_label = value.clone();
                scale.connect_value_changed(move |scale| {
                    let Some(row) = weak.upgrade() else { return };
                    let snapshot = row.store.snapshot();
                    // On a slider over stops, the position is a stop; the value is what it stands for.
                    let v = row.setting.slider(&snapshot).map(|spec| spec.value_at(scale.value())).unwrap_or(scale.value());
                    value_label.set_text(&row.setting.format(&snapshot, v));
                    if row.suppress.get() { return; }
                    let wire = if matches!(&row.setting.kind, Kind::Slider(s) if s.step < 1.0) { Value::Float(v) } else { Value::Int(v.round() as i64) };
                    SettingRow(row).commit(wire);
                });
                // Hold off external updates while the thumb is being dragged.
                let drag = gtk::GestureDrag::new();
                drag.set_propagation_phase(gtk::PropagationPhase::Capture);
                let weak = Rc::downgrade(&self.0);
                drag.connect_drag_begin(move |_, _, _| { if let Some(row) = weak.upgrade() { row.dragging.set(true); row.pressed.set(true); } });
                let weak = Rc::downgrade(&self.0);
                drag.connect_drag_end(move |_, _, _| {
                    if let Some(row) = weak.upgrade() { row.dragging.set(false); row.pressed.set(false); row.last_edit.set(Some(Instant::now())); }
                });
                // The release can be lost altogether (the pointer let go outside the window: on Hyprland with focus
                // on hover, for one). The slider then keeps following the pointer when it comes back. So the
                // pointer is watched: moving over the slider with no button held while it still counts as pressed
                // means the release was lost, and the drag is ended on the spot (going insensitive for a moment
                // resets the slider's own pointer handling).
                let motion = gtk::EventControllerMotion::new();
                motion.set_propagation_phase(gtk::PropagationPhase::Capture);
                let weak = Rc::downgrade(&self.0);
                let check = move |controller: &gtk::EventControllerMotion| {
                    let Some(row) = weak.upgrade() else { return };
                    if !row.pressed.get() { return; }
                    let buttons = gtk::gdk::ModifierType::BUTTON1_MASK | gtk::gdk::ModifierType::BUTTON2_MASK | gtk::gdk::ModifierType::BUTTON3_MASK;
                    if controller.current_event_state().intersects(buttons) { return; }
                    row.pressed.set(false);
                    row.dragging.set(false);
                    row.last_edit.set(Some(Instant::now()));
                    if let Some(scale) = controller.widget() {
                        if scale.is_sensitive() { scale.set_sensitive(false); scale.set_sensitive(true); }
                    }
                };
                let on_enter = check.clone();
                motion.connect_enter(move |controller, _, _| on_enter(controller));
                motion.connect_motion(move |controller, _, _| check(controller));
                scale.add_controller(motion);
                // A drag can also be cancelled rather than ended: the pointer let go outside the window, or the
                // window lost focus mid-drag (some window managers never send the release back). Without this
                // the row would keep ignoring the camera's value for good.
                let weak = Rc::downgrade(&self.0);
                drag.connect_cancel(move |_, _| {
                    if let Some(row) = weak.upgrade() { row.dragging.set(false); row.last_edit.set(Some(Instant::now())); }
                });
                scale.add_controller(drag);
                // A press with no drag (a click on the bar) ends without a drag-end: the click release says so.
                let click = gtk::GestureClick::new();
                click.set_propagation_phase(gtk::PropagationPhase::Capture);
                let weak = Rc::downgrade(&self.0);
                click.connect_released(move |_, _, _, _| { if let Some(row) = weak.upgrade() { row.pressed.set(false); } });
                let weak = Rc::downgrade(&self.0);
                click.connect_pressed(move |_, _, _, _| { if let Some(row) = weak.upgrade() { row.pressed.set(true); } });
                scale.add_controller(click);
                // When the window stops being the active one in the middle of a drag, end the drag outright: the
                // slider itself can otherwise stay half in a drag and pick up the pointer at an offset when it
                // comes back. Making it insensitive for a moment resets its own pointer handling.
                {
                    let weak = Rc::downgrade(&self.0);
                    let watched = Rc::new(Cell::new(false));
                    scale.connect_map(move |scale| {
                        if watched.replace(true) { return; }
                        let Some(window) = scale.root().and_then(|r| r.downcast::<gtk::Window>().ok()) else { return };
                        let (weak, scale) = (weak.clone(), scale.downgrade());
                        window.connect_is_active_notify(move |window| {
                            if window.is_active() { return; }
                            let (Some(row), Some(scale)) = (weak.upgrade(), scale.upgrade()) else { return };
                            if row.dragging.replace(false) | row.pressed.replace(false) {
                                row.last_edit.set(Some(Instant::now()));
                                if scale.is_sensitive() { scale.set_sensitive(false); scale.set_sensitive(true); }
                            }
                        });
                    });
                }
                let weak = Rc::downgrade(&self.0);
                let scale_reset = scale.clone();
                reset.connect_clicked(move |_| {
                    let Some(row) = weak.upgrade() else { return };
                    let snapshot = row.store.snapshot();
                    if let Some(spec) = row.setting.slider(&snapshot) {
                        if let Some(default) = spec.default { scale_reset.set_value(spec.position_of(default)); }
                    }
                });
            }
        }
        let _ = setting;
    }

    /// The user picked a choice; confirm first when the option asks for it.
    fn choose(&self, value: i64) {
        let row = &self.0;
        let snapshot = row.store.snapshot();
        let option = row.setting.options(&snapshot).into_iter().find(|o| o.value == value);
        if let Some(confirm) = option.as_ref().and_then(|o| o.confirm) {
            let this = self.clone();
            // Put the control back to the real state until the user agrees, once the change notification has
            // finished (see the switch above: a reset from inside it arrives unguarded).
            {
                let this = this.clone();
                let previous = row.current.get();
                glib::idle_add_local_once(move || this.show(previous));
            }
            row.store.confirm("warning", confirm.title, confirm.body, confirm.action, move || {
                this.show(Some(Value::Int(value)));
                this.commit(Value::Int(value));
            });
            return;
        }
        self.commit(Value::Int(value));
    }

    fn commit(&self, value: Value) {
        let row = &self.0;
        row.last_edit.set(Some(Instant::now()));
        // A setting the camera is slow to switch rests for a moment, so it cannot be flipped back and forth.
        if let Some(wait) = row.setting.cooldown {
            row.cooling.set(true);
            row.control_box.set_sensitive(false);
            row.control_box.set_tooltip_text(Some("The camera is switching. Wait a moment before changing it again."));
            let weak = Rc::downgrade(&self.0);
            glib::timeout_add_local_once(wait, move || {
                let Some(row) = weak.upgrade() else { return };
                row.cooling.set(false);
                row.control_box.set_tooltip_text(None);
                let snapshot = row.store.snapshot();
                SettingRow(row).apply(&snapshot);
            });
        }
        row.current.set(Some(value));
        let snapshot = row.store.snapshot();
        let message = self.describe(&snapshot, value);
        let setting = row.setting;
        let store = row.store.clone();
        let weak = Rc::downgrade(&self.0);
        let announce = !matches!(row.setting.kind, Kind::Slider(_)) || !row.dragging.get();
        row.store.write(setting, value, move |result| {
            match result {
                Ok(confirmed) => {
                    // Say what the camera confirmed, not just that the command went out.
                    match confirmed {
                        Some(actual) if actual.as_i64() == value.as_i64() => {
                            if announce { store.toast(&message); }
                        }
                        Some(actual) => {
                            // The camera kept something else: show that, not what was asked for.
                            if let Some(row) = weak.upgrade() {
                                let snapshot = row.store.snapshot();
                                let (wanted, got) = (SettingRow(row.clone()).display(&snapshot, value), SettingRow(row.clone()).display(&snapshot, actual));
                                row.current.set(Some(actual));
                                row.last_edit.set(Some(Instant::now()));
                                SettingRow(row.clone()).show(Some(actual));
                                let _ = wanted;
                                store.error(&format!("{} did not change: the camera is still on {got}.", setting.label));
                            }
                        }
                        None => {
                            if announce { store.toast(&message); }
                        }
                    }
                    // Once the hold on this control ends, show what the camera really has
                    // (it may have rounded or limited the value).
                    if let Some(row) = weak.upgrade() {
                        let (weak_again, store_again) = (Rc::downgrade(&row), row.store.clone());
                        glib::timeout_add_local_once(EDIT_HOLD + Duration::from_millis(250), move || {
                            if let Some(row) = weak_again.upgrade() { SettingRow(row).apply(&store_again.snapshot()); }
                        });
                    }
                }
                Err(_) => {
                    // Show what the camera really has.
                    if let Some(row) = weak.upgrade() {
                        row.last_edit.set(None);
                        SettingRow(row.clone()).apply(&row.store.snapshot());
                    }
                }
            }
        });
    }

    /// A value as the row shows it ("On", "Face", "1/30 s").
    fn display(&self, snapshot: &Snapshot, value: Value) -> String {
        let setting = self.0.setting;
        match &setting.kind {
            Kind::Toggle => if value.as_bool() { "on" } else { "off" }.to_string(),
            Kind::Slider(_) => setting.format(snapshot, value.as_f64()),
            _ => setting.options(snapshot).into_iter().find(|o| o.value == value.as_i64()).map(|o| o.label.to_string()).unwrap_or_default(),
        }
    }

    fn describe(&self, snapshot: &Snapshot, value: Value) -> String {
        let setting = self.0.setting;
        match &setting.kind {
            Kind::Toggle => format!("{} {}", setting.label, if value.as_bool() { "on" } else { "off" }),
            Kind::Slider(_) => format!("{} set to {}", setting.label, setting.format(snapshot, value.as_f64())),
            _ => {
                let label = setting.options(snapshot).into_iter().find(|o| o.value == value.as_i64()).map(|o| o.label).unwrap_or("");
                format!("{}: {}", setting.label, label)
            }
        }
    }

    /// Show a value in the control without sending anything.
    fn show(&self, value: Option<Value>) {
        let row = &self.0;
        row.suppress.set(true);
        match &row.control {
            Control::Switch(switch) => switch.set_active(value.is_some_and(Value::as_bool)),
            Control::Segments(segments) => segments.select(value.map(Value::as_i64)),
            Control::Menu { drop, values } => {
                let position = value.and_then(|v| values.borrow().iter().position(|x| *x == v.as_i64()));
                drop.set_selected(position.map(|p| p as u32).unwrap_or(gtk::INVALID_LIST_POSITION));
            }
            Control::Slider { scale, value: label, reset, .. } => {
                if let Some(v) = value {
                    let snapshot = row.store.snapshot();
                    let position = row.setting.slider(&snapshot).map(|spec| spec.position_of(v.as_f64())).unwrap_or(v.as_f64());
                    scale.set_value(position);
                    label.set_text(&row.setting.format(&snapshot, v.as_f64()));
                    // Lit when not at the default (on a slider over stops, when not at the default's stop).
                    let spec = row.setting.slider(&snapshot);
                    let off_default = spec.as_ref().and_then(|s| s.default.map(|d| (s.position_of(d) - s.position_of(v.as_f64())).abs() > f64::EPSILON)).unwrap_or(false);
                    reset.set_sensitive(off_default);
                } else {
                    label.set_text("—");
                    reset.set_sensitive(false);
                }
            }
        }
        row.suppress.set(false);
    }

    /// Bring the row in line with a new snapshot.
    fn apply(&self, snapshot: &Snapshot) {
        let row = &self.0;
        let setting = row.setting;
        let present = setting.present(snapshot);
        row.root.set_visible(present);
        if !present { return; }

        let blockers = setting.blockers(snapshot);
        let blocked = !blockers.is_empty();
        row.control_box.set_sensitive(!blocked && !row.cooling.get());
        if let Control::Slider { stepper, .. } = &row.control { stepper.set_sensitive(!blocked && !row.cooling.get()); }
        if blocked { row.root.add_css_class("blocked"); } else { row.root.remove_css_class("blocked"); }
        row.reason.set_visible(blocked);
        if let Some(first) = blockers.first() { row.reason_text.set_text(first); }

        self.sync_structure(snapshot);

        // While the thumb is held, and for a moment after a change, the camera's value does not move it. A drag
        // that has gone quiet for a while is over, whatever the pointer said, so a lost release cannot freeze it.
        let recent = row.last_edit.get().is_some_and(|at| at.elapsed() < EDIT_HOLD);
        let held = row.dragging.get() && row.last_edit.get().is_some_and(|at| at.elapsed() < Duration::from_secs(4));
        if row.dragging.get() && !held { row.dragging.set(false); }
        let editing = held || recent;
        if !editing {
            let (value, origin) = match setting.reading(snapshot) { Some((v, o)) => (Some(v), o), None => (None, Origin::Unknown) };
            row.current.set(value);
            self.show(value);
            self.update_hint(snapshot, value);
            self.show_origin(origin);
        }
    }

    /// Label where the value came from. A camera-confirmed value needs no label.
    fn show_origin(&self, origin: Origin) {
        let row = &self.0;
        // Only an unknown state is worth a label. A value the app remembers just shows, without comment.
        row.unknown.set_visible(origin == Origin::Unknown);
        match origin {
            Origin::Camera | Origin::Saved => {}
            Origin::Unknown => {
                row.unknown.set_text("Unknown");
                row.unknown.remove_css_class("neutral");
                row.unknown.set_tooltip_text(Some("The camera did not report this, and this app has no saved value. The control shows a starting position, not the camera's real state."));
            }
        }
    }

    /// Options, slider range and info text can change with the driver.
    fn sync_structure(&self, snapshot: &Snapshot) {
        let row = &self.0;
        let setting = row.setting;
        match &row.control {
            Control::Segments(segments) => {
                let options = setting.options(snapshot);
                let key: Vec<(i64, String)> = options.iter().map(|o| (o.value, o.label.to_string())).collect();
                if *row.shown_options.borrow() != key {
                    segments.set_options(&options);
                    *row.shown_options.borrow_mut() = key;
                }
            }
            Control::Menu { drop, values } => {
                let options = setting.options(snapshot);
                let key: Vec<(i64, String)> = options.iter().map(|o| (o.value, o.label.to_string())).collect();
                if *row.shown_options.borrow() != key {
                    row.suppress.set(true);
                    let labels: Vec<&str> = options.iter().map(|o| o.label).collect();
                    drop.set_model(Some(&gtk::StringList::new(&labels)));
                    *values.borrow_mut() = options.iter().map(|o| o.value).collect();
                    row.suppress.set(false);
                    *row.shown_options.borrow_mut() = key;
                }
            }
            Control::Slider { scale, .. } => {
                if let Some(spec) = setting.slider(snapshot) {
                    // A slider over stops is rebuilt when its stops change (the shutter's, with the frame rate).
                    let range = (spec.min, spec.max, if spec.stops.is_empty() { spec.step } else { 1000.0 + spec.stops.len() as f64 });
                    let shown = row.shown_range.get();
                    if shown.0.is_nan() || shown != range {
                        row.suppress.set(true);
                        if spec.stops.is_empty() {
                            scale.set_range(spec.min, spec.max.max(spec.min + spec.step));
                            scale.adjustment().set_step_increment(spec.step);
                            scale.adjustment().set_page_increment((spec.max - spec.min) / 10.0);
                        } else {
                            scale.set_range(0.0, (spec.stops.len().max(2) - 1) as f64);
                            scale.adjustment().set_step_increment(1.0);
                            scale.adjustment().set_page_increment(3.0);
                        }
                        mark_slider(scale, &spec);
                        row.suppress.set(false);
                        row.shown_range.set(range);
                    }
                }
            }
            Control::Switch(_) => {}
        }
        // The info popover
        let paragraphs = self.info_paragraphs(snapshot);
        let key = paragraphs.join("\n");
        if *row.shown_info.borrow() != key {
            while let Some(child) = row.info_slot.first_child() { row.info_slot.remove(&child); }
            if !paragraphs.is_empty() { row.info_slot.append(&info_button(setting.label, &paragraphs)); }
            *row.shown_info.borrow_mut() = key;
        }
    }

    fn info_paragraphs(&self, snapshot: &Snapshot) -> Vec<String> {
        let setting = self.0.setting;
        let mut out = Vec::new();
        if setting.density_summary_hidden(self.0.density) { out.push(setting.summary.to_string()); }
        if !setting.info.is_empty() { out.push(setting.info.to_string()); }
        if let Kind::Choice(options) = &setting.kind {
            let described: Vec<String> = options.iter().filter(|o| !o.hint.is_empty()).map(|o| format!("{}: {}.", o.label, o.hint)).collect();
            if !described.is_empty() { out.push(described.join(" ")); }
        }
        if let Some(spec) = setting.slider(snapshot) {
            let range = format!("Range {} to {}", spec.fmt.format(spec.min), spec.fmt.format(spec.max));
            out.push(match spec.default {
                Some(default) => format!("{range}. Default {}.", spec.fmt.format(default)),
                None => format!("{range}."),
            });
        }
        if !setting.deps.is_empty() {
            let needs: Vec<&str> = setting.deps.iter().map(|d| d.reason).collect();
            out.push(format!("Available when: {}", needs.join(" ")));
        }
        if setting.readback == Readback::LastSent {
            out.push("The camera can't report this one, so it shows what you last chose here.".into());
        }
        if setting.experimental { out.push("Experimental: not fully tested on this camera.".into()); }
        if out.is_empty() { return out; }
        out
    }

    /// Show the selected option's hint under the title.
    fn update_hint(&self, snapshot: &Snapshot, value: Option<Value>) {
        let row = &self.0;
        if row.density != Density::Full { return; }
        if row.setting.id == "obsbot:mic_disabled" && crate::backend::registry::microphone_waits_for_restart(snapshot) {
            row.summary.set_text("The camera is restarting to make the change…");
        } else if let (Kind::Choice(_), Some(value)) = (&row.setting.kind, value) {
            let hint = row.setting.options(snapshot).into_iter().find(|o| o.value == value.as_i64()).map(|o| o.hint).unwrap_or("");
            row.summary.set_text(if hint.is_empty() { row.setting.summary } else { hint });
        } else {
            row.summary.set_text(row.setting.summary);
        }
    }
}

fn row_alive() -> bool { true }

impl Control {
    fn widget(&self) -> gtk::Widget {
        match self {
            Control::Switch(w) => w.clone().upcast(),
            Control::Segments(s) => s.widget().clone().upcast(),
            Control::Menu { drop, .. } => drop.clone().upcast(),
            Control::Slider { scale, reset, .. } => {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                line.append(scale);
                line.append(reset);
                line.upcast()
            }
        }
    }
}

trait SummaryVisibility { fn density_summary_hidden(&self, density: Density) -> bool; }
impl SummaryVisibility for Setting {
    /// In the slim window the summary is not shown inline, so it moves into the info popover.
    fn density_summary_hidden(&self, density: Density) -> bool { density == Density::Slim }
}

#[cfg(test)]
mod tests {
    use crate::backend::registry::Group;

    /// Every icon the UI asks for must exist in the bundled font subset, or it
    /// would draw as a question mark. Names are read from the source so a typo
    /// fails here instead of on someone's screen.
    #[test]
    fn every_icon_used_in_the_ui_is_in_the_font() {
        let mut names: Vec<String> = Group::ALL.iter().map(|g| g.icon().to_string()).collect();
        names.extend(super::super::full::page_icons().into_iter().map(String::from));
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let Ok(source) = std::fs::read_to_string(entry.path()) else { continue };
            for call in ["icon(\"", "icon_button(\"", "icon_text_button(\"", "set_icon(&", "show_message(\"", "confirm(\""] {
                for (at, _) in source.match_indices(call) {
                    let rest = &source[at + call.len()..];
                    // For set_icon the name is the literal after the label argument.
                    let rest = if call.starts_with("set_icon") { rest.split_once(", \"").map(|x| x.1).unwrap_or("") } else { rest };
                    // For confirm and the others, the first literal is the icon.
                    if let Some((name, _)) = rest.split_once('"') {
                        if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()) { names.push(name.to_string()); }
                    }
                }
            }
        }
        assert!(names.len() > 20, "the scan found only {} icon names", names.len());
        for name in names {
            assert!(super::super::icons::glyph(&name).is_some(), "icon `{name}` is not in the font; add it to assets/icon-names.txt and run tools/subset-fonts.py");
        }
    }

    #[test]
    fn every_listed_icon_resolves() {
        for name in super::super::icons::NAMES { assert!(super::super::icons::glyph(name).is_some()); }
    }
}

#[cfg(test)]
mod summary_tests {
    use super::short_summary;

    #[test]
    fn long_summaries_keep_only_a_short_first_sentence() {
        assert_eq!(short_summary("Short and sweet."), ("Short and sweet.".to_string(), false));
        let long = "The camera follows you only within the limits. A limit that is off lets tracking use the full range on that axis, and more words follow here.";
        assert_eq!(short_summary(long), ("The camera follows you only within the limits.".to_string(), true));
        let one = "One very long sentence without any full stop in the middle that keeps going and going past eighty characters";
        assert_eq!(short_summary(one), (String::new(), true));
    }
}


/// A level meter for the camera's microphone, under its switch: shown while the microphone is on, and measuring
/// only while it is shown (the sound is measured here and not kept).
pub fn mic_meter(store: &Store) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("mic-meter");
    let glyph = icon("mic");
    glyph.add_css_class("small-icon");
    let bar = gtk::LevelBar::for_interval(0.0, 1.0);
    bar.set_mode(gtk::LevelBarMode::Continuous);
    bar.set_hexpand(true);
    bar.set_valign(gtk::Align::Center);
    // Its own colours (below), not the theme's low/high/full steps.
    for name in ["low", "high", "full"] { bar.remove_offset_value(Some(name)); }
    label_accessible(&bar, "Microphone level");
    let waiting = text("Waiting for the microphone…", &["small", "dim"]);
    waiting.set_visible(false);
    row.append(&glyph);
    row.append(&bar);
    row.append(&waiting);
    row.set_visible(false);

    let meter: Rc<RefCell<Option<crate::backend::audio::Meter>>> = Rc::default();
    let on = Rc::new(Cell::new(false));
    // Run it while it can be seen and the microphone is on.
    let sync: Rc<dyn Fn()> = {
        let (row, meter, on) = (row.downgrade(), meter.clone(), on.clone());
        Rc::new(move || {
            let Some(row) = row.upgrade() else { return };
            row.set_visible(on.get());
            let wanted = on.get() && row.is_mapped();
            if wanted && meter.borrow().is_none() { *meter.borrow_mut() = crate::backend::audio::start_meter(); }
            if !wanted { meter.borrow_mut().take(); }
        })
    };
    {
        let (on, sync) = (on.clone(), sync.clone());
        let setting = crate::backend::registry::find("obsbot:mic_disabled");
        let show = move |snapshot: &Snapshot| -> bool {
            // Shown unless the microphone is disabled: with nothing known, the meter itself shows whether sound comes.
            let now = setting.and_then(|s| s.reading(snapshot)).is_none_or(|(v, _)| !v.as_bool());
            if on.replace(now) != now { sync(); }
            true
        };
        show(&store.snapshot());
        store.subscribe(show);
    }
    { let sync = sync.clone(); row.connect_map(move |_| sync()); }
    { let meter = meter.clone(); row.connect_unmap(move |_| { meter.borrow_mut().take(); }); }
    // Draw the newest level, falling back slowly like a real meter. Without a meter (the microphone's sound card
    // has not come back yet after switching on), try again every two seconds.
    {
        let (bar, meter, row_weak, waiting, on) = (bar.downgrade(), meter.clone(), row.downgrade(), waiting.downgrade(), on.clone());
        let tries = Rc::new(Cell::new(0u32));
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let (Some(bar), Some(row)) = (bar.upgrade(), row_weak.upgrade()) else { return glib::ControlFlow::Break };
            let missing = on.get() && row.is_mapped() && meter.borrow().is_none();
            if missing && tries.get() % 50 == 0 { *meter.borrow_mut() = crate::backend::audio::start_meter(); }
            tries.set(if missing { tries.get() + 1 } else { 0 });
            let missing = on.get() && meter.borrow().is_none();
            bar.set_visible(!missing);
            if let Some(waiting) = waiting.upgrade() { waiting.set_visible(missing); }
            let fresh = meter.borrow().as_ref().map(|m| m.levels.try_iter().fold(0.0f64, f64::max));
            let shown = bar.value();
            bar.set_value(match fresh { Some(level) if level >= shown => level, Some(_) => (shown - 0.04).max(0.0), None => 0.0 });
            glib::ControlFlow::Continue
        });
    }
    row
}
