//! Palette and stylesheet.
//!
//! One seed colour produces every role (see the GTK app family design
//! language, section COL). The stylesheet is a template over those roles;
//! `§n§` is a scaled pixel size and `¶n¶` a gently scaled font size.

use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeMode { System, Light, Dark }

impl ThemeMode {
    pub fn from_name(value: &str) -> Self {
        match value { "light" => Self::Light, "dark" => Self::Dark, _ => Self::System }
    }
    pub fn name(self) -> &'static str {
        match self { Self::System => "system", Self::Light => "light", Self::Dark => "dark" }
    }
}

pub const ACCENTS: [(&str, &str, &str); 6] = [
    ("mint", "Mint", "#53dbb0"),
    ("sky", "Sky", "#6aa6ff"),
    ("lilac", "Lilac", "#b38cff"),
    ("rose", "Rose", "#ff8fb1"),
    ("amber", "Amber", "#ffb35c"),
    ("leaf", "Leaf", "#9ccc65"),
];

pub fn accent_hex(name: &str) -> String {
    if let Some((_, _, hex)) = ACCENTS.iter().find(|(id, _, _)| *id == name) { return (*hex).to_string(); }
    if parse_hex(name).is_some() { return name.to_string(); }
    ACCENTS[1].2.to_string()
}

// ── Colour maths ──────────────────────────────────────────────────────────

fn parse_hex(value: &str) -> Option<(f64, f64, f64)> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 { return None; }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|v| f64::from(v) / 255.0);
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// RGB (0–1) to (hue 0–1, lightness, saturation), the HLS model.
fn rgb_to_hls(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON { return (0.0, l, 0.0); }
    let d = max - min;
    let s = if l <= 0.5 { d / (max + min) } else { d / (2.0 - max - min) };
    let h = if (max - r).abs() < f64::EPSILON { ((g - b) / d).rem_euclid(6.0) }
        else if (max - g).abs() < f64::EPSILON { (b - r) / d + 2.0 }
        else { (r - g) / d + 4.0 };
    (h / 6.0, l, s)
}

