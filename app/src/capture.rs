use gtk::prelude::*;
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct Checkpoint {
    elapsed: f64,
    note: String,
}

struct CaptureSession {
    child: Child,
    pcap_path: PathBuf,
    bus: u32,
    address: u32,
    started: Instant,
    started_epoch: u64,
    test_name: String,
    checkpoints: Vec<Checkpoint>,
    writer: Option<JoinHandle<io::Result<()>>>,
    stderr_reader: Option<JoinHandle<String>>,
    stopped: bool,
}

impl CaptureSession {
    fn add_checkpoint(&mut self, note: String) -> Result<f64, String> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("The capture process has stopped. Finish this session and inspect its error log.".into());
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        self.checkpoints.push(Checkpoint { elapsed, note });
        Ok(elapsed)
    }

    fn stop_process(&mut self) -> Result<String, String> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            let pid = self.child.id().to_string();
            let _ = Command::new("kill").args(["-INT", &pid]).status();
        }
        let status = self.child.wait().map_err(|e| format!("Could not stop tshark: {e}"))?;
        self.stopped = true;

        let writer_result = self.writer.take().map(|join| join.join());
        if let Some(Ok(Err(error))) = writer_result {
            return Err(format!("Could not finish writing the capture: {error}"));
        }
        if let Some(Err(_)) = writer_result {
            return Err("The capture writer thread failed.".into());
        }

        let stderr = self.stderr_reader.take()
            .and_then(|join| join.join().ok()).unwrap_or_default();
        if !status.success() && !status.code().is_some_and(|code| code == 130 || code == 2) {
            return Err(format!("tshark exited with {status}. {}", stderr.trim()));
        }
        Ok(stderr)
    }

    fn finish(mut self) -> Result<(String, PathBuf), String> {
        let stderr = self.stop_process()?;
        let filter = format!("usb.device_address == {} && usb.transfer_type == 0x02", self.address);
        let decoded = Command::new("tshark")
            .arg("-r").arg(&self.pcap_path)
            .arg("-Y").arg(&filter)
            .args(["-T", "fields", "-E", "header=y", "-E", "separator=/t"])
            .args([
                "-e", "frame.number",
                "-e", "frame.time_relative",
                "-e", "usb.urb_type",
                "-e", "usb.endpoint_address",
                "-e", "usb.bmRequestType",
                "-e", "usb.setup.bRequest",
                "-e", "usb.setup.wValue",
                "-e", "usb.setup.wIndex",
                "-e", "usb.data_len",
                "-e", "usb.control.Response",
                "-e", "usb.capdata",
            ])
            .output()
            .map_err(|e| format!("Could not run tshark to decode the capture: {e}"))?;

        let mut report = String::new();
        report.push_str("OBSBOT TINY 2 LITE USB CAPTURE\n");
        report.push_str("==============================\n\n");
        report.push_str(&format!("Test: {}\n", self.test_name));
        report.push_str(&format!("Started (Unix seconds): {}\n", self.started_epoch));
        report.push_str(&format!("USB ID: 3564:fef9\nBus: {}\nDevice address: {}\nMonitor: usbmon{}\n",
            self.bus, self.address, self.bus));
        report.push_str(&format!("Raw capture: {}\n", self.pcap_path.display()));
        report.push_str(&format!("Capture size: {} bytes\n\n",
            fs::metadata(&self.pcap_path).map(|m| m.len()).unwrap_or(0)));

        report.push_str("CHECKPOINTS\n-----------\n");
        for (index, checkpoint) in self.checkpoints.iter().enumerate() {
            report.push_str(&format!("{:02}. +{:.6}s  {}\n",
                index + 1, checkpoint.elapsed, checkpoint.note));
        }

        report.push_str("\nCAMERA CONTROL TRANSFERS\n------------------------\n");
        report.push_str("Filtered to this camera's USB device address and control transfers.\n");
        if decoded.status.success() {
            report.push_str(&String::from_utf8_lossy(&decoded.stdout));
        } else {
            report.push_str("Could not decode the capture with tshark:\n");
            report.push_str(&String::from_utf8_lossy(&decoded.stderr));
        }

        if !stderr.trim().is_empty() {
            report.push_str("\nCAPTURE PROCESS LOG\n-------------------\n");
            report.push_str(stderr.trim());
            report.push('\n');
        }

        let report_path = self.pcap_path.with_extension("txt");
        fs::write(&report_path, &report)
            .map_err(|e| format!("Could not save report {}: {e}", report_path.display()))?;
        Ok((report, report_path))
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.stop_process();
        }
    }
}

