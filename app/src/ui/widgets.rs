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

/// An "i" button with a popover. Used instead of long help text in rows.
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
    button.connect_clicked(move |_| target.popup());
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
            if !option.hint.is_empty() { button.set_tooltip_text(Some(option.hint)); }
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
    Slider { scale: gtk::Scale, value: gtk::Label, reset: gtk::Button },
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
    dragging: Cell<bool>,
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
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
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
        unknown.set_tooltip_text(Some("The camera can't report this setting, and you haven't changed it in this app yet. The control shows a starting position, not the camera's real state."));

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);
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
            if let Control::Slider { value, .. } = &control { head.insert_child_after(value, Some(&titles)); }
            root.append(&head);
            root.append(&control_box);
        }
        root.append(&reason);

        let row = Rc::new(Row {
            setting, store: store.clone(), root, control, control_box, summary, reason, reason_text, info_slot, unknown,
            density, suppress: Cell::new(false), last_edit: Cell::new(None), dragging: Cell::new(false),
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
                let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, spec.min, spec.max, spec.step.max(f64::EPSILON));
                scale.set_draw_value(false);
                scale.set_hexpand(true);
                scale.set_round_digits(if spec.step < 1.0 { 2 } else { 0 });
                for (at, label) in &spec.ticks { scale.add_mark(*at, gtk::PositionType::Bottom, Some(label)); }
                label_accessible(&scale, setting.label);
                lock_wheel(&scale);
                let value = line("", &["setting-value", "tnum"]);
                value.set_xalign(1.0);
                value.set_valign(gtk::Align::Center);
                let reset = icon_button("restart_alt", "Reset to default", true);
                reset.add_css_class("flat-icon");
                reset.set_visible(false);
                (Control::Slider { scale, value, reset }, false)
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
                    SettingRow(row).commit(Value::Bool(switch.is_active()));
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
            Control::Slider { scale, value, reset } => {
                let weak = Rc::downgrade(&self.0);
                let value_label = value.clone();
                scale.connect_value_changed(move |scale| {
                    let Some(row) = weak.upgrade() else { return };
                    let v = scale.value();
                    let snapshot = row.store.snapshot();
                    value_label.set_text(&row.setting.format(&snapshot, v));
                    if row.suppress.get() { return; }
                    let wire = if matches!(&row.setting.kind, Kind::Slider(s) if s.step < 1.0) { Value::Float(v) } else { Value::Int(v.round() as i64) };
                    SettingRow(row).commit(wire);
                });
                // Hold off external updates while the thumb is being dragged.
                let drag = gtk::GestureDrag::new();
                drag.set_propagation_phase(gtk::PropagationPhase::Capture);
                let weak = Rc::downgrade(&self.0);
                drag.connect_drag_begin(move |_, _, _| { if let Some(row) = weak.upgrade() { row.dragging.set(true); } });
                let weak = Rc::downgrade(&self.0);
                drag.connect_drag_end(move |_, _, _| {
                    if let Some(row) = weak.upgrade() { row.dragging.set(false); row.last_edit.set(Some(Instant::now())); }
                });
                scale.add_controller(drag);
                let weak = Rc::downgrade(&self.0);
                let scale_reset = scale.clone();
                reset.connect_clicked(move |_| {
                    let Some(row) = weak.upgrade() else { return };
                    let snapshot = row.store.snapshot();
                    if let Some(default) = row.setting.slider(&snapshot).and_then(|s| s.default) { scale_reset.set_value(default); }
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
            // Put the control back to the real state until the user agrees.
            this.show(row.current.get());
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
                            if announce { store.toast(&format!("{message} (confirmed by the camera)")); }
                        }
                        Some(actual) => {
                            // The camera kept something else: show that, not what was asked for.
                            if let Some(row) = weak.upgrade() {
                                let snapshot = row.store.snapshot();
                                let (wanted, got) = (SettingRow(row.clone()).display(&snapshot, value), SettingRow(row.clone()).display(&snapshot, actual));
                                row.current.set(Some(actual));
                                row.last_edit.set(Some(Instant::now()));
                                SettingRow(row.clone()).show(Some(actual));
                                store.error(&format!("{}: the camera reports {got}, not {wanted}.", setting.label));
                            }
                        }
                        None => {
                            if announce {
                                // Only say so where no read-back exists at all.
                                let note = if setting.readback == Readback::LastSent { " (sent; the camera can't confirm this)" } else { "" };
                                store.toast(&format!("{message}{note}"));
                            }
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
            Control::Slider { scale, value: label, reset } => {
                if let Some(v) = value {
                    scale.set_value(v.as_f64());
                    let snapshot = row.store.snapshot();
                    label.set_text(&row.setting.format(&snapshot, v.as_f64()));
                    let default = row.setting.slider(&snapshot).and_then(|s| s.default);
                    reset.set_visible(default.is_some_and(|d| (d - v.as_f64()).abs() > f64::EPSILON));
                } else {
                    label.set_text("—");
                    reset.set_visible(false);
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
        row.control_box.set_sensitive(!blocked);
        if blocked { row.root.add_css_class("blocked"); } else { row.root.remove_css_class("blocked"); }
        row.reason.set_visible(blocked);
        if let Some(first) = blockers.first() { row.reason_text.set_text(first); }

        self.sync_structure(snapshot);

        let editing = row.dragging.get() || row.last_edit.get().is_some_and(|at| at.elapsed() < EDIT_HOLD);
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
        row.unknown.set_visible(origin != Origin::Camera);
        match origin {
            Origin::Camera => {}
            Origin::Saved => {
                row.unknown.set_text("Saved");
                row.unknown.add_css_class("neutral");
                row.unknown.set_tooltip_text(Some("The last value this app sent. The camera has not confirmed it, and it may differ if you changed it elsewhere."));
            }
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
                    let range = (spec.min, spec.max, spec.step);
                    let shown = row.shown_range.get();
                    if shown.0.is_nan() || shown != range {
                        row.suppress.set(true);
                        scale.set_range(spec.min, spec.max.max(spec.min + spec.step));
                        scale.adjustment().set_step_increment(spec.step);
                        scale.adjustment().set_page_increment((spec.max - spec.min) / 10.0);
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
            out.push("The camera does not report this back. The app shows the last value it sent, labelled Saved, or Unknown if it has none. It may differ if you change it in another program.".into());
        } else if matches!(setting.source, crate::backend::registry::Source::Custom { .. }) {
            out.push("Shown from the camera's own report. If the camera can't be read, the row says Unknown (or Saved, for a value this app sent earlier).".into());
        }
        if setting.experimental { out.push("Experimental: not fully verified on this camera.".into()); }
        if out.is_empty() { return out; }
        out
    }

    /// Show the selected option's hint under the title.
    fn update_hint(&self, snapshot: &Snapshot, value: Option<Value>) {
        let row = &self.0;
        if row.density != Density::Full { return; }
        if let (Kind::Choice(_), Some(value)) = (&row.setting.kind, value) {
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
