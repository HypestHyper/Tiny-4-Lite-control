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
    frame: gtk::AspectFrame,
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
}

#[derive(Clone)]
pub struct PreviewHub(Rc<HubInner>);

impl PreviewHub {
    pub fn new(store: &Store) -> PreviewHub {
        PreviewHub(Rc::new(HubInner {
            store: store.clone(),
            views: RefCell::default(),
            child: RefCell::default(),
            generation: Cell::new(0),
            running: Cell::new(false),
            enabled: Cell::new(!current_config().preview_paused),
            mode: RefCell::default(),
            target_width: Arc::new(AtomicU32::new(640)),
            stop_timer: RefCell::default(),
            last_texture: RefCell::default(),
            retries: Cell::new(0),
            mode_listeners: RefCell::default(),
        }))
    }

    pub fn is_enabled(&self) -> bool { self.0.enabled.get() }
    pub fn is_running(&self) -> bool { self.0.running.get() }

    /// Be told the capture mode the camera is really using each time the preview starts, so other
    /// parts of the window can show the same one.
    pub fn on_mode(&self, listener: impl Fn(&VideoMode) + 'static) {
        self.0.mode_listeners.borrow_mut().push(Rc::new(listener));
    }

    /// Start the stream again after it stopped or failed.
    pub fn restart(&self) {
        let hub = &self.0;
        hub.retries.set(0);
        let child = self.take_stream();
        if let Some(mut child) = child { std::thread::spawn(move || { let _ = child.kill(); let _ = child.wait(); }); }
        if !hub.enabled.get() { hub.enabled.set(true); update_config(|c| c.preview_paused = false); }
        // A moment for the camera to let go of the old stream.
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(500), move || this.update_demand());
    }

    /// Pause or resume the live picture (kept off the camera when paused).
    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.set(enabled);
        self.0.retries.set(0);
        update_config(|c| c.preview_paused = !enabled);
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
                    if sender.send_blocking(test_card(w, h)).is_err() { return; }
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
        if let Some(first) = self.placeholder.first_child().and_then(|c| c.downcast::<gtk::Label>().ok()) { widgets::set_icon(&first, glyph); }
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
        // The picture keeps the camera's shape (16:9 until the stream says otherwise), so the frame
        // hugs it and there are no black bars. The frame's height follows its width.
        let frame = gtk::AspectFrame::new(0.5, 0.5, 16.0 / 9.0, false);
        frame.set_child(Some(&picture));
        root.set_child(Some(&frame));
        root.add_overlay(&placeholder);
        root.add_overlay(&chip);
        root.set_size_request(240, 135);

        let inner = Rc::new(ViewInner { picture, placeholder, placeholder_text, retry: retry.clone(), chip, root: root.clone(), frame: frame.clone() });
        if let Some(mode) = hub.0.mode.borrow().as_ref() { frame.set_ratio(mode.width as f32 / mode.height.max(1) as f32); }
        {
            let weak = Rc::downgrade(&inner);
            hub.on_mode(move |mode| {
                if let Some(view) = weak.upgrade() { view.frame.set_ratio(mode.width as f32 / mode.height.max(1) as f32); }
            });
        }
        let restart_hub = hub.clone();
        retry.connect_clicked(move |_| restart_hub.restart());
        if !hub.is_enabled() { inner.show_message("Preview paused", "pause_circle"); }
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
                if let Some(frame) = decode_jpeg(&jpeg, width_hint.load(Ordering::Relaxed)) {
                    if sender.send_blocking(frame).is_err() { return; }
                }
            }
        } else if mode.pixel_format == "YUYV" {
            while raw_frame_size > 0 && buffer.len() >= raw_frame_size {
                let raw: Vec<u8> = buffer.drain(..raw_frame_size).collect();
                if last_decode.elapsed() < min_gap || sender.is_full() { continue; }
                last_decode = Instant::now();
                if let Some(frame) = decode_yuyv(&raw, &mode) {
                    if sender.send_blocking(frame).is_err() { return; }
                }
            }
            if buffer.len() > raw_frame_size.saturating_mul(3) { buffer.drain(..buffer.len() - raw_frame_size); }
        } else {
            return;
        }
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

