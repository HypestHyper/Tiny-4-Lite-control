//! Live camera preview.
//!
//! One capture process feeds every place the preview is shown (the main
//! window, the slim window and the pop-out), so showing it twice costs no
//! more than once. It only runs while some view is on screen, decodes
//! JPEG frames at reduced size on a background thread, and drops frames
//! instead of queueing them.

use super::store::Store;
use super::widgets;
use crate::backend::{current_config, update_config};
use crate::video::{self, VideoMode};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use jpeg_decoder::{Decoder, PixelFormat};
use std::cell::{Cell, RefCell};
use std::io::{Cursor, Read};
use std::process::{Child, Command, Stdio};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_FPS: u64 = 30;
const STOP_GRACE: Duration = Duration::from_millis(1500);

struct Frame {
    width: i32,
    height: i32,
    rgb: Vec<u8>,
}

struct ViewInner {
    picture: gtk::Picture,
    placeholder: gtk::Box,
    placeholder_text: gtk::Label,
    /// Shown with an error, to try the stream again.
    retry: gtk::Button,
    /// The big play button in the middle, while the preview is paused or not started.
    play: gtk::Button,
    glyph: gtk::Label,
    /// Width over height of the picture the camera sends, and whether the view's height follows its width.
    ratio: Cell<f32>,
    locked: Cell<bool>,
    chip: gtk::Label,
    root: gtk::Overlay,
    /// Where the last click on the picture landed, and what it did, drawn for a moment as a ring.
    marker: gtk::DrawingArea,
    mark: Cell<Option<(f64, f64, u8)>>,
}

struct HubInner {
    store: Store,
    views: RefCell<Vec<Weak<ViewInner>>>,
    child: RefCell<Option<Child>>,
    generation: Cell<u64>,
    running: Cell<bool>,
    enabled: Cell<bool>,
    mode: RefCell<Option<VideoMode>>,
    target_width: Arc<AtomicU32>,
    stop_timer: RefCell<Option<glib::SourceId>>,
    last_texture: RefCell<Option<gdk::Texture>>,
    /// Automatic restarts since the last frame arrived.
    retries: Cell<u32>,
    /// Told the mode the camera is really streaming, whenever a stream starts.
    mode_listeners: RefCell<Vec<Rc<dyn Fn(&VideoMode)>>>,
    /// Told when the preview is started or paused, from any of its buttons.
    enabled_listeners: RefCell<Vec<Rc<dyn Fn(bool) -> bool>>>,
    /// Told when the source (camera or Studio) changes, so every window's list shows the same.
    source_listeners: RefCell<Vec<Rc<dyn Fn(bool) -> bool>>>,
}

#[derive(Clone)]
pub struct PreviewHub(Rc<HubInner>);

impl PreviewHub {
    pub fn new(store: &Store) -> PreviewHub {
        FOCUS_PEAKING.store(current_config().preview_focus_peaking, Ordering::Relaxed);
        PEAKING_STRICTNESS.store(current_config().preview_peaking_strictness.min(100), Ordering::Relaxed);
        PreviewHub(Rc::new(HubInner {
            store: store.clone(),
            views: RefCell::default(),
            child: RefCell::default(),
            generation: Cell::new(0),
            running: Cell::new(false),
            // Only running from the start when asked for, and then only if it was not left paused.
            enabled: Cell::new({ let c = current_config(); c.preview_on_start && !c.preview_paused }),
            mode: RefCell::default(),
            target_width: Arc::new(AtomicU32::new(640)),
            stop_timer: RefCell::default(),
            last_texture: RefCell::default(),
            retries: Cell::new(0),
            mode_listeners: RefCell::default(),
            enabled_listeners: RefCell::default(),
            source_listeners: RefCell::default(),
        }))
    }

    pub fn is_enabled(&self) -> bool { self.0.enabled.get() }
    pub fn is_running(&self) -> bool { self.0.running.get() }

    /// Be told the capture mode the camera is really using each time the preview starts, so other
    /// parts of the window can show the same one.
    pub fn on_mode(&self, listener: impl Fn(&VideoMode) + 'static) {
        self.0.mode_listeners.borrow_mut().push(Rc::new(listener));
    }

    /// Be told each time the preview is started or paused. The listener returns false once it is no
    /// longer needed.
    pub fn on_enabled(&self, listener: impl Fn(bool) -> bool + 'static) {
        self.0.enabled_listeners.borrow_mut().push(Rc::new(listener));
    }

    fn tell_enabled(&self) {
        let enabled = self.0.enabled.get();
        let listeners: Vec<_> = self.0.enabled_listeners.borrow().clone();
        let keep: Vec<bool> = listeners.iter().map(|l| l(enabled)).collect();
        let mut i = 0;
        self.0.enabled_listeners.borrow_mut().retain(|_| { let k = keep.get(i).copied().unwrap_or(true); i += 1; k });
    }

