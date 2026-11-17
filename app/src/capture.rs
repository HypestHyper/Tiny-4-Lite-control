use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
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


fn summarize_control_transfers(decoded: &str) -> String {
    let mut lines = decoded.lines();
    let Some(header) = lines.next() else { return "No decoded transfer rows.\n".into(); };
    let columns: Vec<&str> = header.split('\t').collect();
    let index = |name: &str| columns.iter().position(|column| *column == name);
    let (Some(urb_type_i), Some(urb_id_i), Some(request_i), Some(value_i), Some(index_i)) =
        (index("usb.urb_type"), index("usb.urb_id"), index("usb.setup.bRequest"),
         index("usb.setup.wValue"), index("usb.setup.wIndex")) else {
        return "Summary unavailable: expected TShark fields were not present in the decoded output.\n".into();
    };

    let mut transfer_types = BTreeMap::<String, usize>::new();
    let mut request_groups = BTreeMap::<(String, String, String), usize>::new();
    let mut urb_ids = BTreeMap::<String, usize>::new();
    let mut rows = 0usize;
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() <= [urb_type_i, urb_id_i, request_i, value_i, index_i].into_iter().max().unwrap_or(0) {
            continue;
        }
        rows += 1;
        *transfer_types.entry(fields[urb_type_i].to_string()).or_default() += 1;
        *request_groups.entry((
            fields[request_i].to_string(),
            fields[value_i].to_string(),
            fields[index_i].to_string(),
        )).or_default() += 1;
        *urb_ids.entry(fields[urb_id_i].to_string()).or_default() += 1;
    }

    let mut report = format!("TRANSFER SUMMARY\n----------------\nDecoded rows: {rows}\n\nURB types:\n");
    for (kind, count) in transfer_types {
        report.push_str(&format!("  {kind}: {count}\n"));
    }
    report.push_str("\nControl request groups (bRequest / wValue / wIndex):\n");
    for ((request, value, index), count) in request_groups {
        report.push_str(&format!("  {request} / {value} / {index}: {count}\n"));
    }
    let reused = urb_ids.values().filter(|count| **count > 1).count();
    report.push_str(&format!(
        "\nURB IDs appearing more than once: {reused}\nPair submissions and completions by URB ID in capture order; IDs can be reused.\n\n"
    ));
    report
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
            let pid = self.child.id();
            let _ = Command::new("kill").arg("-INT").arg(pid.to_string()).status();
        }
        let status = self.child.wait().map_err(|e| format!("Could not stop tshark: {e}"))?;
        self.stopped = true;

        match self.writer.take().map(|join| join.join()) {
            Some(Ok(Err(error))) => return Err(format!("Could not finish writing the capture: {error}")),
            Some(Err(_)) => return Err("The capture writer thread failed.".into()),
            _ => {}
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
                "-e", "usb.urb_id",
                "-e", "frame.time_relative",
                "-e", "usb.urb_type",
                "-e", "usb.urb_status",
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
            let decoded_text = String::from_utf8_lossy(&decoded.stdout);
            report.push_str(&summarize_control_transfers(&decoded_text));
            report.push_str("RAW TRANSFER ROWS\n-----------------\n");
            report.push_str(&decoded_text);
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

fn usb_location() -> Result<(u32, u32), String> {
    // Discover the USB device directly. Do not depend on /dev/videoN:
    // the camera may be claimed by usbfs while passed through to a VM.
    let entries = fs::read_dir("/sys/bus/usb/devices")
        .map_err(|e| format!("Could not enumerate USB devices: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
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
    }
    Err("OBSBOT Tiny 2 Lite (USB 3564:fef9) was not found in /sys/bus/usb/devices.".into())
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

fn start_session(test_name: String, baseline: String) -> Result<CaptureSession, String> {
    if Command::new("tshark").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        return Err("tshark is not installed. Install Wireshark/tshark first.".into());
    }
    let (bus, address) = usb_location()?;
    let directory = capture_directory()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let started_epoch = now.as_secs();
    let file_name = format!("obsbot-capture-{}-{:09}.pcapng", now.as_secs(), now.subsec_nanos());
    let pcap_path = directory.join(file_name);

    // usbmon requires elevated access on many distributions. pkexec prompts once,
    // while stdout is piped back into a file owned by the logged-in user.
    let interface = format!("usbmon{bus}");
    let mut child = Command::new("pkexec")
        .arg("tshark")
        .arg("-i").arg(interface)
        .args(["-w", "-"])
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


fn saved_report_paths() -> Vec<PathBuf> {
    let Ok(directory) = capture_directory() else { return Vec::new(); };
    let mut paths: Vec<PathBuf> = fs::read_dir(directory).ok().into_iter().flatten()
        .flatten().map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    paths.sort_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());
    paths.reverse();
    paths
}

fn refresh_saved_list(list: &gtk::ListBox, paths: &Rc<RefCell<Vec<PathBuf>>>) {
    while let Some(child) = list.first_child() { list.remove(&child); }
    let found = saved_report_paths();
    for path in &found {
        let label = gtk::Label::new(Some(&format!(
            "{}\n{}",
            path.file_stem().unwrap_or_default().to_string_lossy(),
            path.parent().unwrap_or(Path::new("")).display()
        )));
        label.set_xalign(0.0);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        label.set_margin_start(12);
        label.set_margin_end(12);
        label.set_wrap(true);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        list.append(&row);
    }
    *paths.borrow_mut() = found;
}

fn text_view_contents(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true).to_string()
}

pub fn build_page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    page.add_css_class("content-page");
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

    let status = gtk::Label::new(Some("Ready. The USB camera is detected independently of its video node."));
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

    let saved_frame = gtk::Frame::new(Some("Saved captures"));
    let saved_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let saved_help = gtk::Label::new(Some("Browse earlier USB sessions. Reports and raw traces stay in your user data folder."));
    saved_help.set_xalign(0.0);
    saved_help.set_wrap(true);
    saved_help.add_css_class("dim-label");
    saved_box.append(&saved_help);
    let saved_list = gtk::ListBox::new();
    saved_list.set_selection_mode(gtk::SelectionMode::Single);
    saved_list.set_vexpand(false);
    saved_list.set_size_request(-1, 150);
    let saved_scroll = gtk::ScrolledWindow::new();
    saved_scroll.set_min_content_height(120);
    saved_scroll.set_max_content_height(190);
    saved_scroll.set_child(Some(&saved_list));
    saved_box.append(&saved_scroll);
    let saved_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let saved_refresh = gtk::Button::with_label("Refresh list");
    let saved_open_report = gtk::Button::with_label("Open report");
    let saved_open_raw = gtk::Button::with_label("Open raw trace");
    saved_open_report.set_sensitive(false);
    saved_open_raw.set_sensitive(false);
    saved_actions.append(&saved_refresh);
    saved_actions.append(&saved_open_report);
    saved_actions.append(&saved_open_raw);
    saved_box.append(&saved_actions);
    saved_frame.set_child(Some(&saved_box));
    page.append(&saved_frame);

    let saved_paths = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    refresh_saved_list(&saved_list, &saved_paths);
    let selected_saved = Rc::new(RefCell::new(None::<PathBuf>));
    let selected_state = selected_saved.clone();
    let paths_for_selection = saved_paths.clone();
    let open_report_button = saved_open_report.clone();
    let open_raw_button = saved_open_raw.clone();
    saved_list.connect_row_selected(move |_, row| {
        let path = row.and_then(|row| paths_for_selection.borrow().get(row.index() as usize).cloned());
        open_report_button.set_sensitive(path.is_some());
        open_raw_button.set_sensitive(path.as_ref().is_some_and(|p| p.with_extension("pcapng").exists()));
        *selected_state.borrow_mut() = path;
    });
    let list_refresh = saved_list.clone();
    let paths_refresh = saved_paths.clone();
    let selected_refresh = selected_saved.clone();
    saved_refresh.connect_clicked(move |_| {
        refresh_saved_list(&list_refresh, &paths_refresh);
        *selected_refresh.borrow_mut() = None;
    });
    let selected_report = selected_saved.clone();
    let status_report = status.clone();
    saved_open_report.connect_clicked(move |_| {
        if let Some(path) = selected_report.borrow().as_ref() {
            if let Err(error) = Command::new("xdg-open").arg(path).spawn() {
                status_report.set_text(&format!("Could not open report: {error}"));
            }
        }
    });
    let selected_raw = selected_saved.clone();
    let status_raw = status.clone();
    saved_open_raw.connect_clicked(move |_| {
        if let Some(path) = selected_raw.borrow().as_ref() {
            let raw = path.with_extension("pcapng");
            if raw.exists() {
                if let Err(error) = Command::new("xdg-open").arg(raw).spawn() {
                    status_raw.set_text(&format!("Could not open raw trace: {error}"));
                }
            } else {
                status_raw.set_text("The raw trace for this report is not present.");
            }
        }
    });

    let session: Rc<RefCell<Option<CaptureSession>>> = Rc::new(RefCell::new(None));
    let report_text = Rc::new(RefCell::new(String::new()));
    let report_file: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let checkpoint_summary = Rc::new(RefCell::new(Vec::<String>::new()));

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
        match start_session(name, baseline.clone()) {
            Ok(new_session) => {
                let bus = new_session.bus;
                let address = new_session.address;
                *session_start.borrow_mut() = Some(new_session);
                note_start.buffer().set_text("");
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
    let finish_button = finish.clone();
    let copy_finish = copy.clone();
    let open_finish = open_folder.clone();
    finish.connect_clicked(move |_| {
        let Some(active) = session_finish.borrow_mut().take() else {
            status_finish.set_text("No capture session is running.");
            return;
        };
        status_finish.set_text("Stopping capture and decoding camera control transfers…");
        checkpoint_finish.set_sensitive(false);
        finish_button.set_sensitive(false);
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
