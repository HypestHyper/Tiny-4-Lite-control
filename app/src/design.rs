use gtk::prelude::*;

const FAMILY_CSS: &str = r#"
@define-color tiny_surface_dim #151311;
@define-color tiny_surface #1e1a18;
@define-color tiny_surface_low #25201e;
@define-color tiny_surface_high #302825;
@define-color tiny_surface_highest #3a302c;
@define-color tiny_primary #f5ad73;
@define-color tiny_primary_hover #ffc18f;
@define-color tiny_on_primary #2d190d;
@define-color tiny_on_surface #f1e9e4;
@define-color tiny_on_surface_variant #bdb0a8;
@define-color tiny_outline #75665e;

window.tiny4-window { background: @tiny_surface_dim; color: @tiny_on_surface; }
.tiny4-root { padding: 18px; background: @tiny_surface_dim; color: @tiny_on_surface; }
.app-header { min-height: 58px; margin-bottom: 4px; }
.app-title { color: @tiny_on_surface; font-size: 24px; font-weight: 700; }
.device-row { background: @tiny_surface; border-radius: 20px; padding: 12px 16px; margin-bottom: 2px; }
.status-chip { background: @tiny_surface_low; color: @tiny_on_surface_variant; border-radius: 14px; padding: 9px 13px; margin-bottom: 4px; }
.navigation-sidebar { min-width: 154px; padding: 4px 0; }
stacksidebar row { min-height: 38px; border-radius: 16px; margin: 2px 0; padding: 2px 10px; color: @tiny_on_surface_variant; font-weight: 600; }
stacksidebar row:hover { background: @tiny_surface_high; color: @tiny_on_surface; }
stacksidebar row:selected { background: #50321f; color: #ffd0a8; }
stacksidebar row:focus-visible { outline: 2px solid @tiny_primary; outline-offset: 1px; }
.content-stack { background: transparent; }
.content-page { padding: 12px; color: @tiny_on_surface; }
frame { background: @tiny_surface; border: 1px solid alpha(@tiny_outline, 0.18); border-radius: 22px; padding: 10px; }
frame > label { color: @tiny_on_surface; font-weight: 650; }
button { min-height: 34px; padding: 5px 14px; border: 1px solid transparent; border-radius: 16px; background: @tiny_surface_high; color: @tiny_on_surface; font-weight: 600; }
button:hover { background: @tiny_surface_highest; }
button:active { background: @tiny_surface_low; }
button:focus-visible { outline: 2px solid @tiny_primary; outline-offset: 2px; }
button.suggested-action { background: @tiny_primary; color: @tiny_on_primary; }
button.suggested-action:hover { background: @tiny_primary_hover; }
entry, combobox, dropdown, spinbutton { min-height: 36px; border-radius: 12px; background: @tiny_surface_high; color: @tiny_on_surface; }
scale trough { min-height: 6px; border-radius: 99px; background: @tiny_surface_highest; }
scale highlight { background: @tiny_primary; }
scale slider { min-width: 16px; min-height: 16px; border-radius: 99px; background: @tiny_primary; }
.dim-label { color: @tiny_on_surface_variant; }
textview, textview text { background: @tiny_surface_dim; color: @tiny_on_surface; }
scrolledwindow { border: none; }
separator { background: alpha(@tiny_outline, 0.28); }
"#;

pub fn install() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(FAMILY_CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
