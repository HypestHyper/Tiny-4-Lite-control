//! Applies the theme: fonts, palette, scale and motion preference.

use super::theme::{self, Palette, ThemeMode, UiScale};
use crate::backend::{current_config, update_config};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

#[link(name = "fontconfig")]
extern "C" {
    fn FcConfigAppFontAddFile(config: *mut std::ffi::c_void, file: *const u8) -> i32;
    fn FcConfigBuildFonts(config: *mut std::ffi::c_void) -> i32;
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("tiny4lite-control").join("fonts")
}

/// Register the bundled fonts for this process only (nothing is installed system-wide).
fn register_fonts() {
    let dir = cache_dir();
    if fs::create_dir_all(&dir).is_err() { return; }
    let fonts: [(&str, &[u8]); 4] = [
        ("GoogleSansFlex-latin-400.ttf", include_bytes!("../../assets/fonts/GoogleSansFlex-latin-400.ttf")),
        ("GoogleSansFlex-latin-500.ttf", include_bytes!("../../assets/fonts/GoogleSansFlex-latin-500.ttf")),
        ("GoogleSansFlex-latin-700.ttf", include_bytes!("../../assets/fonts/GoogleSansFlex-latin-700.ttf")),
        ("MaterialSymbolsRounded.ttf", include_bytes!("../../assets/fonts/MaterialSymbolsRounded.ttf")),
    ];
    let mut registered = false;
    for (name, bytes) in fonts {
        let path = dir.join(name);
        if fs::read(&path).ok().as_deref() != Some(bytes) && fs::write(&path, bytes).is_err() { continue; }
        let Ok(cpath) = CString::new(path.to_string_lossy().as_bytes()) else { continue };
        unsafe { registered |= FcConfigAppFontAddFile(std::ptr::null_mut(), cpath.as_ptr() as *const u8) != 0; }
    }
    if registered { unsafe { FcConfigBuildFonts(std::ptr::null_mut()); } }
}

pub struct Style {
    provider: gtk::CssProvider,
    mode: Cell<ThemeMode>,
    accent: RefCell<String>,
    scale: Cell<UiScale>,
    reduce_motion: Cell<bool>,
    window_buttons: Cell<bool>,
    auto_compact: Cell<bool>,
    system_dark: Cell<bool>,
    system_reduce: bool,
    listeners: RefCell<Vec<Box<dyn Fn()>>>,
}

impl Style {
    pub fn install() -> Rc<Style> {
        register_fonts();
        let provider = gtk::CssProvider::new();
        if let Some(display) = gtk::gdk::Display::default() {
            // Above any desktop theme, so it cannot change sizes or colours underneath us.
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
        }
        let config = current_config();
        // Read the desktop's preference before we set the property ourselves.
        let system_reduce = system_reduces_motion();
        let style = Rc::new(Style {
            provider,
            mode: Cell::new(ThemeMode::from_name(&config.theme)),
            accent: RefCell::new(config.accent.clone()),
            scale: Cell::new(UiScale::from_name(&config.ui_scale)),
            reduce_motion: Cell::new(config.reduce_motion || system_reduce),
            window_buttons: Cell::new(config.window_buttons),
            auto_compact: Cell::new(false),
            system_dark: Cell::new(true),
            system_reduce,
            listeners: RefCell::default(),
        });
        style.apply();
        style.watch_portal();
        style
    }

    pub fn mode(&self) -> ThemeMode { self.mode.get() }
    pub fn accent(&self) -> String { self.accent.borrow().clone() }
    pub fn scale(&self) -> UiScale { self.scale.get() }
    pub fn reduce_motion(&self) -> bool { self.reduce_motion.get() }
    pub fn window_buttons(&self) -> bool { self.window_buttons.get() }

    pub fn set_window_buttons(&self, show: bool) {
        self.window_buttons.set(show);
        update_config(|c| c.window_buttons = show);
        for listener in self.listeners.borrow().iter() { listener(); }
    }