fn hls_to_rgb(h: f64, l: f64, s: f64) -> (f64, f64, f64) {
    if s == 0.0 { return (l, l, l); }
    let m2 = if l <= 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let m1 = 2.0 * l - m2;
    let channel = |hue: f64| {
        let hue = hue.rem_euclid(1.0);
        if hue < 1.0 / 6.0 { m1 + (m2 - m1) * hue * 6.0 }
        else if hue < 0.5 { m2 }
        else if hue < 2.0 / 3.0 { m1 + (m2 - m1) * (2.0 / 3.0 - hue) * 6.0 }
        else { m1 }
    };
    (channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

fn hex(rgb: (f64, f64, f64)) -> String {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(rgb.0), byte(rgb.1), byte(rgb.2))
}

fn luminance(rgb: (f64, f64, f64)) -> f64 {
    let lin = |c: f64| if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(rgb.0) + 0.7152 * lin(rgb.1) + 0.0722 * lin(rgb.2)
}

pub fn contrast_ratio(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(parse_hex(a).unwrap_or((0.0, 0.0, 0.0))), luminance(parse_hex(b).unwrap_or((1.0, 1.0, 1.0))));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

// ── Palette ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Palette {
    roles: Vec<(&'static str, String)>,
}

impl Palette {
    pub fn role(&self, name: &str) -> &str {
        self.roles.iter().find(|(role, _)| *role == name).map(|(_, value)| value.as_str()).unwrap_or("#ff00ff")
    }

    /// RGBA 0–1 for drawing with cairo.
    pub fn rgb(&self, name: &str) -> (f64, f64, f64) {
        parse_hex(self.role(name)).unwrap_or((1.0, 0.0, 1.0))
    }

    pub fn new(seed: &str, dark: bool) -> Palette {
        use std::collections::HashMap;
        let (r, g, b) = parse_hex(seed).unwrap_or((0.416, 0.651, 1.0));
        let (hue, _, seed_sat) = rgb_to_hls(r, g, b);
        let sat = seed_sat.clamp(0.45, 0.85);
        let tertiary_hue = (hue + 0.17).rem_euclid(1.0);
        // role -> (hue, saturation, lightness), dark value then light value.
        let table: [(&'static str, f64, (f64, f64), (f64, f64)); 24] = [
            ("surface_dim", hue, (0.14, 0.065), (0.22, 0.955)),
            ("surface_lowest", hue, (0.14, 0.05), (0.14, 1.0)),
            ("surface_low", hue, (0.14, 0.095), (0.22, 0.97)),
            ("surface", hue, (0.14, 0.115), (0.28, 0.985)),
            ("surface_high", hue, (0.14, 0.15), (0.22, 0.925)),
            ("surface_highest", hue, (0.14, 0.19), (0.22, 0.895)),
            ("primary", hue, (sat, 0.66), (sat, 0.34)),
            ("primary_hover", hue, (sat, 0.74), (sat, 0.28)),
            ("on_primary", hue, (sat, 0.12), (sat, 0.99)),
            ("primary_container", hue, (sat * 0.8, 0.24), (sat * 0.8, 0.86)),
            ("on_primary_container", hue, (sat, 0.84), (sat, 0.14)),
            ("secondary_container", hue, (0.20, 0.26), (0.28, 0.86)),
            ("secondary_container_hover", hue, (0.22, 0.31), (0.28, 0.81)),
            ("on_secondary_container", hue, (0.25, 0.88), (0.30, 0.14)),
            ("tertiary", tertiary_hue, (0.45, 0.72), (0.45, 0.40)),
            ("tertiary_container", tertiary_hue, (0.25, 0.28), (0.45, 0.89)),
            ("on_tertiary_container", tertiary_hue, (0.35, 0.88), (0.40, 0.16)),
            ("on_surface", hue, (0.10, 0.90), (0.12, 0.11)),
            ("on_surface_variant", hue, (0.08, 0.76), (0.08, 0.30)),
            ("outline", hue, (0.07, 0.57), (0.07, 0.46)),
            ("outline_variant", hue, (0.09, 0.27), (0.10, 0.80)),
            ("inverse_surface", hue, (0.10, 0.89), (0.10, 0.19)),
            ("inverse_on_surface", hue, (0.12, 0.18), (0.12, 0.94)),
            ("inverse_primary", hue, (sat, 0.38), (sat, 0.78)),
        ];
        let mut map: HashMap<&'static str, (f64, f64, f64)> = HashMap::new();
        for (name, h, d, l) in table {
            let (s, light) = if dark { d } else { l };
            map.insert(name, (h, s, light));
        }
        let color = |m: &HashMap<&'static str, (f64, f64, f64)>, name: &str| {
            let (h, s, l) = m[name];
            hex(hls_to_rgb(h, l, s))
        };
        // The reference table is a starting point. Measure, and move a role's
        // lightness (away from its background) until the pair is legible.
        let ensure = |m: &mut HashMap<&'static str, (f64, f64, f64)>, fg: &'static str, bgs: &[&str], min: f64, direction: f64| {
            for _ in 0..90 {
                let fg_hex = color(m, fg);
                if bgs.iter().all(|bg| contrast_ratio(&fg_hex, &color(m, bg)) >= min) { break; }
                let entry = m.get_mut(fg).expect("known role");
                entry.2 = (entry.2 + direction * 0.01).clamp(0.0, 1.0);
            }
        };
        let surfaces = ["surface_dim", "surface_low", "surface", "surface_high", "surface_highest"];
        let toward_text = if dark { 1.0 } else { -1.0 }; // text gets lighter on dark, darker on light
        ensure(&mut map, "on_surface", &surfaces, 7.0, toward_text);
        ensure(&mut map, "on_surface_variant", &surfaces, 4.5, toward_text);
        ensure(&mut map, "outline", &["surface"], 3.0, toward_text);
        ensure(&mut map, "primary", &surfaces, 3.0, toward_text);
        ensure(&mut map, "on_primary", &["primary"], 4.5, -toward_text);
        // If the on-colour is already at its extreme, the primary has to give.
        ensure(&mut map, "primary", &["on_primary"], 4.5, toward_text);
        ensure(&mut map, "tertiary", &surfaces, 3.0, toward_text);
        ensure(&mut map, "on_primary_container", &["primary_container"], 4.5, toward_text);
        ensure(&mut map, "on_secondary_container", &["secondary_container"], 4.5, toward_text);
        ensure(&mut map, "on_tertiary_container", &["tertiary_container"], 4.5, toward_text);
        ensure(&mut map, "inverse_on_surface", &["inverse_surface"], 4.5, -toward_text);
        // Hover tracks the (possibly adjusted) primary.
        let primary = map["primary"];
        let hover_shift = if dark { 0.08 } else { -0.06 };
        map.insert("primary_hover", (primary.0, primary.1, (primary.2 + hover_shift).clamp(0.0, 1.0)));

        let mut roles: Vec<(&'static str, String)> = table.iter().map(|(name, ..)| (*name, color(&map, name))).collect();
        let fixed = |d: &str, l: &str| (if dark { d } else { l }).to_string();
        roles.push(("error", fixed("#ffb4ab", "#ba1a1a")));
        roles.push(("error_container", fixed("#93000a", "#ffdad6")));
        roles.push(("on_error_container", fixed("#ffdad6", "#410002")));
        roles.push(("error_hover", fixed("#b3261e", "#ba1a1a")));
        // The preview and joystick stay dark in both themes: they show a camera image.
        roles.push(("stage", "#0d0f12".to_string()));
        roles.push(("on_stage", "#e6e9ee".to_string()));
        Palette { roles }
    }
}

thread_local! {
    static CURRENT: RefCell<Palette> = RefCell::new(Palette::new("#6aa6ff", true));
}

pub fn set_current(palette: Palette) { CURRENT.with(|p| *p.borrow_mut() = palette); }

// ── Scale ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiScale { Auto, Compact, Default, Large }

impl UiScale {
    pub fn from_name(value: &str) -> Self {
        match value { "compact" => Self::Compact, "default" => Self::Default, "large" => Self::Large, _ => Self::Auto }
    }
    pub fn name(self) -> &'static str {
        match self { Self::Auto => "auto", Self::Compact => "compact", Self::Default => "default", Self::Large => "large" }
    }
    /// The geometric factor, given the window width for Auto.
    pub fn factor(self, auto_compact: bool) -> f64 {
        match self {
            Self::Compact => 0.8, Self::Default => 0.9, Self::Large => 1.0,
            Self::Auto => if auto_compact { 0.8 } else { 0.9 },
        }
    }
}

fn scale_template(template: &str, geometric: f64) -> String {
    let font = 1.0 - (1.0 - geometric) * 0.6;
    let mut out = String::with_capacity(template.len() + 1024);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '§' || c == '¶' {
            let mut number = String::new();
            for n in chars.by_ref() { if n == c { break; } number.push(n); }
            let value: f64 = number.parse().unwrap_or(0.0);
            let scaled = if c == '§' {
                // Hairlines and full pills keep their size.
                if value <= 2.0 || value >= 900.0 { value } else { value * geometric }
            } else {
                (value * font).max(10.0)
            };
            out.push_str(&format!("{}px", scaled.round() as i64));
        } else {
            out.push(c);
        }
    }
    out
}