    /// The round play/pause button for the corner of a preview, which follows the preview however it
    /// is started or paused.
    pub fn pause_button(&self) -> gtk::Button {
        let button = widgets::icon_button("pause", "Pause the preview", true);
        button.add_css_class("on-stage");
        let show = |button: &gtk::Button, enabled: bool| {
            if let Some(glyph) = button.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                widgets::set_icon(&glyph, if enabled { "pause" } else { "play_arrow" });
            }
            let words = if enabled { "Pause the preview" } else { "Start the preview" };
            button.set_tooltip_text(Some(words));
            widgets::label_accessible(button, words);
        };
        show(&button, self.is_enabled());
        let weak = button.downgrade();
        self.on_enabled(move |enabled| match weak.upgrade() { Some(button) => { show(&button, enabled); true } None => false });
        let hub = self.clone();
        button.connect_clicked(move |_| hub.set_enabled(!hub.is_enabled()));
        button
    }

    /// The Studio camera's node, if the preview shows it and it is there.
    fn showing_studio(&self) -> Option<String> {
        if !current_config().preview_studio { return None; }
        crate::studio::loopback_node()
    }

    /// Show the Studio camera or the camera itself, and start the picture again from the new one.
    pub fn set_studio(&self, on: bool) {
        if current_config().preview_studio == on { return; }
        update_config(|c| c.preview_studio = on);
        // Every window's list follows.
        let listeners: Vec<_> = self.0.source_listeners.borrow().clone();
        let keep: Vec<bool> = listeners.iter().map(|l| l(on)).collect();
        let mut i = 0;
        self.0.source_listeners.borrow_mut().retain(|_| { let k = keep.get(i).copied().unwrap_or(true); i += 1; k });
        self.0.retries.set(0);
        // Back to the camera: ask the service to let go of it now, not after its usual pause.
        if !on { crate::studio::request_release(); }
        if self.0.running.get() { self.restart(); } else { self.update_demand(); }
    }

    /// Why the Studio camera cannot give a picture right now, before trying: not set up, the background
    /// service not running, turned off, or what the service says is wrong. None when it should work.
    fn studio_blocked(&self) -> Option<String> {
        if crate::studio::loopback_node().is_none() {
            return Some("The Studio camera is not set up yet: Device tab \u{2192} Studio camera \u{2192} Set up\u{2026}. Or pick Camera above.".into());
        }
        let Some(status) = crate::service::running_status() else {
            return Some("The Studio camera needs the background service, and it is not running: Device tab \u{2192} Background service \u{2192} Start (or Install\u{2026}). Or pick Camera above.".into());
        };
        match status.studio {
            // Turned on, but the service says nothing about it: it is an older version.
            None if current_config().studio.enabled => Some("The background service that is running is an older version without the Studio camera. Install this version (Device tab \u{2192} App), or stop and start the service (Device tab \u{2192} Background service).".into()),
            None => Some("The Studio camera is turned off: Device tab \u{2192} Studio camera. Or pick Camera above.".into()),
            Some(r) if r.state == "off" => Some("The Studio camera is turned off: Device tab \u{2192} Studio camera. Or pick Camera above.".into()),
            Some(r) if r.state == "unavailable" || r.state == "error" => Some(format!("The Studio camera is not working: {}", r.detail)),
            Some(_) => None,
        }
    }

    /// Why the Studio picture is not coming: what the background service says about it, and what the
    /// stream reader said.
    fn studio_problem(&self) -> String {
        let service = match crate::service::running_status() {
            None => "The background service is not running, and it runs the Studio camera.".to_string(),
            Some(status) => match status.studio {
                None => "The Studio camera is turned off (Device tab \u{2192} Studio camera).".to_string(),
                Some(r) if r.detail.is_empty() => format!("The Studio camera is {}.", r.state),
                Some(r) => r.detail,
            },
        };
        let reader = std::fs::read_to_string(preview_log_path()).unwrap_or_default();
        let reader = reader.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        // What v4l2-ctl says when nothing writes to the virtual camera, in plain words.
        let reader = if reader.contains("unsupported stream type") { "nothing is sending a picture into the Studio camera.".to_string() } else { reader };
        if reader.is_empty() { service } else { format!("{service} The stream said: {reader}") }
    }

    /// A small list on the preview to pick what it shows: the camera, or the Studio camera. Always
    /// there; picking Studio before it is set up says how to set it up.
    pub fn source_picker(&self) -> gtk::DropDown {
        let picker = gtk::DropDown::from_strings(&["Camera", "Studio"]);
        picker.add_css_class("on-stage-picker");
        picker.set_valign(gtk::Align::Center);
        picker.set_tooltip_text(Some("What the preview shows: the camera itself, or the Studio camera (with its filters) that your apps can use"));
        widgets::label_accessible(&picker, "Preview source");
        picker.set_selected(u32::from(current_config().preview_studio));
        let hub = self.clone();
        picker.connect_selected_notify(move |p| hub.set_studio(p.selected() == 1));
        // Changed in another window: show the same here.
        let weak = picker.downgrade();
        self.0.source_listeners.borrow_mut().push(Rc::new(move |on| match weak.upgrade() {
            Some(picker) => { if picker.selected() != u32::from(on) { picker.set_selected(u32::from(on)); } true }
            None => false,
        }));
        picker
    }

    /// The round button that picks what a click on the picture does: nothing, focus there, or follow
    /// the person there. Lit while clicks do something.
    pub fn click_button(&self) -> gtk::Button {
        let button = widgets::icon_button("ads_click", "What a click on the picture does", true);
        button.add_css_class("on-stage");
        show_click_mode(&button);
        CLICK_BUTTONS.with(|buttons| buttons.borrow_mut().push(button.downgrade()));
        let popover = gtk::Popover::new();
        popover.set_parent(&button);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.set_margin_top(6); list.set_margin_bottom(6); list.set_margin_start(6); list.set_margin_end(6);
        list.append(&widgets::text("A click on the picture", &["setting-label"]));
        let mut first: Option<gtk::CheckButton> = None;
        let mut choices = Vec::new();
        for (mode, label) in [(CLICK_OFF, "Does nothing"), (CLICK_FOCUS, "Focuses there"), (CLICK_TRACK, "Follows that person")] {
            let choice = gtk::CheckButton::with_label(label);
            if let Some(first) = &first { choice.set_group(Some(first)); } else { first = Some(choice.clone()); }
            let pop = popover.downgrade();
            choice.connect_toggled(move |c| {
                if c.is_active() { set_click_mode(mode); if let Some(pop) = pop.upgrade() { pop.popdown(); } }
            });
            list.append(&choice);
            choices.push((mode, choice));
        }
        list.append(&widgets::info_button("Clicking on the picture", &[
            "Focuses there: the camera focuses on that spot (OBSBOT's tap to focus). Autofocus must be on.".into(),
            "Follows that person: the camera tracks whoever is there. If tracking is off, Normal tracking is turned on first.".into(),
            "It works on the Studio camera's picture as well, so you can click while your apps use the Studio camera.".into(),
        ]));
        popover.set_child(Some(&list));
        let owned = popover.clone();
        button.connect_destroy(move |_| owned.unparent());
        button.connect_clicked(move |_| {
            for (mode, choice) in &choices { if *mode == click_mode() { choice.set_active(true); } }
            popover.popup();
        });
        button
    }

    /// The round button that turns focus peaking on and off, lit while it is on, with its strictness
    /// slider beside it while it is on.
    pub fn peaking_button(&self) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        slider.set_draw_value(false);
        slider.set_size_request(110, -1);
        slider.set_value(f64::from(PEAKING_STRICTNESS.load(Ordering::Relaxed)));
        slider.add_css_class("on-stage-slider");
        slider.set_tooltip_text(Some("Strictness: to the right, only the strongest, sharpest edges are outlined"));
        widgets::label_accessible(&slider, "Focus peaking strictness");
        slider.connect_value_changed(|s| set_peaking_strictness(s.value().round() as u8));
        let reveal = gtk::Revealer::new();
        reveal.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
        reveal.set_child(Some(&slider));
        reveal.set_valign(gtk::Align::Center);
        reveal.set_reveal_child(focus_peaking_on());
        let button = widgets::icon_button("filter_center_focus", "Focus peaking: outline what is in focus in green", true);
        button.add_css_class("on-stage");
        let show = |button: &gtk::Button| {
            if focus_peaking_on() { button.add_css_class("lit"); } else { button.remove_css_class("lit"); }
        };
        show(&button);
        {
            let reveal = reveal.clone();
            button.connect_clicked(move |button| {
                set_focus_peaking(!focus_peaking_on());
                show(button);
                reveal.set_reveal_child(focus_peaking_on());
            });
        }
        row.append(&reveal);
        row.append(&button);
        row
    }

    /// Start the stream again after it stopped or failed.
    pub fn restart(&self) {
        let hub = &self.0;
        hub.retries.set(0);
        let child = self.take_stream();
        if let Some(mut child) = child { std::thread::spawn(move || { let _ = child.kill(); let _ = child.wait(); }); }
        if !hub.enabled.get() { hub.enabled.set(true); update_config(|c| c.preview_paused = false); self.tell_enabled(); }
        // A moment for the camera to let go of the old stream.
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(500), move || this.update_demand());
    }

    /// Pause or resume the live picture (kept off the camera when paused).
    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.set(enabled);
        self.0.retries.set(0);
        update_config(|c| c.preview_paused = !enabled);
        self.tell_enabled();
        if enabled { self.update_demand(); } else { self.stop(); self.each_view(|v| v.show_message("Preview paused", "pause_circle")); }
    }

    fn live_views(&self) -> Vec<Rc<ViewInner>> {
        self.0.views.borrow().iter().filter_map(Weak::upgrade).collect()
    }

    fn each_view(&self, f: impl Fn(&ViewInner)) {
        for view in self.live_views() { f(&view); }
    }

    fn demand(&self) -> bool {
        self.0.enabled.get() && self.live_views().iter().any(|v| v.root.is_mapped())
    }

    /// Start or schedule a stop depending on what is on screen.
    pub fn update_demand(&self) {
        let hub = &self.0;
        if self.demand() {
            if let Some(id) = hub.stop_timer.borrow_mut().take() { id.remove(); }
            if !hub.running.get() { self.start(); }
        } else if hub.running.get() && hub.stop_timer.borrow().is_none() {
            // Wait a moment: switching between windows should not restart the stream.
            let this = self.clone();
            let id = glib::timeout_add_local_once(STOP_GRACE, move || {
                this.0.stop_timer.borrow_mut().take();
                if !this.demand() { this.stop(); }
            });
            *hub.stop_timer.borrow_mut() = Some(id);
        }
    }

    /// Run `change` with the stream stopped (the camera refuses a format
    /// change while streaming), then resume.
    pub fn while_stopped<T: Send + 'static>(
        &self,
        change: impl FnOnce(&str) -> T + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) {
        // The stream has to be really gone before the format changes, and the camera needs a moment
        // after it: starting and stopping streams in quick succession can make it drop off the bus.
        // So the stream is ended on the camera thread, in front of the change itself.
        let child = self.take_stream();
        self.each_view(|v| v.show_message("Changing the video mode…", "videocam"));
        let hub = self.clone();
        self.0.store.call(None, move |device| {
            if let Some(mut child) = child {
                let _ = child.kill();
                let _ = child.wait();
                std::thread::sleep(Duration::from_millis(300));
            }
            change(device)
        }, move |value| {
            done(value);
            // Let the camera settle before the preview asks it for frames again.
            glib::timeout_add_local_once(Duration::from_millis(700), move || hub.update_demand());
        });
    }

    /// Stop the stream from being shown and hand back the process that makes it, to be ended by the caller.
    fn take_stream(&self) -> Option<Child> {
        let hub = &self.0;
        hub.generation.set(hub.generation.get().wrapping_add(1));
        hub.running.set(false);
        hub.child.borrow_mut().take()
    }

    fn start(&self) {
        let hub = &self.0;
        hub.running.set(true);
        let generation = hub.generation.get().wrapping_add(1);
        hub.generation.set(generation);
        self.each_view(|v| v.show_message("Starting preview…", "videocam"));
        let this = self.clone();
        // The Studio camera, if that is what the preview shows and it is there; otherwise the camera.
        if current_config().preview_studio {
            if let Some(why) = self.studio_blocked() {
                hub.running.set(false);
                self.each_view(|v| v.show_message(&why, "videocam_off"));
                return;
            }
        }
        let from_studio = self.showing_studio();
        // Reading the current mode asks the camera, so do it off the UI thread.
        hub.store.call(None, move |device| match &from_studio {
            // The Studio camera always sends raw 4:2:0 at the camera's own size. It only answers questions
            // while something writes to it (and the writer changes as the preview opens it), so the size
            // comes from the camera itself.
            Some(node) => (node.clone(), video::current_mode(device).map(|mode| VideoMode { pixel_format: "YUYV".into(), ..mode })
                .or_else(|| video::current_format(node))),
            None => (device.to_string(), video::current_mode(device)),
        }, move |(node, mode)| {
            if this.0.generation.get() != generation { return; }
            let Some(mode) = mode else {
                this.0.running.set(false);
                this.each_view(|v| v.show_message("The camera did not report a video mode, so there is nothing to preview.", "videocam_off"));
                return;
            };
            this.spawn_stream(mode, generation, node);
        });
    }

    fn spawn_stream(&self, mode: VideoMode, generation: u64, device: String) {
        let hub = &self.0;
        let from_studio = device != hub.store.device();
        *hub.mode.borrow_mut() = Some(mode.clone());
        // The capture mode card follows what the camera itself sends, not the Studio copy.
        if !from_studio {
            let listeners: Vec<_> = hub.mode_listeners.borrow().clone();
            for listener in listeners { listener(&mode); }
        }
        let mut child = if super::super::backend::ui_test_mode() {
            None
        } else {
            // What the reader says goes to a small log, to explain a picture that does not come.
            let log = std::fs::create_dir_all(crate::service::runtime_dir()).ok()
                .and_then(|_| std::fs::File::create(preview_log_path()).ok()).map(Stdio::from).unwrap_or_else(Stdio::null);
            match Command::new("v4l2-ctl").arg("-d").arg(&device)
                .arg("--stream-mmap=4").arg("--stream-count=1000000").arg("--stream-to=-")
                .stdout(Stdio::piped()).stderr(log).spawn()
            {
                Ok(child) => Some(child),
                Err(error) => {
                    hub.running.set(false);
                    self.each_view(|v| v.show_message(&format!("Could not start the preview: {error}"), "videocam_off"));
                    return;
                }
            }
        };
        let (sender, receiver) = async_channel::bounded::<Frame>(1);
        let width_hint = hub.target_width.clone();
        let reader_mode = mode.clone();
        match child.as_mut().and_then(|c| c.stdout.take()) {
            Some(stdout) => { std::thread::spawn(move || read_frames(stdout, sender, reader_mode, width_hint)); }
            None => {
                // Test mode has no camera: draw a still test card so the layout can be checked.
                let (w, h) = (mode.width.min(960) as i32, mode.height.min(540) as i32);
                std::thread::spawn(move || loop {
                    let mut card = test_card(w, h);
                    if FOCUS_PEAKING.load(Ordering::Relaxed) { focus_peaking(&mut card); }
                    if sender.send_blocking(card).is_err() { return; }
                    std::thread::sleep(Duration::from_millis(500));
                });
            }
        }
        *hub.child.borrow_mut() = child;

        let this = self.clone();
        let label = if from_studio { format!("Studio · {} × {}", mode.width, mode.height) }
            else { format!("{} × {} · {} · {:.0} fps", mode.width, mode.height, mode.pixel_format, mode.frame_rate_milli as f64 / 1000.0) };
        glib::spawn_future_local(async move {
            while let Ok(frame) = receiver.recv().await {
                if this.0.generation.get() != generation { return; }
                this.present(frame, &label);
            }
            if this.0.generation.get() == generation && this.0.running.get() {
                this.0.running.set(false);
                // A hiccup in the stream is common, so try again a few times before giving up. The Studio
                // camera changes its writer as it starts (placeholder, then the real picture), which takes
                // a few seconds, so it gets more tries.
                let tries = if from_studio { 6 } else { 3 };
                if this.0.retries.get() < tries && this.0.enabled.get() {
                    this.0.retries.set(this.0.retries.get() + 1);
                    this.each_view(|v| v.show_message("The picture stopped. Reconnecting…", "videocam"));
                    let again = this.clone();
                    glib::timeout_add_local_once(Duration::from_secs(2), move || {
                        if !again.0.running.get() { again.update_demand(); }
                    });
                } else {
                    let why = if from_studio { format!("The Studio picture stopped. {}", this.studio_problem()) } else { "The picture stopped.".to_string() };
                    this.each_view(|v| v.show_message(&why, "videocam_off"));
                }
            }
        });
    }

    fn present(&self, frame: Frame, label: &str) {
        let bytes = glib::Bytes::from_owned(frame.rgb);
        let texture = gdk::MemoryTexture::new(frame.width, frame.height, gdk::MemoryFormat::R8g8b8, &bytes, frame.width as usize * 3);
        let texture: gdk::Texture = texture.upcast();
        let views = self.live_views();
        let mut wanted = 0;
        for view in &views {
            if view.root.is_mapped() {
                view.picture.set_paintable(Some(&texture));
                view.placeholder.set_visible(false);
                view.chip.set_text(label);
                view.chip.set_visible(true);
                wanted = wanted.max((view.picture.width() * view.picture.scale_factor()).max(0) as u32);
            }
        }
        // Tell the decoder how big the largest view is, so it can decode smaller.
        if wanted > 0 { self.0.target_width.store(wanted, Ordering::Relaxed); }
        *self.0.last_texture.borrow_mut() = Some(texture);
        self.0.retries.set(0);
    }

    pub fn stop(&self) {
        let hub = &self.0;
        hub.generation.set(hub.generation.get().wrapping_add(1));
        hub.running.set(false);
        if let Some(mut child) = hub.child.borrow_mut().take() {
            // Reaping can block briefly; keep it off the UI thread.
            std::thread::spawn(move || { let _ = child.kill(); let _ = child.wait(); });
        }
    }
}

