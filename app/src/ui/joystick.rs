//! Gimbal joystick.
//!
//! The old pad sent two blocking `v4l2-ctl` calls from every pointer event on
//! the UI thread, so a drag stalled the window and a release could be lost.
//! This one:
//!  * tracks the pointer as an absolute position inside the ring (no offset jump);
//!  * turns it into speeds with a dead zone and a gentle curve;
//!  * sends at most 25 updates a second, and only when the value changed,
//!    through the camera worker (newest value wins, one process for both axes);
//!  * always sends a stop on release, cancel, focus loss, hide and close;
//!  * is operable from the keyboard with the arrow keys.

use super::gimbal_model::GimbalModel;
use super::store::Store;
use super::{theme, widgets};
use crate::backend::{self, current_config, run_v4l2, update_config};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const KEY: &str = "gimbal-speed";
const POSITION_KEY: &str = "gimbal-position";
const SEND_INTERVAL: Duration = Duration::from_millis(40);
const DEAD_ZONE: f64 = 0.08;
const KEYBOARD_DEFLECTION: f64 = 0.55;
/// The most the speed may rise in one send (40 ms): full speed takes about a fifth of a second.
/// Slowing down is never limited.
const RAMP_PER_SEND: f64 = 0.2;
/// Without a recent position reading the gimbal is driven gently, as it may be close to an end.
const UNKNOWN_POSITION_CAP: f64 = 0.35;
/// A position reading older than this is no longer trusted.
const POSITION_FRESH: Duration = Duration::from_millis(1500);
/// Movement this large, against what the speed asked for, shows a direction is reversed.
const SIGN_EVIDENCE: f64 = 1.5;

struct State {
    store: Store,
    pad: gtk::DrawingArea,
    pan_value: gtk::Label,
    tilt_value: gtk::Label,
    hint: gtk::Label,
    /// Stick position, -1..1 on both axes, +y is up.
    stick: Cell<(f64, f64)>,
    velocity: Cell<(f64, f64)>,
    /// What the user is holding (pointer or keys); the stick follows it.
    engaged: Cell<bool>,
    sent: Cell<(i64, i64)>,
    sensitivity: Cell<f64>,
    keys: RefCell<HashSet<gtk::gdk::Key>>,
    send_timer: RefCell<Option<glib::SourceId>>,
    poll_timer: RefCell<Option<glib::SourceId>>,
    polling: Cell<bool>,
    last_position: Cell<Option<(f64, f64)>>,
    last_position_at: Cell<Option<Instant>>,
    /// The speed fractions after the limits and the ramp, as last sent.
    ramped: Cell<(f64, f64)>,
    /// +1 when a positive speed raises the angle, −1 if the camera turned out to be the other way.
    axis_sign: Cell<(f64, f64)>,
    send_failed: Arc<AtomicBool>,
    poll_error: RefCell<Option<String>>,
    animating: Cell<bool>,
    save_timer: RefCell<Option<glib::SourceId>>,
}

#[derive(Clone)]
pub struct Joystick(Rc<State>);

static FOLLOW_MIRROR: AtomicBool = AtomicBool::new(true);

/// Turn the stick the other way while Mirror image is on (or back), and save the choice.
pub fn set_follow_mirror(follow: bool) {
    FOLLOW_MIRROR.store(follow, Ordering::Relaxed);
    update_config(|c| c.joystick_follows_mirror = follow);
}

/// The pan speed to send for a left-right stick position, so that pushing the stick right moves the
/// picture you see to the right. On this camera a positive pan speed shows more of the scene's left
/// (checked by the user on the live video), so the plain direction is the opposite of the stick.
/// Mirror image flips the picture, which turns it the other way round again.
pub fn stick_pan(x: f64, mirrored: bool, follow: bool) -> f64 { if mirrored && follow { x } else { -x } }

/// Map a stick deflection (-1..1) to a fraction of full speed.
pub fn shape(deflection: f64, sensitivity: f64) -> f64 {
    let magnitude = deflection.abs();
    if magnitude <= DEAD_ZONE { return 0.0; }
    let scaled = ((magnitude - DEAD_ZONE) / (1.0 - DEAD_ZONE)).clamp(0.0, 1.0);
    (scaled.powf(1.6) * sensitivity).clamp(0.0, 1.0) * deflection.signum()
}

/// Move a speed fraction towards `target`, rising by at most `step` at a time. Slowing down, stopping
/// and a target of zero take effect at once.
pub fn ramp(current: f64, target: f64, step: f64) -> f64 {
    if target == 0.0 || (target.signum() == current.signum() && target.abs() <= current.abs()) { return target; }
    current + (target - current).clamp(-step, step)
}

