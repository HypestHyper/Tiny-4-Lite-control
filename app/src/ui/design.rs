use gtk::prelude::*;
use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[link(name = "fontconfig")]
extern "C" {
    fn FcConfigAppFontAddFile(config: *mut std::ffi::c_void, file: *const u8) -> i32;
    fn FcConfigBuildFonts(config: *mut std::ffi::c_void) -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode { System, Light, Dark }

impl ThemeMode {
    pub fn from_name(value: &str) -> Self {
        match value { "light" => Self::Light, "dark" => Self::Dark, _ => Self::System }
    }
    pub fn name(self) -> &'static str {
        match self { Self::System => "system", Self::Light => "light", Self::Dark => "dark" }
    }
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("tiny4lite-control").join("fonts")
}

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
        let Ok(cpath) = CString::new(path.to_string_lossy().as_bytes()) else { continue; };
        unsafe { registered |= FcConfigAppFontAddFile(std::ptr::null_mut(), cpath.as_ptr() as *const u8) != 0; }
    }
    if registered { unsafe { FcConfigBuildFonts(std::ptr::null_mut()); } }
}

fn system_prefers_dark() -> bool {
    let portal = Command::new("gdbus").args([
        "call", "--session", "--dest", "org.freedesktop.portal.Desktop",
        "--object-path", "/org/freedesktop/portal/desktop",
        "--method", "org.freedesktop.portal.Settings.Read",
        "org.freedesktop.appearance", "color-scheme",
    ]).output().ok();
    if let Some(output) = portal.filter(|o| o.status.success()) {
        let value = String::from_utf8_lossy(&output.stdout);
        if value.contains("uint32 1") { return true; }
        if value.contains("uint32 2") || value.contains("uint32 0") { return false; }
    }
    gtk::Settings::default()
        .map(|s| s.property::<bool>("gtk-application-prefer-dark-theme"))
        .unwrap_or(false)
}