impl ViewInner {
    fn show_message(&self, message: &str, glyph: &str) {
        self.placeholder_text.set_text(message);
        // Paused or not started: the middle is a button that starts it, not just a picture of one.
        let startable = matches!(glyph, "play_arrow" | "pause_circle");
        widgets::set_icon(&self.glyph, glyph);
        self.glyph.set_visible(!startable);
        self.play.set_visible(startable);
        self.placeholder.set_visible(true);
        // A stopped or failed stream can be started again from here.
        self.retry.set_visible(glyph == "videocam_off");
        self.chip.set_text("");
        self.chip.set_visible(false);
    }
}

/// A place the preview is shown. Add `widget()` to a window.
#[derive(Clone)]
pub struct PreviewView {
    inner: Rc<ViewInner>,
}

impl PreviewView {
    pub fn new(hub: &PreviewHub) -> PreviewView {
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        widgets::label_accessible(&picture, "Live camera preview");

        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
        placeholder.set_halign(gtk::Align::Center);
        placeholder.set_valign(gtk::Align::Center);
        let glyph = widgets::icon("videocam");
        glyph.add_css_class("big");
        let placeholder_text = widgets::text("Starting preview…", &["preview-note"]);
        placeholder_text.set_justify(gtk::Justification::Center);
        placeholder_text.set_xalign(0.5);
        placeholder_text.set_max_width_chars(34);
        let retry = gtk::Button::with_label("Restart preview");
        retry.add_css_class("filled");
        retry.set_halign(gtk::Align::Center);
        retry.set_visible(false);
        let play = widgets::icon_button("play_arrow", "Start the preview", false);
        play.add_css_class("filled");
        play.add_css_class("preview-play");
        play.set_halign(gtk::Align::Center);
        play.set_visible(false);
        let start_hub = hub.clone();
        play.connect_clicked(move |_| start_hub.set_enabled(true));
        placeholder.append(&play);
        placeholder.append(&glyph);
        placeholder.append(&placeholder_text);
        placeholder.append(&retry);

        let chip = gtk::Label::new(None);
        chip.add_css_class("overlay-chip");
        chip.set_halign(gtk::Align::Start);
        chip.set_valign(gtk::Align::End);
        chip.set_margin_start(12);
        chip.set_margin_bottom(12);
        chip.set_can_target(false);
        chip.set_visible(false);

        let root = gtk::Overlay::new();
        root.add_css_class("preview-surface");
        root.set_overflow(gtk::Overflow::Hidden);
        root.set_child(Some(&picture));
        // Measures the view, so a locked one can take the height its width and the camera's shape give.
        let sensor = gtk::DrawingArea::new();
        sensor.set_can_target(false);
        sensor.set_hexpand(true);
        sensor.set_vexpand(true);
        root.add_overlay(&sensor);
        root.add_overlay(&placeholder);
        root.add_overlay(&chip);
        let marker = gtk::DrawingArea::new();
        marker.set_can_target(false);
        root.add_overlay(&marker);
        root.set_size_request(240, 135);

        let inner = Rc::new(ViewInner { picture, placeholder, placeholder_text, retry: retry.clone(), play, glyph, chip, root: root.clone(), ratio: Cell::new(16.0 / 9.0), locked: Cell::new(false), marker: marker.clone(), mark: Cell::new(None) });
        {
            let weak = Rc::downgrade(&inner);
            marker.set_draw_func(move |_, cr, _, _| {
                let Some(view) = weak.upgrade() else { return };
                let Some((x, y, mode)) = view.mark.get() else { return };
                // A square for focus, a circle for a person; white with a dark edge, to show on any picture.
                for (width, shade) in [(4.0, 0.0), (2.0, 1.0)] {
                    cr.set_source_rgba(shade, shade, shade, 0.9);
                    cr.set_line_width(width);
                    if mode == CLICK_FOCUS { cr.rectangle(x - 22.0, y - 22.0, 44.0, 44.0); } else { cr.arc(x, y, 26.0, 0.0, std::f64::consts::TAU); }
                    let _ = cr.stroke();
                }
            });
        }
        {
            // A click on the picture focuses there or follows that person, as the click button says.
            let click = gtk::GestureClick::new();
            click.set_button(1);
            let (weak, store) = (Rc::downgrade(&inner), hub.0.store.clone());
            click.connect_released(move |_, _, px, py| {
                let mode = click_mode();
                let Some(view) = weak.upgrade() else { return };
                if mode == CLICK_OFF || view.placeholder.is_visible() { return; }
                let Some((x, y)) = picture_point(view.root.width() as f64, view.root.height() as f64, view.ratio.get() as f64, px, py) else { return };
                view.mark.set(Some((px, py, mode)));
                view.marker.queue_draw();
                let later = Rc::downgrade(&view);
                glib::timeout_add_local_once(Duration::from_millis(900), move || {
                    if let Some(view) = later.upgrade() { view.mark.set(None); view.marker.queue_draw(); }
                });
                let done = store.clone();
                store.call(None, move |device| if mode == CLICK_FOCUS {
                    crate::backend::device::set_focus_point(device, x, y)
                } else {
                    crate::backend::device::track_at(device, x, y)
                }, move |result| {
                    if let Err(error) = result { done.error(&format!("The camera did not take the click: {error}")); }
                });
            });
            root.add_controller(click);
            let motion = gtk::EventControllerMotion::new();
            let weak = root.downgrade();
            motion.connect_enter(move |_, _, _| {
                if let Some(root) = weak.upgrade() { root.set_cursor_from_name(if click_mode() == CLICK_OFF { None } else { Some("crosshair") }); }
            });
            root.add_controller(motion);
        }
        if let Some(mode) = hub.0.mode.borrow().as_ref() { inner.ratio.set(mode.width as f32 / mode.height.max(1) as f32); }
        {
            let weak = Rc::downgrade(&inner);
            hub.on_mode(move |mode| {
                if let Some(view) = weak.upgrade() {
                    view.ratio.set(mode.width as f32 / mode.height.max(1) as f32);
                    view.root.queue_resize();
                }
            });
        }
        {
            let weak = Rc::downgrade(&inner);
            sensor.connect_resize(move |_, width, height| {
                let Some(view) = weak.upgrade() else { return };
                if !view.locked.get() || width <= 0 { return; }
                let wanted = (width as f32 / view.ratio.get()).round() as i32;
                if (height - wanted).abs() > 1 { view.root.set_size_request(240, wanted.max(60)); }
            });
        }
        let restart_hub = hub.clone();
        retry.connect_clicked(move |_| restart_hub.restart());
        if !hub.is_enabled() {
            // Paused with its button, or simply not started yet when the app opened.
            if current_config().preview_paused { inner.show_message("Preview paused", "pause_circle"); }
            else { inner.show_message("Press play to start the preview", "play_arrow"); }
        }
        hub.0.views.borrow_mut().push(Rc::downgrade(&inner));
        if let Some(texture) = hub.0.last_texture.borrow().as_ref() {
            inner.picture.set_paintable(Some(texture));
            inner.placeholder.set_visible(false);
        }
        let h = hub.clone();
        root.connect_map(move |_| h.update_demand());
        let h = hub.clone();
        root.connect_unmap(move |_| h.update_demand());
        PreviewView { inner }
    }

