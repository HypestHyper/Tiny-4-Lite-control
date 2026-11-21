//! The catalogue of camera settings.
//!
//! Each setting is one entry here: where it lives in the UI, how it reads and
//! writes, and what it depends on. The windows are generated from this list,
//! so adding or changing an option means editing one entry (plus a payload
//! function in `device.rs` for a new vendor command). Nothing in `ui/` knows
//! about individual settings.

use super::device::*;
use super::state::current_config;
use crate::config::AppConfig;
use std::sync::OnceLock;

// ── Layout ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Tab { Camera, Tracking, Audio, Device }

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Group {
    Exposure, WhiteBalance, Focus, Zoom, Picture,
    Tracking, Gestures,
    Microphone,
    Lighting, Power,
    /// Driver controls the app has no entry for.
    Other,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Exposure => "Exposure", Group::WhiteBalance => "White balance", Group::Focus => "Focus",
            Group::Zoom => "Zoom and field of view", Group::Picture => "Picture",
            Group::Tracking => "AI tracking", Group::Gestures => "Gestures",
            Group::Microphone => "Microphone", Group::Lighting => "Light", Group::Power => "Power",
            Group::Other => "More controls",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Group::Exposure => "exposure", Group::WhiteBalance => "wb_sunny", Group::Focus => "center_focus_strong",
            Group::Zoom => "zoom_in", Group::Picture => "palette",
            Group::Tracking => "person_search", Group::Gestures => "waving_hand",
            Group::Microphone => "mic", Group::Lighting => "lightbulb", Group::Power => "power_settings_new",
            Group::Other => "more_horiz",
        }
    }
    pub fn tab(self) -> Tab {
        match self {
            Group::Exposure | Group::WhiteBalance | Group::Focus | Group::Zoom | Group::Picture | Group::Other => Tab::Camera,
            Group::Tracking | Group::Gestures => Tab::Tracking,
            Group::Microphone => Tab::Audio,
            Group::Lighting | Group::Power => Tab::Device,
        }
    }
    pub const ALL: [Group; 11] = [
        Group::Exposure, Group::WhiteBalance, Group::Focus, Group::Zoom, Group::Picture,
        Group::Tracking, Group::Gestures, Group::Microphone, Group::Lighting, Group::Power, Group::Other,
    ];
}

// ── Values ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value { Bool(bool), Int(i64), Float(f64) }

impl Value {
    pub fn as_i64(self) -> i64 {
        match self { Value::Bool(b) => i64::from(b), Value::Int(i) => i, Value::Float(f) => f.round() as i64 }
    }
    pub fn as_bool(self) -> bool { self.as_i64() != 0 }
    pub fn as_f64(self) -> f64 {
        match self { Value::Bool(b) => f64::from(u8::from(b)), Value::Int(i) => i as f64, Value::Float(f) => f }
    }
}

/// Wording for a confirmation dialog.
#[derive(Clone, Copy, Debug)]
pub struct Confirm {
    pub title: &'static str,
    pub body: &'static str,
    pub action: &'static str,
}

#[derive(Clone, Debug)]
pub struct Opt {
    pub value: i64,
    pub label: &'static str,
    pub hint: &'static str,
    /// If set, changing to this option first asks the user to confirm.
    pub confirm: Option<Confirm>,
}

const fn opt(value: i64, label: &'static str, hint: &'static str) -> Opt {
    Opt { value, label, hint, confirm: None }
}

/// Number formats. `Iso` is here for drivers that report ISO sensitivity.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fmt { Plain, Shutter, Kelvin, Ev, Ratio, Level, Iso, Zoom }