fn css(dark: bool) -> String {
    let (dim, surface, low, high, highest, primary, hover, on_primary, on_surface, variant, outline, selected, selected_text) =
        if dark {
            ("#151311","#1e1a18","#25201e","#302825","#3a302c","#f5ad73","#ffc18f","#2d190d","#f1e9e4","#bdb0a8","#75665e","#50321f","#ffd0a8")
        } else {
            ("#f7f3ef","#fffaf6","#f4eee9","#eee5de","#e7dcd3","#8b4b20","#713b16","#ffffff","#241a14","#65574f","#88776c","#f3dfcf","#512b12")
        };
    format!(r#"
@define-color tiny_surface_dim {dim};
@define-color tiny_surface {surface};
@define-color tiny_surface_low {low};
@define-color tiny_surface_high {high};
@define-color tiny_surface_highest {highest};
@define-color tiny_primary {primary};
@define-color tiny_primary_hover {hover};
@define-color tiny_on_primary {on_primary};
@define-color tiny_on_surface {on_surface};
@define-color tiny_on_surface_variant {variant};
@define-color tiny_outline {outline};
@define-color tiny_selected {selected};
@define-color tiny_selected_text {selected_text};

* {{ font-family: "Google Sans Flex", "Cantarell", sans-serif; }}
window.tiny4-window {{ background: @tiny_surface_dim; color: @tiny_on_surface; }}
.tiny4-root {{ padding: 12px; background: @tiny_surface_dim; color: @tiny_on_surface; }}
.tiny4-root.compact-layout {{ padding: 6px; }}
.content-page.compact-layout {{ padding: 6px; }}
.compact-layout .category-sidebar {{ min-width: 0; }}
.compact-layout .category-workspace {{ min-width: 0; }}
.compact-layout .settings-page {{ padding: 2px; }}
.compact-layout .control-row {{ min-height: 42px; padding: 2px 4px; }}
.compact-layout .control-label {{ min-width: 92px; }}
.compact-layout frame {{ padding: 6px; }}
.compact-layout .app-title {{ font-size: 20px; }}
.compact-layout button {{ padding-left: 12px; padding-right: 12px; }}
.app-header {{ min-height: 64px; margin-bottom: 4px; }}
.app-title {{ color: @tiny_on_surface; font-size: 24px; font-weight: 700; }}
.material-icon {{ font-family: "Material Symbols Rounded"; font-size: 28px; color: @tiny_primary; padding-right: 8px; }}
.compact-switcher {{ margin: 4px 0 8px; }}
.device-row {{ background: @tiny_surface; border-radius: 24px; padding: 14px 18px; margin-bottom: 2px; }}
.status-chip {{ background: @tiny_surface_low; color: @tiny_on_surface_variant; border-radius: 999px; padding: 9px 14px; margin-bottom: 4px; }}
.navigation-sidebar {{ min-width: 168px; padding: 4px 0; }}
stacksidebar row {{ min-height: 42px; border-radius: 18px; margin: 2px 0; padding: 3px 12px; color: @tiny_on_surface_variant; font-weight: 600; }}
stacksidebar row:hover {{ background: @tiny_surface_high; color: @tiny_on_surface; }}
stacksidebar row:selected {{ background: @tiny_selected; color: @tiny_selected_text; }}
stacksidebar row:focus-visible {{ outline: 2px solid @tiny_primary; outline-offset: 1px; }}
.content-stack {{ background: transparent; }}
.workspace-content {{ min-width: 0; }}
.content-page {{ padding: 16px; color: @tiny_on_surface; }}
.controls-toolbar {{ padding: 4px 8px 8px; }}
.category-workspace {{ min-width: 0; }}
.category-sidebar {{ min-width: 148px; padding: 6px 0; }}
.category-sidebar row {{ min-height: 42px; border-radius: 16px; margin: 3px 0; padding: 4px 10px; color: @tiny_on_surface_variant; font-weight: 650; }}
.category-sidebar row:selected {{ background: @tiny_selected; color: @tiny_selected_text; }}
.category-sidebar row:hover {{ background: @tiny_surface_high; }}
.category-switcher {{ margin: 2px 0 8px; }}
.settings-stack {{ background: transparent; }}
.settings-page {{ min-width: 0; }}
.section-intro {{ padding: 8px 4px 12px; }}
.settings-section-list {{ min-width: 0; }}
.control-card {{ background: @tiny_surface; border: 1px solid alpha(@tiny_outline, 0.18); border-radius: 22px; padding: 10px; }}
.control-card > label {{ font-size: 14px; font-weight: 700; }}
.control-row {{ min-height: 48px; padding: 4px 8px; border-radius: 14px; }}
.control-row:hover {{ background: alpha(@tiny_surface_high, 0.65); }}
.control-label {{ font-size: 13px; font-weight: 600; text-transform: capitalize; }}
.control-reset {{ min-height: 32px; padding: 3px 12px; border-radius: 14px; font-size: 11px; }}
.availability-note {{ font-size: 11px; padding: 2px 8px 6px; }}
.device-caption {{ font-size: 11px; }}
.preview-panel {{ background: @tiny_surface; border: 1px solid alpha(@tiny_outline, 0.22); border-radius: 28px; padding: 6px; }}
.preview-panel > border {{ border-radius: 24px; }}
.preview-heading {{ min-height: 28px; }}
.eyebrow {{ color: @tiny_on_surface; font-size: 11px; font-weight: 800; letter-spacing: 1.1px; }}
.preview-meta {{ color: @tiny_on_surface_variant; font-size: 9px; font-weight: 700; letter-spacing: 0.5px; }}
.preview-panel picture {{ border-radius: 18px; background: #100e0d; }}
.preview-panel .dim-label {{ font-size: 12px; }}
frame {{ background: @tiny_surface; border: 1px solid alpha(@tiny_outline, 0.16); border-radius: 24px; padding: 12px; }}
frame > label {{ color: @tiny_on_surface; font-weight: 700; }}
button {{ min-height: 40px; padding: 6px 16px; border: 1px solid transparent; border-radius: 20px; background: @tiny_surface_high; color: @tiny_on_surface; font-weight: 600; }}
button:hover {{ background: @tiny_surface_highest; }}
button:active {{ border-radius: 10px; background: @tiny_surface_low; }}
button:focus-visible {{ outline: 2px solid @tiny_primary; outline-offset: 2px; }}
button.suggested-action {{ background: @tiny_primary; color: @tiny_on_primary; }}
button.suggested-action:hover {{ background: @tiny_primary_hover; }}
.linked button {{ border-radius: 0; border-right-width: 0; }}
.linked button:first-child {{ border-radius: 999px 0 0 999px; }}
.linked button:last-child {{ border-radius: 0 999px 999px 0; border-right-width: 1px; }}
.linked button:checked {{ background: @tiny_selected; color: @tiny_selected_text; }}
.linked button:checked:hover {{ background: @tiny_primary; color: @tiny_on_primary; }}

entry, combobox, dropdown, spinbutton {{ min-height: 40px; border-radius: 12px; background: @tiny_surface_high; color: @tiny_on_surface; }}
scale trough {{ min-height: 6px; border-radius: 99px; background: @tiny_surface_highest; }}
scale highlight {{ background: @tiny_primary; }}
scale slider {{ min-width: 18px; min-height: 18px; border-radius: 99px; background: @tiny_primary; }}
.dim-label {{ color: @tiny_on_surface_variant; }}
textview, textview text {{ background: @tiny_surface_dim; color: @tiny_on_surface; }}
scrolledwindow {{ border: none; background: transparent; }}
separator {{ background: alpha(@tiny_outline, 0.24); }}
/* Compact, readable defaults for a dense camera-control workspace. */
.tiny4-root {{ padding: 6px 8px; }}
.content-page {{ padding: 8px; }}
.app-header {{ min-height: 52px; }}
.device-row {{ padding: 8px 12px; border-radius: 18px; }}
.status-chip {{ padding: 6px 10px; margin-bottom: 2px; }}
.preview-panel {{ border-radius: 20px; padding: 4px; }}
.preview-panel picture {{ border-radius: 14px; }}
.preview-meta {{ font-size: 8px; }}
frame {{ background: @tiny_surface; border: 1px solid alpha(@tiny_outline, 0.20); border-radius: 18px; padding: 7px; }}
frame > label {{ color: @tiny_on_surface; font-weight: 650; padding: 0 4px; }}
.adaptive-row {{ min-height: 34px; }}
.control-label {{ font-size: 13px; }}
.availability-note {{ font-size: 11px; padding: 1px 6px 4px; }}
button {{ min-height: 35px; padding: 4px 12px; font-size: 13px; }}
button:disabled, scale:disabled, switch:disabled, checkbutton:disabled {{ opacity: 0.42; }}
scale trough {{ min-height: 6px; border-radius: 99px; background: @tiny_surface_highest; }}
scale highlight {{ min-height: 6px; border-radius: 99px; background: @tiny_primary; }}
scale slider {{ min-width: 18px; min-height: 18px; border-radius: 99px; background: @tiny_primary; }}
scrolledwindow > viewport {{ background: transparent; }}
"#)
}

pub fn install() -> gtk::CssProvider {
    register_fonts();
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display is available"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    provider
}

pub fn apply_theme(provider: &gtk::CssProvider, mode: ThemeMode) {
    let dark = match mode { ThemeMode::Dark => true, ThemeMode::Light => false, ThemeMode::System => system_prefers_dark() };
    provider.load_from_data(&css(dark));
    if let Some(settings) = gtk::Settings::default() {
        settings.set_property("gtk-application-prefer-dark-theme", dark);
    }
}