    pub fn widget(&self) -> &gtk::Overlay { &self.inner.root }

    /// Make the view exactly as tall as its width and the camera's shape say, so the picture fills it
    /// with no black bars. The width is whatever the layout gives it.
    pub fn lock_aspect(&self) { self.inner.locked.set(true); self.inner.root.queue_resize(); }

    /// Put a control on top of the picture, in the top-right corner row.
    pub fn add_control(&self, overlay_row: &gtk::Box) {
        overlay_row.set_halign(gtk::Align::End);
        overlay_row.set_valign(gtk::Align::Start);
        overlay_row.set_margin_top(10);
        overlay_row.set_margin_end(10);
        self.inner.root.add_overlay(overlay_row);
    }
}

// ── Frame reading and decoding (background thread) ────────────────────────

fn read_frames<R: Read>(mut reader: R, sender: async_channel::Sender<Frame>, mode: VideoMode, width_hint: Arc<AtomicU32>) {
    let mut buffer = Vec::<u8>::new();
    let mut chunk = [0u8; 64 * 1024];
    let raw_frame_size = mode.width as usize * mode.height as usize * 2;
    // Frames of a 30 fps stream arrive about 33 ms apart, a little early or late; a gap of a full 33 ms
    // threw away every early one. Three quarters of it shows every frame at 30 fps and every other at 60.
    let min_gap = Duration::from_millis(1000 / MAX_FPS * 3 / 4);
    let mut last_decode = Instant::now() - min_gap;
    loop {
        let count = match reader.read(&mut chunk) { Ok(0) | Err(_) => return, Ok(count) => count };
        buffer.extend_from_slice(&chunk[..count]);
        if mode.pixel_format == "MJPG" {
            loop {
                let Some(start) = buffer.windows(2).position(|pair| pair == [0xff, 0xd8]) else {
                    if buffer.len() > 1 { buffer.drain(..buffer.len() - 1); }
                    break;
                };
                if start > 0 { buffer.drain(..start); }
                let Some(end) = buffer.windows(2).position(|pair| pair == [0xff, 0xd9]) else {
                    if buffer.len() > 16 * 1024 * 1024 { buffer.clear(); }
                    break;
                };
                let jpeg: Vec<u8> = buffer.drain(..end + 2).collect();
                // Drop frames we cannot show in time: decoding is the expensive part.
                if last_decode.elapsed() < min_gap || sender.is_full() { continue; }
                last_decode = Instant::now();
                if let Some(mut frame) = decode_jpeg(&jpeg, decode_width(width_hint.load(Ordering::Relaxed))) {
                    if FOCUS_PEAKING.load(Ordering::Relaxed) { focus_peaking(&mut frame); }
                    if sender.send_blocking(frame).is_err() { return; }
                }
            }
        } else if mode.pixel_format == "YU12" {
            // Raw 4:2:0 (what the Studio camera sends): a full-size brightness plane, then two
            // quarter-size colour planes.
            let size = mode.width as usize * mode.height as usize * 3 / 2;
            while size > 0 && buffer.len() >= size {
                let raw: Vec<u8> = buffer.drain(..size).collect();
                if last_decode.elapsed() < min_gap || sender.is_full() { continue; }
                last_decode = Instant::now();
                if let Some(mut frame) = decode_i420(&raw, &mode) {
                    if FOCUS_PEAKING.load(Ordering::Relaxed) { focus_peaking(&mut frame); }
                    if sender.send_blocking(frame).is_err() { return; }
                }
            }
            if buffer.len() > size.saturating_mul(3) { buffer.drain(..buffer.len() - size); }
        } else if mode.pixel_format == "YUYV" {
            while raw_frame_size > 0 && buffer.len() >= raw_frame_size {
                let raw: Vec<u8> = buffer.drain(..raw_frame_size).collect();
                if last_decode.elapsed() < min_gap || sender.is_full() { continue; }
                last_decode = Instant::now();
                if let Some(mut frame) = decode_yuyv(&raw, &mode) {
                    if FOCUS_PEAKING.load(Ordering::Relaxed) { focus_peaking(&mut frame); }
                    if sender.send_blocking(frame).is_err() { return; }
                }
            }
            if buffer.len() > raw_frame_size.saturating_mul(3) { buffer.drain(..buffer.len() - raw_frame_size); }
        } else {
            return;
        }
    }
}

