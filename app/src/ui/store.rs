//! UI-side state shared by every window: the latest camera snapshot, the
//! worker handle, and a bridge that runs blocking camera work off the GTK
//! thread and delivers results back on it.

use crate::backend::registry::{Setting, Snapshot, Value};
use crate::backend::worker::Camera;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub struct Toaster {
    revealer: gtk::Revealer,
    label: gtk::Label,
    toast: gtk::Box,
    timer: RefCell<Option<glib::SourceId>>,
    /// How long the message showing stays, and whether the pointer is on it (which holds it open).
    duration: Cell<Duration>,
    hovered: Cell<bool>,
}

/// How long a message stays on screen, and how long after the pointer leaves it.
const TOAST_TIME: Duration = Duration::from_millis(5500);
const TOAST_ERROR_TIME: Duration = Duration::from_millis(8000);
const TOAST_AFTER_HOVER: Duration = Duration::from_millis(2500);

impl Toaster {
    /// A bottom-centre toast. Add `widget()` to the window's overlay.
    pub fn new() -> Rc<Toaster> {
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_max_width_chars(60);
        let toast = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toast.add_css_class("toast");
        label.set_hexpand(true);
        label.set_xalign(0.0);
        toast.append(&label);
        let close = super::widgets::icon_button("close", "Close", true);
        close.add_css_class("toast-close");
        close.set_valign(gtk::Align::Center);
        toast.append(&close);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        revealer.set_transition_duration(220);
        revealer.set_halign(gtk::Align::Center);
        revealer.set_valign(gtk::Align::End);
        revealer.set_margin_bottom(24);
        revealer.set_child(Some(&toast));
        let this = Rc::new(Toaster { revealer, label, toast, timer: RefCell::new(None),
            duration: Cell::new(TOAST_TIME), hovered: Cell::new(false) });
        // The pointer on the message holds it open; leaving it starts a short countdown again.
        let motion = gtk::EventControllerMotion::new();
        {
            let weak = Rc::downgrade(&this);
            motion.connect_enter(move |_, _, _| {
                let Some(this) = weak.upgrade() else { return };
                this.hovered.set(true);
                let pending = this.timer.borrow_mut().take();
                if let Some(id) = pending { id.remove(); }
            });
        }
        {
            let weak = Rc::downgrade(&this);
            motion.connect_leave(move |_| {
                let Some(this) = weak.upgrade() else { return };
                this.hovered.set(false);
                if this.revealer.reveals_child() { this.start_timer(TOAST_AFTER_HOVER); }
            });
        }
        this.toast.add_controller(motion);
        {
            let weak = Rc::downgrade(&this);
            close.connect_clicked(move |_| { if let Some(this) = weak.upgrade() { this.hide(); } });
        }
        this
    }

    fn hide(&self) {
        if let Some(id) = self.timer.borrow_mut().take() { id.remove(); }
        self.revealer.set_reveal_child(false);
    }

    fn start_timer(self: &Rc<Self>, after: Duration) {
        if let Some(id) = self.timer.borrow_mut().take() { id.remove(); }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(after, move || {
            let Some(this) = weak.upgrade() else { return };
            this.timer.borrow_mut().take();
            if !this.hovered.get() { this.revealer.set_reveal_child(false); }
        });
        *self.timer.borrow_mut() = Some(id);
    }

    pub fn widget(&self) -> &gtk::Revealer { &self.revealer }

    pub fn show(self: &Rc<Self>, message: &str, error: bool) {
        self.label.set_text(message);
        if error { self.toast.add_css_class("error"); } else { self.toast.remove_css_class("error"); }
        self.revealer.set_reveal_child(true);
        // Announce to assistive technology.
        self.label.update_property(&[gtk::accessible::Property::Label(message)]);
        self.duration.set(if error { TOAST_ERROR_TIME } else { TOAST_TIME });
        // While the pointer is on it, it stays; the countdown starts when the pointer leaves.
        if self.hovered.get() {
            if let Some(id) = self.timer.borrow_mut().take() { id.remove(); }
        } else {
            self.start_timer(self.duration.get());
        }
    }

    pub fn is_visible(&self) -> bool { self.revealer.is_mapped() }
}

/// In-window modal cards (confirmations). Works on every window manager,
/// unlike a second toplevel.
pub struct Dialogs {
    overlay: gtk::Overlay,
    scrim: gtk::Box,
}