/// The complete stylesheet for a palette and scale.
pub fn stylesheet(palette: &Palette, geometric: f64, reduce_motion: bool, composited: bool) -> String {
    let mut css = String::new();
    for (name, value) in &palette.roles {
        css.push_str(&format!("@define-color t_{name} {value};\n"));
    }
    css.push_str(&scale_template(TEMPLATE, geometric));
    for (id, _, hex) in ACCENTS { css.push_str(&format!(".t4 button.swatch-{id} {{ background: {hex}; }}\n")); }
    if reduce_motion { css.push_str("\n* { transition: none; animation: none; }\n"); }
    // Without a compositor a shadow is drawn as a black frame, so drop them.
    if !composited { css.push_str("\n.t4 popover > contents, .t4 tooltip.background, .toast, .dialog-card { box-shadow: none; }\n"); }
    css
}

const TEMPLATE: &str = r#"
* { box-shadow: none; text-shadow: none; -gtk-icon-shadow: none; outline-style: none; }
* { font-family: "Google Sans Flex", "Cantarell", "Inter", sans-serif; }

window.t4, window.t4 > contents { background: @t_surface_dim; color: @t_on_surface; }
window.t4.csd { border-radius: §18§; }
.t4 tooltip.background { background: @t_surface_high; color: @t_on_surface; border-radius: §8§; padding: §6§ §10§; border: 1px solid alpha(@t_on_surface, 0.12); box-shadow: 0 4px 12px alpha(#000000, 0.35); }
.t4 tooltip.background label { color: @t_on_surface; font-size: ¶12.5¶; }

/* Type */
.app-title { font-size: ¶24¶; font-weight: 700; }
.display { font-size: ¶28¶; font-weight: 700; }
.title { font-size: ¶20¶; font-weight: 700; }
.heading { font-size: ¶15¶; font-weight: 700; }
.subheading { font-size: ¶13¶; font-weight: 600; }
.body { font-size: ¶14¶; }
.small { font-size: ¶13¶; }
.tiny { font-size: ¶11.5¶; }
.mono { font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: ¶12.5¶; }
.dim { color: @t_on_surface_variant; }
.faint { color: @t_outline; }
.accent { color: @t_primary; }
.warn-text { color: @t_tertiary; }
.error-text { color: @t_error; }
.tnum { font-feature-settings: "tnum"; }

/* Icons: Material Symbols Rounded, outlined; filled when selected */
.icon { font-family: "Material Symbols Rounded"; font-size: §20§; font-weight: 400; font-feature-settings: "liga"; font-variation-settings: "FILL" 0, "wght" 400, "opsz" 24; min-width: §20§; }
.icon.filled { font-variation-settings: "FILL" 1, "wght" 500, "opsz" 24; }
.icon.big { font-size: §64§; min-width: §64§; color: @t_outline; }
.icon.small-icon { font-size: §16§; min-width: §16§; }

/* Focus ring */
.t4 button:focus-visible, .t4 entry:focus-within, .t4 dropdown:focus-within > button, .t4 switch:focus-visible, .t4 scale:focus-visible slider, .t4 .pad:focus-visible, .t4 row:focus-visible {
  outline: 2px solid @t_primary; outline-offset: 2px; }

/* Buttons */
.t4 button { min-height: §40§; min-width: §40§; padding: 0 §18§; border: none; border-radius: §20§; background: @t_secondary_container; color: @t_on_secondary_container; font-size: ¶14¶; font-weight: 600; transition: background 200ms cubic-bezier(0.2,0,0,1), border-radius 220ms cubic-bezier(0.34,1.36,0.64,1), color 200ms; }
/* The desktop's GTK theme must not move or resize anything under the pointer: some grow, shift or scale buttons
   on hover through properties this stylesheet did not set. These win over the theme in every state. */
.t4 button, .t4 switch, .t4 checkbutton, .t4 scale slider { margin: 0; transform: none; box-shadow: none; text-shadow: none; -gtk-icon-shadow: none; }
.t4 entry, .t4 spinbutton, .t4 dropdown, .t4 expander, .t4 row, .t4 scale, .t4 checkbutton check, .t4 radio { transform: none; margin: 0; }
.t4 button:hover { background: @t_secondary_container_hover; }
.t4 button:active { border-radius: §10§; }
.t4 button:disabled { opacity: 0.45; }
.t4 button.filled { background: @t_primary; color: @t_on_primary; }
.t4 button.filled:hover { background: @t_primary_hover; }
.t4 button.quiet { background: transparent; color: @t_primary; padding: 0 §12§; }
.t4 button.quiet:hover { background: alpha(@t_primary, 0.10); }
.t4 button.danger { background: @t_error_container; color: @t_on_error_container; }
.t4 button.danger:hover { background: @t_error_hover; color: #ffffff; }
.t4 button.round { padding: 0; border-radius: §20§; }
.t4 button.round:active { border-radius: §12§; }
.t4 button.preset { min-width: §38§; min-height: §38§; padding: 0; border-radius: §10§; background: transparent; border: 2px solid @t_outline; font-weight: 700; color: @t_on_surface; }
.t4 button.preset:hover { background: alpha(@t_primary, 0.14); }
.t4 button.preset:disabled { opacity: 0.4; }
.t4 button.preset.worded { padding: 0 §8§; font-size: ¶11¶; border-radius: §12§; }
.t4 button.preset.empty { border-style: dashed; color: @t_on_surface_variant; opacity: 0.7; }
.t4 button.preset.memory { border-color: @t_tertiary; }
.t4 .quick-chooser > contents { border-radius: §20§; padding: §6§; }
.t4 .quick-chooser button.flat-item { background: transparent; color: @t_on_surface; border-radius: §12§; padding: §8§ §14§; min-height: §40§; font-weight: 500; box-shadow: none; transition: background 150ms ease, border-radius 200ms cubic-bezier(0.2, 0, 0, 1); }
.t4 .quick-chooser button.flat-item:hover { background: alpha(@t_on_surface, 0.08); }
.t4 .quick-chooser button.flat-item:active { background: alpha(@t_on_surface, 0.12); border-radius: §20§; }
.t4 .quick-chooser button.flat-item.chosen { background: @t_secondary_container; color: @t_on_secondary_container; border-radius: §20§; }
.t4 .quick-chooser button.flat-item.chosen label { color: @t_on_secondary_container; }
.t4 .quick-chooser .chooser-title { margin: §4§ §8§ §6§; }
.t4 .quick-chooser .chooser-note { margin: §8§ §8§ §2§; }
.t4 .quick-chooser .chooser-head { margin-top: §6§; }
.t4 button.fold .icon { font-size: §22§; }
.t4 button.look-bubble { border-radius: 999px; padding: §6§ §14§ §6§ §10§; min-height: §28§; background: @t_surface_highest; }
.t4 button.look-bubble.on-stage { background: alpha(#000000, 0.45); color: @t_on_stage; min-height: §24§; padding: §4§ §12§ §4§ §8§; }
.t4 button.look-bubble.on-stage.lit { background: alpha(#000000, 0.6); color: @t_primary; }
.t4 button.look-bubble.lit { background: alpha(@t_primary, 0.22); color: @t_primary; }
.t4 button.round.small { min-height: §32§; min-width: §32§; padding: 0; border-radius: §16§; }
.t4 button.round.small:active { border-radius: §8§; }
.t4 button.flat-icon { background: transparent; color: @t_on_surface_variant; }
.t4 button.flat-icon:hover { background: alpha(@t_on_surface, 0.08); color: @t_on_surface; }
/* A reset button with something to reset: tinted, so a value away from its default is easy to spot. */
.t4 button.flat-icon.reset:not(:disabled) { background: alpha(@t_primary, 0.14); color: @t_primary; }
.t4 button.flat-icon.reset:not(:disabled):hover { background: alpha(@t_primary, 0.24); }
.t4 button.small { min-height: §32§; padding: 0 §14§; border-radius: §16§; font-size: ¶13¶; }
.t4 button.small:active { border-radius: §8§; }
.t4 button.on-stage { background: alpha(#000000, 0.45); color: @t_on_stage; }
.t4 button.on-stage:hover { background: alpha(#000000, 0.65); }
.t4 button.on-stage.lit { background: #28c85a; color: #04210d; }
.t4 dropdown.on-stage-picker > button { background: alpha(#000000, 0.45); color: @t_on_stage; min-height: §32§; padding: 0 §10§; border-radius: 999px; font-size: ¶13¶; }
.t4 scale.on-stage-slider { background: alpha(#000000, 0.45); border-radius: 999px; padding: 0 §10§; min-height: §32§; }
.t4 scale.on-stage-slider trough { min-height: §6§; }
.t4 scale.on-stage-slider highlight { min-height: §6§; background: #28c85a; }
.t4 button.on-stage.lit:hover { background: #3ad66b; }
.t4 button.close-button:hover { background: @t_error_hover; color: #ffffff; }
.t4 button > box { }
.t4 button.has-icon > box { }

/* Window buttons: 32 px, round; close turns red */
.t4 windowcontrols { margin: 0; }
.t4 windowcontrols button { min-width: §32§; min-height: §32§; padding: 0; border-radius: §16§; background: transparent; color: @t_on_surface_variant; }
.t4 windowcontrols button:hover { background: alpha(@t_on_surface, 0.10); color: @t_on_surface; }
.t4 windowcontrols button.close:hover { background: @t_error_hover; color: #ffffff; }

.sleep-thumb { border-radius: §8§; }
/* A newer version: a small dot on the Settings tab, and a tinted line on the Updates card. */
.news-dot { min-width: §8§; min-height: §8§; border-radius: 999px; background: @t_primary; margin-top: §2§; }
.update-news { background: @t_secondary_container; color: @t_on_secondary_container; border-radius: §12§; padding: §8§ §12§; }
/* A folded card says it opens with a click; a card opened from a link glows for a moment. */
.t4 .card.folded { transition: box-shadow 300ms cubic-bezier(0.2,0,0,1); }
.t4 .card.folded:hover { background-color: @t_surface_high; }
.t4 .card.flash { box-shadow: 0 0 0 2px @t_primary; }

/* Header */
.header { min-height: §64§; padding: 0 §20§; background: @t_surface_dim; }
.logo-tile { background: @t_primary_container; border-radius: §14§; min-width: §40§; min-height: §40§; color: @t_on_primary_container; }
.logo-tile .icon { color: @t_on_primary_container; font-size: §24§; font-variation-settings: "FILL" 1, "wght" 500, "opsz" 24; }
.status-chip { background: @t_surface_high; color: @t_on_surface_variant; border-radius: 999px; padding: §6§ §14§ §6§ §10§; }
.status-chip.ok { background: @t_primary_container; color: @t_on_primary_container; }
.status-chip.warn { background: @t_tertiary_container; color: @t_on_tertiary_container; }
.status-chip.error { background: alpha(@t_error, 0.14); color: @t_error; }
.status-chip label { font-size: ¶13¶; font-weight: 600; }

/* Tab bar */
.tabbar { background: @t_surface_low; border-radius: §24§; padding: §4§; }
.t4 button.tab { background: transparent; color: @t_on_surface_variant; border-radius: §20§; padding: 0 §16§; min-height: §40§; min-width: §40§; }
.t4 button.tab:hover { background: alpha(@t_on_surface, 0.08); color: @t_on_surface; }
.t4 button.tab:checked { background: @t_secondary_container; color: @t_on_secondary_container; border-radius: §12§; }
.t4 button.tab:checked .icon { font-variation-settings: "FILL" 1, "wght" 500, "opsz" 24; }

/* Segmented (connected) buttons */
.segmented { background: transparent; }
.t4 button.segment { background: @t_surface_highest; color: @t_on_surface; border-radius: §6§; min-height: §40§; padding: 0 §16§; margin: 0 1px; }
.t4 button.segment:hover { background: @t_surface_high; }
.t4 button.segment.first { border-radius: §20§ §6§ §6§ §20§; margin-left: 0; }
.t4 button.segment.last { border-radius: §6§ §20§ §20§ §6§; margin-right: 0; }
.t4 button.segment.first.last { border-radius: §20§; }
.t4 button.segment:checked { background: @t_primary; color: @t_on_primary; border-radius: §20§; }
.t4 button.segment:checked:hover { background: @t_primary_hover; }
.t4 button.segment:active { border-radius: §4§; }
.segmented.small button.segment { min-height: §32§; padding: 0 §12§; font-size: ¶13¶; }

/* Surfaces */
.card { background: @t_surface; border-radius: §24§; padding: §14§ §16§; }
.t4 .service-notice { background: alpha(@t_tertiary, 0.14); color: @t_on_surface; border-radius: §12§; padding: §10§ §12§; }
.t4 .service-notice .icon { color: @t_tertiary; }
.t4 label.code { font-family: monospace; font-size: ¶12¶; background: @t_surface_high; border-radius: §10§; padding: §10§ §12§; }
.t4 .setup-banner { background: alpha(@t_primary, 0.14); border-radius: §16§; padding: §12§ §16§; }
.t4 .setup-banner > .icon { color: @t_primary; font-size: §24§; }
.t4 spinbutton { background: @t_surface_high; border-radius: §12§; border: none; padding: 0; min-height: §36§; box-shadow: none; }
.t4 spinbutton > text { background: transparent; border: none; padding: 0 §6§ 0 §12§; min-width: §36§; color: @t_on_surface; font-feature-settings: "tnum"; }
.t4 spinbutton > button { background: transparent; border: none; box-shadow: none; border-radius: §9§; min-width: §30§; min-height: §30§; margin: §3§ §2§; padding: 0; color: @t_on_surface_variant; }
.t4 spinbutton > button:last-child { margin-right: §3§; }
.t4 spinbutton > button:hover { background: alpha(@t_on_surface, 0.09); color: @t_on_surface; }
.t4 spinbutton > button:active { background: alpha(@t_primary, 0.2); color: @t_primary; }
.t4 spinbutton > button:disabled { background: transparent; opacity: 0.35; }
.t4 spinbutton:focus-within { box-shadow: inset 0 0 0 2px alpha(@t_primary, 0.8); }
.t4 .limit-entry spinbutton { border-top-right-radius: 0; border-bottom-right-radius: 0; }
.t4 .limit-entry button.take-current { border-radius: 0 §12§ §12§ 0; min-width: §38§; min-height: §36§; margin: 0 0 0 1px; background: @t_surface_high; color: @t_on_surface_variant; }
.t4 .limit-entry button.take-current:hover { background: alpha(@t_primary, 0.2); color: @t_primary; }
.t4 .warning-text { color: @t_error; }
.t4 .gesture-icon { background: @t_secondary_container; color: @t_on_secondary_container; border-radius: 999px; }
.t4 .gesture-icon .gesture-mark { color: @t_primary; }
.t4 .studio-state { background: @t_surface_high; border-radius: §12§; padding: §10§ §12§; }
.t4 .studio-state .icon { color: @t_primary; }
.card.tight-x { padding-left: §12§; padding-right: §12§; }
.card.tight { padding: §16§ §18§; }
.card-inner { background: @t_surface_high; border-radius: §20§; padding: §16§; }
.stage-card { background: @t_surface; border-radius: §32§; padding: §10§; }
.stage { background: @t_stage; border-radius: §24§; color: @t_on_stage; }
.stage label { color: @t_on_stage; }
.group-title { font-size: ¶20¶; font-weight: 700; }
.group-head .icon { color: @t_primary; }
.divider { background: @t_outline_variant; min-height: 1px; }

/* Setting rows */
.setting-row { padding: §6§ 0; }
.setting-row.slim { padding: §3§ 0; }
.setting-row.slim .slim-head { min-height: §30§; }
/* The small window's shortcut buttons: equal cells, names shortened rather than cut off. */
.t4 .slim-presets button.preset { min-width: 0; padding: 0 §6§; }
.setting-label { font-size: ¶14¶; font-weight: 600; }
.setting-summary { font-size: ¶13¶; color: @t_on_surface_variant; }
.setting-value { font-size: ¶13¶; font-weight: 600; color: @t_primary; font-feature-settings: "tnum"; }
.t4 .value-pill { background: alpha(@t_primary, 0.12); border-radius: 999px; padding: §1§ §3§ §1§ §10§; min-height: §28§; }
.t4 .value-pill .pill-arrow { min-width: §20§; min-height: §14§; border-radius: §5§; color: @t_primary; }
.t4 .value-pill .pill-arrow .icon { font-size: §16§; margin: -§2§ 0; }
.t4 .value-pill .pill-arrow:hover { background: alpha(@t_primary, 0.16); }
.t4 .value-pill:disabled { background: alpha(@t_on_surface, 0.06); }
.t4 .value-pill:disabled .setting-value, .t4 .value-pill:disabled .pill-arrow { color: alpha(@t_on_surface, 0.38); }
.setting-row.blocked .setting-label { color: @t_on_surface_variant; }
.reason { color: @t_tertiary; font-size: ¶12.5¶; }
.reason .icon { color: @t_tertiary; font-size: §16§; min-width: §16§; }
.badge { background: @t_tertiary_container; color: @t_on_tertiary_container; border-radius: 999px; padding: 1px §8§; font-size: ¶11.5¶; font-weight: 600; }
.badge.neutral { background: @t_surface_highest; color: @t_on_surface_variant; }

/* Switch: 44×24, thumb grows when on */
.t4 switch { min-width: §44§; min-height: §24§; border-radius: §16§; padding: 0; margin: §4§ 0; background: @t_surface_highest; border: 2px solid @t_outline; color: transparent; transition: background 250ms cubic-bezier(0.2,0,0,1), border-color 250ms; }
.t4 switch slider { min-width: §16§; min-height: §16§; margin: 2px; border-radius: 999px; border: none; background: @t_outline; transition: min-width 250ms cubic-bezier(0.34,1.36,0.64,1), min-height 250ms cubic-bezier(0.34,1.36,0.64,1), background 250ms; }
.t4 switch:checked { background: @t_primary; border-color: @t_primary; }
.t4 switch:checked slider { min-width: §20§; min-height: §20§; margin: 0; background: @t_on_primary; }
.t4 switch:disabled { opacity: 0.45; }
.t4 switch label { font-size: 0; min-width: 0; }

/* Slider: tall pill track, bar handle with a ring */
.t4 .setting-row scale { padding: §6§ §2§ §2§ §2§; }
.t4 scale trough { min-height: §14§; border-radius: §8§; background: @t_surface_highest; border: none; }
.t4 scale highlight { min-width: 0; margin: 0; padding: 0; min-height: §14§; border-radius: §8§; background: @t_primary; border: none; }
/* Colour sliders: the bar shows the colours it moves between, and the filled part lets them show through. */
/* Colour temperature tells the camera what colour the light is, and it corrects for it: a low setting makes
   the picture cold (blue), a high one warm (red). The bar shows what happens to the picture. */
.t4 scale.grad-temperature trough { background-image: linear-gradient(to right, #5f8dff, #b9d0ff 30%, #f2f0ea 50%, #ffb27a 70%, #ff5a36); }
/* Hue on this camera only shifts the tint a little, from bluish to reddish, through neutral in the middle. */
.t4 scale.grad-hue trough { background-image: linear-gradient(to right, #d05bc0, #a49aa8 50%, #5bbf6a); }
.t4 scale.grad-temperature highlight, .t4 scale.grad-hue highlight { background: transparent; }
.t4 scale slider { min-width: 4px; min-height: §32§; margin: -§8§ 0; border-radius: 3px; background: @t_primary; border: none; box-shadow: 0 0 0 §4§ @t_surface; transition: min-width 160ms cubic-bezier(0.2,0,0,1); }
.card-inner scale slider { box-shadow: 0 0 0 §4§ @t_surface_high; }
/* The slider keeps its width while pressed, so nothing around it moves. */
.t4 scale:disabled { opacity: 0.45; }
.t4 scale marks { color: @t_on_surface_variant; }
.t4 scale mark label { font-size: ¶11.5¶; color: @t_on_surface_variant; }
.t4 scale mark indicator { background: @t_outline_variant; min-width: 1px; min-height: §4§; }
.t4 scale.vertical trough { min-width: §16§; }

/* Dropdown and entries */
.t4 dropdown > button, .t4 entry { background: @t_surface_highest; color: @t_on_surface; border-radius: §12§; min-height: §38§; border: none; padding: 0 §12§; }
.t4 dropdown > button:hover { background: @t_surface_high; }
.t4 entry { padding: 0 §14§; caret-color: @t_primary; }
.t4 entry:focus-within { outline: 2px solid @t_primary; outline-offset: -2px; }
.t4 entry.search { border-radius: §24§; min-height: §44§; padding: 0 §18§; }
.t4 entry placeholder { color: @t_outline; }
.t4 popover > contents, .t4 popover.menu > contents { background: @t_surface_high; color: @t_on_surface; border-radius: §16§; padding: §6§; border: none; box-shadow: 0 6px 18px alpha(#000000, 0.4); }
.t4 popover.no-shadow > contents { box-shadow: none; }
.t4 popover modelbutton, .t4 popover listview row, .t4 popover row { border-radius: §10§; min-height: §36§; padding: 0 §12§; }
.t4 popover row:hover, .t4 popover modelbutton:hover { background: alpha(@t_on_surface, 0.08); }
.t4 popover row:selected { background: @t_secondary_container; color: @t_on_secondary_container; }
.info-pop { padding: §6§; }
.info-pop label.dim { font-size: ¶13¶; }

/* Text areas (developer tools) */
.t4 textview, .t4 textview text { background: @t_surface_high; color: @t_on_surface; }
.t4 textview { border-radius: §12§; padding: §6§; font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: ¶12.5¶; }
.t4 textview text selection { background: alpha(@t_primary, 0.35); }

/* Scrolling */
.t4 scrolledwindow, .t4 viewport { background: transparent; border: none; }
/* Thin scrollbars: GTK's own padding and trough are taken out so they are as wide as the slider. */
.t4 scrollbar { background: transparent; padding: 0; margin: 0; border: none; min-width: 0; min-height: 0; }
.t4 scrollbar trough { background: transparent; border: none; padding: 0; margin: 0; min-width: 0; min-height: 0; }
.t4 scrollbar.vertical { min-width: §8§; }
.t4 scrollbar.horizontal { min-height: §8§; }
.t4 scrollbar slider { min-width: §4§; min-height: §4§; margin: 0 2px; border-radius: 3px; background: alpha(@t_on_surface, 0.28); }
.t4 scrollbar.horizontal slider { margin: 2px 0; }
.t4 scrollbar.vertical slider { min-height: §32§; }
.t4 scrollbar.horizontal slider { min-width: §32§; }
.t4 scrollbar slider:hover { background: alpha(@t_on_surface, 0.5); }
/* The scrollbar keeps one size, hovered or not: where it takes room, changing it would move the page. */
.t4 paned > separator { min-width: §8§; min-height: §8§; background: transparent; background-image: none; }
.t4 button.preview-play { min-width: §64§; min-height: §64§; border-radius: 999px; padding: 0; }
.t4 button.preview-play .icon { font-size: §36§; }

/* Toast */
.toast { background: @t_inverse_surface; color: @t_inverse_on_surface; border-radius: §14§; padding: §12§ §18§; box-shadow: 0 6px 20px alpha(#000000, 0.45); }
.toast label { color: @t_inverse_on_surface; font-size: ¶14¶; }
.toast.error { background: @t_error_container; }
.toast.error label { color: @t_on_error_container; }
.toast.no-shadow { box-shadow: none; }

/* In-window dialog */
.scrim { background: alpha(#000000, 0.5); }
.dialog-card { background: @t_surface_high; border-radius: §28§; padding: §24§; min-width: §320§; box-shadow: 0 8px 28px alpha(#000000, 0.5); }
.dialog-card.no-shadow { box-shadow: none; }
.dialog-icon { color: @t_tertiary; font-size: §32§; min-width: §32§; }

/* Preview */
.preview-surface { background: @t_stage; border-radius: §24§; }
.preview-surface picture { border-radius: §24§; }
.preview-note { color: @t_on_stage; font-size: ¶13¶; }
.overlay-chip { background: alpha(#000000, 0.5); color: @t_on_stage; border-radius: 999px; padding: §4§ §12§; font-size: ¶12.5¶; font-weight: 600; font-feature-settings: "tnum"; }
.overlay-chip label { color: @t_on_stage; }

/* Gimbal model */
.model-frame { background: @t_surface_high; border-radius: §20§; }
.model { background: transparent; }

/* Joystick */
.pad { background: transparent; }
.t4 .compact .readout { font-size: ¶14¶; min-width: §40§; }
.t4 .compact button.small { min-height: §24§; padding: 0 §6§; font-size: ¶12¶; border-radius: §12§; }
.t4 .compact button.round.small { min-height: §26§; min-width: §26§; padding: 0; }
.readout { font-size: ¶20¶; font-weight: 700; font-feature-settings: "tnum"; min-width: §72§; }
.t4 .model-readouts { padding: 0 §8§ §10§ §8§; }
.t4 .model-readouts .readout, .t4 .model-readouts .setting-value { font-size: ¶14¶; font-weight: 700; min-width: 0; color: @t_on_surface; }

/* Preset list items */
.list-item { background: @t_surface_high; border-radius: §20§; padding: §12§ §16§; }
.list-item.empty { background: transparent; border: 1px dashed @t_outline_variant; }
.chip { background: @t_surface_highest; border-radius: §8§; padding: §4§ §10§; font-size: ¶12.5¶; }
.chip.checked { background: @t_secondary_container; color: @t_on_secondary_container; }

/* Accent swatches */
.t4 button.swatch { min-width: §44§; min-height: §44§; padding: 0; border-radius: §22§; border: 2px solid transparent; transition: border-radius 320ms cubic-bezier(0.34,1.36,0.64,1); }
.t4 button.swatch:checked { border-radius: §12§; border-color: @t_on_surface; }
.t4 button.swatch:hover { background-image: none; }

/* Slim window */
.slim-root { padding: 0 §14§ §14§ §14§; }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hls_round_trips_a_seed() {
        let (r, g, b) = parse_hex("#6aa6ff").unwrap();
        let (h, l, s) = rgb_to_hls(r, g, b);
        assert_eq!(hex(hls_to_rgb(h, l, s)), "#6aa6ff");
    }

    #[test]
    fn scale_template_keeps_hairlines_and_pills() {
        let out = scale_template("a:§40§; b:§2§; c:§999§; d:¶14¶;", 0.8);
        assert_eq!(out, "a:32px; b:2px; c:999px; d:12px;");
    }

    #[test]
    fn every_family_accent_meets_contrast_in_both_themes() {
        let mut seeds: Vec<String> = ACCENTS.iter().map(|a| a.2.to_string()).collect();
        // Extreme custom seeds must stay legible too.
        seeds.extend(["#ffffff", "#000000", "#ff0000", "#00ff00", "#0000ff", "#ffff00"].map(String::from));
        for seed in seeds {
            for dark in [true, false] {
                let p = Palette::new(&seed, dark);
                let check = |fg: &str, bg: &str, min: f64| {
                    let ratio = contrast_ratio(p.role(fg), p.role(bg));
                    assert!(ratio >= min, "{seed} dark={dark}: {fg} on {bg} is {ratio:.2}, need {min}");
                };
                for surface in ["surface_dim", "surface_low", "surface", "surface_high", "surface_highest"] {
                    check("on_surface", surface, 7.0);
                    check("on_surface_variant", surface, 4.5);
                    check("primary", surface, 3.0);
                    check("tertiary", surface, 3.0);
                }
                check("on_primary", "primary", 4.5);
                check("on_primary_container", "primary_container", 4.5);
                check("on_secondary_container", "secondary_container", 4.5);
                check("on_tertiary_container", "tertiary_container", 4.5);
                check("on_error_container", "error_container", 4.5);
                check("inverse_on_surface", "inverse_surface", 4.5);
                check("outline", "surface", 3.0);
            }
        }
    }
}

/// Borrow the current palette without cloning it (used while drawing).
pub fn with<R>(f: impl FnOnce(&Palette) -> R) -> R { CURRENT.with(|p| f(&p.borrow())) }

#[cfg(test)]
mod hover_tests {
    /// Nothing may change size under the pointer: a hovered or pressed widget that grows or shrinks moves
    /// everything around it, which shows as the window "breathing". Colour and corner shape are fine.
    #[test]
    fn hover_and_press_never_change_a_size() {
        let source = include_str!("theme.rs");
        let sizes = ["min-width", "min-height", "padding", "margin", "border-width", "font-size", "font-weight", "width", "height", "transform", "letter-spacing"];
        let start = source.find("const TEMPLATE").or_else(|| source.find("r#\"")).unwrap_or(0);
        for line in source[start..].lines() {
            let selector = line.split('{').next().unwrap_or("");
            if !(selector.contains(":hover") || selector.contains(":active") || selector.contains(".hovering")) { continue; }
            if !line.contains('{') || line.contains("fn ") || line.contains("contains(") { continue; }
            let body = line.split_once('{').map(|(_, rest)| rest).unwrap_or("");
            for size in sizes {
                assert!(!body.split(';').any(|rule| rule.trim().starts_with(size)), "a size changes under the pointer: {line}");
            }
        }
        // And the desktop theme's own hover effects are switched off for every button.
        assert!(source.contains(".t4 button, .t4 switch, .t4 checkbutton, .t4 scale slider { margin: 0; transform: none;"));
    }
}