/// Where the preview's stream reader writes what it says.
fn preview_log_path() -> std::path::PathBuf { crate::service::runtime_dir().join("preview.log") }

/// Focus peaking: paint the sharpest edges in the picture green, so what is in focus stands out.
// ── Clicking on the picture ──────────────────────────────────────────────

pub const CLICK_OFF: u8 = 0;
pub const CLICK_FOCUS: u8 = 1;
pub const CLICK_TRACK: u8 = 2;
static CLICK_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(CLICK_OFF);
thread_local! {
    static CLICK_BUTTONS: RefCell<Vec<glib::WeakRef<gtk::Button>>> = const { RefCell::new(Vec::new()) };
}

pub fn click_mode() -> u8 { CLICK_MODE.load(Ordering::Relaxed) }

fn set_click_mode(mode: u8) {
    CLICK_MODE.store(mode, Ordering::Relaxed);
    CLICK_BUTTONS.with(|buttons| buttons.borrow_mut().retain(|weak| match weak.upgrade() {
        Some(button) => { show_click_mode(&button); true }
        None => false,
    }));
}

fn show_click_mode(button: &gtk::Button) {
    let (glyph, tip) = match click_mode() {
        CLICK_FOCUS => ("center_focus_strong", "Click on the picture: focus there"),
        CLICK_TRACK => ("person_search", "Click on the picture: follow that person"),
        _ => ("ads_click", "What a click on the picture does"),
    };
    button.set_child(Some(&widgets::icon(glyph)));
    button.set_tooltip_text(Some(tip));
    if click_mode() == CLICK_OFF { button.remove_css_class("lit"); } else { button.add_css_class("lit"); }
}

