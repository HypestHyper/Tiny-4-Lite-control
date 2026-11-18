use gdk_pixbuf::{Colorspace, Pixbuf, PixbufLoader};
use gtk::prelude::*;
use std::cell::RefCell;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, SyncSender, TryRecvError, TrySendError};
use std::thread;
use std::time::Duration;
use crate::video::VideoMode;

#[derive(Clone)]
pub struct PreviewContext {
    picture: gtk::Picture,
    placeholder: gtk::Label,
    status: gtk::Label,
    child: Rc<RefCell<Option<Child>>>,
}

impl PreviewContext {
    pub fn new(picture: gtk::Picture, placeholder: gtk::Label, status: gtk::Label) -> Self {
        Self { picture, placeholder, status, child: Rc::new(RefCell::new(None)) }
    }

    pub fn stop(&self) {
        if let Some(mut child) = self.child.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn unavailable(&self, message: &str) {
        self.stop();
        self.picture.set_pixbuf(None);
        self.placeholder.set_text(message);
        self.placeholder.set_visible(true);
        self.status.set_text("Preview unavailable");
    }

    pub fn start(&self, device: &str, mode: &VideoMode) {
        self.stop();
        self.picture.set_pixbuf(None);
        self.placeholder.set_visible(true);
        self.placeholder.set_text("Starting camera preview…");
        self.status.set_text(&format!(
            "{} × {} · {} · {:.3} fps",
            mode.width, mode.height, mode.pixel_format,
            mode.frame_rate_milli as f64 / 1000.0
        ));

        let mut child = match Command::new("v4l2-ctl")
            .arg("-d").arg(device)
            .arg("--stream-mmap=4")
            .arg("--stream-count=1000000")
            .arg("--stream-to=-")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                self.placeholder.set_text(&format!("Could not start preview: {error}"));
                self.status.set_text("Preview unavailable");
                return;
            }
        };
        let Some(stdout) = child.stdout.take() else {
            self.placeholder.set_text("The capture process did not provide a video stream.");
            self.status.set_text("Preview unavailable");
            let _ = child.kill();
            return;
        };
        *self.child.borrow_mut() = Some(child);

        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(2);
        let mode_reader = mode.clone();
        thread::spawn(move || read_frames(stdout, sender, mode_reader));
        let mode_display = mode.clone();

        let picture = self.picture.clone();
        let placeholder = self.placeholder.clone();
        let status = self.status.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(30), move || {
            let mut latest = None;
            loop {
                match receiver.try_recv() {
                    Ok(frame) => latest = Some(frame),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return gtk::glib::ControlFlow::Break,
                }
            }
            if let Some(frame) = latest {
                if let Some(pixbuf) = decode_frame(&frame, &mode_display) {
                    picture.set_pixbuf(Some(&pixbuf));
                    placeholder.set_visible(false);
                    status.set_text(&format!(
                        "Live preview · {} × {} · {} · {:.3} fps",
                        mode_display.width, mode_display.height, mode_display.pixel_format,
                        mode_display.frame_rate_milli as f64 / 1000.0
                    ));
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }
}

fn read_frames<R: Read>(mut reader: R, sender: SyncSender<Vec<u8>>, mode: VideoMode) {
    let mut buffer = Vec::<u8>::new();
    let mut chunk = [0u8; 64 * 1024];
    let raw_frame_size = mode.width as usize * mode.height as usize * 2;
    loop {
        let count = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
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
                let frame: Vec<u8> = buffer.drain(..end + 2).collect();
                match sender.try_send(frame) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
        } else if mode.pixel_format == "YUYV" {
            while raw_frame_size > 0 && buffer.len() >= raw_frame_size {
                let frame: Vec<u8> = buffer.drain(..raw_frame_size).collect();
                match sender.try_send(frame) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
            if buffer.len() > raw_frame_size.saturating_mul(3) {
                buffer.drain(..buffer.len() - raw_frame_size);
            }
        } else {
            return;
        }
    }
}

fn decode_frame(frame: &[u8], mode: &VideoMode) -> Option<Pixbuf> {
    if mode.pixel_format == "MJPG" {
        let loader = PixbufLoader::new();
        loader.write(frame).ok()?;
        loader.close().ok()?;
        return loader.pixbuf();
    }
    if mode.pixel_format != "YUYV" || mode.width % 2 != 0 { return None; }
    let width = mode.width as usize;
    let height = mode.height as usize;
    if frame.len() < width * height * 2 { return None; }
    let mut rgb = Vec::with_capacity(width * height * 3);
    for pair in frame[..width * height * 2].chunks_exact(4) {
        let u = pair[1] as i32 - 128;
        let v = pair[3] as i32 - 128;
        for y in [pair[0], pair[2]] {
            let c = (y as i32 - 16).max(0);
            let r = ((298 * c + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * u + 128) >> 8).clamp(0, 255) as u8;
            rgb.extend_from_slice(&[r, g, b]);
        }
    }
    let bytes = gtk::glib::Bytes::from_owned(rgb);
    Some(Pixbuf::from_bytes(
        &bytes, Colorspace::Rgb, false, 8,
        mode.width as i32, mode.height as i32, mode.width as i32 * 3,
    ))
}
