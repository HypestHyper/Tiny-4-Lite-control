//! The pop-out preview: its own resizable window.

use super::app::Ctx;
use super::preview::PreviewView;
use super::{widgets, windowing};
use crate::backend::{current_config, update_config};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

pub struct PreviewWindow {
    pub window: gtk::Window,
}

impl PreviewWindow {
    pub fn new(ctx: &Rc<Ctx>) -> PreviewWindow {
        let config = current_config();
        let window = gtk::Window::builder()
            .title(windowing::TITLE_PREVIEW)
            .default_width(config.popup_width.clamp(240, 4000))
            .default_height(config.popup_height.clamp(135, 3000))
            .resizable(true)
            .build();
        windowing::set_class(&window, windowing::CLASS_PREVIEW);

        let view = PreviewView::new(&ctx.hub);
        view.widget().set_size_request(240, 135);
        // The picture is the whole window: no card, no padding.
        view.widget().remove_css_class("preview-surface");
        view.widget().add_css_class("stage");

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let pause = ctx.hub.pause_button();
        let dock = widgets::icon_button("dock_to_right", "Return the preview to the main window", true);
        dock.add_css_class("on-stage");
        let weak_ctx = Rc::downgrade(ctx);
        dock.connect_clicked(move |_| { if let Some(ctx) = weak_ctx.upgrade() { ctx.set_popped_out(false); } });
        controls.append(&ctx.hub.source_picker());
        controls.append(&ctx.hub.look_picker());
        controls.append(&ctx.hub.peaking_button());
        controls.append(&pause);
        controls.append(&dock);
        view.add_control(&controls);

        window.set_child(Some(view.widget()));
        // Hold the view for the life of the window.
        unsafe { window.set_data("preview-view", view); }

        // Remember the size the user chose.
        let weak_ctx = Rc::downgrade(ctx);
        window.connect_close_request(move |window| {
            let (width, height) = (window.width(), window.height());
            if width > 0 && height > 0 {
                update_config(|c| { c.popup_width = width; c.popup_height = height; });
            }
            if let Some(ctx) = weak_ctx.upgrade() { ctx.set_popped_out(false); }
            glib::Propagation::Stop
        });
        let keys = gtk::EventControllerKey::new();
        let weak_ctx = Rc::downgrade(ctx);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(ctx) = weak_ctx.upgrade() { ctx.set_popped_out(false); }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        PreviewWindow { window }
    }
}
