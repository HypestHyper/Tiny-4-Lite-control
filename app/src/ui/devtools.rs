//! Developer tools: read-only XU diagnostics and USB capture sessions.
//! These keep their simple layout; they are for protocol work, not daily use.

use crate::backend::*;
use gtk::prelude::*;

pub fn open(parent: &gtk::Window, device: &str) {
    let window = gtk::Window::builder()
        .title("Tiny 2 Lite Control - Developer tools")
        .transient_for(parent)
        .default_width(760)
        .default_height(640)
        .build();
    window.add_css_class("t4");
    let combo = gtk::ComboBoxText::new();
    combo.append(Some(device), device);
    combo.set_active(Some(0));
    let stack = gtk::Stack::new();
    stack.add_titled(&add_diagnostics_page(&combo), Some("diag"), "Diagnostics");
    stack.add_titled(&crate::capture::build_page(), Some("capture"), "USB capture");
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Center);
    switcher.set_margin_top(12);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&switcher);
    root.append(&stack);
    window.set_child(Some(&root));
    window.present();
}

fn add_diagnostics_page(device_combo: &gtk::ComboBoxText) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);

    let heading = gtk::Label::new(Some("Read-only XU diagnostics"));
    heading.add_css_class("title-2");
    heading.set_xalign(0.0);
    page.append(&heading);

    let intro = gtk::Label::new(Some(
        "Reads 60-byte UVC Extension Unit responses. These actions are GET_CUR only; no camera settings are changed."
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    page.append(&intro);

    let device_info = gtk::TextView::new();
    device_info.set_editable(false);
    device_info.set_cursor_visible(false);
    device_info.set_monospace(true);
    device_info.set_wrap_mode(gtk::WrapMode::WordChar);
    device_info.set_size_request(-1, 140);
    if let Ok(info) = usb_descriptor_info() { device_info.buffer().set_text(&info); }
    else { device_info.buffer().set_text("USB descriptor information unavailable until the camera is visible to Linux."); }
    let info_scroll = gtk::ScrolledWindow::new();
    info_scroll.set_min_content_height(140);
    info_scroll.set_child(Some(&device_info));
    page.append(&info_scroll);

    let model_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    model_row.add_css_class("adaptive-row");
    let model_button = gtk::Button::with_label("Read model string (XU 0x08)");
    let model_output = gtk::Label::new(Some("Not read."));
    model_output.set_xalign(0.0);
    model_output.set_wrap(true);
    model_row.append(&model_button);
    model_row.append(&model_output);
    page.append(&model_row);

    let selector_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    selector_row.add_css_class("adaptive-row");
    let read_status = gtk::Button::with_label("Read status (0x06)");
    let read_command = gtk::Button::with_label("Read command channel (0x02)");
    selector_row.append(&read_status);
    selector_row.append(&read_command);
    page.append(&selector_row);

    let output = gtk::TextView::new();
    output.set_editable(false);
    output.set_cursor_visible(false);
    output.set_monospace(true);
    output.set_wrap_mode(gtk::WrapMode::None);
    output.set_vexpand(true);
    let output_scroll = gtk::ScrolledWindow::new();
    output_scroll.set_vexpand(true);
    output_scroll.set_child(Some(&output));
    page.append(&output_scroll);

    let device = device_combo.clone();
    let model_label = model_output.clone();
    model_button.connect_clicked(move |_| {
        let result = device.active_id()
            .ok_or_else(|| "Select a Tiny 2 Lite V4L2 node first.".to_string())
            .and_then(|node| {
                let mut bytes = [0u8; 60];
                xu_query(node.as_str(), 0x08, UVC_GET_CUR, &mut bytes)?;
                let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
                let value = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
                Ok(if value.is_empty() { format!("Raw: {}", hex_dump(&bytes)) } else { value })
            });
        model_label.set_text(&result.unwrap_or_else(|e| format!("Read failed: {e}")));
    });

    let device = device_combo.clone();
    let view = output.clone();
    read_status.connect_clicked(move |_| {
        let result = device.active_id()
            .ok_or_else(|| "Select a Tiny 2 Lite V4L2 node first.".to_string())
            .and_then(|node| xu_status(node.as_str()));
        match result {
            Ok(bytes) => {
                let text = format!("Selector 0x06 · GET_CUR · 60 bytes\n\n{}\n{}",
                    decode_xu_status(&bytes), hex_dump(&bytes));
                view.buffer().set_text(&text);
            }
            Err(error) => view.buffer().set_text(&format!("Read failed: {error}")),
        }
    });

    let device = device_combo.clone();
    let view = output.clone();
    read_command.connect_clicked(move |_| {
        let result = device.active_id()
            .ok_or_else(|| "Select a Tiny 2 Lite V4L2 node first.".to_string())
            .and_then(|node| {
                let mut bytes = [0u8; 60];
                xu_query(node.as_str(), 0x02, UVC_GET_CUR, &mut bytes)?;
                Ok(bytes)
            });
        match result {
            Ok(bytes) => {
                let text = format!("Selector 0x02 · GET_CUR · 60 bytes\n\n{}\n{}",
                    decode_xu_command_response(&bytes), hex_dump(&bytes));
                view.buffer().set_text(&text);
            }
            Err(error) => view.buffer().set_text(&format!("Read failed: {error}")),
        }
    });

    page
}

