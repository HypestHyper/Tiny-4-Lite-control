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
}

impl Toaster {
    /// A bottom-centre toast. Add `widget()` to the window's overlay.
    pub fn new() -> Rc<Toaster> {
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_max_width_chars(60);
        let toast = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toast.add_css_class("toast");
        toast.append(&label);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        revealer.set_transition_duration(220);
        revealer.set_halign(gtk::Align::Center);
        revealer.set_valign(gtk::Align::End);
        revealer.set_margin_bottom(24);
        revealer.set_child(Some(&toast));
        revealer.set_can_target(false);
        Rc::new(Toaster { revealer, label, toast, timer: RefCell::new(None) })
    }

    pub fn widget(&self) -> &gtk::Revealer { &self.revealer }

    pub fn show(self: &Rc<Self>, message: &str, error: bool) {
        self.label.set_text(message);
        if error { self.toast.add_css_class("error"); } else { self.toast.remove_css_class("error"); }
        self.revealer.set_reveal_child(true);
        // Announce to assistive technology.
        self.label.update_property(&[gtk::accessible::Property::Label(message)]);
        if let Some(id) = self.timer.borrow_mut().take() { id.remove(); }
        let this = self.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(if error { 6000 } else { 3500 }), move || {
            this.timer.borrow_mut().take();
            this.revealer.set_reveal_child(false);
        });
        *self.timer.borrow_mut() = Some(id);
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
    listeners: RefCell<Vec<Listener>>,
    toasters: RefCell<Vec<Rc<Toaster>>>,
    dialogs: RefCell<Vec<Rc<Dialogs>>>,
    refresh_timer: RefCell<Option<glib::SourceId>>,
    last_refresh: Cell<Option<Instant>>,
}

#[derive(Clone)]
pub struct Store(Rc<Inner>);

impl Store {
    pub fn new(camera: Camera, connection: Connection) -> Store {
        Store(Rc::new(Inner {
            camera: RefCell::new(camera),
            connection: Cell::new(connection),
            snapshot: RefCell::new(Rc::new(Snapshot::default())),
            listeners: RefCell::new(Vec::new()),
            toasters: RefCell::new(Vec::new()),
            dialogs: RefCell::new(Vec::new()),
            refresh_timer: RefCell::new(None),
            last_refresh: Cell::new(None),
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

    fn publish(&self, snapshot: Snapshot) {
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
    pub fn write(&self, setting: &'static Setting, value: Value, done: impl FnOnce(Result<(), String>) + 'static) {
        let this = self.clone();
        self.call(Some(setting.id), move |device| setting.write(device, value), move |result| {
            if let Err(error) = &result { this.error(&format!("Could not change {}: {error}", setting.label.to_lowercase())); }
            done(result);
            this.refresh_soon();
        });
    }

    pub fn add_toaster(&self, toaster: Rc<Toaster>) { self.0.toasters.borrow_mut().push(toaster); }
    pub fn add_dialogs(&self, dialogs: Rc<Dialogs>) { self.0.dialogs.borrow_mut().push(dialogs); }

    pub fn toast(&self, message: &str) { self.show_toast(message, false); }
    pub fn error(&self, message: &str) { self.show_toast(message, true); }

    fn show_toast(&self, message: &str, error: bool) {
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