impl Fmt {
    pub fn format(self, value: f64) -> String {
        match self {
            Fmt::Plain => format!("{value:.0}"),
            Fmt::Shutter => {
                // V4L2 exposure_time_absolute is in 100-microsecond units.
                let units = value.round() as i64;
                if units <= 0 { return "—".into(); }
                let denominator = (10_000.0 / units as f64).round() as i64;
                if denominator >= 2 { format!("1/{denominator} s") } else { format!("{:.2} s", units as f64 / 10_000.0) }
            }
            Fmt::Kelvin => format!("{value:.0} K"),
            Fmt::Ev => format!("{} EV", exposure_compensation_label(value.round().clamp(0.0, 18.0) as u8)),
            Fmt::Ratio => format!("{value:.2}×"),
            // The control runs 0 to 100; people see the zoom it gives, 1× to 4×.
            Fmt::Zoom => super::zoom::label(super::zoom::factor(value)),
            Fmt::Level => if value.round() as i64 == 0 { "Off".into() } else { format!("Level {:.0}", value) },
            Fmt::Iso => format!("ISO {value:.0}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SliderSpec {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: Option<f64>,
    pub fmt: Fmt,
    pub ticks: Vec<(f64, &'static str)>,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Toggle,
    /// A fixed list of choices (segmented control when short, menu when long).
    Choice(Vec<Opt>),
    /// Choices reported by the V4L2 driver.
    DriverMenu,
    Slider(SliderSpec),
}

// ── Dependencies ──────────────────────────────────────────────────────────

/// A dependency test. `IsNot` is available for entries that need it.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum Cond { Is(i64), IsNot(i64) }
pub const ON: Cond = Cond::Is(1);
pub const OFF: Cond = Cond::Is(0);

#[derive(Clone, Debug)]
pub struct Dep {
    /// Id of the setting this one depends on.
    pub on: &'static str,
    pub cond: Cond,
    /// Shown in the row while the dependency is not met: what to change.
    pub reason: &'static str,
}

/// Whether the camera has a way to report this setting at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Readback {
    /// The camera reports it. If it does not answer, the row shows Unknown.
    Camera,
    /// The camera cannot report it; the app can only show the last value it sent.
    LastSent,
}

/// Where a value shown to the user came from. Never present a saved value as
/// live camera state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Read back from the camera.
    Camera,
    /// The last value this app sent and saved. The camera has not confirmed it.
    Saved,
    /// Nothing is known.
    Unknown,
}

/// A value and where it came from; `None` means unknown.
pub type Reading = Option<(Value, Origin)>;

/// A value the camera reported.
pub fn from_camera(value: Option<Value>) -> Reading { value.map(|v| (v, Origin::Camera)) }
/// A value this app saved after sending it.
pub fn from_saved(value: Option<Value>) -> Reading { value.map(|v| (v, Origin::Saved)) }

// ── Snapshot ──────────────────────────────────────────────────────────────

/// Everything the UI needs to know about the camera right now. Read on the
/// worker thread, then handed to the UI as a plain value.
#[derive(Clone, Default)]
pub struct Snapshot {
    pub controls: Vec<Control>,
    pub status: Option<[u8; 60]>,
    pub exposure_comp: Option<u8>,
    /// Autofocus priority from CAM_GET_FACE_FOCUS; `None` if the camera did not answer.
    pub focus_priority: Option<FaceFocusPriority>,
    pub config: AppConfig,
    pub error: Option<String>,
}

/// Try a reading up to three times, a moment apart, and say why if it never works.
fn retry<T>(mut read: impl FnMut() -> Result<T, String>) -> Option<T> {
    let mut last = String::new();
    for attempt in 0..3 {
        match read() {
            Ok(value) => return Some(value),
            Err(error) => last = error,
        }
        if attempt < 2 && !ui_test_mode() { std::thread::sleep(std::time::Duration::from_millis(120)); }
    }
    eprintln!("Could not read the camera: {last}");
    None
}

impl Snapshot {
    pub fn read(device: &str) -> Snapshot {
        let mut snapshot = Snapshot { config: current_config(), ..Snapshot::default() };
        match run_v4l2(device, "--list-ctrls-menus") {
            Ok(text) => snapshot.controls = parse_controls(&text),
            Err(error) => snapshot.error = Some(error),
        }
        // The camera answers one question at a time and now and then misses one, so ask again before
        // giving up on a reading.
        snapshot.status = retry(|| xu_status(device));
        snapshot.exposure_comp = retry(|| get_exposure_compensation(device));
        snapshot.focus_priority = retry(|| get_face_focus(device));
        snapshot
    }

    pub fn control(&self, name: &str) -> Option<&Control> {
        self.controls.iter().find(|control| control.name == name)
    }

    pub fn byte(&self, offset: usize) -> Option<u8> {
        self.status.as_ref().and_then(|status| status.get(offset).copied())
    }
}

// ── Settings ──────────────────────────────────────────────────────────────

type Reader = fn(&Snapshot) -> Reading;
/// `Ok(Some(value))` is what the camera reports after the change; `Ok(None)` means
/// the command was sent but the camera cannot confirm it.
type Writer = fn(&str, Value) -> Result<Option<Value>, String>;

#[derive(Clone)]
pub enum Source {
    /// A standard V4L2 control, by name. Range and menu come from the driver.
    V4l2(&'static str),
    Custom { read: Reader, write: Writer },
}

#[derive(Clone)]
pub struct Setting {
    pub id: &'static str,
    pub group: Group,
    pub label: &'static str,
    /// One line under the label.
    pub summary: &'static str,
    /// Longer explanation for the info dialog.
    pub info: &'static str,
    pub kind: Kind,
    pub deps: Vec<Dep>,
    /// Eligible for the slim window.
    pub quick: bool,
    pub readback: Readback,
    /// Shown as experimental in the info dialog.
    pub experimental: bool,
    pub source: Source,
}

impl Setting {
    fn new(id: &'static str, group: Group, label: &'static str, summary: &'static str, kind: Kind, source: Source) -> Setting {
        Setting {
            id, group, label, summary, info: "", kind, deps: Vec::new(), quick: true,
            readback: Readback::Camera, experimental: false, source,
        }
    }
    fn v4l2(name: &'static str, id: &'static str, group: Group, label: &'static str, summary: &'static str, kind: Kind) -> Setting {
        Setting::new(id, group, label, summary, kind, Source::V4l2(name))
    }
    fn custom(id: &'static str, group: Group, label: &'static str, summary: &'static str, kind: Kind, read: Reader, write: Writer) -> Setting {
        Setting::new(id, group, label, summary, kind, Source::Custom { read, write })
    }
    fn info(mut self, text: &'static str) -> Setting { self.info = text; self }
    fn needs(mut self, on: &'static str, cond: Cond, reason: &'static str) -> Setting {
        self.deps.push(Dep { on, cond, reason });
        self
    }
    fn last_sent(mut self) -> Setting { self.readback = Readback::LastSent; self }
    fn experimental(mut self) -> Setting { self.experimental = true; self }
    fn not_quick(mut self) -> Setting { self.quick = false; self }

    // Reading

    /// The value and where it came from.
    pub fn reading(&self, snapshot: &Snapshot) -> Reading {
        match &self.source {
            Source::V4l2(name) => {
                let control = snapshot.control(name)?;
                from_camera(Some(match self.kind {
                    Kind::Toggle => Value::Bool(control.value != 0),
                    _ => Value::Int(control.value),
                }))
            }
            Source::Custom { read, .. } => read(snapshot),
        }
    }

    /// The value to show, from the camera or saved.
    pub fn read(&self, snapshot: &Snapshot) -> Option<Value> {
        self.reading(snapshot).map(|(value, _)| value)
    }

    pub fn origin(&self, snapshot: &Snapshot) -> Origin {
        self.reading(snapshot).map_or(Origin::Unknown, |(_, origin)| origin)
    }

    /// Send a value. See `Writer` for what the result means.
    pub fn write(&self, device: &str, value: Value) -> Result<Option<Value>, String> {
        match &self.source {
            Source::V4l2(name) => set_control(device, name, value.as_i64()).map(|()| None),
            Source::Custom { write, .. } => write(device, value),
        }
    }

    /// Whether the camera offers this setting at all. Standard controls are
    /// discovered; vendor commands are assumed present on this model.
    pub fn present(&self, snapshot: &Snapshot) -> bool {
        match &self.source {
            Source::V4l2(name) => snapshot.control(name).is_some(),
            Source::Custom { .. } => true,
        }
    }

    pub fn driver_control<'a>(&self, snapshot: &'a Snapshot) -> Option<&'a Control> {
        match &self.source { Source::V4l2(name) => snapshot.control(name), Source::Custom { .. } => None }
    }

    /// Why the setting cannot be changed right now. Empty means it can.
    pub fn blockers(&self, snapshot: &Snapshot) -> Vec<String> {
        let mut blockers: Vec<String> = self.deps.iter()
            .filter(|dep| !dep_holds(dep, snapshot))
            .map(|dep| dep.reason.to_string())
            .collect();
        if let Some(control) = self.driver_control(snapshot) {
            if blockers.is_empty() {
                let numeric = !matches!(self.kind, Kind::Toggle | Kind::DriverMenu);
                if numeric && (control.value < control.min || control.value > control.max.max(control.min)) {
                    blockers.push(format!(
                        "The camera reports {} outside its own range of {} to {}, so this control is switched off.",
                        control.value, control.min, control.max
                    ));
                } else if control.disabled {
                    blockers.push("The camera driver reports this control as inactive right now.".into());
                }
            }
        }
        if snapshot.error.is_some() && blockers.is_empty() && snapshot.controls.is_empty() {
            blockers.push("The camera is not connected.".into());
        }
        blockers
    }

    pub fn slider(&self, snapshot: &Snapshot) -> Option<SliderSpec> {
        let Kind::Slider(spec) = &self.kind else { return None };
        let mut spec = spec.clone();
        if let Some(control) = self.driver_control(snapshot) {
            spec.min = control.min as f64;
            spec.max = control.max.max(control.min) as f64;
            spec.step = control.step.max(1) as f64;
            spec.default = Some(control.default_value as f64);
        }
        Some(spec)
    }

    pub fn options(&self, snapshot: &Snapshot) -> Vec<Opt> {
        match &self.kind {
            Kind::Choice(options) => options.clone(),
            Kind::DriverMenu => self.driver_control(snapshot).map(|control| {
                control.menu.iter().map(|(value, label)| Opt {
                    value: *value,
                    label: Box::leak(label.clone().into_boxed_str()),
                    hint: "",
                    confirm: None,
                }).collect()
            }).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub fn format(&self, snapshot: &Snapshot, value: f64) -> String {
        match &self.kind {
            Kind::Slider(spec) => spec.fmt.format(value),
            _ => { let _ = snapshot; format!("{value:.0}") }
        }
    }
}

fn dep_holds(dep: &Dep, snapshot: &Snapshot) -> bool {
    let Some(setting) = find(dep.on) else { return true };
    // Unknown state never blocks: only block on what the camera reports.
    let Some(value) = setting.read(snapshot) else { return true };
    match dep.cond {
        Cond::Is(expected) => value.as_i64() == expected,
        Cond::IsNot(other) => value.as_i64() != other,
    }
}

/// The zoom preset the user pressed, or -1. It stays lit only while the zoom is still there.
static PRESSED_PRESET: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

/// Which preset to light for this zoom position: the pressed one if the zoom is still on it, else none.
fn lit_preset(zoom: i64) -> i64 {
    let pressed = PRESSED_PRESET.load(std::sync::atomic::Ordering::Relaxed);
    if pressed >= 0 && pressed == zoom { return pressed; }
    PRESSED_PRESET.store(-1, std::sync::atomic::Ordering::Relaxed);
    -1
}

/// The dependency map as Markdown: what each setting needs before it can be changed.
pub fn dependency_map() -> String {
    let say = |dep: &Dep| -> String {
        let target = find(dep.on).expect("a dependency points at a real setting");
        let (value, is) = match dep.cond { Cond::Is(v) => (v, "is"), Cond::IsNot(v) => (v, "is not") };
        let state = match &target.kind {
            Kind::Toggle => if value == 1 { "on" } else { "off" }.to_string(),
            Kind::Choice(options) => options.iter().find(|o| o.value == value).map(|o| o.label.to_string()).unwrap_or_else(|| value.to_string()),
            // The driver's own menu: the exposure modes are the only ones used here.
            _ => match (dep.on, value) { ("v4l2:auto_exposure", 0) => "Auto".into(), ("v4l2:auto_exposure", 1) => "Manual".into(), _ => format!("menu item {value}") },
        };
        format!("{} {is} {state}", target.label)
    };
    let mut out = String::from("# Dependencies\n\nGenerated by `tiny4lite-control deps`. A setting is switched off, with the reason shown in its row, while its condition is not met. When the camera has not reported the setting it depends on, nothing is blocked.\n");
    for group in Group::ALL {
        let rows: Vec<&Setting> = all().iter().filter(|s| s.group == group && !s.deps.is_empty()).collect();
        if rows.is_empty() { continue; }
        out.push_str(&format!("\n## {}\n\n| Setting | Needs | Shown when blocked |\n|---|---|---|\n", group.title()));
        for setting in rows {
            let needs: Vec<String> = setting.deps.iter().map(|d| say(d)).collect();
            let reasons: Vec<&str> = setting.deps.iter().map(|d| d.reason).collect();
            out.push_str(&format!("| {} (`{}`) | {} | {} |\n", setting.label, setting.id, needs.join(" and "), reasons.join(" ")));
        }
    }
    out.push_str("\n## Every numeric setting\n\nA control is also switched off when the camera driver reports it as inactive, or reports a value outside its own range. Sliders take their minimum, maximum and step from the driver; Reset returns to the driver's default (for the colour balance, hue and exposure compensation that is the middle of the range).\n");
    out
}

// ── Catalogue ─────────────────────────────────────────────────────────────

fn slider(min: f64, max: f64, step: f64, fmt: Fmt) -> Kind {
    Kind::Slider(SliderSpec { min, max, step, default: None, fmt, ticks: Vec::new() })
}

/// A slider that rests at `value` when reset (the middle, for centred ones).
fn resting_at(kind: Kind, value: f64) -> Kind {
    match kind { Kind::Slider(spec) => Kind::Slider(SliderSpec { default: Some(value), ..spec }), other => other }
}

fn ticked(min: f64, max: f64, step: f64, fmt: Fmt, ticks: Vec<(f64, &'static str)>) -> Kind {
    Kind::Slider(SliderSpec { min, max, step, default: None, fmt, ticks })
}

fn flag(value: Option<bool>) -> Option<Value> { value.map(Value::Bool) }
fn number(value: Option<u8>) -> Option<Value> { value.map(|v| Value::Int(i64::from(v))) }

/// The seven AI tracking modes offered, as (saved name, mode byte, sub-mode byte). Each was set and read
/// back exactly from status offsets 0x18 and 0x1C (reference/15). Do not add modes without
/// new hardware evidence.
const TRACKING_MODES: [(&str, u8, u8); 7] = [
    ("off", 0x00, 0x00), ("normal", 0x02, 0x00), ("upper", 0x02, 0x01), ("close", 0x02, 0x02),
    ("headless", 0x02, 0x03), ("lower", 0x02, 0x04), ("group", 0x01, 0x00),
];
const GESTURE_TARGET: [u8; 6] = [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00];
const GESTURE_ZOOM: [u8; 6] = [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00];
const GESTURE_DYNAMIC: [u8; 6] = [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00];
const GESTURE_DIRECTION: [u8; 6] = [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00];

fn gesture_saved(snapshot: &Snapshot, key: &str) -> Reading {
    from_saved(flag(snapshot.config.gesture_states.get(key).copied()))
}

/// Wrap a sender that has no read-back.
fn sent(result: Result<(), String>) -> Result<Option<Value>, String> { result.map(|()| None) }

fn build() -> Vec<Setting> {
    use Group::*;
    let af = "v4l2:focus_automatic_continuous";
    let awb = "v4l2:white_balance_automatic";
    let ae = "v4l2:auto_exposure";
    vec![
        // ── Exposure ──
        Setting::v4l2("auto_exposure", ae, Exposure, "Exposure mode",
            "Let the camera choose exposure, or set the shutter yourself.", Kind::DriverMenu)
            .info("Auto: the camera sets exposure on its own. Manual: you set the shutter speed, and exposure compensation and metering priority switch off."),
        Setting::custom("obsbot:auto_exposure_mode", Exposure, "Metering priority",
            "Which part of the picture automatic exposure aims at.",
            Kind::Choice(vec![opt(0, "Global", "Balance the whole frame"), opt(1, "Face", "Favour a detected face")]),
            |s| from_camera(s.status.as_ref().and_then(auto_exposure_mode_from_status).map(|face| Value::Int(i64::from(face))))
                // Only if the camera could not be read at all, fall back to what this app last sent.
                .or_else(|| if s.status.is_none() {
                    from_saved(s.config.auto_exposure_mode.as_deref().and_then(|m| match m { "face" => Some(Value::Int(1)), "global" => Some(Value::Int(0)), _ => None }))
                } else { None }),
            |d, v| set_auto_exposure_mode(d, v.as_i64() == 1).map(|actual| Some(Value::Int(i64::from(actual)))))
            .needs(ae, Cond::Is(0), "Set Exposure mode to Auto to choose a metering priority.")
            .info("Face: exposure follows a face the camera finds. Global: exposure follows the whole picture. Works while Exposure mode is Auto, and is separate from autofocus priority."),
        Setting::custom("obsbot:exposure_compensation", Exposure, "Exposure compensation",
            "Make automatic exposure brighter or darker.",
            resting_at(ticked(0.0, 18.0, 1.0, Fmt::Ev, vec![(0.0, "−3"), (9.0, "0"), (18.0, "+3")]), 9.0),
            |s| from_camera(number(s.exposure_comp))
                .or_else(|| if s.exposure_comp.is_none() { from_saved(number(s.config.exposure_compensation_position)) } else { None }),
            |d, v| set_exposure_compensation(d, v.as_i64().clamp(0, 18) as u8).map(|actual| Some(Value::Int(i64::from(actual)))))
            .needs(ae, Cond::Is(0), "Set Exposure mode to Auto to use exposure compensation.")
            .info("Makes automatic exposure brighter or darker, in steps of a third of a stop from −3 to +3 EV. Works while Exposure mode is Auto."),
        Setting::v4l2("exposure_time_absolute", "v4l2:exposure_time_absolute", Exposure, "Shutter speed",
            "How long each frame is exposed.", slider(1.0, 2500.0, 1.0, Fmt::Shutter))
            .needs(ae, Cond::Is(1), "Set Exposure mode to Manual to choose a shutter speed.")
            .info("Shorter times freeze motion and darken the picture. Very long times need a frame rate low enough to fit them."),
        Setting::v4l2("gain", "v4l2:gain", Exposure, "Sensor gain",
            "Brightens the picture, with more noise.", slider(1.0, 64.0, 1.0, Fmt::Plain)),
        Setting::v4l2("power_line_frequency", "v4l2:power_line_frequency", Exposure, "Anti-flicker",
            "Match your mains frequency to remove banding under artificial light.", Kind::DriverMenu)
            .info("Use 50 Hz in most of Europe, Asia and Africa, and 60 Hz in the Americas."),

        // ── White balance ──
        Setting::v4l2("white_balance_automatic", awb, WhiteBalance, "Auto white balance",
            "Let the camera correct for the colour of your lighting.", Kind::Toggle),
        Setting::v4l2("white_balance_temperature", "v4l2:white_balance_temperature", WhiteBalance, "Colour temperature",
            "Lower is warmer, higher is cooler.", slider(2000.0, 10000.0, 100.0, Fmt::Kelvin))
            .needs(awb, OFF, "Turn off Auto white balance to set the colour temperature."),
        Setting::v4l2("red_balance", "v4l2:red_balance", WhiteBalance, "Red balance",
            "Fine-tune the red channel.", ticked(0.0, 2048.0, 1.0, Fmt::Plain, vec![(1024.0, "●")]))
            .needs(awb, OFF, "Turn off Auto white balance to adjust red."),
        Setting::v4l2("blue_balance", "v4l2:blue_balance", WhiteBalance, "Blue balance",
            "Fine-tune the blue channel.", ticked(0.0, 2048.0, 1.0, Fmt::Plain, vec![(1024.0, "●")]))
            .needs(awb, OFF, "Turn off Auto white balance to adjust blue."),

        // ── Focus ──
        Setting::v4l2("focus_automatic_continuous", af, Focus, "Autofocus",
            "Keep the picture sharp as things move.", Kind::Toggle),
        Setting::custom("obsbot:focus_mode", Focus, "Autofocus priority",
            "What autofocus locks on to.",
            Kind::Choice(vec![opt(0, "Global", "Focus on whatever is in frame"), opt(1, "Face", "Favour a detected face")]),
            // The camera is the only source. A saved value is never shown as the camera's state.
            |s| from_camera(s.focus_priority.map(|p| Value::Int(i64::from(p == FaceFocusPriority::Face)))),
            |d, v| {
                let wanted = if v.as_i64() == 1 { FaceFocusPriority::Face } else { FaceFocusPriority::Global };
                set_face_focus(d, wanted).map(|actual| Some(Value::Int(i64::from(actual == FaceFocusPriority::Face))))
            })
            .needs(af, ON, "Turn on Autofocus to choose its priority.")
            .info("Face: focus favours a face the camera finds. Global: focus favours the whole scene. Works while Autofocus is on, and is separate from metering priority. If the camera does not answer, this shows Unknown."),
        Setting::v4l2("focus_absolute", "v4l2:focus_absolute", Focus, "Manual focus",
            "Set the focus distance yourself.", slider(0.0, 100.0, 1.0, Fmt::Plain))
            .needs(af, OFF, "Turn off Autofocus to focus by hand."),

        // ── Zoom ──
        Setting::v4l2("zoom_absolute", "v4l2:zoom_absolute", Zoom, "Zoom",
            "Smoothly narrow the view.", ticked(0.0, 100.0, 1.0, Fmt::Zoom, vec![(0.0, "1×"), (15.0, "1.5×"), (100.0, "4×")]))
            .info("Zooms in smoothly, from 1× up to 4×."),
        Setting::custom("obsbot:fov", Zoom, "Field of view",
            "Jump to a common angle.",
            Kind::Choice(vec![opt(0, "Wide · 67°", "The widest view"), opt(5, "Normal · 60°", "A little closer"), opt(15, "Narrow · 48°", "Closer still")]),
            // A preset is lit only after it is pressed, and only while the zoom stays there, so sliding the
            // zoom through a preset does not make the buttons flicker.
            |s| from_camera(s.control("zoom_absolute").map(|c| Value::Int(lit_preset(c.value)))),
            |d, v| { PRESSED_PRESET.store(v.as_i64(), std::sync::atomic::Ordering::Relaxed); sent(set_control(d, "zoom_absolute", v.as_i64())) })
            .info("Jumps to one of three common zoom levels: 1×, 1.15× and 1.47×. The angles are the approximate horizontal field of view at each."),

        // ── Picture ──
        Setting::v4l2("brightness", "v4l2:brightness", Picture, "Brightness", "Lighten or darken the whole picture.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::v4l2("contrast", "v4l2:contrast", Picture, "Contrast", "Separate light from dark.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::v4l2("saturation", "v4l2:saturation", Picture, "Saturation", "How vivid colours look.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::v4l2("hue", "v4l2:hue", Picture, "Hue", "Rotate all colours around the colour wheel.", ticked(0.0, 100.0, 1.0, Fmt::Plain, vec![(50.0, "●")])),
        Setting::v4l2("sharpness", "v4l2:sharpness", Picture, "Sharpness", "Edge enhancement.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::custom("obsbot:mirror_image", Picture, "Mirror image",
            "Flip the picture left to right.", Kind::Toggle,
            |s| from_saved(flag(s.config.mirror_image_enabled)),
            |d, v| sent(set_mirror_image(d, v.as_bool())))
            .last_sent()
            .info("Flips the picture left to right. The camera can't report this setting, so the switch shows what you last chose here."),

        // ── Tracking ──
        Setting::custom("obsbot:ai_tracking_mode", Tracking, "Tracking mode",
            "How the camera follows and frames a person.",
            Kind::Choice(vec![
                opt(0, "No tracking", "The camera stays where you point it"),
                opt(1, "Normal", "Follow one person"),
                opt(2, "Upper body", "Frame head and torso"),
                opt(3, "Close-up", "Frame tightly on the face"),
                opt(4, "Headless", "Frame the body below the head"),
                opt(5, "Lower body", "Frame from the waist down"),
                opt(6, "Group", "Keep several people in frame"),
            ]),
            |s| match (s.byte(0x18), s.byte(0x1c)) {
                // The camera answered: an unrecognised pair is Unknown, never a guess.
                (Some(m), Some(n)) => from_camera(TRACKING_MODES.iter().position(|e| e.1 == m && e.2 == n).map(|i| Value::Int(i as i64))),
                _ => from_saved(s.config.ai_tracking_mode.as_deref()
                    .and_then(|saved| TRACKING_MODES.iter().position(|e| e.0 == saved))
                    .map(|i| Value::Int(i as i64))),
            },
            |d, v| {
                let entry = TRACKING_MODES.get(v.as_i64() as usize).ok_or("Unknown tracking mode.")?;
                // set_ai_mode only succeeds when the camera's status matches the request exactly.
                set_ai_mode(d, (entry.1, entry.2)).map(|()| Some(v))
            })
            .info("How the camera frames the person it follows. Tracking moves the gimbal by itself, so clear the space around the camera first. Target selection is a separate hand-gesture setting."),
        Setting::custom("obsbot:tracking_speed", Tracking, "Tracking speed",
            "How quickly the camera follows movement.",
            Kind::Choice(vec![opt(0, "Standard", "Smooth and steady"), opt(1, "Sport", "Faster, for quick movement")]),
            |s| from_camera(s.byte(0x24).map(|b| Value::Int(i64::from(b != 0))))
                .or_else(|| if s.status.is_none() { from_saved(s.config.tracking_speed.as_deref().map(|v| Value::Int(i64::from(v == "sport")))) } else { None }),
            |d, v| sent(set_tracking_speed(d, v.as_i64() == 1))),

        // ── Gestures ──
        Setting::custom("obsbot:gesture_target", Gestures, "Target selection",
            "Pick who to follow with a hand gesture.", Kind::Toggle,
            |s| from_camera(s.byte(0x01).and_then(|b| match b { 0 => Some(Value::Bool(false)), 1 => Some(Value::Bool(true)), _ => None }))
                .or_else(|| if s.byte(0x01).is_none() { gesture_saved(s, "target_selection") } else { None }),
            |d, v| sent(set_gesture(d, GESTURE_TARGET, v.as_bool())))
            .info("Lets you choose who to follow with a hand gesture. It is a gesture setting, not a tracking mode."),
        Setting::custom("obsbot:gesture_zoom", Gestures, "Zoom gesture",
            "Zoom in and out with an L-shaped hand pose.", Kind::Toggle,
            |s| gesture_saved(s, "zoom_l_pose"),
            |d, v| sent(set_gesture(d, GESTURE_ZOOM, v.as_bool())))
            .last_sent()
            .info("The camera can't report this setting, so the switch shows what you last chose here, or Unknown if you haven't chosen yet."),
        Setting::custom("obsbot:gesture_zoom_ratio", Gestures, "Gesture zoom amount",
            "How far the zoom gesture zooms in.",
            slider(1.0, 4.0, 0.01, Fmt::Ratio),
            |s| from_saved(s.config.gesture_zoom_ratio.map(|r| Value::Float(f64::from(r)))),
            |d, v| sent(set_gesture_zoom_ratio(d, v.as_f64() as f32)))
            .last_sent().not_quick(),
        Setting::custom("obsbot:gesture_dynamic_zoom", Gestures, "Dynamic zoom",
            "Zoom by moving your hand towards or away from the camera.", Kind::Toggle,
            |s| gesture_saved(s, "dynamic_zoom"),
            |d, v| sent(set_gesture(d, GESTURE_DYNAMIC, v.as_bool())))
            .last_sent()
            .info("The camera can't report this setting, so the switch shows what you last chose here, or Unknown if you haven't chosen yet."),
        Setting::custom("obsbot:gesture_dynamic_zoom_direction", Gestures, "Reverse dynamic zoom",
            "Swap which hand movement zooms in.", Kind::Toggle,
            |s| gesture_saved(s, "dynamic_zoom_direction"),
            |d, v| sent(set_gesture(d, GESTURE_DIRECTION, v.as_bool())))
            .last_sent()
            .info("Swaps which hand movement zooms in. The camera can't report this setting, so the switch shows what you last chose here, or Unknown if you haven't chosen yet."),

        // ── Microphone ──
        Setting::custom("obsbot:mic_enabled", Microphone, "Microphone",
            "On: the microphones work. Off: they are muted.", Kind::Toggle,
            |s| from_saved(flag(s.config.audio_microphone_disabled.map(|disabled| !disabled))),
            |d, v| sent(set_microphone_disabled(d, !v.as_bool())))
            .last_sent()
            .info("The camera may restart its audio when this changes, so apps using the microphone can drop out for a moment. The camera can't report this setting, so the switch shows what you last chose here."),
        Setting::custom("obsbot:mic_during_sleep", Microphone, "Listen while asleep",
            "Keep the microphone running when the camera is asleep.", Kind::Toggle,
            |s| from_camera(s.byte(16).and_then(|b| match b { 0 => Some(Value::Bool(false)), 1 => Some(Value::Bool(true)), _ => None }))
                .or_else(|| if s.byte(16).is_none() { from_saved(flag(s.config.audio_microphone_during_sleep)) } else { None }),
            |d, v| sent(set_microphone_during_sleep(d, v.as_bool()))),
        Setting::custom("obsbot:auto_gain", Microphone, "Auto gain",
            "Keep your voice at a steady level.", Kind::Toggle,
            |s| from_camera(s.byte(25).filter(|b| *b <= 1).map(|b| Value::Bool(b != 0)))
                .or_else(|| if s.byte(25).is_none() { from_saved(flag(s.config.audio_auto_gain)) } else { None }),
            |d, v| sent(set_auto_gain(d, v.as_bool()))),
        Setting::custom("obsbot:noise_reduction", Microphone, "Noise reduction",
            "How strongly background noise is removed.",
            Kind::Choice(vec![opt(0, "Off", ""), opt(1, "Weak", ""), opt(2, "Medium", ""), opt(3, "Strong", "")]),
            |s| from_camera(s.byte(8).filter(|b| *b <= 3).map(|b| Value::Int(i64::from(b))))
                .or_else(|| if s.byte(8).is_none() { from_saved(number(s.config.audio_noise_reduction)) } else { None }),
            |d, v| sent(set_noise_reduction(d, v.as_i64().clamp(0, 3) as u8))),
        Setting::custom("obsbot:radio_distance", Microphone, "Pickup distance",
            "How far away the voice you want is.",
            Kind::Choice(vec![opt(0, "Close", "About arm's length"), opt(1, "Standard", ""), opt(2, "Far", "Across a room")]),
            |s| from_saved(number(s.config.audio_radio_distance.filter(|d| *d <= 2))),
            |d, v| sent(set_radio_distance(d, v.as_i64().clamp(0, 2) as u8)))
            .last_sent(),
        Setting::custom("obsbot:pop_filter", Microphone, "Pop filter",
            "Soften plosive sounds such as “p” and “b”.", Kind::Toggle,
            |s| from_saved(flag(s.config.pop_filter_enabled)),
            |d, v| sent(set_pop_filter(d, v.as_bool())))
            .last_sent().experimental()
            .info("Softens harsh “p” and “b” sounds. The camera can't report this setting, so the switch shows what you last chose here."),

        // ── Light ──
        Setting::custom("obsbot:led_brightness", Lighting, "LED brightness",
            "The light on the front. Off, or one of three levels.",
            Kind::Choice(vec![
                Opt { value: 0, label: "Off", hint: "No light at all", confirm: Some(Confirm {
                    title: "Turn the camera light off?",
                    body: "With the light off, nobody near the camera can tell when it is on. Leave it on if other people could be in view or earshot.",
                    action: "Turn off",
                }) },
                opt(1, "1", "Dimmest"), opt(2, "2", "Medium"), opt(3, "3", "Brightest"),
            ]),
            |s| from_camera(s.byte(0x21).map(|b| Value::Int(i64::from(b.min(3)))))
                .or_else(|| if s.status.is_none() { from_saved(number(s.config.led_brightness)) } else { None }),
            |d, v| sent(set_led_brightness(d, v.as_i64().clamp(0, 3) as u8))),

        // ── Power ──
        Setting::custom("obsbot:sleep", Power, "Camera power",
            "Put the camera to sleep, or wake it.",
            Kind::Choice(vec![
                opt(0, "Wake", ""),
                Opt { value: 1, label: "Sleep", hint: "", confirm: Some(Confirm {
                    title: "Put the camera to sleep?",
                    body: "The gimbal will move to its resting position. Clear the space around the camera first.",
                    action: "Sleep",
                }) },
            ]),
            |s| from_camera(s.byte(0x02).and_then(|b| match b { 0 => Some(Value::Int(0)), 1 => Some(Value::Int(1)), _ => None })),
            |d, v| sent(set_sleep_mode(d, v.as_i64() == 1)))
            .not_quick()
            .info("Sleep and wake physically move the gimbal. The state can take a moment to show after the command."),
    ]
}

pub fn all() -> &'static [Setting] {
    static ALL: OnceLock<Vec<Setting>> = OnceLock::new();
    ALL.get_or_init(build)
}

pub fn find(id: &str) -> Option<&'static Setting> {
    all().iter().find(|setting| setting.id == id)
}

/// V4L2 controls the app drives some other way, or deliberately does not offer.
const IGNORED_V4L2: &[&str] = &[
    "pan_absolute", "tilt_absolute", "pan_speed", "tilt_speed", "zoom_continuous",
    // The same hardware control as Exposure compensation (UVC Backlight Compensation, 0 to 18).
    "backlight_compensation",
];

/// Settings for driver controls that have no entry above, so a new firmware
/// or driver control shows up without a code change.
pub fn discovered(snapshot: &Snapshot) -> Vec<Setting> {
    let known: Vec<&str> = all().iter().filter_map(|s| match s.source { Source::V4l2(n) => Some(n), _ => None }).collect();
    snapshot.controls.iter()
        .filter(|c| !known.contains(&c.name.as_str()) && !IGNORED_V4L2.contains(&c.name.as_str()) && c.kind != "button")
        .map(|control| {
            let name: &'static str = Box::leak(control.name.clone().into_boxed_str());
            let id: &'static str = Box::leak(format!("v4l2:{}", control.name).into_boxed_str());
            let label: &'static str = Box::leak(sentence_case(&control.name).into_boxed_str());
            let kind = if control.kind.contains("menu") { Kind::DriverMenu }
                else if control.kind == "bool" { Kind::Toggle }
                else { slider(control.min as f64, control.max as f64, control.step as f64, Fmt::Plain) };
            Setting::v4l2(name, id, Group::Other, label, "Reported by the camera driver.", kind)
        })
        .collect()
}

fn sentence_case(name: &str) -> String {
    let spaced = name.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn control(name: &str, value: i64, disabled: bool) -> Control {
        Control { name: name.into(), kind: "int".into(), min: 0, max: 10_000, step: 1, value, default_value: 0, disabled, menu: Vec::new() }
    }

    fn snapshot(controls: Vec<Control>) -> Snapshot { Snapshot { controls, ..Snapshot::default() } }

    #[test]
    fn ids_are_unique_and_dependencies_point_at_real_settings() {
        let mut seen = HashSet::new();
        for setting in all() {
            assert!(seen.insert(setting.id), "duplicate id {}", setting.id);
            for dep in &setting.deps {
                assert!(find(dep.on).is_some(), "{} depends on unknown {}", setting.id, dep.on);
                assert_ne!(dep.on, setting.id);
            }
        }
    }

    /// What the camera reports, as the mock camera (shaped like the real one) describes it.
    fn mock_snapshot() -> Snapshot {
        Snapshot { controls: parse_controls(&crate::backend::mock_v4l2_output("--list-ctrls-menus")), ..Snapshot::default() }
    }

    #[test]
    fn every_slider_has_a_sane_range_step_centre_and_ticks() {
        let snap = mock_snapshot();
        let mut checked = 0;
        for setting in all() {
            let Some(spec) = setting.slider(&snap) else { continue };
            checked += 1;
            let id = setting.id;
            assert!(spec.min < spec.max, "{id}: range {}..{}", spec.min, spec.max);
            assert!(spec.step > 0.0 && spec.step <= spec.max - spec.min, "{id}: step {}", spec.step);
            let on_step = |from: f64| { let n = (from - spec.min) / spec.step; (n - n.round()).abs() < 1e-6 };
            assert!(on_step(spec.max), "{id}: the range is not a whole number of steps");
            if let Some(default) = spec.default {
                assert!((spec.min..=spec.max).contains(&default), "{id}: default {default} outside {}..{}", spec.min, spec.max);
                assert!(on_step(default), "{id}: default is not on a step");
            }
            let mut last = f64::MIN;
            for (at, label) in &spec.ticks {
                assert!((spec.min..=spec.max).contains(at) && *at > last, "{id}: tick {label} at {at}");
                last = *at;
            }
            // The text for the ends and the middle never panics or comes out empty.
            for value in [spec.min, (spec.min + spec.max) / 2.0, spec.max] {
                assert!(!spec.fmt.format(value).is_empty(), "{id}: nothing to show at {value}");
            }
        }
        assert!(checked >= 12, "only {checked} sliders were checked");
    }

    /// A balance, hue or compensation slider should rest in the middle of its range, so its
    /// "reset" and its centre tick agree.
    #[test]
    fn centred_sliders_reset_to_their_middle() {
        let snap = mock_snapshot();
        for id in ["v4l2:hue", "v4l2:red_balance", "v4l2:blue_balance", "obsbot:exposure_compensation"] {
            let setting = find(id).unwrap();
            let spec = setting.slider(&snap).unwrap_or_else(|| panic!("{id} is not a slider"));
            let middle = (spec.min + spec.max) / 2.0;
            assert_eq!(spec.default, Some(middle), "{id}: resets to {:?}, the middle is {middle}", spec.default);
            // The middle is marked under the bar, so it is easy to find.
            assert!(spec.ticks.iter().any(|(at, label)| *at == middle && !label.is_empty()), "{id}: no label at the middle {middle}: {:?}", spec.ticks);
        }
    }

    #[test]
    fn a_zoom_preset_lights_only_after_it_is_pressed_and_only_while_the_zoom_is_there() {
        let at = |zoom: i64| {
            let snap = snapshot(vec![{ let mut c = control("zoom_absolute", zoom, false); c.max = 100; c }]);
            find("obsbot:fov").unwrap().reading(&snap).map(|(v, _)| v.as_i64())
        };
        PRESSED_PRESET.store(-1, std::sync::atomic::Ordering::Relaxed);
        // Sliding through the presets lights nothing, and is never "unknown".
        for zoom in [0, 3, 5, 9, 15, 40] { assert_eq!(at(zoom), Some(-1), "zoom {zoom}"); }
        // Pressing 5 lights it while the zoom is on 5.
        PRESSED_PRESET.store(5, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(at(5), Some(5));
        // Moving away puts it out, and coming back by hand does not light it again.
        assert_eq!(at(6), Some(-1));
        assert_eq!(at(5), Some(-1));
    }

    /// What a person reads in an info button must not be development notes.
    #[test]
    fn info_text_is_for_people_not_developers() {
        let banned = ["byte", "0x", "selector", "captured", "OBSBOT Center", "CAM_", "read-back", "readback", "status block", "firmware", "V4L2", "driver", "replay"];
        for setting in all() {
            for text in [setting.info, setting.summary] {
                for word in banned {
                    assert!(!text.to_lowercase().contains(&word.to_lowercase()), "{}: \"{}\" contains `{word}`", setting.id, text);
                }
            }
        }
    }

    /// docs/dependencies.md is generated: `tiny4lite-control deps > docs/dependencies.md`.
    #[test]
    fn the_dependency_document_is_up_to_date() {
        assert_eq!(include_str!("../../docs/dependencies.md"), dependency_map(), "regenerate docs/dependencies.md");
    }

    #[test]
    fn dependencies_use_values_their_target_can_hold_and_never_loop() {
        let snap = mock_snapshot();
        for setting in all() {
            for dep in &setting.deps {
                let target = find(dep.on).unwrap();
                let value = match dep.cond { Cond::Is(v) | Cond::IsNot(v) => v };
                match &target.kind {
                    Kind::Toggle => assert!(matches!(value, 0 | 1), "{} needs {} to be {value}", setting.id, dep.on),
                    Kind::Choice(options) => assert!(options.iter().any(|o| o.value == value), "{} needs {} to be {value}, which is not an option", setting.id, dep.on),
                    Kind::DriverMenu => assert!(target.options(&snap).iter().any(|o| o.value == value), "{} needs {} to be {value}, which the driver does not offer", setting.id, dep.on),
                    Kind::Slider(_) => panic!("{} depends on a slider", setting.id),
                }
                assert!(!dep.reason.is_empty());
            }
            // Walk the chain: it must end.
            let (mut at, mut steps) = (setting, 0);
            while let Some(next) = at.deps.first().and_then(|d| find(d.on)) {
                at = next; steps += 1;
                assert!(steps < 8 && next.id != setting.id, "{} depends on itself through a loop", setting.id);
            }
        }
    }

    #[test]
    fn manual_white_balance_needs_auto_white_balance_off() {
        let on = snapshot(vec![control("white_balance_automatic", 1, false), control("red_balance", 1043, true)]);
        let setting = find("v4l2:red_balance").unwrap();
        assert_eq!(setting.blockers(&on), vec!["Turn off Auto white balance to adjust red.".to_string()]);
        let off = snapshot(vec![control("white_balance_automatic", 0, false), control("red_balance", 1043, false)]);
        assert!(setting.blockers(&off).is_empty());
    }

    #[test]
    fn shutter_needs_manual_exposure_and_compensation_needs_auto() {
        let auto = snapshot(vec![control("auto_exposure", 0, false), control("exposure_time_absolute", 330, true)]);
        assert_eq!(find("v4l2:exposure_time_absolute").unwrap().blockers(&auto).len(), 1);
        assert!(find("obsbot:exposure_compensation").unwrap().blockers(&auto).is_empty());
        let manual = snapshot(vec![control("auto_exposure", 1, false), control("exposure_time_absolute", 330, false)]);
        assert!(find("v4l2:exposure_time_absolute").unwrap().blockers(&manual).is_empty());
        assert_eq!(find("obsbot:exposure_compensation").unwrap().blockers(&manual).len(), 1);
        assert_eq!(find("obsbot:auto_exposure_mode").unwrap().blockers(&manual).len(), 1);
    }

    #[test]
    fn manual_focus_and_focus_priority_are_opposites() {
        let on = snapshot(vec![control("focus_automatic_continuous", 1, false), control("focus_absolute", 10, true)]);
        assert_eq!(find("v4l2:focus_absolute").unwrap().blockers(&on).len(), 1);
        assert!(find("obsbot:focus_mode").unwrap().blockers(&on).is_empty());
        let off = snapshot(vec![control("focus_automatic_continuous", 0, false), control("focus_absolute", 10, false)]);
        assert!(find("v4l2:focus_absolute").unwrap().blockers(&off).is_empty());
        assert_eq!(find("obsbot:focus_mode").unwrap().blockers(&off).len(), 1);
    }

    #[test]
    fn unknown_state_does_not_block_a_dependent_setting() {
        // No exposure mode reported: priority must stay usable.
        let empty = snapshot(vec![control("exposure_time_absolute", 330, false)]);
        assert!(find("obsbot:auto_exposure_mode").unwrap().blockers(&empty).is_empty());
    }

    #[test]
    fn out_of_range_driver_value_switches_the_control_off_with_a_reason() {
        let mut zoom = control("zoom_continuous", 245, false);
        zoom.min = -100;
        zoom.max = 100;
        let snapshot = snapshot(vec![zoom]);
        let found = discovered(&snapshot);
        assert!(found.is_empty(), "zoom_continuous is deliberately not offered");
        let mut invalid = control("zoom_absolute", 245, false);
        invalid.max = 100;
        let snap = Snapshot { controls: vec![invalid], ..Snapshot::default() };
        assert!(find("v4l2:zoom_absolute").unwrap().blockers(&snap)[0].contains("outside its own range"));
    }

    #[test]
    fn shutter_speed_formats_as_a_fraction() {
        assert_eq!(Fmt::Shutter.format(330.0), "1/30 s");
        assert_eq!(Fmt::Shutter.format(167.0), "1/60 s");
        assert_eq!(Fmt::Shutter.format(2500.0), "1/4 s");
        assert_eq!(Fmt::Ev.format(9.0), "0.0 EV");
        assert_eq!(Fmt::Level.format(0.0), "Off");
    }

    #[test]
    fn unknown_driver_controls_are_discovered_but_movement_is_not() {
        let snap = snapshot(vec![control("gamma", 100, false), control("pan_speed", 0, false), control("brightness", 5, false)]);
        let names: Vec<_> = discovered(&snap).iter().map(|s| s.id).collect();
        assert_eq!(names, vec!["v4l2:gamma"]);
    }

    #[test]
    fn tracking_mode_reads_the_status_tuple() {
        let mut status = [0u8; 60];
        status[0x18] = 0x02;
        status[0x1c] = 0x01;
        let snap = Snapshot { status: Some(status), ..Snapshot::default() };
        assert_eq!(find("obsbot:ai_tracking_mode").unwrap().read(&snap), Some(Value::Int(2)));
    }

    // ── Facts established by hardware read-back (see Assistant/reference 14, 15 and the autofocus notes) ──

    fn with_config(configure: impl FnOnce(&mut AppConfig)) -> Snapshot {
        let mut snapshot = Snapshot::default();
        configure(&mut snapshot.config);
        snapshot
    }

    #[test]
    fn autofocus_priority_comes_only_from_the_camera() {
        let setting = find("obsbot:focus_mode").unwrap();
        // A saved value is never shown as the camera's state.
        let saved_only = with_config(|c| c.focus_mode = Some("face".into()));
        assert_eq!(setting.reading(&saved_only), None);
        let mut answered = saved_only.clone();
        answered.focus_priority = Some(FaceFocusPriority::Global);
        assert_eq!(setting.reading(&answered), Some((Value::Int(0), Origin::Camera)));
        answered.focus_priority = Some(FaceFocusPriority::Face);
        assert_eq!(setting.reading(&answered), Some((Value::Int(1), Origin::Camera)));
    }

    #[test]
    fn autofocus_priority_payloads_use_the_corrected_mapping() {
        // Hardware read-back showed Face = bf fb 01 00 and Global = be 07 00 00;
        // the first capture labels had these the other way round.
        assert_eq!(FaceFocusPriority::Face.payload(), [0xbf, 0xfb, 0x01, 0x00]);
        assert_eq!(FaceFocusPriority::Global.payload(), [0xbe, 0x07, 0x00, 0x00]);
    }

    #[test]
    fn metering_priority_reads_status_byte_7() {
        let setting = find("obsbot:auto_exposure_mode").unwrap();
        let mut status = [0u8; 60];
        status[7] = 0x00;
        let snap = Snapshot { status: Some(status), ..Snapshot::default() };
        assert_eq!(setting.reading(&snap), Some((Value::Int(1), Origin::Camera))); // Face
        status[7] = 0x01;
        let snap = Snapshot { status: Some(status), ..Snapshot::default() };
        assert_eq!(setting.reading(&snap), Some((Value::Int(0), Origin::Camera))); // Global
        // An unrecognised byte is Unknown, even with a saved value on file.
        status[7] = 0x05;
        let mut snap = Snapshot { status: Some(status), ..Snapshot::default() };
        snap.config.auto_exposure_mode = Some("face".into());
        assert_eq!(setting.reading(&snap), None);
        // Only when the camera cannot be read at all does the saved value appear, labelled as saved.
        let unreadable = with_config(|c| c.auto_exposure_mode = Some("face".into()));
        assert_eq!(setting.reading(&unreadable), Some((Value::Int(1), Origin::Saved)));
    }

    #[test]
    fn there_are_exactly_seven_tracking_modes_and_unknown_tuples_stay_unknown() {
        let setting = find("obsbot:ai_tracking_mode").unwrap();
        let labels: Vec<&str> = setting.options(&Snapshot::default()).iter().map(|o| o.label).collect();
        assert_eq!(labels, ["No tracking", "Normal", "Upper body", "Close-up", "Headless", "Lower body", "Group"]);
        for (index, (mode, sub)) in [(0u8, 0u8), (2, 0), (2, 1), (2, 2), (2, 3), (2, 4), (1, 0)].into_iter().enumerate() {
            let mut status = [0u8; 60];
            status[0x18] = mode;
            status[0x1c] = sub;
            let snap = Snapshot { status: Some(status), ..Snapshot::default() };
            assert_eq!(setting.reading(&snap), Some((Value::Int(index as i64), Origin::Camera)), "tuple {mode:02x}/{sub:02x}");
        }
        let mut status = [0u8; 60];
        status[0x18] = 0x05;
        let mut snap = Snapshot { status: Some(status), ..Snapshot::default() };
        snap.config.ai_tracking_mode = Some("upper".into());
        assert_eq!(setting.reading(&snap), None, "an unrecognised pair must not fall back to a saved mode");
    }

    #[test]
    fn target_selection_reads_byte_1_and_zoom_gesture_never_reads_byte_7() {
        let target = find("obsbot:gesture_target").unwrap();
        let zoom = find("obsbot:gesture_zoom").unwrap();
        let mut status = [0u8; 60];
        status[0x01] = 1;
        status[0x07] = 0; // the old, disproven "zoom gesture is on" signal
        let snap = Snapshot { status: Some(status), ..Snapshot::default() };
        assert_eq!(target.reading(&snap), Some((Value::Bool(true), Origin::Camera)));
        assert_eq!(zoom.reading(&snap), None, "byte 7 says nothing about the zoom gesture");
        let mut saved = snap.clone();
        saved.config.gesture_states.insert("zoom_l_pose".into(), true);
        assert_eq!(zoom.reading(&saved), Some((Value::Bool(true), Origin::Saved)));
        for id in ["obsbot:gesture_dynamic_zoom", "obsbot:gesture_dynamic_zoom_direction"] {
            assert_eq!(find(id).unwrap().reading(&snap), None, "{id} has no read-back");
        }
    }

    #[test]
    fn exposure_compensation_is_one_setting_and_hdr_is_gone() {
        assert!(find("obsbot:hdr").is_none(), "the HDR switch was removed");
        assert!(find("v4l2:backlight_compensation").is_none());
        // Backlight compensation is the same hardware control as Exposure compensation (V4L2 backlight_compensation).
        let snap = snapshot(vec![control("backlight_compensation", 9, false), control("gamma", 1, false)]);
        let names: Vec<_> = discovered(&snap).iter().map(|s| s.id).collect();
        assert_eq!(names, vec!["v4l2:gamma"]);
        let setting = find("obsbot:exposure_compensation").unwrap();
        let mut snap = Snapshot::default();
        snap.exposure_comp = Some(15);
        assert_eq!(setting.reading(&snap), Some((Value::Int(15), Origin::Camera)));
        assert_eq!(Fmt::Ev.format(15.0), "+2.0 EV");
        assert_eq!(Fmt::Ev.format(16.0), "+2.3 EV");
        assert_eq!(Fmt::Ev.format(9.0), "0.0 EV");
    }

    #[test]
    fn settings_without_read_back_are_saved_or_unknown_never_camera() {
        let snap = with_config(|c| { c.mirror_image_enabled = Some(true); c.pop_filter_enabled = Some(false); });
        assert_eq!(find("obsbot:mirror_image").unwrap().reading(&snap), Some((Value::Bool(true), Origin::Saved)));
        assert_eq!(find("obsbot:pop_filter").unwrap().reading(&snap), Some((Value::Bool(false), Origin::Saved)));
        assert_eq!(find("obsbot:mic_enabled").unwrap().reading(&snap), None);
        for id in ["obsbot:mirror_image", "obsbot:pop_filter", "obsbot:mic_enabled", "obsbot:radio_distance",
                   "obsbot:gesture_zoom", "obsbot:gesture_dynamic_zoom", "obsbot:gesture_dynamic_zoom_direction"] {
            assert_eq!(find(id).unwrap().readback, Readback::LastSent, "{id}");
        }
    }
}
