//! Application start-up, window switching and camera connection.

use super::full::FullWindow;
use super::popup::PreviewWindow;
use super::preview::PreviewHub;
use super::slim::SlimWindow;
use super::store::{Connection, Store};
use super::style::Style;
use super::windowing;
use crate::backend::worker::Camera;
use crate::backend::{self, current_config, install_config, update_config};
use crate::config;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub struct Ctx {
    pub app: gtk::Application,
    pub store: Store,
    pub hub: PreviewHub,
    pub style: Rc<Style>,
    full: RefCell<Option<FullWindow>>,
    slim: RefCell<Option<SlimWindow>>,
    popup: RefCell<Option<PreviewWindow>>,
    scan_timer: RefCell<Option<glib::SourceId>>,
}

/// Find the camera and describe how reachable it is.
fn discover() -> (Option<String>, Connection) {
    match backend::discover_tiny4lite().into_iter().next() {
        Some(path) => (Some(path), Connection::Connected),
        None if backend::usb_camera_present() => (None, Connection::NoVideoNode),
        None => (None, Connection::Absent),
    }
}

impl Ctx {
    fn build(app: &gtk::Application) -> Rc<Ctx> {
        match config::load() {
            Ok(loaded) => install_config(loaded),
            Err(error) => eprintln!("Using default settings: {error}"),
        }
        let style = Style::install();
        if crate::backend::ui_test_mode() {
            if let Ok(mode) = std::env::var("TINY4LITE_UI_TEST_THEME") { style.set_mode(super::theme::ThemeMode::from_name(&mode)); }
            if let Ok(scale) = std::env::var("TINY4LITE_UI_TEST_SCALE") { style.set_scale(super::theme::UiScale::from_name(&scale)); }
        }
        let (path, connection) = discover();
        let camera = Camera::spawn(path.as_deref().unwrap_or(""));
        let store = Store::new(camera, connection);
        let hub = PreviewHub::new(&store);
        {
            // After a reconnect, bring the picture back without the user doing anything.
            let hub = hub.clone();
            store.subscribe(move |snapshot| {
                if snapshot.error.is_none() && !snapshot.controls.is_empty() && !hub.is_running() { hub.update_demand(); }
                true
            });
        }
        let ctx = Rc::new(Ctx {
            app: app.clone(), store, hub, style,
            full: RefCell::default(), slim: RefCell::default(), popup: RefCell::default(), scan_timer: RefCell::default(),
        });
        ctx.watch_unplug();
        // The camera is controlled through v4l2-ctl, so say so plainly if it is not there.
        if !backend::ui_test_mode() && !crate::setup::program_found("v4l2-ctl") {
            ctx.store.error("v4l2-ctl is not installed, so the camera cannot be controlled. Install it from the Device tab, under Background service.");
        }
        // A new version: make it the installed one and put the background service on it, without the user
        // removing and installing anything (away from the window, since restarting the service takes a moment).
        if !backend::ui_test_mode() {
            let (sender, receiver) = async_channel::bounded::<Vec<Result<String, String>>>(1);
            std::thread::spawn(move || { let _ = sender.send_blocking(crate::install::keep_up_to_date()); });
            let store = ctx.store.clone();
            glib::spawn_future_local(async move {
                if let Ok(results) = receiver.recv().await {
                    for result in results { match result { Ok(m) => store.toast(&m), Err(e) => store.error(&e) } }
                }
            });
        }
        if connection == Connection::Connected {
            // Restore the remembered capture mode; this never moves the gimbal.
            ctx.store.call(None, |device| backend::apply_saved_video_mode(device), |_| {});
            ctx.store.refresh();
        } else {
            ctx.start_scanning();
        }
        ctx
    }

    /// Look for the camera now; `announce` reports the outcome.
    pub fn reconnect(self: &Rc<Self>, announce: bool) {
        let (path, connection) = discover();
        match (path, connection) {
            (Some(path), Connection::Connected) => {
                let was = self.store.connection();
                self.store.set_camera(Camera::spawn(&path), Connection::Connected);
                if let Some(id) = self.scan_timer.borrow_mut().take() { id.remove(); }
                if was != Connection::Connected {
                    self.store.toast("Camera connected");
                    // A camera that was unplugged comes back in its own default mode: put the one chosen
                    // here back, with the preview out of the way while it changes.
                    let store = self.store.clone();
                    self.hub.while_stopped(|device| backend::apply_saved_video_mode(device), move |message| {
                        if let Some(message) = message { store.toast(&message); }
                    });
                }
            }
            (_, connection) => {
                if self.store.connection() != connection {
                    self.store.set_camera(Camera::spawn(""), connection);
                }
                if announce {
                    self.store.error(match connection {
                        Connection::NoVideoNode => "The camera is plugged in, but has no video device yet.",
                        _ => "Still no camera found.",
                    });
                }
                self.start_scanning();
            }
        }
    }