fn usb_location(video_node: &str) -> Result<(u32, u32), String> {
    let name = Path::new(video_node).file_name()
        .ok_or_else(|| "Invalid camera video node.".to_string())?;
    let mut path = fs::canonicalize(Path::new("/sys/class/video4linux").join(name).join("device"))
        .map_err(|e| format!("Could not resolve camera sysfs path: {e}"))?;

    loop {
        let vendor = fs::read_to_string(path.join("idVendor")).unwrap_or_default();
        let product = fs::read_to_string(path.join("idProduct")).unwrap_or_default();
        if vendor.trim().eq_ignore_ascii_case("3564")
            && product.trim().eq_ignore_ascii_case("fef9")
        {
            let bus = fs::read_to_string(path.join("busnum"))
                .map_err(|e| format!("Could not read camera USB bus number: {e}"))?
                .trim().parse::<u32>().map_err(|e| e.to_string())?;
            let address = fs::read_to_string(path.join("devnum"))
                .map_err(|e| format!("Could not read camera USB device address: {e}"))?
                .trim().parse::<u32>().map_err(|e| e.to_string())?;
            return Ok((bus, address));
        }
        if !path.pop() {
            break;
        }
    }
    Err("The selected video node is not attached to OBSBOT Tiny 2 Lite (3564:fef9).".into())
}

fn capture_directory() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or_else(|| "Could not determine the user's data directory.".to_string())?;
    let directory = base.join("tiny4lite-control").join("captures");
    fs::create_dir_all(&directory)
        .map_err(|e| format!("Could not create {}: {e}", directory.display()))?;
    Ok(directory)
}

fn start_session(video_node: &str, test_name: String, baseline: String) -> Result<CaptureSession, String> {
    if Command::new("tshark").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        return Err("tshark is not installed. Install Wireshark/tshark first.".into());
    }
    let (bus, address) = usb_location(video_node)?;
    let directory = capture_directory()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let started_epoch = now.as_secs();
    let file_name = format!("obsbot-capture-{}-{:09}.pcapng", now.as_secs(), now.subsec_nanos());
    let pcap_path = directory.join(file_name);

    // usbmon requires elevated access on many distributions. pkexec prompts once,
    // while stdout is piped back into a file owned by the logged-in user.
    let mut child = Command::new("pkexec")
        .arg("tshark")
        .args(["-i", &format!("usbmon{bus}"), "-w", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start pkexec/tshark: {e}. Install polkit (pkexec) and tshark."))?;

    let stdout = child.stdout.take()
        .ok_or_else(|| "Could not open tshark capture output.".to_string())?;
    let stderr = child.stderr.take()
        .ok_or_else(|| "Could not open tshark error output.".to_string())?;
    let writer_path = pcap_path.clone();
    let writer = thread::spawn(move || -> io::Result<()> {
        let mut output = File::create(writer_path)?;
        let mut input = stdout;
        io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()
    });
    let stderr_reader = thread::spawn(move || {
        let mut input = stderr;
        let mut text = String::new();
        let _ = input.read_to_string(&mut text);
        text
    });

    let mut session = CaptureSession {
        child, pcap_path, bus, address, started: Instant::now(), started_epoch,
        test_name, checkpoints: Vec::new(), writer: Some(writer),
        stderr_reader: Some(stderr_reader), stopped: false,
    };
    session.add_checkpoint(baseline)?;
    Ok(session)
}

fn text_view_contents(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true).to_string()
}