/// Convert a speed fraction to the signed range a V4L2 speed control reports.
pub fn to_axis(fraction: f64, min: i64, max: i64) -> i64 {
    if fraction < 0.0 {
        ((fraction * min.abs() as f64).round() as i64).clamp(min.min(0), 0)
    } else {
        ((fraction * max as f64).round() as i64).clamp(0, max.max(0))
    }
}

impl Joystick {
    pub fn new(store: &Store) -> Joystick {
        let pad = gtk::DrawingArea::new();
        pad.add_css_class("pad");
        pad.set_content_width(200);
        pad.set_content_height(200);
        pad.set_hexpand(true);
        pad.set_vexpand(true);
        pad.set_halign(gtk::Align::Fill);
        pad.set_valign(gtk::Align::Fill);
        pad.set_focusable(true);
        pad.set_can_focus(true);
        pad.set_cursor_from_name(Some("grab"));
        widgets::label_accessible(&pad, "Gimbal joystick. Use the arrow keys to move the gimbal. Release the key to stop.");

        let pan_value = widgets::line("—", &["readout", "tnum"]);
        let tilt_value = widgets::line("—", &["readout", "tnum"]);
        let hint = widgets::text("Drag the stick, or focus it and use the arrow keys. The gimbal stops when you let go.", &["small", "dim"]);

        FOLLOW_MIRROR.store(current_config().joystick_follows_mirror, Ordering::Relaxed);
        let sensitivity = (current_config().joystick_sensitivity_percent.clamp(10, 200) as f64) / 100.0;
        let state = Rc::new(State {
            store: store.clone(), pad: pad.clone(), pan_value, tilt_value, hint,
            stick: Cell::new((0.0, 0.0)), velocity: Cell::new((0.0, 0.0)), engaged: Cell::new(false),
            sent: Cell::new((0, 0)), sensitivity: Cell::new(sensitivity), keys: RefCell::default(),
            send_timer: RefCell::default(), poll_timer: RefCell::default(), polling: Cell::new(false),
            last_position: Cell::new(None), last_position_at: Cell::new(None), ramped: Cell::new((0.0, 0.0)),
            axis_sign: Cell::new((1.0, 1.0)), send_failed: Arc::default(), poll_error: RefCell::default(), animating: Cell::new(false),
            save_timer: RefCell::default(),
        });
        let joystick = Joystick(state);
        joystick.wire();
        joystick
    }

    /// The pad with its readout and speed control.
    /// The pad with its readout and speed control, at full size.
    pub fn widget(&self) -> gtk::Box { self.widget_for(widgets::Density::Full) }

    /// The same, closer together for the slim window: a smaller model beside a smaller pad, and
    /// smaller buttons.
    pub fn widget_for(&self, density: widgets::Density) -> gtk::Box {
        let compact = density == widgets::Density::Slim;
        let state = &self.0;
        let column = gtk::Box::new(gtk::Orientation::Vertical, if compact { 8 } else { 12 });
        if compact { column.add_css_class("compact"); }

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = widgets::line("Gimbal", &["heading"]);
        title.set_hexpand(true);
        head.append(&title);
        head.append(&widgets::info_button("Gimbal joystick", &[
            "Drag the stick to turn the camera. The further you push, the faster it turns. Let go and it stops.".into(),
            "With the stick focused, the arrow keys also move the gimbal and stop it when released.".into(),
            "Speed limits the fastest the stick can go. The readout shows where the camera is pointing, in degrees.".into(),
            "With Mirror image on, the stick turns the camera the other way so the picture still moves the way you push. You can turn that off in Settings (the Device tab, under Appearance).".into(),
            "If the camera model turns the wrong way when the camera pans, you can reverse it there too.".into(),
        ]));
        column.append(&head);

        // A wireframe of the camera that follows the live pan and tilt, with the numbers on top.
        let model = GimbalModel::new(&state.store);
        if compact { model.widget().set_content_height(120); model.widget().set_content_width(120); }
        let readouts = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        readouts.set_halign(gtk::Align::Fill);
        readouts.set_valign(gtk::Align::End);
        readouts.set_margin_start(14);
        readouts.set_margin_end(14);
        readouts.set_margin_bottom(8);
        readouts.set_can_target(false);
        for (name, value, align) in [("Pan", &state.pan_value, gtk::Align::Start), ("Tilt", &state.tilt_value, gtk::Align::End)] {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 0);
            cell.set_hexpand(true);
            cell.set_halign(align);
            cell.append(&widgets::text(name, &["small", "dim"]));
            cell.append(value);
            readouts.append(&cell);
        }
        let stage = gtk::Overlay::new();
        stage.add_css_class("model-frame");
        stage.set_child(Some(model.widget()));
        stage.add_overlay(&readouts);
        if compact {
            // Side by side: the model takes what the pad leaves.
            state.pad.set_content_width(124);
            state.pad.set_content_height(124);
            state.pad.set_hexpand(false);
            state.pad.set_vexpand(false);
            state.pad.set_halign(gtk::Align::Center);
            state.pad.set_valign(gtk::Align::Center);
            stage.set_hexpand(true);
            let both = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            both.append(&stage);
            both.append(&state.pad);
            column.append(&both);
        } else {
            column.append(&stage);
            column.append(&state.pad);
        }

