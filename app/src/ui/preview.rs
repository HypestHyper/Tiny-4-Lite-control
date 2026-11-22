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

    /// The round button that turns focus peaking on and off, lit while it is on, with its strictness
    /// slider beside it while it is on.
    pub fn peaking_button(&self) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        slider.set_draw_value(false);
        slider.set_size_request(110, -1);
        slider.set_value(f64::from(PEAKING_STRICTNESS.load(Ordering::Relaxed)));
        slider.add_css_class("on-stage-slider");
        slider.set_tooltip_text(Some("Strictness: to the right, only the sharpest edges are outlined"));
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
        // Reading the current mode asks the camera, so do it off the UI thread.
        hub.store.call(None, |device| video::current_mode(device), move |mode| {
            if this.0.generation.get() != generation { return; }
            let Some(mode) = mode else {
                this.0.running.set(false);
                this.each_view(|v| v.show_message("The camera did not report a video mode, so there is nothing to preview.", "videocam_off"));
                return;
            };
            this.spawn_stream(mode, generation);
        });
    }

    fn spawn_stream(&self, mode: VideoMode, generation: u64) {
        let hub = &self.0;
        *hub.mode.borrow_mut() = Some(mode.clone());
        let listeners: Vec<_> = hub.mode_listeners.borrow().clone();
        for listener in listeners { listener(&mode); }
        let device = hub.store.device();
        let mut child = if super::super::backend::ui_test_mode() {
            None
        } else {
            match Command::new("v4l2-ctl").arg("-d").arg(&device)
                .arg("--stream-mmap=4").arg("--stream-count=1000000").arg("--stream-to=-")
                .stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
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
        let label = format!("{} × {} · {} · {:.0} fps", mode.width, mode.height, mode.pixel_format, mode.frame_rate_milli as f64 / 1000.0);
        glib::spawn_future_local(async move {
            while let Ok(frame) = receiver.recv().await {
                if this.0.generation.get() != generation { return; }
                this.present(frame, &label);
            }
            if this.0.generation.get() == generation && this.0.running.get() {
                this.0.running.set(false);
                // A hiccup in the stream is common, so try again a few times before giving up.
                if this.0.retries.get() < 3 && this.0.enabled.get() {
                    this.0.retries.set(this.0.retries.get() + 1);
                    this.each_view(|v| v.show_message("The picture stopped. Reconnecting…", "videocam"));
                    let again = this.clone();
                    glib::timeout_add_local_once(Duration::from_secs(2), move || {
                        if !again.0.running.get() { again.update_demand(); }
                    });
                } else {
                    this.each_view(|v| v.show_message("The picture stopped.", "videocam_off"));
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
        root.set_size_request(240, 135);

        let inner = Rc::new(ViewInner { picture, placeholder, placeholder_text, retry: retry.clone(), play, glyph, chip, root: root.clone(), ratio: Cell::new(16.0 / 9.0), locked: Cell::new(false) });
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
    let min_gap = Duration::from_millis(1000 / MAX_FPS);
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
                if let Some(mut frame) = decode_jpeg(&jpeg, width_hint.load(Ordering::Relaxed)) {
                    if FOCUS_PEAKING.load(Ordering::Relaxed) { focus_peaking(&mut frame); }
                    if sender.send_blocking(frame).is_err() { return; }
                }
            }
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

/// Focus peaking: paint the sharpest edges in the picture green, so what is in focus stands out.
static FOCUS_PEAKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn focus_peaking_on() -> bool { FOCUS_PEAKING.load(Ordering::Relaxed) }

/// How strict focus peaking is, 0–100.
static PEAKING_STRICTNESS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(60);

pub fn set_peaking_strictness(value: u8) {
    PEAKING_STRICTNESS.store(value.min(100), Ordering::Relaxed);
    update_config(|c| c.preview_peaking_strictness = value.min(100));
}

/// What strictness means: the share of the picture's edges that are marked (12% at 0, 1% at 100), and
/// the smallest step in brightness that can count as an edge at all (30 at 0, 140 at 100).
fn peaking_levels(strictness: u8) -> (u32, u16) {
    let s = u32::from(strictness.min(100));
    let per_thousand = 120 - s * 110 / 100;
    let floor = 30 + (s * 110 / 100) as u16;
    (per_thousand, floor)
}

pub fn set_focus_peaking(on: bool) {
    FOCUS_PEAKING.store(on, Ordering::Relaxed);
    update_config(|c| c.preview_focus_peaking = on);
}

/// The colour edges in focus are painted.
const PEAK_COLOUR: [u8; 3] = [40, 255, 90];

/// Mark the pixels with the strongest local contrast. The contrast of each pixel is the difference
/// to its neighbours left/right and up/down (in brightness). What counts as sharp adapts to the
/// picture: the strongest few per cent, but never weak edges in a flat or blurred picture.
fn focus_peaking(frame: &mut Frame) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w < 3 || h < 3 || frame.rgb.len() < w * h * 3 { return; }
    let luma: Vec<u16> = frame.rgb.chunks_exact(3).map(|p| (u16::from(p[0]) * 77 + u16::from(p[1]) * 150 + u16::from(p[2]) * 29) >> 8).collect();
    let mut strength = vec![0u16; w * h];
    let mut histogram = [0u32; 512];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let dx = luma[i + 1].abs_diff(luma[i - 1]);
            let dy = luma[i + w].abs_diff(luma[i - w]);
            let s = (dx + dy).min(511);
            strength[i] = s;
            histogram[usize::from(s)] += 1;
        }
    }
    // The threshold: the strongest few per cent of edges, and at least a clear step in brightness, both
    // set by the strictness.
    let (per_thousand, floor) = peaking_levels(PEAKING_STRICTNESS.load(Ordering::Relaxed));
    let wanted = ((w - 2) * (h - 2)) as u32 * per_thousand / 1000;
    let mut count = 0u32;
    let mut threshold = 511u16;
    for level in (0..512).rev() {
        count += histogram[level];
        if count >= wanted { threshold = level as u16; break; }
    }
    let threshold = threshold.max(floor);
    for (i, s) in strength.iter().enumerate() {
        if *s >= threshold { frame.rgb[i * 3..i * 3 + 3].copy_from_slice(&PEAK_COLOUR); }
    }
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

fn decode_yuyv(frame: &[u8], mode: &VideoMode) -> Option<Frame> {
    if mode.width % 2 != 0 { return None; }
    let (width, height) = (mode.width as usize, mode.height as usize);
    if frame.len() < width * height * 2 { return None; }
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
    fn a_sharp_edge_is_painted_and_a_flat_or_soft_picture_is_not() {
        // A hard step from dark to light down the middle: the pixels at the step are marked.
        let mut sharp = frame(64, 32, |x, _| if x < 32 { 20 } else { 230 });
        focus_peaking(&mut sharp);
        assert!(green(&sharp) > 0);
        let at_edge = (1..31).all(|y| { let i = (y * 64 + 32) as usize * 3; sharp.rgb[i..i + 3] == PEAK_COLOUR });
        assert!(at_edge, "the step itself is marked");
        // A flat picture and a gentle gradient (out of focus) have no edges worth marking.
        let mut flat = frame(64, 32, |_, _| 120);
        focus_peaking(&mut flat);
        assert_eq!(green(&flat), 0);
        let mut soft = frame(64, 32, |x, _| (x * 3) as u8);
        focus_peaking(&mut soft);
        assert_eq!(green(&soft), 0);
    }
}