/// Where (px, py) on a view of w × h falls on the picture it shows (fitted inside, keeping its shape
/// `ratio`), from 0 to 1 across and down; `None` on the bars beside or above it.
pub fn picture_point(w: f64, h: f64, ratio: f64, px: f64, py: f64) -> Option<(f32, f32)> {
    if w <= 0.0 || h <= 0.0 || ratio <= 0.0 { return None; }
    let (shown_w, shown_h) = if w / h > ratio { (h * ratio, h) } else { (w, w / ratio) };
    let x = (px - (w - shown_w) / 2.0) / shown_w;
    let y = (py - (h - shown_h) / 2.0) / shown_h;
    ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some((x as f32, y as f32))
}

static FOCUS_PEAKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn focus_peaking_on() -> bool { FOCUS_PEAKING.load(Ordering::Relaxed) }

/// How strict focus peaking is, 0–100.
static PEAKING_STRICTNESS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(60);

pub fn set_peaking_strictness(value: u8) {
    PEAKING_STRICTNESS.store(value.min(100), Ordering::Relaxed);
    update_config(|c| c.preview_peaking_strictness = value.min(100));
}

/// What strictness means: the edge strength Canny's detector needs to start an edge (its high threshold, on
/// the Sobel gradient of the slightly blurred picture): 40 at 0, 240 at 100. Edges then continue down to
/// half of that.
fn peaking_threshold(strictness: u8) -> f32 {
    40.0 + f32::from(strictness.min(100)) * 2.0
}

pub fn set_focus_peaking(on: bool) {
    FOCUS_PEAKING.store(on, Ordering::Relaxed);
    update_config(|c| c.preview_focus_peaking = on);
}

/// The colour edges in focus are painted.
const PEAK_COLOUR: [u8; 3] = [40, 255, 90];

/// The edges in focus, as a mask of the picture: the Canny edge detector from the imageproc library (a light
/// Gaussian blur against noise, Sobel gradients, thinning to one-pixel lines, and a strong/weak threshold
/// pair so an edge is followed as long as it holds up). Sharper focus makes edges steeper, so more of them
/// clear the threshold; the strictness slider sets the threshold.
fn peak_mask(width: u32, height: u32, luma: Vec<u8>) -> Option<Vec<u8>> {
    let gray = imageproc::image::GrayImage::from_raw(width, height, luma)?;
    let high = peaking_threshold(PEAKING_STRICTNESS.load(Ordering::Relaxed));
    Some(imageproc::edges::canny(&gray, high / 2.0, high).into_raw())
}

fn luma_of(frame: &Frame) -> Vec<u8> {
    frame.rgb.chunks_exact(3).map(|p| ((u16::from(p[0]) * 77 + u16::from(p[1]) * 150 + u16::from(p[2]) * 29) >> 8) as u8).collect()
}

/// Paint a mask's edges green, two pixels wide so they stay visible when the picture is shown smaller.
fn paint_mask(frame: &mut Frame, mask: &[u8]) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if mask.len() != w * h || frame.rgb.len() < w * h * 3 { return; }
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask[i] > 0 || (x > 0 && mask[i - 1] > 0) || (y > 0 && mask[i - w] > 0) {
                frame.rgb[i * 3..i * 3 + 3].copy_from_slice(&PEAK_COLOUR);
            }
        }
    }
}

/// The edges found last, with the picture size they belong to.
struct PeakState { width: i32, height: i32, mask: Vec<u8>, found: Instant, asked: Option<Instant> }

static PEAKS: std::sync::Mutex<Option<PeakState>> = std::sync::Mutex::new(None);
static PEAK_WORKER: std::sync::OnceLock<std::sync::mpsc::SyncSender<(i32, i32, Vec<u8>)>> = std::sync::OnceLock::new();
/// How often the edges are looked for: the detector is the expensive part, and focus changes slowly next to
/// the frame rate, so the picture keeps its full speed and the outline follows a few times a second.
const PEAK_EVERY: Duration = Duration::from_millis(200);