pub fn build_page(devices: &gtk::ComboBoxText) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let heading = gtk::Label::new(Some("USB capture session"));
    heading.add_css_class("title-2");
    heading.set_xalign(0.0);
    page.append(&heading);

    let help = gtk::Label::new(Some(
        "Record one continuous USB trace while changing one camera setting at a time. \
         Start with the baseline, change the setting in OBSBOT Center, then add a checkpoint \
         with a short description. The app keeps the raw capture and creates a decoded text report."
    ));
    help.set_wrap(true);
    help.set_xalign(0.0);
    help.add_css_class("dim-label");
    page.append(&help);

    let test_name = gtk::Entry::new();
    test_name.set_placeholder_text(Some("Test name, e.g. HDR"));
    page.append(&test_name);

    let note_label = gtk::Label::new(Some("What is the current state / what did you change?"));
    note_label.set_xalign(0.0);
    page.append(&note_label);

    let note = gtk::TextView::new();
    note.set_wrap_mode(gtk::WrapMode::WordChar);
    note.set_vexpand(false);
    note.set_size_request(-1, 76);
    note.buffer().set_text("Baseline: before changing anything.");
    let note_scroll = gtk::ScrolledWindow::new();
    note_scroll.set_min_content_height(76);
    note_scroll.set_child(Some(&note));
    page.append(&note_scroll);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let start = gtk::Button::with_label("Start capture + baseline");
    let checkpoint = gtk::Button::with_label("Add checkpoint");
    let finish = gtk::Button::with_label("Finish session");
    let copy = gtk::Button::with_label("Copy report");
    let open_folder = gtk::Button::with_label("Open capture folder");
    checkpoint.set_sensitive(false);
    finish.set_sensitive(false);
    copy.set_sensitive(false);
    open_folder.set_sensitive(false);
    actions.append(&start);
    actions.append(&checkpoint);
    actions.append(&finish);
    page.append(&actions);

    let status = gtk::Label::new(Some("Ready. Select the camera node above, then start a session."));
    status.set_xalign(0.0);
    status.set_wrap(true);
    page.append(&status);

    let checkpoints_log = gtk::Label::new(Some("No checkpoints yet."));
    checkpoints_log.set_xalign(0.0);
    checkpoints_log.set_wrap(true);
    checkpoints_log.add_css_class("dim-label");
    page.append(&checkpoints_log);

    let report_view = gtk::TextView::new();
    report_view.set_editable(false);
    report_view.set_cursor_visible(false);
    report_view.set_monospace(true);
    report_view.set_wrap_mode(gtk::WrapMode::None);
    report_view.set_size_request(-1, 220);
    let report_scroll = gtk::ScrolledWindow::new();
    report_scroll.set_vexpand(true);
    report_scroll.set_child(Some(&report_view));
    page.append(&report_scroll);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.append(&copy);
    footer.append(&open_folder);
    page.append(&footer);

    let session: Rc<RefCell<Option<CaptureSession>>> = Rc::new(RefCell::new(None));
    let report_text = Rc::new(RefCell::new(String::new()));
    let report_file: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let checkpoint_summary = Rc::new(RefCell::new(Vec::<String>::new()));

    let devices_start = devices.clone();
    let name_start = test_name.clone();
    let note_start = note.clone();
    let session_start = session.clone();
    let status_start = status.clone();
    let checkpoint_start = checkpoint.clone();
    let finish_start = finish.clone();
    let start_button = start.clone();
    let log_start = checkpoints_log.clone();
    let summary_start = checkpoint_summary.clone();
    start.connect_clicked(move |_| {
        if session_start.borrow().is_some() { return; }
        let Some(device) = devices_start.active_id() else {
            status_start.set_text("Select a camera video node first.");
            return;
        };
        let name = name_start.text().trim().to_string();
        if name.is_empty() {
            status_start.set_text("Enter a test name first, such as HDR or tracking mode.");
            return;
        }
        let baseline = text_view_contents(&note_start);
        if baseline.trim().is_empty() {
            status_start.set_text("Describe the baseline state before starting.");
            return;
        }
        match start_session(device.as_str(), name, baseline.clone()) {
            Ok(new_session) => {
                let bus = new_session.bus;
                let address = new_session.address;
                *session_start.borrow_mut() = Some(new_session);
                *summary_start.borrow_mut() = vec![format!("+0.000s  Baseline: {}", baseline.trim())];
                log_start.set_text(&summary_start.borrow().join("\n"));
                status_start.set_text(&format!(
                    "Capture running on usbmon{bus}, camera USB address {address}. Change one setting, then add a checkpoint."
                ));
                checkpoint_start.set_sensitive(true);
                finish_start.set_sensitive(true);
                start_button.set_sensitive(false);
            }
            Err(error) => status_start.set_text(&format!("Could not start capture: {error}")),
        }
    });

    let session_mark = session.clone();
    let note_mark = note.clone();
    let status_mark = status.clone();
    let summary_mark = checkpoint_summary.clone();
    let log_mark = checkpoints_log.clone();
    checkpoint.connect_clicked(move |_| {
        let current_note = text_view_contents(&note_mark);
        if current_note.trim().is_empty() {
            status_mark.set_text("Describe the state change before adding a checkpoint.");
            return;
        }
        let mut active = session_mark.borrow_mut();
        let Some(active) = active.as_mut() else {
            status_mark.set_text("No capture session is running.");
            return;
        };
        match active.add_checkpoint(current_note.trim().to_string()) {
            Ok(elapsed) => {
                summary_mark.borrow_mut().push(format!("+{elapsed:.3}s  {}", current_note.trim()));
                log_mark.set_text(&summary_mark.borrow().join("\n"));
                note_mark.buffer().set_text("");
                status_mark.set_text(&format!("Checkpoint added at +{elapsed:.3}s. Change the next setting when ready."));
            }
            Err(error) => status_mark.set_text(&format!("Could not add checkpoint: {error}")),
        }
    });

    let session_finish = session.clone();
    let status_finish = status.clone();
    let checkpoint_finish = checkpoint.clone();
    let start_finish = start.clone();
    let report_finish = report_text.clone();
    let report_file_finish = report_file.clone();
    let report_view_finish = report_view.clone();
    let copy_finish = copy.clone();
    let open_finish = open_folder.clone();
    let name_finish = test_name.clone();
    finish.connect_clicked(move |_| {
        let Some(active) = session_finish.borrow_mut().take() else {
            status_finish.set_text("No capture session is running.");
            return;
        };
        status_finish.set_text("Stopping capture and decoding camera control transfers…");
        checkpoint_finish.set_sensitive(false);
        finish.set_sensitive(false);
        match active.finish() {
            Ok((text, path)) => {
                report_view_finish.buffer().set_text(&text);
                *report_finish.borrow_mut() = text;
                *report_file_finish.borrow_mut() = Some(path.clone());
                copy_finish.set_sensitive(true);
                open_finish.set_sensitive(true);
                start_finish.set_sensitive(true);
                status_finish.set_text(&format!(
                    "Session saved. Report: {}  |  Raw trace: {}",
                    path.display(),
                    path.with_extension("pcapng").display()
                ));
            }
            Err(error) => {
                start_finish.set_sensitive(true);
                status_finish.set_text(&format!("Capture stopped, but report generation failed: {error}"));
            }
        }
    });

    let report_copy = report_text.clone();
    let status_copy = status.clone();
    copy.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&report_copy.borrow());
            status_copy.set_text("Report copied to clipboard. Attach the text report and .pcapng file to the conversation.");
        } else {
            status_copy.set_text("Could not access the desktop clipboard.");
        }
    });

    let file_open = report_file.clone();
    let status_open = status.clone();
    open_folder.connect_clicked(move |_| {
        let folder = file_open.borrow().as_ref().and_then(|path| path.parent()).map(Path::to_path_buf);
        if let Some(folder) = folder {
            let result = Command::new("xdg-open").arg(&folder).spawn();
            if let Err(error) = result {
                status_open.set_text(&format!("Could not open capture folder: {error}"));
            }
        }
    });

    page
}