impl Dialogs {
    pub fn new() -> Rc<Dialogs> {
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("scrim");
        scrim.set_visible(false);
        scrim.set_halign(gtk::Align::Fill);
        scrim.set_valign(gtk::Align::Fill);
        let overlay = gtk::Overlay::new();
        // Escape cancels.
        let keys = gtk::EventControllerKey::new();
        let target = scrim.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape && target.is_visible() {
                target.set_visible(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        scrim.add_controller(keys);
        Rc::new(Dialogs { overlay, scrim })
    }

    /// The overlay to use as the window's root; put the page in `set_child`.
    pub fn overlay(&self) -> &gtk::Overlay { &self.overlay }

    pub fn install(&self) { self.overlay.add_overlay(&self.scrim); }

    pub fn is_visible(&self) -> bool { self.overlay.is_mapped() }

    /// Ask before doing something with a physical effect.
    pub fn confirm(self: &Rc<Self>, icon: &str, title: &str, body: &str, action: &str, on_confirm: impl Fn() + 'static) {
        while let Some(child) = self.scrim.first_child() { self.scrim.remove(&child); }
        let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.add_css_class("dialog-card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.set_vexpand(true);
        card.set_size_request(340, -1);
        let glyph = super::widgets::icon(icon);
        glyph.add_css_class("dialog-icon");
        glyph.set_halign(gtk::Align::Start);
        let heading = super::widgets::text(title, &["title"]);
        let detail = super::widgets::text(body, &["body", "dim"]);
        detail.set_max_width_chars(44);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        buttons.set_margin_top(8);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("quiet");
        let go = gtk::Button::with_label(action);
        go.add_css_class("filled");
        buttons.append(&cancel);
        buttons.append(&go);
        card.append(&glyph);
        card.append(&heading);
        card.append(&detail);
        card.append(&buttons);
        self.scrim.append(&card);
        self.scrim.set_visible(true);
        go.grab_focus();
        let hide = {
            let this = self.clone();
            move || this.scrim.set_visible(false)
        };
        let hide_cancel = hide.clone();
        cancel.connect_clicked(move |_| hide_cancel());
        go.connect_clicked(move |_| { hide(); on_confirm(); });
    }
}

type Listener = Box<dyn Fn(&Snapshot) -> bool>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Connection {
    Connected,
    /// The USB device is present but Linux has no usable video node for it.
    NoVideoNode,
    Absent,
}

struct Inner {
    camera: RefCell<Camera>,
    connection: Cell<Connection>,
    snapshot: RefCell<Rc<Snapshot>>,
    /// How many reads in a row lost something the camera had answered before.
    read_misses: Cell<u32>,
    listeners: RefCell<Vec<Listener>>,
    toasters: RefCell<Vec<Rc<Toaster>>>,
    dialogs: RefCell<Vec<Rc<Dialogs>>>,
    refresh_timer: RefCell<Option<glib::SourceId>>,
    last_refresh: Cell<Option<Instant>>,
    gimbal: Cell<Option<(f64, f64)>>,
    /// The whole degrees shown, which only change when the reading moves clearly.
    gimbal_filter: RefCell<(crate::backend::angles::Steady, crate::backend::angles::Steady)>,
    gimbal_listeners: RefCell<Vec<Box<dyn Fn(Option<(f64, f64)>) -> bool>>>,
}

#[derive(Clone)]
pub struct Store(Rc<Inner>);

impl Store {
    pub fn new(camera: Camera, connection: Connection) -> Store {
        Store(Rc::new(Inner {
            camera: RefCell::new(camera),
            connection: Cell::new(connection),
            snapshot: RefCell::new(Rc::new(Snapshot::default())),
            read_misses: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
            toasters: RefCell::new(Vec::new()),
            dialogs: RefCell::new(Vec::new()),
            refresh_timer: RefCell::new(None),
            last_refresh: Cell::new(None),
            gimbal: Cell::new(None),
            gimbal_filter: RefCell::default(),
            gimbal_listeners: RefCell::new(Vec::new()),
        }))
    }

    pub fn camera(&self) -> Camera { self.0.camera.borrow().clone() }
    pub fn device(&self) -> String { self.0.camera.borrow().path().to_string() }
    pub fn connection(&self) -> Connection { self.0.connection.get() }

    /// Point the store at a (possibly different) camera and re-read it.
    pub fn set_camera(&self, camera: Camera, connection: Connection) {
        *self.0.camera.borrow_mut() = camera;
        self.0.connection.set(connection);
        if connection == Connection::Connected {
            self.refresh();
        } else {
            self.publish(Snapshot { error: Some("The camera is not connected.".into()), config: crate::backend::current_config(), ..Snapshot::default() });
        }
    }
    pub fn snapshot(&self) -> Rc<Snapshot> { self.0.snapshot.borrow().clone() }

    /// Register a callback for every new snapshot. Return `false` to unsubscribe
    /// (done automatically by rows when their widget has gone).
    pub fn subscribe(&self, listener: impl Fn(&Snapshot) -> bool + 'static) {
        self.0.listeners.borrow_mut().push(Box::new(listener));
    }

    fn publish(&self, mut snapshot: Snapshot) {
        // One missed answer from the camera must not turn every row to Unknown: keep what it said last
        // time, until it has stayed quiet for several reads in a row.
        {
            let previous = self.0.snapshot.borrow().clone();
            let mut misses = self.0.read_misses.get();
            let missed = (snapshot.status.is_none() && previous.status.is_some())
                || (snapshot.exposure_comp.is_none() && previous.exposure_comp.is_some())
                || (snapshot.focus_priority.is_none() && previous.focus_priority.is_some());
            if missed && misses < 4 {
                misses += 1;
                if snapshot.status.is_none() { snapshot.status = previous.status.clone(); }
                if snapshot.exposure_comp.is_none() { snapshot.exposure_comp = previous.exposure_comp; }
                if snapshot.focus_priority.is_none() { snapshot.focus_priority = previous.focus_priority; }
            } else if !missed {
                misses = 0;
            }
            self.0.read_misses.set(misses);
        }
        let snapshot = Rc::new(snapshot);
        *self.0.snapshot.borrow_mut() = snapshot.clone();
        self.0.last_refresh.set(Some(Instant::now()));
        // Listeners may subscribe while we iterate (rows being built); take the list first.
        let mut listeners = std::mem::take(&mut *self.0.listeners.borrow_mut());
        listeners.retain(|listener| listener(&snapshot));
        let mut added = std::mem::take(&mut *self.0.listeners.borrow_mut());
        listeners.append(&mut added);
        *self.0.listeners.borrow_mut() = listeners;
    }

    /// Run blocking camera work on the worker; `done` runs on the GTK thread.
    /// With a key, a newer call with the same key replaces one still waiting.
    pub fn call<T: Send + 'static>(
        &self,
        key: Option<&str>,
        job: impl FnOnce(&str) -> T + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) {
        let (sender, receiver) = async_channel::bounded::<T>(1);
        let run = move |device: &str| { let value = job(device); let _ = sender.send_blocking(value); };
        match key {
            Some(key) => self.camera().submit_keyed(key, run),
            None => self.camera().submit(run),
        }
        glib::spawn_future_local(async move {
            if let Ok(value) = receiver.recv().await { done(value); }
        });
    }

    /// Run camera work for something the user just pressed: ahead of the polling and reading that
    /// is waiting, so a button never has to wait for them.
    pub fn call_now<T: Send + 'static>(&self, job: impl FnOnce(&str) -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
        use std::sync::atomic::{AtomicU64, Ordering};
        crate::backend::usage::note_activity();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let (sender, receiver) = async_channel::bounded::<T>(1);
        let run = move |device: &str| { let value = job(device); let _ = sender.send_blocking(value); };
        // A key of its own, so it neither replaces nor is replaced by anything else.
        self.camera().submit_urgent(&format!("now-{}", NEXT.fetch_add(1, Ordering::Relaxed)), run);
        glib::spawn_future_local(async move {
            if let Ok(value) = receiver.recv().await { done(value); }
        });
    }

    /// Re-read everything from the camera now.
    pub fn refresh(&self) {
        let this = self.clone();
        self.call(Some("snapshot"), Snapshot::read, move |snapshot| this.publish(snapshot));
    }

    /// Re-read shortly, merging bursts of changes into one read.
    pub fn refresh_soon(&self) {
        if let Some(id) = self.0.refresh_timer.borrow_mut().take() { id.remove(); }
        let this = self.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(180), move || {
            this.0.refresh_timer.borrow_mut().take();
            this.refresh();
        });
        *self.0.refresh_timer.borrow_mut() = Some(id);
    }