/// Outline the edges in focus. The detector runs on its own thread, at most five times a second, on the
/// newest picture; every picture is painted with the latest edges found, so the preview keeps its speed.
fn focus_peaking(frame: &mut Frame) {
    if frame.width < 8 || frame.height < 8 { return; }
    let worker = PEAK_WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<(i32, i32, Vec<u8>)>(1);
        std::thread::spawn(move || {
            while let Ok((width, height, luma)) = receiver.recv() {
                if let Some(mask) = peak_mask(width as u32, height as u32, luma) {
                    if let Ok(mut peaks) = PEAKS.lock() {
                        let asked = peaks.as_ref().and_then(|p| p.asked);
                        *peaks = Some(PeakState { width, height, mask, found: Instant::now(), asked });
                    }
                }
            }
        });
        sender
    });
    let Ok(mut peaks) = PEAKS.lock() else { return };
    let due = peaks.as_ref().and_then(|p| p.asked).is_none_or(|at| at.elapsed() >= PEAK_EVERY);
    if due && worker.try_send((frame.width, frame.height, luma_of(frame))).is_ok() {
        match peaks.as_mut() {
            Some(p) => p.asked = Some(Instant::now()),
            None => *peaks = Some(PeakState { width: 0, height: 0, mask: Vec::new(), found: Instant::now(), asked: Some(Instant::now()) }),
        }
    }
    // The latest edges, if they belong to a picture of this size and are recent.
    if let Some(p) = peaks.as_ref() {
        if p.width == frame.width && p.height == frame.height && p.found.elapsed() < Duration::from_secs(2) {
            paint_mask(frame, &p.mask);
        }
    }
}

/// Outline the edges at once, on this thread (for tests).
#[cfg(test)]
fn focus_peaking_now(frame: &mut Frame) {
    if let Some(mask) = peak_mask(frame.width as u32, frame.height as u32, luma_of(frame)) { paint_mask(frame, &mask); }
}

/// How wide to decode for the preview: the window's width, or, with focus peaking on, at least 960 pixels,
/// so edges are judged on a picture not shrunk too far (shrinking makes a soft edge look sharp). A JPEG is
/// decoded at a half or a quarter of its size for that, which costs far less than a full decode.
fn decode_width(target: u32) -> u32 {
    if FOCUS_PEAKING.load(Ordering::Relaxed) { target.max(960) } else { target }
}

fn decode_jpeg(data: &[u8], target_width: u32) -> Option<Frame> {
    let mut decoder = Decoder::new(Cursor::new(data));
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    // JPEG can be decoded at 1/2, 1/4 or 1/8 size for a fraction of the work.
    let wanted = target_width.clamp(160, 4096) as u16;
    let ratio = f64::from(info.height) / f64::from(info.width);
    let _ = decoder.scale(wanted, (f64::from(wanted) * ratio).ceil() as u16);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    let rgb = match info.pixel_format {
        PixelFormat::RGB24 => pixels,
        PixelFormat::L8 => pixels.into_iter().flat_map(|v| [v, v, v]).collect(),
        _ => return None,
    };
    if rgb.len() != width * height * 3 { return None; }
    Some(Frame { width: width as i32, height: height as i32, rgb })
}