    /// While disconnected, look again every few seconds. Costs a directory read.
    fn start_scanning(self: &Rc<Self>) {
        if self.scan_timer.borrow().is_some() { return; }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local(Duration::from_secs(3), move || {
            let Some(ctx) = weak.upgrade() else { return glib::ControlFlow::Break };
            let (path, connection) = discover();
            if let (Some(path), Connection::Connected) = (path, connection) {
                ctx.scan_timer.borrow_mut().take();
                ctx.store.set_camera(Camera::spawn(&path), Connection::Connected);
                ctx.store.toast("Camera connected");
                return glib::ControlFlow::Break;
            }
            if ctx.store.connection() != connection { ctx.store.set_camera(Camera::spawn(""), connection); }
            glib::ControlFlow::Continue
        });
        *self.scan_timer.borrow_mut() = Some(id);
    }

    /// Notice the camera being unplugged: while connected, check once in a while
    /// that its video node still exists (a single `stat`).
    fn watch_unplug(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(4), move || {
            let Some(ctx) = weak.upgrade() else { return glib::ControlFlow::Break };
            if ctx.store.connection() == Connection::Connected
                && !backend::ui_test_mode()
                && !std::path::Path::new(&ctx.store.device()).exists()
            {
                ctx.hub.stop();
                ctx.store.set_camera(Camera::spawn(""), Connection::Absent);
                ctx.store.error("The camera was unplugged.");
                ctx.start_scanning();
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn show_full(self: &Rc<Self>) {
        update_config(|c| c.last_window = "full".into());
        if let Some(slim) = self.slim.borrow().as_ref() {
            slim.joystick.release();
            slim.window.set_visible(false);
        }
        let mut full = self.full.borrow_mut();
        let window = full.get_or_insert_with(|| FullWindow::new(self));
        windowing::present(&window.window);
    }

    pub fn show_slim(self: &Rc<Self>) {
        update_config(|c| c.last_window = "slim".into());
        if let Some(full) = self.full.borrow().as_ref() {
            full.joystick.release();
            full.window.set_visible(false);
        }
        let mut slim = self.slim.borrow_mut();
        let window = slim.get_or_insert_with(|| SlimWindow::new(self));
        windowing::present(&window.window);
    }

    /// Move the preview into its own window, or back.
    pub fn set_popped_out(self: &Rc<Self>, popped: bool) {
        update_config(|c| c.preview_popped_out = popped);
        if let Some(full) = self.full.borrow().as_ref() { full.set_popped_out(popped); }
        if popped {
            let mut popup = self.popup.borrow_mut();
            let window = popup.get_or_insert_with(|| PreviewWindow::new(self));
            windowing::present(&window.window);
        } else if let Some(popup) = self.popup.borrow().as_ref() {
            popup.window.set_visible(false);
        }
    }

    pub fn quit(&self) {
        self.hub.stop();
        backend::flush_config();
        self.app.quit();
    }
}

pub fn run() {
    let app = gtk::Application::builder()
        .application_id(windowing::APP_ID)
        .flags(gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    // The icon in the title bar, the task switcher and the dock, found by name in the icon theme.
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name("tiny4lite-control");
        // Scrollbars float over the page and never take room from it, so the page does not shift when
        // the pointer arrives and they appear.
        if let Some(settings) = gtk::Settings::default() { settings.set_gtk_overlay_scrolling(true); }
    });
    // Options: --slim / --full choose the window; --preview opens the pop-out too.
    app.add_main_option("slim", glib::Char::from(0), glib::OptionFlags::NONE, glib::OptionArg::None, "Open the slim window", None);
    app.add_main_option("full", glib::Char::from(0), glib::OptionFlags::NONE, glib::OptionArg::None, "Open the full window", None);
    app.add_main_option("preview", glib::Char::from(0), glib::OptionFlags::NONE, glib::OptionArg::None, "Open the pop-out preview", None);

    let ctx_cell: Rc<RefCell<Option<Rc<Ctx>>>> = Rc::default();
    let cell = ctx_cell.clone();
    app.connect_command_line(move |app, command_line| {
        let options = command_line.options_dict();
        let ctx = cell.borrow_mut().get_or_insert_with(|| Ctx::build(app)).clone();
        let want_slim = options.contains("slim");
        let want_full = options.contains("full");
        let last_slim = current_config().last_window == "slim";
        if want_slim || (!want_full && last_slim) { ctx.show_slim(); } else { ctx.show_full(); }
        if options.contains("preview") || current_config().preview_popped_out { ctx.set_popped_out(true); }
        if backend::ui_test_mode() && std::env::var("TINY4LITE_UI_TEST_DEVTOOLS").is_ok() {
            if let Some(full) = ctx.full.borrow().as_ref() { super::devtools::open(full.window.upcast_ref(), &ctx.store.device()); }
        }
        0
    });
    app.run();
}