    pub fn on_change(&self, f: impl Fn() + 'static) { self.listeners.borrow_mut().push(Box::new(f)); }

    pub fn is_dark(&self) -> bool {
        match self.mode.get() { ThemeMode::Dark => true, ThemeMode::Light => false, ThemeMode::System => self.system_dark.get() }
    }

    pub fn apply(&self) {
        let dark = self.is_dark();
        let palette = Palette::new(&theme::accent_hex(&self.accent.borrow()), dark);
        let factor = self.scale.get().factor(self.auto_compact.get());
        let composited = gtk::gdk::Display::default().is_none_or(|d| d.is_composited());
        self.provider.load_from_data(&theme::stylesheet(&palette, factor, self.reduce_motion.get(), composited));
        theme::set_current(palette);
        if let Some(settings) = gtk::Settings::default() {
            settings.set_property("gtk-application-prefer-dark-theme", dark);
            settings.set_property("gtk-enable-animations", !self.reduce_motion.get());
        }
        for listener in self.listeners.borrow().iter() { listener(); }
    }

    pub fn set_mode(&self, mode: ThemeMode) {
        self.mode.set(mode);
        update_config(|c| c.theme = mode.name().to_string());
        self.apply();
    }

    pub fn set_accent(&self, accent: &str) {
        *self.accent.borrow_mut() = accent.to_string();
        update_config(|c| c.accent = accent.to_string());
        self.apply();
    }

    pub fn set_scale(&self, scale: UiScale) {
        self.scale.set(scale);
        update_config(|c| c.ui_scale = scale.name().to_string());
        self.apply();
    }

    pub fn set_reduce_motion(&self, reduce: bool) {
        self.reduce_motion.set(reduce || self.system_reduce);
        update_config(|c| c.reduce_motion = reduce);
        self.apply();
    }

    /// Called with the window width. Switches to compact below 900 px, with
    /// 60 px of hysteresis so it does not flicker at the boundary.
    pub fn report_width(&self, width: i32) {
        let compact = self.auto_compact.get();
        let next = if compact { width < 960 } else { width < 900 };
        if next != compact {
            self.auto_compact.set(next);
            if self.scale.get() == UiScale::Auto { self.apply(); }
        }
    }

    // ── Desktop colour-scheme preference (XDG settings portal) ──

    fn watch_portal(self: &Rc<Self>) {
        let style = self.clone();
        glib::spawn_future_local(async move {
            let Ok(proxy) = gio::DBusProxy::for_bus_future(
                gio::BusType::Session, gio::DBusProxyFlags::NONE, None,
                "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Settings",
            ).await else { return };
            let args = ("org.freedesktop.appearance", "color-scheme").to_variant();
            if let Ok(reply) = proxy.call_future("Read", Some(&args), gio::DBusCallFlags::NONE, 2000).await {
                if let Some(dark) = parse_scheme(&reply) { style.set_system_dark(dark); }
            }
            let weak = Rc::downgrade(&style);
            proxy.connect_local("g-signal", false, move |values| {
                let signal = values.get(2).and_then(|v| v.get::<String>().ok())?;
                let params = values.get(3).and_then(|v| v.get::<glib::Variant>().ok())?;
                if signal == "SettingChanged" {
                    let namespace = params.child_value(0).get::<String>()?;
                    let key = params.child_value(1).get::<String>()?;
                    if namespace == "org.freedesktop.appearance" && key == "color-scheme" {
                        let value = unwrap_variant(params.child_value(2));
                        if let (Some(style), Some(scheme)) = (weak.upgrade(), value.get::<u32>()) {
                            style.set_system_dark(scheme == 1);
                        }
                    }
                }
                None
            });
            // Keep the proxy alive for the life of the app.
            std::mem::forget(proxy);
        });
    }

    fn set_system_dark(&self, dark: bool) {
        if self.system_dark.replace(dark) != dark && self.mode.get() == ThemeMode::System { self.apply(); }
    }
}

fn unwrap_variant(mut value: glib::Variant) -> glib::Variant {
    while value.type_().is_variant() {
        match value.as_variant() { Some(inner) => value = inner, None => break }
    }
    value
}

fn parse_scheme(reply: &glib::Variant) -> Option<bool> {
    let value = unwrap_variant(reply.child_value(0));
    // 1 = prefer dark, 2 = prefer light, 0 = no preference (treated as dark: the app's default look).
    value.get::<u32>().map(|scheme| scheme != 2)
}

fn system_reduces_motion() -> bool {
    gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations())
}