/// YUYV to RGB at half size each way, averaging each 2 × 2 block (one YUYV pair holds two pixels across, so
/// a block is two pairs, one above the other).
fn decode_yuyv_half(frame: &[u8], width: usize, height: usize) -> Frame {
    let (out_w, out_h) = (width / 2, height / 2);
    let stride = width * 2;
    let mut rgb = Vec::with_capacity(out_w * out_h * 3);
    for y in 0..out_h {
        let (top, bottom) = (&frame[(y * 2) * stride..(y * 2 + 1) * stride], &frame[(y * 2 + 1) * stride..(y * 2 + 2) * stride]);
        for x in 0..out_w {
            let (a, b) = (&top[x * 4..x * 4 + 4], &bottom[x * 4..x * 4 + 4]);
            let luma = (i32::from(a[0]) + i32::from(a[2]) + i32::from(b[0]) + i32::from(b[2]) + 2) / 4;
            let u = (i32::from(a[1]) + i32::from(b[1]) + 1) / 2 - 128;
            let v = (i32::from(a[3]) + i32::from(b[3]) + 1) / 2 - 128;
            let c = (luma - 16).max(0);
            rgb.push(((298 * c + 409 * v + 128) >> 8).clamp(0, 255) as u8);
            rgb.push(((298 * c - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8);
            rgb.push(((298 * c + 516 * u + 128) >> 8).clamp(0, 255) as u8);
        }
    }
    Frame { width: out_w as i32, height: out_h as i32, rgb }
}

/// Raw 4:2:0 to RGB, at half size each way: plenty for a preview, and a quarter of the work.
fn decode_i420(frame: &[u8], mode: &VideoMode) -> Option<Frame> {
    let (w, h) = (mode.width as usize, mode.height as usize);
    if w < 2 || h < 2 || frame.len() < w * h * 3 / 2 { return None; }
    let (luma, rest) = frame.split_at(w * h);
    let (u_plane, v_plane) = rest.split_at(w * h / 4);
    let (out_w, out_h) = (w / 2, h / 2);
    let mut rgb = Vec::with_capacity(out_w * out_h * 3);
    for y in 0..out_h {
        for x in 0..out_w {
            let c = i32::from(luma[(y * 2) * w + x * 2]) - 16;
            let u = i32::from(u_plane[y * (w / 2) + x]) - 128;
            let v = i32::from(v_plane[y * (w / 2) + x]) - 128;
            let c = c.max(0);
            rgb.push(((298 * c + 409 * v + 128) >> 8).clamp(0, 255) as u8);
            rgb.push(((298 * c - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8);
            rgb.push(((298 * c + 516 * u + 128) >> 8).clamp(0, 255) as u8);
        }
    }
    Some(Frame { width: out_w as i32, height: out_h as i32, rgb })
}

fn decode_yuyv(frame: &[u8], mode: &VideoMode) -> Option<Frame> {
    if mode.width % 2 != 0 { return None; }
    let (width, height) = (mode.width as usize, mode.height as usize);
    if frame.len() < width * height * 2 { return None; }
    // A big picture (the Studio camera at 1080p or 4K) is shown at half size, each pixel the average of
    // the four it covers: smooth edges, a quarter of the work.
    if width >= 1280 && height % 2 == 0 { return Some(decode_yuyv_half(frame, width, height)); }
    let mut rgb = Vec::with_capacity(width * height * 3);
    for pair in frame[..width * height * 2].chunks_exact(4) {
        let u = i32::from(pair[1]) - 128;
        let v = i32::from(pair[3]) - 128;
        for y in [pair[0], pair[2]] {
            let c = (i32::from(y) - 16).max(0);
            rgb.push(((298 * c + 409 * v + 128) >> 8).clamp(0, 255) as u8);
            rgb.push(((298 * c - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8);
            rgb.push(((298 * c + 516 * u + 128) >> 8).clamp(0, 255) as u8);
        }
    }
    Some(Frame { width: width as i32, height: height as i32, rgb })
}

/// A simple gradient with a grid, used only by the synthetic test device.
fn test_card(width: i32, height: i32) -> Frame {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let grid = x % 80 == 0 || y % 80 == 0;
            let base = [(40 + x * 90 / width) as u8, (60 + y * 90 / height) as u8, 110];
            let px = if grid { [150, 160, 180] } else { base };
            rgb.extend_from_slice(&px);
        }
    }
    Frame { width, height, rgb }
}


#[cfg(test)]
mod peaking_tests {
    use super::*;

    fn frame(width: i32, height: i32, pixel: impl Fn(i32, i32) -> u8) -> Frame {
        let mut rgb = Vec::new();
        for y in 0..height { for x in 0..width { let v = pixel(x, y); rgb.extend_from_slice(&[v, v, v]); } }
        Frame { width, height, rgb }
    }

    fn green(f: &Frame) -> usize { f.rgb.chunks_exact(3).filter(|p| *p == PEAK_COLOUR).count() }

    #[test]
    fn a_big_yuyv_picture_is_shown_at_half_size_with_averaged_pixels() {
        let mode = VideoMode { width: 1280, height: 2, pixel_format: "YUYV".into(), frame_rate_milli: 30_000 };
        // Alternating black and white columns, neutral colour: averaged to grey, not jagged.
        let mut raw = Vec::new();
        for _ in 0..2 { for _ in 0..640 { raw.extend([16u8, 128, 235, 128]); } }
        let frame = decode_yuyv(&raw, &mode).expect("a frame");
        assert_eq!((frame.width, frame.height), (640, 1));
        assert!(frame.rgb.iter().all(|v| (120..=136).contains(v)), "{:?}", &frame.rgb[..6]);
    }

    #[test]
    fn a_click_maps_onto_the_picture_and_not_the_bars_beside_it() {
        // A 16:9 picture in a 400 × 300 view: 400 × 225, with 37.5 px bars above and below.
        assert_eq!(picture_point(400.0, 300.0, 16.0 / 9.0, 200.0, 150.0), Some((0.5, 0.5)));
        assert_eq!(picture_point(400.0, 300.0, 16.0 / 9.0, 0.0, 37.5), Some((0.0, 0.0)));
        assert_eq!(picture_point(400.0, 300.0, 16.0 / 9.0, 200.0, 20.0), None);
        // In a wide view the bars are at the sides: 160 × 90 picture in 320 × 90, from x 80.
        assert_eq!(picture_point(320.0, 90.0, 16.0 / 9.0, 120.0, 45.0), Some((0.25, 0.5)));
        assert_eq!(picture_point(320.0, 90.0, 16.0 / 9.0, 40.0, 45.0), None);
    }

    #[test]
    fn raw_4_2_0_from_the_studio_camera_is_shown_at_half_size() {
        let mode = VideoMode { width: 4, height: 2, pixel_format: "YU12".into(), frame_rate_milli: 30_000 };
        // White brightness, neutral colour: comes out white, 2 × 1 pixels.
        let mut raw = vec![235u8; 8];
        raw.extend([128u8; 4]);
        let frame = decode_i420(&raw, &mode).expect("a frame");
        assert_eq!((frame.width, frame.height), (2, 1));
        assert!(frame.rgb.iter().all(|v| *v >= 250), "{:?}", frame.rgb);
        assert!(decode_i420(&raw[..10], &mode).is_none(), "a short frame is skipped");
    }

    #[test]
    fn a_sharp_edge_is_painted_and_a_flat_or_soft_picture_is_not() {
        // A hard step from dark to light down the middle: the pixels at the step are marked.
        let mut sharp = frame(64, 32, |x, _| if x < 32 { 20 } else { 230 });
        focus_peaking_now(&mut sharp);
        assert!(green(&sharp) > 0);
        let at_edge = (4..28).all(|y| { let i = (y * 64 + 32) as usize * 3; sharp.rgb[i..i + 3] == PEAK_COLOUR });
        assert!(at_edge, "the step itself is marked");
        // A flat picture and a gentle gradient (out of focus) have no edges worth marking.
        let mut flat = frame(64, 32, |_, _| 120);
        focus_peaking_now(&mut flat);
        assert_eq!(green(&flat), 0);
        let mut soft = frame(64, 32, |x, _| (x * 3) as u8);
        focus_peaking_now(&mut soft);
        assert_eq!(green(&soft), 0);
        // A big but blurred step (dark to light over twelve pixels) is high contrast, yet not in focus.
        let mut blurred = frame(64, 32, |x, _| (20 + (x - 26).clamp(0, 12) * 17) as u8);
        focus_peaking_now(&mut blurred);
        assert_eq!(green(&blurred), 0, "a blurred edge is not marked");
    }

    #[test]
    fn a_blurred_edge_is_not_marked_and_a_sharp_one_is() {
        // Bars across the picture: hard edges on the left, the same bars blurred on the right.
        let bars = |x: i32| -> f64 {
            let phase = (x % 32) as f64;
            let soft = x >= 160;
            let edge = |d: f64| if soft { (d / 10.0 + 0.5).clamp(0.0, 1.0) } else if d >= 0.0 { 1.0 } else { 0.0 };
            40.0 + 180.0 * (edge(phase - 8.0) - edge(phase - 24.0))
        };
        let mut picture = frame(320, 96, |x, _| bars(x).round() as u8);
        focus_peaking_now(&mut picture);
        let marked = |from: i32, to: i32| (8..88).flat_map(|y| (from..to).map(move |x| (y * 320 + x) as usize * 3))
            .filter(|i| picture.rgb[*i..*i + 3] == PEAK_COLOUR).count();
        assert!(marked(16, 144) > 100, "the sharp side is outlined");
        assert_eq!(marked(176, 304), 0, "the blurred side is not");
    }

    #[test]
    #[ignore]
    fn time_jpeg_decodes() {
        let dir = std::env::var("T4_JPEGS").unwrap_or_default();
        for (name, width) in [("f4k.jpg", 640), ("f4k.jpg", 960), ("f4k.jpg", 1920), ("f1080.jpg", 640), ("f1080.jpg", 960)] {
            let Ok(data) = std::fs::read(format!("{dir}/{name}")) else { continue };
            let start = std::time::Instant::now();
            for _ in 0..10 { decode_jpeg(&data, width).unwrap(); }
            println!("{name} at {width}: {:?} a frame", start.elapsed() / 10);
        }
        let raw = vec![128u8; 3840 * 2160 * 2];
        let mode = VideoMode { width: 3840, height: 2160, pixel_format: "YUYV".into(), frame_rate_milli: 30_000 };
        let start = std::time::Instant::now();
        for _ in 0..10 { decode_yuyv(&raw, &mode).unwrap(); }
        println!("YUYV 4K: {:?} a frame", start.elapsed() / 10);
    }

    #[test]
    #[ignore]
    fn time_a_1080p_frame() {
        for (w, h) in [(1920, 1080), (1280, 720), (960, 540)] {
            let mut picture = frame(w, h, |x, y| ((x * 7 + y * 13) % 251) as u8);
            let start = std::time::Instant::now();
            focus_peaking_now(&mut picture);
            println!("focus peaking on {w}x{h}: {:?}", start.elapsed());
        }
    }

}