        column.append(&self.step_row(compact));

        let speed = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        speed.append(&widgets::line("Speed", &["setting-label"]));
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 10.0, 200.0, 5.0);
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_value(state.sensitivity.get() * 100.0);
        widgets::label_accessible(&scale, "Joystick speed limit");
        widgets::lock_wheel(&scale);
        let value = widgets::line(&format!("{:.0}%", state.sensitivity.get() * 100.0), &["setting-value", "tnum"]);
        value.set_xalign(1.0);
        value.set_width_chars(5);
        let this = self.clone();
        let value_label = value.clone();
        scale.connect_value_changed(move |scale| {
            let percent = scale.value().round();
            this.0.sensitivity.set(percent / 100.0);
            value_label.set_text(&format!("{percent:.0}%"));
            // Save once the slider settles rather than on every step.
            if let Some(id) = this.0.save_timer.borrow_mut().take() { id.remove(); }
            let timer_state = this.0.clone();
            let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
                timer_state.save_timer.borrow_mut().take();
                update_config(|config| config.joystick_sensitivity_percent = percent as u16);
            });
            *this.0.save_timer.borrow_mut() = Some(id);
        });
        speed.append(&scale);
        speed.append(&value);
        column.append(&speed);

        // Awake and asleep are how the camera is turned on and off, so they sit right here.
        if let Some(power) = backend::registry::find("obsbot:sleep") {
            column.append(widgets::SettingRow::new(&state.store, power, widgets::Density::Slim).widget());
        }
        column
    }

    /// Small, bounded turns: 1° at a time, for precise moves and careful first tests.
    fn step_row(&self, compact: bool) -> gtk::Box {
        // The groups sit side by side while there is room and wrap onto a second line when there is not,
        // so the words are never squeezed into dots.
        let row = gtk::Box::new(gtk::Orientation::Horizontal, if compact { 8 } else { 0 });
        row.set_halign(gtk::Align::Center);
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_homogeneous(false);
        flow.set_min_children_per_line(1);
        flow.set_max_children_per_line(3);
        flow.set_column_spacing(10);
        flow.set_row_spacing(6);
        flow.set_halign(gtk::Align::Center);
        flow.set_activate_on_single_click(false);
        // The slim window keeps it on one line; the full window lets it wrap.
        if !compact { row.append(&flow); }
        let add = |widget: &gtk::Widget| if compact { row.append(widget) } else { flow.insert(widget, -1) };
        let buttons: Rc<RefCell<Vec<gtk::Button>>> = Rc::default();
        for (axis, control) in [("Pan", "pan_absolute"), ("Tilt", "tilt_absolute")] {
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            group.append(&widgets::line(axis, &["small", "dim"]));
            for degrees in [-1_i64, 1] {
                let sign = if degrees < 0 { "−" } else { "+" };
                let button = gtk::Button::with_label(&format!("{sign}1°"));
                button.add_css_class("small");
                button.set_tooltip_text(Some(&format!("Turn {} by 1° ({sign}). The readout above shows the direction it went.", axis.to_lowercase())));
                widgets::label_accessible(&button, &format!("{axis} {sign}1 degree"));
                let this = self.clone();
                button.connect_clicked(move |_| {
                    let store = this.0.store.clone();
                    let snapshot = store.snapshot();
                    let Some(range) = snapshot.control(control).map(|c| (c.min, c.max)) else {
                        store.error("This camera does not offer pan and tilt steps.");
                        return;
                    };
                    let again = this.clone();
                    store.call_now(move |d| backend::step_gimbal(d, control, degrees, range.0, range.1), move |result| {
                        match result {
                            Ok(target) => again.0.store.toast(&format!("{axis} target {}", backend::angles::format(target))),
                            Err(error) => again.0.store.error(&format!("Could not turn {}: {error}", axis.to_lowercase())),
                        }
                        again.poll_position_once();
                    });
                });
                group.append(&button);
                buttons.borrow_mut().push(button);
            }
            add(group.upcast_ref());
        }
        // Centre: pan 0, tilt 0, straight away.
        let centre = widgets::icon_button("center_focus_strong", "Centre: turn the camera to pan 0°, tilt 0°", true);
        let this = self.clone();
        centre.connect_clicked(move |_| {
            let again = this.clone();
            this.0.store.clone().call_now(|d| backend::centre_gimbal(d), move |result| {
                match result {
                    Ok(()) => again.0.store.toast("Centring the camera"),
                    Err(error) => again.0.store.error(&format!("Could not centre the camera: {error}")),
                }
                again.poll_now();
            });
        });
        centre.set_halign(gtk::Align::Center);
        add(centre.upcast_ref());
        buttons.borrow_mut().push(centre);
        // Only usable when the driver offers working absolute pan and tilt.
        let weak: Vec<_> = buttons.borrow().iter().map(|b| b.downgrade()).collect();
        self.0.store.subscribe(move |snapshot| {
            let usable = ["pan_absolute", "tilt_absolute"].iter().all(|n| snapshot.control(n).is_some_and(|c| !c.disabled));
            let mut alive = false;
            for button in &weak {
                if let Some(button) = button.upgrade() { button.set_sensitive(usable); alive = true; }
            }
            alive
        });
        row
    }

    fn wire(&self) {
        let state = &self.0;
        let pad = &state.pad;

        let this = self.clone();
        pad.set_draw_func(move |_, cr, width, height| this.draw(cr, width, height));

        // Pointer
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let origin = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
        let this = self.clone();
        let begin_origin = origin.clone();
        drag.connect_drag_begin(move |_, x, y| {
            begin_origin.set((x, y));
            this.0.pad.grab_focus();
            this.0.pad.set_cursor_from_name(Some("grabbing"));
            this.engage();
            this.set_from_pointer(x, y);
        });
        let this = self.clone();
        let update_origin = origin.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let (x, y) = update_origin.get();
            this.set_from_pointer(x + dx, y + dy);
        });
        let this = self.clone();
        drag.connect_drag_end(move |_, _, _| { this.0.pad.set_cursor_from_name(Some("grab")); this.release(); });
        pad.add_controller(drag);

        // Keyboard
        let keys = gtk::EventControllerKey::new();
        let this = self.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            use gtk::gdk::Key;
            match key {
                Key::Left | Key::Right | Key::Up | Key::Down => {
                    this.0.keys.borrow_mut().insert(key);
                    this.engage();
                    this.set_from_keys();
                    glib::Propagation::Stop
                }
                Key::space | Key::Escape => { this.0.keys.borrow_mut().clear(); this.release(); glib::Propagation::Stop }
                _ => glib::Propagation::Proceed,
            }
        });
        let this = self.clone();
        keys.connect_key_released(move |_, key, _, _| {
            this.0.keys.borrow_mut().remove(&key);
            if this.0.keys.borrow().is_empty() { this.release(); } else { this.set_from_keys(); }
        });
        pad.add_controller(keys);

        // Stop whenever the pad can no longer be trusted to be held.
        let this = self.clone();
        pad.connect_unmap(move |_| { this.release(); this.stop_polling(); });
        let this = self.clone();
        pad.connect_map(move |_| this.start_polling());
        let focus = gtk::EventControllerFocus::new();
        let this = self.clone();
        focus.connect_leave(move |_| { this.0.keys.borrow_mut().clear(); this.release(); });
        pad.add_controller(focus);
        let this = self.clone();
        pad.connect_root_notify(move |pad| {
            if let Some(window) = pad.root().and_then(|r| r.downcast::<gtk::Window>().ok()) {
                let joystick = this.clone();
                window.connect_is_active_notify(move |window| { if !window.is_active() { joystick.release(); } });
            }
        });

        // The readouts show whatever the shared position feed says.
        let (pan_label, tilt_label) = (state.pan_value.downgrade(), state.tilt_value.downgrade());
        let weak = Rc::downgrade(&self.0);
        state.store.subscribe_gimbal(move |position| {
            let (Some(pan), Some(tilt)) = (pan_label.upgrade(), tilt_label.upgrade()) else { return false };
            match position {
                Some((p, t)) => {
                    // Avoid "-0.0°" for tiny negative values.
                    pan.set_text(&backend::angles::format(p));
                    tilt.set_text(&backend::angles::format(t));
                    pan.set_tooltip_text(None);
                    tilt.set_tooltip_text(None);
                }
                None => {
                    let reason = weak.upgrade().and_then(|s| s.poll_error.borrow().clone());
                    for label in [&pan, &tilt] { label.set_text("—"); label.set_tooltip_text(reason.as_deref()); }
                }
            }
            true
        });

        // Availability follows the camera's reported speed controls.
        let weak = Rc::downgrade(&self.0);
        state.store.subscribe(move |snapshot| {
            let Some(state) = weak.upgrade() else { return false };
            let available = snapshot.control("pan_speed").is_some_and(|c| !c.disabled)
                && snapshot.control("tilt_speed").is_some_and(|c| !c.disabled);
            state.pad.set_sensitive(available);
            state.hint.set_visible(!available);
            if !available {
                state.hint.set_text("Gimbal speed controls are not available on this video node.");
            }
            true
        });
    }

    // ── Geometry ──

    fn geometry(&self, width: i32, height: i32) -> (f64, f64, f64) {
        let size = f64::from(width.min(height));
        (f64::from(width) / 2.0, f64::from(height) / 2.0, (size / 2.0 - 14.0).max(20.0))
    }

    fn set_from_pointer(&self, x: f64, y: f64) {
        let (cx, cy, radius) = self.geometry(self.0.pad.width(), self.0.pad.height());
        let (mut dx, mut dy) = ((x - cx) / radius, -(y - cy) / radius);
        let length = (dx * dx + dy * dy).sqrt();
        if length > 1.0 { dx /= length; dy /= length; }
        self.0.stick.set((dx, dy));
        self.0.velocity.set((0.0, 0.0));
        self.0.pad.queue_draw();
    }

    fn set_from_keys(&self) {
        use gtk::gdk::Key;
        let keys = self.0.keys.borrow();
        let axis = |plus: Key, minus: Key| (f64::from(u8::from(keys.contains(&plus))) - f64::from(u8::from(keys.contains(&minus)))) * KEYBOARD_DEFLECTION;
        let (mut x, mut y) = (axis(Key::Right, Key::Left), axis(Key::Up, Key::Down));
        let length = (x * x + y * y).sqrt();
        if length > 1.0 { x /= length; y /= length; }
        self.0.stick.set((x, y));
        self.0.velocity.set((0.0, 0.0));
        self.0.pad.queue_draw();
    }

    // ── Output ──

    fn engage(&self) {
        let state = &self.0;
        if state.engaged.replace(true) { return; }
        state.animating.set(false);
        let weak = Rc::downgrade(state);
        let id = glib::timeout_add_local(SEND_INTERVAL, move || {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            Joystick(state).send_if_changed();
            glib::ControlFlow::Continue
        });
        *state.send_timer.borrow_mut() = Some(id);
    }

    fn desired(&self) -> (i64, i64) {
        let snapshot = self.0.store.snapshot();
        let (Some(pan), Some(tilt)) = (snapshot.control("pan_speed"), snapshot.control("tilt_speed")) else { return (0, 0) };
        let (x, y) = self.0.stick.get();
        let sensitivity = self.0.sensitivity.get();
        // The picture is flipped while Mirror image is on, so the stick turns the other way with it.
        let mirrored = backend::registry::find("obsbot:mirror_image")
            .and_then(|setting| setting.read(&snapshot)).is_some_and(|value| value.as_bool());
        let x = stick_pan(x, mirrored, FOLLOW_MIRROR.load(Ordering::Relaxed));
        let (fx, fy) = self.limited(shape(x, sensitivity), shape(y, sensitivity));
        let (rx, ry) = self.0.ramped.get();
        let (rx, ry) = (ramp(rx, fx, RAMP_PER_SEND), ramp(ry, fy, RAMP_PER_SEND));
        self.0.ramped.set((rx, ry));
        (to_axis(rx, pan.min, pan.max), to_axis(ry, tilt.min, tilt.max))
    }

    /// The speeds the stick asks for, held back near the ends of the range, and gentle when the
    /// position is not known.
    fn limited(&self, pan: f64, tilt: f64) -> (f64, f64) {
        let state = &self.0;
        let fresh = state.last_position_at.get().is_some_and(|at| at.elapsed() < POSITION_FRESH);
        let position = state.last_position.get().filter(|_| fresh);
        let Some((at_pan, at_tilt)) = position else {
            return (pan.clamp(-UNKNOWN_POSITION_CAP, UNKNOWN_POSITION_CAP), tilt.clamp(-UNKNOWN_POSITION_CAP, UNKNOWN_POSITION_CAP));
        };
        let (sign_pan, sign_tilt) = state.axis_sign.get();
        use backend::angles::{speed_allowance, PAN_RANGE, TILT_RANGE};
        (
            pan * speed_allowance(at_pan, pan.signum() * sign_pan, PAN_RANGE),
            tilt * speed_allowance(at_tilt, tilt.signum() * sign_tilt, TILT_RANGE),
        )
    }

    /// Compare what the gimbal did with what was asked: if a speed turned it the other way, the limits
    /// must work the other way round for that axis.
    fn learn_direction(&self, before: Option<(f64, f64)>, after: (f64, f64)) {
        let state = &self.0;
        let (Some(before), (sent_pan, sent_tilt)) = (before, state.sent.get()) else { return };
        let (mut sign_pan, mut sign_tilt) = state.axis_sign.get();
        let learn = |sent: i64, moved: f64, sign: &mut f64, name: &str| {
            if sent != 0 && moved.abs() >= SIGN_EVIDENCE {
                let observed = moved.signum() * (sent as f64).signum();
                if observed != *sign {
                    eprintln!("{name}: a positive speed turned the camera the other way; the limits follow that now.");
                    *sign = observed;
                }
            }
        };
        learn(sent_pan, after.0 - before.0, &mut sign_pan, "Pan");
        learn(sent_tilt, after.1 - before.1, &mut sign_tilt, "Tilt");
        state.axis_sign.set((sign_pan, sign_tilt));
    }

    fn send_if_changed(&self) {
        let want = self.desired();
        let camera = self.0.store.camera();
        // A failed send leaves the camera on the old speed, so try again rather than assume.
        if self.0.send_failed.swap(false, Ordering::Relaxed) { self.0.sent.set((i64::MIN, i64::MIN)); }
        if want != (0, 0) { camera.hold_motion(); } else { camera.release_motion(); }
        if want == self.0.sent.get() { return; }
        self.0.sent.set(want);
        let failed = self.0.send_failed.clone();
        camera.submit_keyed(KEY, move |device| {
            let command = format!("--set-ctrl=pan_speed={},tilt_speed={}", want.0, want.1);
            if let Err(error) = run_v4l2(device, &command) {
                eprintln!("Joystick update failed: {error}");
                failed.store(true, Ordering::Relaxed);
            }
        });
    }

    /// Let go: stop the gimbal now and let the stick spring home.
    pub fn release(&self) {
        let state = &self.0;
        if let Some(id) = state.send_timer.borrow_mut().take() { id.remove(); }
        let was_engaged = state.engaged.replace(false);
        let moving = state.sent.get() != (0, 0);
        state.keys.borrow_mut().clear();
        state.ramped.set((0.0, 0.0));
        state.store.camera().release_motion();
        if was_engaged || moving {
            state.sent.set((0, 0));
            let store = state.store.clone();
            // The urgent job runs before anything waiting and discards queued speed updates.
            let (sender, receiver) = async_channel::bounded::<Result<(), String>>(1);
            state.store.camera().submit_urgent(KEY, move |device| {
                let mut result = Err("not attempted".to_string());
                for attempt in 0..4 {
                    result = run_v4l2(device, "--set-ctrl=pan_speed=0,tilt_speed=0").map(|_| ());
                    if result.is_ok() { break; }
                    std::thread::sleep(Duration::from_millis(40 * (attempt + 1)));
                }
                let _ = sender.send_blocking(result);
            });
            glib::spawn_future_local(async move {
                if let Ok(Err(error)) = receiver.recv().await {
                    store.error(&format!("Could not confirm the gimbal stopped: {error}. Press Space to try again."));
                }
            });
            self.poll_position_once();
        }
        self.animate_home();
    }

    /// Block briefly so a stop is sent even while the app is closing.
    pub fn stop_now_blocking(&self) {
        let (sender, receiver) = std::sync::mpsc::channel::<()>();
        if let Some(id) = self.0.send_timer.borrow_mut().take() { id.remove(); }
        self.0.engaged.set(false);
        self.0.ramped.set((0.0, 0.0));
        self.0.store.camera().release_motion();
        self.0.store.camera().submit_urgent(KEY, move |device| {
            let _ = run_v4l2(device, "--set-ctrl=pan_speed=0,tilt_speed=0");
            let _ = sender.send(());
        });
        let _ = receiver.recv_timeout(Duration::from_millis(600));
    }

    // ── Spring-back animation ──

    fn animate_home(&self) {
        let state = &self.0;
        if state.animating.replace(true) { return; }
        let reduce = current_config().reduce_motion;
        if reduce {
            state.stick.set((0.0, 0.0));
            state.animating.set(false);
            state.pad.queue_draw();
            return;
        }
        let weak = Rc::downgrade(state);
        let last = Rc::new(Cell::new(0_i64));
        state.pad.add_tick_callback(move |_, clock| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            if !state.animating.get() { return glib::ControlFlow::Break; }
            let now = clock.frame_time();
            let dt = if last.get() == 0 { 1.0 / 60.0 } else { ((now - last.get()) as f64 / 1_000_000.0).clamp(0.001, 0.05) };
            last.set(now);
            // Damped spring with a slight overshoot.
            let (k, c) = (380.0, 24.0);
            let (mut x, mut y) = state.stick.get();
            let (mut vx, mut vy) = state.velocity.get();
            vx += (-k * x - c * vx) * dt;
            vy += (-k * y - c * vy) * dt;
            x += vx * dt;
            y += vy * dt;
            state.velocity.set((vx, vy));
            state.stick.set((x, y));
            state.pad.queue_draw();
            let settled = x.abs() < 0.003 && y.abs() < 0.003 && vx.abs() < 0.02 && vy.abs() < 0.02;
            if settled {
                state.stick.set((0.0, 0.0));
                state.velocity.set((0.0, 0.0));
                state.animating.set(false);
                state.pad.queue_draw();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    // ── Position readout ──

    /// Ask for the position now and keep asking: quickly while the camera is moving, slowly when it
    /// is still, and not at all when the pad is hidden or the window is in the background.
    fn start_polling(&self) {
        if self.0.polling.replace(true) { return; }
        self.poll_now();
    }

    fn stop_polling(&self) {
        self.0.polling.set(false);
        if let Some(id) = self.0.poll_timer.borrow_mut().take() { id.remove(); }
    }

    fn schedule_poll(&self, milliseconds: u64) {
        let state = &self.0;
        if let Some(id) = state.poll_timer.borrow_mut().take() { id.remove(); }
        let weak = Rc::downgrade(state);
        let id = glib::timeout_add_local_once(Duration::from_millis(milliseconds), move || {
            let Some(state) = weak.upgrade() else { return };
            state.poll_timer.borrow_mut().take();
            Joystick(state).poll_now();
        });
        *state.poll_timer.borrow_mut() = Some(id);
    }

    /// Read the position once; the answer decides when to ask again.
    fn poll_now(&self) {
        let state = &self.0;
        if !state.polling.get() { return; }
        let active = state.pad.root().and_then(|r| r.downcast::<gtk::Window>().ok()).is_some_and(|w| w.is_active());
        if !(active && state.pad.is_mapped()) {
            self.schedule_poll(1000);
            return;
        }
        if let Some(id) = state.poll_timer.borrow_mut().take() { id.remove(); }
        let this = self.clone();
        state.store.call(Some(POSITION_KEY), |device| crate::backend::get_gimbal_position(device), move |result| {
            let state = &this.0;
            let (position, moving) = match result {
                Ok(position) => {
                    let before = state.last_position.get();
                    let moved = before.is_none_or(|(p, t)| (p - position.0).abs() > 0.2 || (t - position.1).abs() > 0.2);
                    this.learn_direction(before, position);
                    state.last_position.set(Some(position));
                    state.last_position_at.set(Some(Instant::now()));
                    *state.poll_error.borrow_mut() = None;
                    (Some(position), moved)
                }
                Err(error) => {
                    state.last_position.set(None);
                    state.last_position_at.set(None);
                    *state.poll_error.borrow_mut() = Some(error);
                    (None, false)
                }
            };
            state.store.set_gimbal(position);
            // Quick while it moves (or while the stick is held); relaxed when it is still.
            let wait = if moving || state.engaged.get() { 150 } else { 700 };
            this.schedule_poll(wait);
        });
    }

    /// Read again right away (after a step or a release).
    fn poll_position_once(&self) {
        if self.0.polling.get() { self.poll_now(); }
    }

    // ── Drawing ──

    fn draw(&self, cr: &gtk::cairo::Context, width: i32, height: i32) {
        let state = &self.0;
        let (cx, cy, radius) = self.geometry(width, height);
        let (x, y) = state.stick.get();
        let engaged = state.engaged.get();
        let sensitive = state.pad.is_sensitive();
        theme::with(|p| {
            let (tr, tg, tb) = p.rgb("primary");
            let (or, og, ob) = p.rgb("outline_variant");
            let (sr, sg, sb) = p.rgb("surface_high");
            let (vr, vg, vb) = p.rgb("on_surface_variant");
            let tau = std::f64::consts::TAU;
            let alpha = if sensitive { 1.0 } else { 0.4 };

            // Disc
            cr.set_source_rgba(sr, sg, sb, alpha);
            cr.arc(cx, cy, radius, 0.0, tau);
            let _ = cr.fill();
            cr.set_line_width(2.0);
            cr.set_source_rgba(or, og, ob, alpha);
            cr.arc(cx, cy, radius, 0.0, tau);
            let _ = cr.stroke();
            // Dead zone
            cr.set_source_rgba(or, og, ob, 0.6 * alpha);
            cr.set_dash(&[3.0, 5.0], 0.0);
            cr.arc(cx, cy, radius * DEAD_ZONE, 0.0, tau);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            // Axes and direction marks
            cr.set_line_width(1.5);
            cr.set_source_rgba(or, og, ob, alpha);
            cr.move_to(cx - radius * 0.9, cy);
            cr.line_to(cx + radius * 0.9, cy);
            cr.move_to(cx, cy - radius * 0.9);
            cr.line_to(cx, cy + radius * 0.9);
            let _ = cr.stroke();
            cr.set_source_rgba(vr, vg, vb, 0.9 * alpha);
            for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                let (mx, my) = (cx + dx * radius * 0.84, cy + dy * radius * 0.84);
                cr.save().ok();
                cr.translate(mx, my);
                cr.rotate(dy.atan2(dx) + std::f64::consts::FRAC_PI_2);
                cr.move_to(0.0, -5.0);
                cr.line_to(5.0, 4.0);
                cr.line_to(-5.0, 4.0);
                cr.close_path();
                let _ = cr.fill();
                cr.restore().ok();
            }

            // Thumb
            let (px, py) = (cx + x * radius, cy - y * radius);
            if engaged {
                cr.set_source_rgba(tr, tg, tb, 0.20);
                cr.arc(cx, cy, radius, 0.0, tau);
                let _ = cr.fill();
                cr.set_source_rgba(tr, tg, tb, 0.55);
                cr.set_line_width(3.0);
                cr.move_to(cx, cy);
                cr.line_to(px, py);
                let _ = cr.stroke();
            }
            let thumb = if engaged { 30.0 } else { 26.0 };
            cr.set_source_rgba(tr, tg, tb, alpha);
            cr.arc(px, py, thumb, 0.0, tau);
            let _ = cr.fill();
            let (ar, ag, ab) = p.rgb("on_primary");
            cr.set_source_rgba(ar, ag, ab, 0.9 * alpha);
            cr.arc(px, py, 5.0, 0.0, tau);
            let _ = cr.fill();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stick_turns_the_other_way_with_a_mirrored_picture() {
        // Picture not mirrored: right on the stick is a negative pan speed, which shows more on the right.
        assert_eq!(stick_pan(0.5, false, true), -0.5);
        // Mirrored: the picture is flipped, so it is the other way round.
        assert_eq!(stick_pan(0.5, true, true), 0.5);
        // Switched off in Settings: the mirror is ignored.
        assert_eq!(stick_pan(0.5, true, false), -0.5);
        assert_eq!(stick_pan(0.0, true, true).abs(), 0.0);
    }

    #[test]
    fn dead_zone_stops_small_deflections() {
        assert_eq!(shape(0.05, 1.0), 0.0);
        assert_eq!(shape(-0.08, 1.0), 0.0);
        assert!(shape(0.2, 1.0) > 0.0);
    }

    #[test]
    fn curve_is_symmetric_monotonic_and_bounded() {
        let mut previous = 0.0;
        for i in 0..=100 {
            let d = f64::from(i) / 100.0;
            let value = shape(d, 1.0);
            assert!(value >= previous - 1e-12, "not monotonic at {d}");
            assert!((shape(-d, 1.0) + value).abs() < 1e-12);
            assert!(value <= 1.0);
            previous = value;
        }
        assert!((shape(1.0, 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn sensitivity_limits_top_speed_but_never_exceeds_full_scale() {
        assert!((shape(1.0, 0.5) - 0.5).abs() < 1e-12);
        assert_eq!(shape(1.0, 2.0), 1.0);
    }

    #[test]
    fn speed_rises_gradually_but_stops_at_once() {
        let mut speed = 0.0;
        let mut sends = 0;
        while speed < 1.0 { speed = ramp(speed, 1.0, RAMP_PER_SEND); sends += 1; assert!(sends < 20); }
        assert_eq!(sends, 5);
        assert_eq!(ramp(1.0, 0.0, RAMP_PER_SEND), 0.0);
        assert_eq!(ramp(1.0, 0.4, RAMP_PER_SEND), 0.4);
        // Reversing passes through zero step by step, never jumping from one way to the other.
        let mut speed = 1.0;
        let mut seen = vec![speed];
        for _ in 0..12 { speed = ramp(speed, -1.0, RAMP_PER_SEND); seen.push(speed); }
        assert!(seen.windows(2).all(|w| (w[1] - w[0]).abs() <= RAMP_PER_SEND + 1e-9), "{seen:?}");
        assert_eq!(*seen.last().unwrap(), -1.0);
    }

    #[test]
    fn axis_uses_the_device_range_for_each_direction() {
        // Tiny 2 Lite reports pan_speed -80..80 and tilt_speed -120..120.
        assert_eq!(to_axis(1.0, -80, 80), 80);
        assert_eq!(to_axis(-1.0, -80, 80), -80);
        assert_eq!(to_axis(0.5, -120, 120), 60);
        assert_eq!(to_axis(0.0, -120, 120), 0);
        // Asymmetric or one-sided ranges stay inside what the device allows.
        assert_eq!(to_axis(-1.0, -30, 80), -30);
        assert_eq!(to_axis(-1.0, 0, 80), 0);
    }
}
