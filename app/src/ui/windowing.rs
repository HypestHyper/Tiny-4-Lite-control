//! Window identity: titles and class names that tiling window managers can match.
//!
//! Hyprland (and sway, niri) match the Wayland `app_id`, shown as `class`;
//! X11 window managers match the title. GTK gives every window of an
//! application the same app id, so each window sets its own on its Wayland
//! surface.

use gtk::prelude::*;

pub const APP_ID: &str = "io.github.hypesthyper.Tiny4LiteControl";
pub const CLASS_FULL: &str = "io.github.hypesthyper.Tiny4LiteControl";
pub const CLASS_SLIM: &str = "io.github.hypesthyper.Tiny4LiteControl.Slim";
pub const CLASS_PREVIEW: &str = "io.github.hypesthyper.Tiny4LiteControl.Preview";

pub const TITLE_FULL: &str = "Tiny 2 Lite Control";
pub const TITLE_SLIM: &str = "Tiny 2 Lite Control - Quick controls";
pub const TITLE_PREVIEW: &str = "Tiny 2 Lite Control - Preview";

/// Give a window its own class. GTK reads the program name when it creates a
/// window's surface, and uses it as the Wayland `app_id` and the X11 `WM_CLASS`
/// instance, so each window sets it just before it first appears.
pub fn set_class<W: IsA<gtk::Window> + IsA<gtk::Widget> + IsA<gtk::Native>>(window: &W, class: &'static str) {
    window.add_css_class("t4");
    // Also set it on the Wayland surface directly, for GTK versions that read it there.
    window.connect_realize(move |window| {
        let Some(surface) = window.surface() else { return };
        if let Some(toplevel) = surface.downcast_ref::<gdk4_wayland::WaylandToplevel>() {
            toplevel.set_application_id(class);
        }
    });
    // Remember which class this window wants; `present` applies it.
    unsafe { window.set_data("t4-class", class); }
}

/// Show a window with its own class (see `set_class`).
pub fn present<W: IsA<gtk::Window>>(window: &W) {
    let class = unsafe { window.data::<&'static str>("t4-class").map(|p| *p.as_ref()) };
    if let Some(class) = class { gtk::glib::set_prgname(Some(class)); }
    window.present();
}