    /// Refresh on return to the window, unless we read very recently.
    pub fn refresh_if_stale(&self) {
        let fresh = self.0.last_refresh.get().is_some_and(|at| at.elapsed() < Duration::from_secs(3));
        if !fresh { self.refresh(); }
    }

    /// Send one setting to the camera, then re-read so dependent settings update.
    /// `done` gets `Ok(Some(value))` when the camera confirmed what it now has, `Ok(None)` when the
    /// command was sent but cannot be confirmed.
    pub fn write(&self, setting: &'static Setting, value: Value, done: impl FnOnce(Result<Option<Value>, String>) + 'static) {
        crate::backend::usage::note_activity();
        let this = self.clone();
        let run = move |device: &str| setting.write(device, value);
        let finish = move |result: Result<Option<Value>, String>| {
            if let Err(error) = &result { this.error(&format!("Could not change {}: {error}", setting.label.to_lowercase())); }
            done(result);
            this.refresh_soon();
        };
        // Turning the camera on or off goes ahead of the polling; everything else keeps its place.
        if setting.id == "obsbot:sleep" { self.call_now(run, finish); } else { self.call(Some(setting.id), run, finish); }
    }

    /// The camera's live pan and tilt in degrees, if it has answered recently.
    pub fn gimbal(&self) -> Option<(f64, f64)> { self.0.gimbal.get() }

