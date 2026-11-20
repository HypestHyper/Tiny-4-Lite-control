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
pub enum Fmt { Plain, Shutter, Kelvin, Ev, Ratio, Level, Iso }

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

/// How trustworthy the displayed value is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Readback {
    /// Read from the camera.
    Camera,
    /// The camera cannot report it; the app shows the last value it sent.
    LastSent,
}

// ── Snapshot ──────────────────────────────────────────────────────────────

/// Everything the UI needs to know about the camera right now. Read on the
/// worker thread, then handed to the UI as a plain value.
#[derive(Clone, Default)]
pub struct Snapshot {
    pub controls: Vec<Control>,
    pub status: Option<[u8; 60]>,
    pub exposure_comp: Option<u8>,
    pub config: AppConfig,
    pub error: Option<String>,
}

impl Snapshot {
    pub fn read(device: &str) -> Snapshot {
        let mut snapshot = Snapshot { config: current_config(), ..Snapshot::default() };
        match run_v4l2(device, "--list-ctrls-menus") {
            Ok(text) => snapshot.controls = parse_controls(&text),
            Err(error) => snapshot.error = Some(error),
        }
        snapshot.status = xu_status(device).ok();
        snapshot.exposure_comp = get_exposure_compensation(device).ok();
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

type Reader = fn(&Snapshot) -> Option<Value>;
type Writer = fn(&str, Value) -> Result<(), String>;

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

    pub fn read(&self, snapshot: &Snapshot) -> Option<Value> {
        match &self.source {
            Source::V4l2(name) => {
                let control = snapshot.control(name)?;
                Some(match self.kind {
                    Kind::Toggle => Value::Bool(control.value != 0),
                    _ => Value::Int(control.value),
                })
            }
            Source::Custom { read, .. } => read(snapshot),
        }
    }

    pub fn write(&self, device: &str, value: Value) -> Result<(), String> {
        match &self.source {
            Source::V4l2(name) => set_control(device, name, value.as_i64()),
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

// ── Catalogue ─────────────────────────────────────────────────────────────

fn slider(min: f64, max: f64, step: f64, fmt: Fmt) -> Kind {
    Kind::Slider(SliderSpec { min, max, step, default: None, fmt, ticks: Vec::new() })
}

fn ticked(min: f64, max: f64, step: f64, fmt: Fmt, ticks: Vec<(f64, &'static str)>) -> Kind {
    Kind::Slider(SliderSpec { min, max, step, default: None, fmt, ticks })
}

fn yes_no_config(flag: Option<bool>) -> Option<Value> { flag.map(Value::Bool) }

const TRACKING_MODES: [(&str, u8, u8); 7] = [
    ("off", 0x00, 0x00), ("normal", 0x02, 0x00), ("upper", 0x02, 0x01), ("close", 0x02, 0x02),
    ("headless", 0x02, 0x03), ("lower", 0x02, 0x04), ("group", 0x01, 0x00),
];
const GESTURE_TARGET: [u8; 6] = [0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00];
const GESTURE_ZOOM: [u8; 6] = [0x0a, 0x04, 0x44, 0x31, 0x01, 0x00];
const GESTURE_DYNAMIC: [u8; 6] = [0x0a, 0x04, 0x44, 0x33, 0x01, 0x00];
const GESTURE_DIRECTION: [u8; 6] = [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00];

fn gesture_saved(snapshot: &Snapshot, key: &str) -> Option<Value> {
    snapshot.config.gesture_states.get(key).copied().map(Value::Bool)
}

fn build() -> Vec<Setting> {
    use Group::*;
    let af = "v4l2:focus_automatic_continuous";
    let awb = "v4l2:white_balance_automatic";
    let ae = "v4l2:auto_exposure";
    vec![
        // ── Exposure ──
        Setting::v4l2("auto_exposure", ae, Exposure, "Exposure mode",
            "Let the camera choose exposure, or set the shutter yourself.", Kind::DriverMenu)
            .info("Auto: the camera sets exposure on its own. Manual: you set the shutter speed, and exposure compensation and metering priority switch off. The list of modes comes from the camera driver."),
        Setting::custom("obsbot:auto_exposure_mode", Exposure, "Metering priority",
            "Which part of the picture automatic exposure aims at.",
            Kind::Choice(vec![opt(0, "Global", "Balance the whole frame"), opt(1, "Face", "Favour a detected face")]),
            |s| match s.config.auto_exposure_mode.as_deref() { Some("face") => Some(Value::Int(1)), Some("global") => Some(Value::Int(0)), _ => None },
            |d, v| set_auto_exposure_mode(d, v.as_i64() == 1))
            .needs(ae, Cond::Is(0), "Set Exposure mode to Auto to choose a metering priority.")
            .last_sent()
            .info("Sends OBSBOT's own exposure-priority command. The camera does not report this setting back, so the highlighted choice is the last one this app sent."),
        Setting::custom("obsbot:exposure_compensation", Exposure, "Exposure compensation",
            "Make automatic exposure brighter or darker.",
            ticked(0.0, 18.0, 1.0, Fmt::Ev, vec![(0.0, "−3"), (9.0, "0"), (18.0, "+3")]),
            |s| s.exposure_comp.or(s.config.exposure_compensation_position).map(|p| Value::Int(i64::from(p))),
            |d, v| set_exposure_compensation(d, v.as_i64().clamp(0, 18) as u8))
            .needs(ae, Cond::Is(0), "Set Exposure mode to Auto to use exposure compensation.")
            .info("Moves the automatic exposure target in thirds of a stop, from −3 to +3 EV."),
        Setting::v4l2("exposure_time_absolute", "v4l2:exposure_time_absolute", Exposure, "Shutter speed",
            "How long each frame is exposed.", slider(1.0, 2500.0, 1.0, Fmt::Shutter))
            .needs(ae, Cond::Is(1), "Set Exposure mode to Manual to choose a shutter speed.")
            .info("Shorter times freeze motion and darken the picture. Very long times need a frame rate low enough to fit them."),
        Setting::v4l2("gain", "v4l2:gain", Exposure, "Sensor gain",
            "Brightens the picture, with more noise.", slider(1.0, 64.0, 1.0, Fmt::Plain)),
        Setting::v4l2("backlight_compensation", "v4l2:backlight_compensation", Exposure, "Backlight compensation",
            "Lifts a subject that sits in front of a bright window.", slider(0.0, 18.0, 1.0, Fmt::Plain)),
        Setting::v4l2("power_line_frequency", "v4l2:power_line_frequency", Exposure, "Anti-flicker",
            "Match your mains frequency to remove banding under artificial light.", Kind::DriverMenu)
            .info("Use 50 Hz in most of Europe, Asia and Africa, and 60 Hz in the Americas."),
        Setting::custom("obsbot:hdr", Exposure, "HDR",
            "Keeps detail in bright and dark areas of the same scene.", Kind::Toggle,
            |s| s.status.as_ref().and_then(hdr_state_from_status).or(s.config.hdr_enabled).map(Value::Bool),
            |d, v| set_hdr_enabled(d, v.as_bool()))
            .info("The camera briefly reconfigures its sensor when this changes, so the picture can flicker."),

        // ── White balance ──
        Setting::v4l2("white_balance_automatic", awb, WhiteBalance, "Auto white balance",
            "Let the camera correct for the colour of your lighting.", Kind::Toggle),
        Setting::v4l2("white_balance_temperature", "v4l2:white_balance_temperature", WhiteBalance, "Colour temperature",
            "Lower is warmer, higher is cooler.", slider(2000.0, 10000.0, 100.0, Fmt::Kelvin))
            .needs(awb, OFF, "Turn off Auto white balance to set the colour temperature."),
        Setting::v4l2("red_balance", "v4l2:red_balance", WhiteBalance, "Red balance",
            "Fine-tune the red channel.", slider(0.0, 2048.0, 1.0, Fmt::Plain))
            .needs(awb, OFF, "Turn off Auto white balance to adjust red."),
        Setting::v4l2("blue_balance", "v4l2:blue_balance", WhiteBalance, "Blue balance",
            "Fine-tune the blue channel.", slider(0.0, 2048.0, 1.0, Fmt::Plain))
            .needs(awb, OFF, "Turn off Auto white balance to adjust blue."),

        // ── Focus ──
        Setting::v4l2("focus_automatic_continuous", af, Focus, "Autofocus",
            "Keep the picture sharp as things move.", Kind::Toggle),
        Setting::custom("obsbot:focus_mode", Focus, "Autofocus priority",
            "What autofocus locks on to.",
            Kind::Choice(vec![opt(0, "Global", "Focus on whatever is in frame"), opt(1, "Face", "Favour a detected face")]),
            |s| match s.config.focus_mode.as_deref() { Some("face") => Some(Value::Int(1)), Some("global") => Some(Value::Int(0)), _ => None },
            |d, v| set_face_focus(d, v.as_i64() == 1))
            .needs(af, ON, "Turn on Autofocus to choose its priority.")
            .last_sent()
            .info("Sends OBSBOT's own focus-priority command. The camera does not report it back, so the highlighted choice is the last one this app sent."),
        Setting::v4l2("focus_absolute", "v4l2:focus_absolute", Focus, "Manual focus",
            "Set the focus distance yourself.", slider(0.0, 100.0, 1.0, Fmt::Plain))
            .needs(af, OFF, "Turn off Autofocus to focus by hand."),

        // ── Zoom ──
        Setting::v4l2("zoom_absolute", "v4l2:zoom_absolute", Zoom, "Zoom",
            "Smoothly narrow the view.", slider(0.0, 100.0, 1.0, Fmt::Plain))
            .info("Uses the standard zoom control. The camera's separate continuous-zoom control reports an invalid value, so it is not offered."),
        Setting::custom("obsbot:fov", Zoom, "Field of view",
            "Jump to a common angle.",
            Kind::Choice(vec![opt(0, "86°", "Widest"), opt(5, "78°", "Normal"), opt(15, "65°", "Narrow")]),
            |s| s.control("zoom_absolute").map(|c| c.value).filter(|v| matches!(v, 0 | 5 | 15)).map(Value::Int),
            |d, v| set_control(d, "zoom_absolute", v.as_i64()))
            .info("Moves the zoom control to one of three common positions."),

        // ── Picture ──
        Setting::v4l2("brightness", "v4l2:brightness", Picture, "Brightness", "Lighten or darken the whole picture.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::v4l2("contrast", "v4l2:contrast", Picture, "Contrast", "Separate light from dark.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::v4l2("saturation", "v4l2:saturation", Picture, "Saturation", "How vivid colours look.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::v4l2("hue", "v4l2:hue", Picture, "Hue", "Rotate all colours around the colour wheel.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::v4l2("sharpness", "v4l2:sharpness", Picture, "Sharpness", "Edge enhancement.", slider(0.0, 100.0, 1.0, Fmt::Plain)),
        Setting::custom("obsbot:mirror_image", Picture, "Mirror image",
            "Flip the picture left to right.", Kind::Toggle,
            |s| yes_no_config(s.config.mirror_image_enabled),
            |d, v| set_mirror_image(d, v.as_bool()))
            .last_sent()
            .info("Uses a command captured from OBSBOT Center. The camera does not report it back, so the switch shows the last value this app sent."),

        // ── Tracking ──
        Setting::custom("obsbot:ai_tracking_mode", Tracking, "Tracking mode",
            "How the camera follows and frames a person.",
            Kind::Choice(vec![
                opt(0, "Off", "The camera stays where you point it"),
                opt(1, "Normal", "Follow one person"),
                opt(2, "Upper body", "Frame head and torso"),
                opt(3, "Close-up", "Frame tightly on the face"),
                opt(4, "Headless", "Frame the body below the head"),
                opt(5, "Lower body", "Frame from the waist down"),
                opt(6, "Group", "Keep several people in frame"),
            ]),
            |s| {
                let (m, n) = (s.byte(0x18)?, s.byte(0x1c)?);
                if let Some(i) = TRACKING_MODES.iter().position(|e| e.1 == m && e.2 == n) { return Some(Value::Int(i as i64)); }
                let saved = s.config.ai_tracking_mode.as_deref()?;
                TRACKING_MODES.iter().position(|e| e.0 == saved).map(|i| Value::Int(i as i64))
            },
            |d, v| {
                let entry = TRACKING_MODES.get(v.as_i64() as usize).ok_or("Unknown tracking mode.")?;
                set_ai_mode(d, (entry.1, entry.2))
            })
            .info("The framing names are OBSBOT's. Tracking moves the gimbal by itself, so clear the space around the camera first."),
        Setting::custom("obsbot:tracking_speed", Tracking, "Tracking speed",
            "How quickly the camera follows movement.",
            Kind::Choice(vec![opt(0, "Standard", "Smooth and steady"), opt(1, "Sport", "Faster, for quick movement")]),
            |s| s.byte(0x24).map(|b| Value::Int(i64::from(b != 0)))
                .or_else(|| s.config.tracking_speed.as_deref().map(|v| Value::Int(i64::from(v == "sport")))),
            |d, v| set_tracking_speed(d, v.as_i64() == 1)),

        // ── Gestures ──
        Setting::custom("obsbot:gesture_target", Gestures, "Target selection",
            "Pick who to follow with a hand gesture.", Kind::Toggle,
            |s| gesture_saved(s, "target_selection"),
            |d, v| set_gesture(d, GESTURE_TARGET, v.as_bool())).last_sent(),
        Setting::custom("obsbot:gesture_zoom", Gestures, "Zoom gesture",
            "Zoom in and out with an L-shaped hand pose.", Kind::Toggle,
            |s| s.byte(0x07).map(|b| Value::Bool(b == 0)).or_else(|| gesture_saved(s, "zoom_l_pose")),
            |d, v| set_gesture(d, GESTURE_ZOOM, v.as_bool()))
            .info("This is the one gesture the camera reports back."),
        Setting::custom("obsbot:gesture_zoom_ratio", Gestures, "Gesture zoom amount",
            "How far the zoom gesture zooms in.",
            slider(1.0, 4.0, 0.01, Fmt::Ratio),
            |s| s.config.gesture_zoom_ratio.map(|r| Value::Float(f64::from(r))).or(Some(Value::Float(1.0))),
            |d, v| set_gesture_zoom_ratio(d, v.as_f64() as f32))
            .last_sent().not_quick(),
        Setting::custom("obsbot:gesture_dynamic_zoom", Gestures, "Dynamic zoom",
            "Zoom by moving your hand towards or away from the camera.", Kind::Toggle,
            |s| gesture_saved(s, "dynamic_zoom"),
            |d, v| set_gesture(d, GESTURE_DYNAMIC, v.as_bool())).last_sent(),
        Setting::custom("obsbot:gesture_dynamic_zoom_direction", Gestures, "Reverse dynamic zoom",
            "Swap which hand movement zooms in.", Kind::Toggle,
            |s| gesture_saved(s, "dynamic_zoom_direction"),
            |d, v| set_gesture(d, GESTURE_DIRECTION, v.as_bool())).last_sent(),

        // ── Microphone ──
        Setting::custom("obsbot:mic_enabled", Microphone, "Microphone",
            "Turn the camera's microphones on or off.", Kind::Toggle,
            |s| s.config.audio_microphone_disabled.map(|disabled| Value::Bool(!disabled)),
            |d, v| set_microphone_disabled(d, !v.as_bool()))
            .last_sent()
            .info("The camera may restart its audio when this changes, and apps using the microphone can drop out for a moment. The camera does not report this back."),
        Setting::custom("obsbot:mic_during_sleep", Microphone, "Listen while asleep",
            "Keep the microphone running when the camera is asleep.", Kind::Toggle,
            |s| s.byte(16).and_then(|b| match b { 0 => Some(Value::Bool(false)), 1 => Some(Value::Bool(true)), _ => None })
                .or(s.config.audio_microphone_during_sleep.map(Value::Bool)),
            |d, v| set_microphone_during_sleep(d, v.as_bool())),
        Setting::custom("obsbot:auto_gain", Microphone, "Auto gain",
            "Keep your voice at a steady level.", Kind::Toggle,
            |s| s.byte(25).filter(|b| *b <= 1).map(|b| Value::Bool(b != 0)).or(s.config.audio_auto_gain.map(Value::Bool)),
            |d, v| set_auto_gain(d, v.as_bool())),
        Setting::custom("obsbot:noise_reduction", Microphone, "Noise reduction",
            "How strongly background noise is removed.",
            Kind::Choice(vec![opt(0, "Off", ""), opt(1, "Weak", ""), opt(2, "Medium", ""), opt(3, "Strong", "")]),
            |s| s.byte(8).filter(|b| *b <= 3).map(|b| Value::Int(i64::from(b))).or(s.config.audio_noise_reduction.map(|l| Value::Int(i64::from(l)))),
            |d, v| set_noise_reduction(d, v.as_i64().clamp(0, 3) as u8)),
        Setting::custom("obsbot:radio_distance", Microphone, "Pickup distance",
            "How far away the voice you want is.",
            Kind::Choice(vec![opt(0, "Close", "About arm's length"), opt(1, "Standard", ""), opt(2, "Far", "Across a room")]),
            |s| s.config.audio_radio_distance.filter(|d| *d <= 2).map(|d| Value::Int(i64::from(d))),
            |d, v| set_radio_distance(d, v.as_i64().clamp(0, 2) as u8))
            .last_sent(),
        Setting::custom("obsbot:pop_filter", Microphone, "Pop filter",
            "Soften plosive sounds such as “p” and “b”.", Kind::Toggle,
            |s| yes_no_config(s.config.pop_filter_enabled),
            |d, v| set_pop_filter(d, v.as_bool()))
            .last_sent().experimental()
            .info("Experimental: this replays two commands captured from OBSBOT Center without decoding them. The camera does not report the state back."),

        // ── Light ──
        Setting::custom("obsbot:led_brightness", Lighting, "LED brightness",
            "Off, or one of three brightness levels.",
            ticked(0.0, 3.0, 1.0, Fmt::Level, vec![(0.0, "Off"), (1.0, "1"), (2.0, "2"), (3.0, "3")]),
            |s| s.byte(0x21).map(|b| Value::Int(i64::from(b.min(3)))).or(s.config.led_brightness.map(|l| Value::Int(i64::from(l)))),
            |d, v| set_led_brightness(d, v.as_i64().clamp(0, 3) as u8)),

        // ── Power ──
        Setting::custom("obsbot:sleep", Power, "Camera power",
            "Put the camera to sleep, or wake it.",
            Kind::Choice(vec![
                opt(0, "Awake", ""),
                Opt { value: 1, label: "Asleep", hint: "", confirm: Some(Confirm {
                    title: "Put the camera to sleep?",
                    body: "The gimbal will move to its resting position. Clear the space around the camera first.",
                    action: "Sleep",
                }) },
            ]),
            |s| s.byte(0x02).and_then(|b| match b { 0 => Some(Value::Int(0)), 1 => Some(Value::Int(1)), _ => None }),
            |d, v| set_sleep_mode(d, v.as_i64() == 1))
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
}