    /// Publish a new live position (or `None` when the camera could not be read).
    pub fn set_gimbal(&self, position: Option<(f64, f64)>) {
        // Every window sees the same whole degrees, so nothing flickers between readings.
        let position = match position {
            Some((pan, tilt)) => {
                let mut filter = self.0.gimbal_filter.borrow_mut();
                Some((filter.0.push(pan), filter.1.push(tilt)))
            }
            None => { *self.0.gimbal_filter.borrow_mut() = Default::default(); None }
        };
        self.0.gimbal.set(position);
        let mut listeners = std::mem::take(&mut *self.0.gimbal_listeners.borrow_mut());
        listeners.retain(|listener| listener(position));
        let mut added = std::mem::take(&mut *self.0.gimbal_listeners.borrow_mut());
        listeners.append(&mut added);
        *self.0.gimbal_listeners.borrow_mut() = listeners;
    }

    /// Be told about every new live position. Return `false` to stop.
    pub fn subscribe_gimbal(&self, listener: impl Fn(Option<(f64, f64)>) -> bool + 'static) {
        self.0.gimbal_listeners.borrow_mut().push(Box::new(listener));
    }

    pub fn add_toaster(&self, toaster: Rc<Toaster>) { self.0.toasters.borrow_mut().push(toaster); }
    pub fn add_dialogs(&self, dialogs: Rc<Dialogs>) { self.0.dialogs.borrow_mut().push(dialogs); }

    pub fn toast(&self, message: &str) { self.show_toast(message, false); }
    pub fn error(&self, message: &str) { self.show_toast(message, true); }

    fn show_toast(&self, message: &str, error: bool) {
        let scrubbed = if error { plain_error(message) } else { message.to_string() };
        let message = scrubbed.as_str();
        let toasters = self.0.toasters.borrow();
        if let Some(toaster) = toasters.iter().find(|t| t.is_visible()) { toaster.show(message, error); }
    }

    pub fn confirm(&self, icon: &str, title: &str, body: &str, action: &str, on_confirm: impl Fn() + 'static) {
        let dialogs = self.0.dialogs.borrow();
        if let Some(dialogs) = dialogs.iter().find(|d| d.is_visible()) {
            dialogs.confirm(icon, title, body, action, on_confirm);
        } else {
            // No window is showing a dialog layer; do not act without asking.
            self.error("Could not ask for confirmation, so nothing was changed.");
        }
    }
}

/// Words that mean a message is for developers, not for people.
const TECHNICAL: [&str; 14] = ["0x", "selector", "ioctl", "UVC", "SET_CUR", "GET_CUR", "read-back", "CAM_", "XU ", "os error", "errno", "v4l2-ctl", "VIDIOC", "sequence"];

/// A message for the screen: errors that are development notes are written to the terminal for us and
/// replaced by something a person can act on.
pub fn plain_error(message: &str) -> String {
    let lower = message.to_lowercase();
    if !TECHNICAL.iter().any(|word| lower.contains(&word.to_lowercase())) { return message.to_string(); }
    eprintln!("{message}");
    if lower.contains("cannot open") || lower.contains("permission denied") || lower.contains("busy") {
        return "The camera could not be opened. Another program may be using it.".to_string();
    }
    let head = message.split(": ").next().unwrap_or("").trim().trim_end_matches('.');
    if head.is_empty() || head.len() > 60 || TECHNICAL.iter().any(|word| head.to_lowercase().contains(&word.to_lowercase())) {
        "The camera did not respond as expected. Try again, or unplug and replug it.".to_string()
    } else {
        format!("{head}. The camera did not respond as expected; try again, or unplug and replug it.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_do_not_see_development_notes() {
        let shown = plain_error("Could not change tracking mode: UVC Camera Terminal 1 selector 0x0d request 0x81: Input/output error");
        assert_eq!(shown, "Could not change tracking mode. The camera did not respond as expected; try again, or unplug and replug it.");
        assert!(plain_error("Cannot open /dev/video1 for UVC Camera Terminal read-back: Permission denied").contains("Another program"));
        assert!(!plain_error("Timed out waiting for a matching CAM_GET_FACE_FOCUS reply.").contains("CAM_"));
        // A message that is already plain is left alone.
        assert_eq!(plain_error("The camera was unplugged."), "The camera was unplugged.");
        assert_eq!(plain_error("Name the preset first."), "Name the preset first.");
    }
}
