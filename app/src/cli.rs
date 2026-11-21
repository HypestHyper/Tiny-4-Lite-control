//! Command line: every setting in the registry, and the other things the
//! windows do (gimbal, presets, saved positions, capture mode), without GTK.
//!
//! Settings are looked up in the same catalogue the windows are built from, so
//! a setting added there gets `get`, `set`, `toggle`, `reset`, `info`, shell
//! completion and JSON output with no extra code here.

use crate::backend::registry::{self, Fmt, Group, Kind, Origin, Setting, Snapshot, Value};
use crate::backend::{self, current_config, flush_config, install_config, update_config_checked};
use crate::{config, presets, video};
use serde_json::{json, Value as Json};
use std::time::Duration;

pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const USAGE: i32 = 2;
pub const BLOCKED: i32 = 3;
pub const NEEDS_YES: i32 = 4;
pub const NO_CAMERA: i32 = 5;

const COMMANDS: &[&str] = &[
    "list", "get", "set", "toggle", "reset", "info", "deps", "service", "status", "gimbal", "preset", "position",
    "startup", "capture", "completions", "help", "__complete",
];

/// Should this invocation run the command line instead of opening a window?
pub fn is_cli(args: &[String]) -> bool {
    args.first().is_some_and(|first| COMMANDS.contains(&first.as_str()) || matches!(first.as_str(), "--help" | "-h" | "--version" | "-V"))
}

struct Options {
    json: bool,
    yes: bool,
    force: bool,
    device: Option<String>,
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options, String> {
        let mut options = Options { json: false, yes: false, force: false, device: std::env::var("TINY4LITE_DEVICE").ok(), positional: Vec::new(), flags: Vec::new() };
        let mut iter = args.iter().peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--json" => options.json = true,
                "--yes" | "-y" => options.yes = true,
                "--force" => options.force = true,
                "--device" | "-d" => options.device = Some(iter.next().ok_or("--device needs a path, for example /dev/video0")?.clone()),
                other if other.starts_with("--device=") => options.device = Some(other["--device=".len()..].to_string()),
                // Negative numbers are values, not flags (for example `--tilt -20`).
                other if other.starts_with("--") => {
                    if let Some((name, value)) = other.split_once('=') {
                        options.flags.push((name.to_string(), Some(value.to_string())));
                    } else {
                        let takes_value = iter.peek().is_some_and(|next| !next.starts_with("--") || next.parse::<f64>().is_ok());
                        let value = if takes_value { iter.next().cloned() } else { None };
                        options.flags.push((other.to_string(), value));
                    }
                }
                _ => options.positional.push(arg.clone()),
            }
        }
        Ok(options)
    }

    fn flag(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.as_deref())
    }
}

pub fn run(args: Vec<String>) -> i32 {
    match config::load() {
        Ok(loaded) => install_config(loaded),
        Err(error) => eprintln!("Using default settings: {error}"),
    }
    let code = dispatch(&args);
    flush_config();
    code
}

fn dispatch(args: &[String]) -> i32 {
    if matches!(args.first().map(String::as_str), Some("--version" | "-V")) {
        println!("tiny4lite-control {}", env!("CARGO_PKG_VERSION"));
        return OK;
    }
    if matches!(args.first().map(String::as_str), Some("--help" | "-h")) {
        print!("{}", help(None));
        return OK;
    }
    let options = match Options::parse(&args[1..]) {
        Ok(options) => options,
        Err(error) => return usage_error(&error),
    };
    let rest = &options.positional;
    match args[0].as_str() {
        "help" => { print!("{}", help(rest.first().map(String::as_str))); OK }
        "completions" => completions(rest.first().map(String::as_str)),
        "__complete" => complete(rest),
        "deps" => { print!("{}", registry::dependency_map()); OK }
        "service" => {
            let flag_on = |name: &str| options.flags.iter().any(|(n, _)| n == name);
            // A flag written before the word it precedes swallows it as a value; give it back.
            let mut words = options.positional.clone();
            for (n, v) in &options.flags { if matches!(n.as_str(), "--write" | "--once") { if let Some(v) = v { words.push(v.clone()); } } }
            crate::service::run(&crate::service::Request { words: &words, json: options.json, write: flag_on("--write"), once: flag_on("--once"), device: options.device.clone() })
        }
        "list" => with_camera(&options, |device, snapshot| list(&options, device, snapshot)),
        "get" => with_camera(&options, |_, snapshot| get(&options, snapshot)),
        "info" => with_camera(&options, |_, snapshot| info(&options, snapshot)),
        "set" => with_camera(&options, |device, snapshot| set(&options, device, snapshot)),
        "toggle" => with_camera(&options, |device, snapshot| toggle(&options, device, snapshot)),
        "reset" => with_camera(&options, |device, snapshot| reset(&options, device, snapshot)),
        "status" => with_camera(&options, |device, snapshot| status(&options, device, snapshot)),
        "gimbal" => with_camera(&options, |device, snapshot| gimbal(&options, device, snapshot)),
        "preset" => with_camera(&options, |device, snapshot| preset(&options, device, snapshot)),
        "position" => with_camera(&options, |device, _| position(&options, device)),
        "startup" => with_camera(&options, |device, _| startup(&options, device)),
        "capture" => with_camera(&options, |device, _| capture(&options, device)),
        other => usage_error(&format!("Unknown command `{other}`.")),
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("{message}\nRun `tiny4lite-control help` for the commands.");
    USAGE
}

fn fail(message: &str) -> i32 {
    eprintln!("{message}");
    FAILED
}

/// Find the camera (or the one named with --device), read it once, and run `action`.
fn with_camera(options: &Options, action: impl FnOnce(&str, &Snapshot) -> i32) -> i32 {
    let device = match options.device.clone().or_else(|| backend::discover_tiny4lite().into_iter().next()) {
        Some(device) => device,
        None => {
            eprintln!("No Tiny 2 Lite was found. Plug it in, or name a video device with --device /dev/videoN.");
            return NO_CAMERA;
        }
    };
    let snapshot = Snapshot::read(&device);
    if let (Some(error), true) = (&snapshot.error, snapshot.controls.is_empty()) {
        eprintln!("Could not read {device}: {error}");
        return NO_CAMERA;
    }
    action(&device, &snapshot)
}

// ── Names ─────────────────────────────────────────────────────────────────

/// The name used on the command line: the label the windows show, as a slug
/// (`Shutter speed` becomes `shutter-speed`).
pub fn alias(setting: &Setting) -> String {
    let words: Vec<String> = setting.label.split_whitespace()
        .map(|w| w.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    words.join("-")
}

/// The older, driver-style name (`exposure-time-absolute`), still accepted so scripts keep working.
fn technical_name(setting: &Setting) -> String {
    setting.id.split_once(':').map_or(setting.id, |(_, rest)| rest).replace('_', "-")
}

fn group_slug(group: Group) -> String {
    group.title().to_lowercase().replace(' ', "-").replace("zoom-and-field-of-view", "zoom")
}

/// The window tab a group sits on, as a command-line word (`camera`, `tracking`, `audio`, `device`).
fn tab_slug(group: Group) -> &'static str {
    match group.tab() {
        registry::Tab::Camera => "camera",
        registry::Tab::Tracking => "tracking",
        registry::Tab::Audio => "audio",
        registry::Tab::Device => "device",
    }
}

/// Degrees for display, without a negative zero.

fn normalise(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// All settings the camera offers: the catalogue, plus unrecognised driver controls.
fn all_settings(snapshot: &Snapshot) -> Vec<&'static Setting> {
    let mut all: Vec<&'static Setting> = registry::all().iter().filter(|s| s.present(snapshot)).collect();
    for extra in registry::discovered(snapshot) {
        all.push(Box::leak(Box::new(extra)));
    }
    all
}

fn resolve(snapshot: &Snapshot, name: &str) -> Result<&'static Setting, String> {
    let wanted = normalise(name);
    let all = all_settings(snapshot);
    let exact: Vec<_> = all.iter()
        .filter(|s| normalise(&alias(s)) == wanted || normalise(s.id) == wanted || normalise(&technical_name(s)) == wanted)
        .collect();
    if let [one] = exact.as_slice() { return Ok(**one); }
    // A unique label also works: `set "exposure mode" auto`.
    let by_label: Vec<_> = all.iter().filter(|s| normalise(s.label) == wanted).collect();
    if let [one] = by_label.as_slice() { return Ok(**one); }
    let near: Vec<String> = all.iter().filter(|s| normalise(&alias(s)).contains(&wanted)).map(|s| alias(s)).take(6).collect();
    if near.is_empty() {
        Err(format!("There is no setting called `{name}`. Run `tiny4lite-control list` to see them."))
    } else {
        Err(format!("There is no setting called `{name}`. Did you mean: {}?", near.join(", ")))
    }
}

// ── Values ────────────────────────────────────────────────────────────────

fn show(setting: &Setting, snapshot: &Snapshot, value: Option<Value>) -> String {
    let Some(value) = value else { return "unknown".into() };
    match &setting.kind {
        Kind::Toggle => if value.as_bool() { "on" } else { "off" }.into(),
        Kind::Slider(_) => setting.format(snapshot, value.as_f64()),
        _ => setting.options(snapshot).into_iter().find(|o| o.value == value.as_i64()).map(|o| o.label.to_string()).unwrap_or_else(|| value.as_i64().to_string()),
    }
}

fn parse_bool(text: &str) -> Result<bool, String> {
    match text.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "enable" | "enabled" => Ok(true),
        "off" | "false" | "no" | "0" | "disable" | "disabled" => Ok(false),
        other => Err(format!("`{other}` is not on or off.")),
    }
}

fn parse_number(text: &str, suffixes: &[&str]) -> Result<f64, String> {
    let mut clean = text.trim().to_lowercase();
    for suffix in suffixes {
        if let Some(stripped) = clean.strip_suffix(suffix) { clean = stripped.trim().to_string(); }
    }
    clean.parse::<f64>().map_err(|_| format!("`{text}` is not a number."))
}

/// Turn what the user typed into the camera's own unit, using the same
/// units the windows show (EV, kelvin, 1/30 s, ×).
pub fn parse_slider(fmt: Fmt, text: &str) -> Result<f64, String> {
    match fmt {
        Fmt::Shutter => {
            let lower = text.trim().to_lowercase();
            let lower = lower.trim_end_matches('s').trim();
            if let Some((one, denominator)) = lower.split_once('/') {
                let (one, denominator) = (parse_number(one, &[])?, parse_number(denominator, &[])?);
                if one <= 0.0 || denominator <= 0.0 { return Err("A shutter fraction needs positive numbers, for example 1/60.".into()); }
                Ok((10_000.0 * one / denominator).round())
            } else if text.trim().to_lowercase().ends_with('s') {
                Ok((parse_number(lower, &[])? * 10_000.0).round())
            } else {
                parse_number(text, &[])
            }
        }
        Fmt::Ev => {
            let ev = parse_number(text, &["ev"])?;
            if !(-3.0..=3.0).contains(&ev) { return Err("Exposure compensation is between -3 and +3 EV.".into()); }
            Ok(((ev + 3.0) * 3.0).round())
        }
        Fmt::Kelvin => parse_number(text, &["k"]),
        Fmt::Ratio => parse_number(text, &["x", "×"]),
        Fmt::Level => if text.trim().eq_ignore_ascii_case("off") { Ok(0.0) } else { parse_number(text, &["level"]) },
        Fmt::Iso => parse_number(text, &["iso"]),
        Fmt::Plain => parse_number(text, &["%"]),
    }
}

fn parse_value(setting: &Setting, snapshot: &Snapshot, text: &str) -> Result<Value, String> {
    match &setting.kind {
        Kind::Toggle => parse_bool(text).map(Value::Bool),
        Kind::Choice(_) | Kind::DriverMenu => {
            let options = setting.options(snapshot);
            let wanted = normalise(text);
            let found = options.iter().find(|o| normalise(o.label) == wanted)
                .or_else(|| text.trim().parse::<i64>().ok().and_then(|n| options.iter().find(|o| o.value == n)));
            found.map(|o| Value::Int(o.value)).ok_or_else(|| format!(
                "`{text}` is not one of: {}.", options.iter().map(|o| o.label).collect::<Vec<_>>().join(", ")))
        }
        Kind::Slider(spec) => {
            let number = parse_slider(spec.fmt, text)?;
            let spec = setting.slider(snapshot).unwrap_or_else(|| spec.clone());
            if number < spec.min - 1e-9 || number > spec.max + 1e-9 {
                return Err(format!("{} must be between {} and {}.", setting.label, spec.fmt.format(spec.min), spec.fmt.format(spec.max)));
            }
            let stepped = if spec.step >= 1.0 { spec.min + ((number - spec.min) / spec.step).round() * spec.step } else { number };
            Ok(if spec.step < 1.0 { Value::Float(stepped) } else { Value::Int(stepped.round() as i64) })
        }
    }
}

// ── Commands ──────────────────────────────────────────────────────────────

fn setting_json(setting: &Setting, snapshot: &Snapshot) -> Json {
    let value = setting.read(snapshot);
    let blockers = setting.blockers(snapshot);
    let mut object = json!({
        "id": setting.id,
        "name": alias(setting),
        "group": group_slug(setting.group),
        "label": setting.label,
        "summary": setting.summary,
        "kind": match setting.kind { Kind::Toggle => "toggle", Kind::Choice(_) | Kind::DriverMenu => "choice", Kind::Slider(_) => "slider" },
        "value": value.map(|v| match setting.kind { Kind::Toggle => json!(v.as_bool()), Kind::Slider(_) => json!(v.as_f64()), _ => json!(show(setting, snapshot, Some(v))) }),
        "display": show(setting, snapshot, value),
        "available": blockers.is_empty(),
        "blocked_by": blockers,
        "reported_by_camera": setting.readback == registry::Readback::Camera,
        "source": match setting.origin(snapshot) { Origin::Camera => "camera", Origin::Saved => "saved", Origin::Unknown => "unknown" },
        "experimental": setting.experimental,
    });
    match &setting.kind {
        Kind::Choice(_) | Kind::DriverMenu => {
            object["options"] = json!(setting.options(snapshot).iter().map(|o| o.label).collect::<Vec<_>>());
        }
        Kind::Slider(_) => {
            if let Some(spec) = setting.slider(snapshot) {
                object["range"] = json!({ "min": spec.min, "max": spec.max, "step": spec.step, "default": spec.default });
            }
        }
        Kind::Toggle => {}
    }
    object
}

fn list(options: &Options, device: &str, snapshot: &Snapshot) -> i32 {
    let filter = options.positional.first().map(|g| normalise(g));
    let settings: Vec<&Setting> = all_settings(snapshot).into_iter()
        .filter(|s| filter.as_ref().is_none_or(|f| normalise(&group_slug(s.group)) == *f || tab_slug(s.group) == f.as_str()))
        .collect();
    if let Some(wanted) = &filter {
        if settings.is_empty() {
            let mut groups: Vec<String> = ["camera", "tracking", "audio", "device"].iter().map(|s| s.to_string()).collect();
            groups.extend(Group::ALL.iter().map(|g| group_slug(*g)));
            return usage_error(&format!("There is no group `{wanted}`. Use a tab or a group: {}.", groups.join(", ")));
        }
    }
    if options.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "device": device, "settings": settings.iter().map(|s| setting_json(s, snapshot)).collect::<Vec<_>>() })).unwrap_or_default());
        return OK;
    }
    println!("Camera: {device}");
    let width = settings.iter().map(|s| alias(s).len()).max().unwrap_or(10);
    let mut current: Option<Group> = None;
    for setting in settings {
        if current != Some(setting.group) {
            println!("\n{}", setting.group.title());
            current = Some(setting.group);
        }
        let blockers = setting.blockers(snapshot);
        let value = show(setting, snapshot, setting.read(snapshot));
        let source = match setting.origin(snapshot) { Origin::Camera => "", Origin::Saved => " (saved)", Origin::Unknown => "" };
        let note = if let Some(reason) = blockers.first() { format!("  (unavailable: {reason})") } else { String::new() };
        println!("  {:<width$}  {}{}{}", alias(setting), value, source, note, width = width);
    }
    OK
}

fn get(options: &Options, snapshot: &Snapshot) -> i32 {
    let Some(name) = options.positional.first() else { return usage_error("get needs a setting name, for example: get exposure-mode") };
    match resolve(snapshot, name) {
        Ok(setting) => {
            if options.json {
                println!("{}", serde_json::to_string_pretty(&setting_json(setting, snapshot)).unwrap_or_default());
            } else {
                // Just the value, so scripts can use it directly.
                println!("{}", show(setting, snapshot, setting.read(snapshot)));
                match setting.origin(snapshot) {
                    Origin::Camera => {}
                    Origin::Saved => eprintln!("(saved value: the camera does not confirm this one)"),
                    Origin::Unknown => eprintln!("(unknown: the camera did not report this)"),
                }
            }
            OK
        }
        Err(error) => usage_error(&error),
    }
}

fn info(options: &Options, snapshot: &Snapshot) -> i32 {
    let Some(name) = options.positional.first() else { return usage_error("info needs a setting name, for example: info shutter-speed") };
    let setting = match resolve(snapshot, name) { Ok(s) => s, Err(e) => return usage_error(&e) };
    if options.json {
        println!("{}", serde_json::to_string_pretty(&setting_json(setting, snapshot)).unwrap_or_default());
        return OK;
    }
    println!("{}  ({})", setting.label, alias(setting));
    println!("{}", setting.summary);
    if !setting.info.is_empty() { println!("\n{}", setting.info); }
    println!("\nGroup:   {}", setting.group.title());
    println!("Current: {}", show(setting, snapshot, setting.read(snapshot)));
    match &setting.kind {
        Kind::Toggle => println!("Values:  on, off"),
        Kind::Choice(_) | Kind::DriverMenu => {
            println!("Values:");
            for option in setting.options(snapshot) {
                println!("  {}{}", option.label, if option.hint.is_empty() { String::new() } else { format!(" - {}", option.hint) });
            }
        }
        Kind::Slider(_) => if let Some(spec) = setting.slider(snapshot) {
            println!("Range:   {} to {}{}", spec.fmt.format(spec.min), spec.fmt.format(spec.max),
                spec.default.map(|d| format!(", default {}", spec.fmt.format(d))).unwrap_or_default());
        },
    }
    for dep in &setting.deps { println!("Needs:   {}", dep.reason); }
    for blocker in setting.blockers(snapshot) { println!("Now:     unavailable. {blocker}"); }
    println!("Source:  {}", match setting.origin(snapshot) {
        Origin::Camera => "read from the camera",
        Origin::Saved => "saved: the last value this app sent; the camera has not confirmed it",
        Origin::Unknown => "unknown: the camera did not report it",
    });
    if setting.readback == registry::Readback::LastSent { println!("Note:    the camera does not report this setting back, so only the last value sent from this app can be shown."); }
    if setting.experimental { println!("Note:    experimental; not fully verified on this camera."); }
    OK
}

fn apply(options: &Options, device: &str, snapshot: &Snapshot, setting: &'static Setting, value: Value) -> i32 {
    let blockers = setting.blockers(snapshot);
    if !blockers.is_empty() && !options.force {
        eprintln!("{} is unavailable right now. {}", setting.label, blockers[0]);
        eprintln!("(Use --force to send it anyway.)");
        return BLOCKED;
    }
    let confirm = setting.options(snapshot).into_iter().find(|o| matches!(setting.kind, Kind::Choice(_)) && o.value == value.as_i64()).and_then(|o| o.confirm);
    if let Some(confirm) = confirm {
        if !options.yes {
            eprintln!("{}\n{}\nRun the command again with --yes to go ahead.", confirm.title, confirm.body);
            return NEEDS_YES;
        }
    }
    match setting.write(device, value) {
        Ok(confirmed) => {
            let shown = show(setting, snapshot, Some(value));
            // What the camera confirmed beats what was asked for.
            let (state, code) = match confirmed {
                Some(actual) if actual.as_i64() == value.as_i64() => ("confirmed by the camera".to_string(), OK),
                Some(actual) => (format!("the camera reports {} instead", show(setting, snapshot, Some(actual))), FAILED),
                None if setting.readback == registry::Readback::LastSent => ("sent; the camera can't confirm this".to_string(), OK),
                None => ("sent".to_string(), OK),
            };
            if options.json {
                println!("{}", json!({ "id": setting.id, "name": alias(setting), "requested": shown,
                    "confirmed": confirmed.map(|a| show(setting, snapshot, Some(a))), "ok": code == OK }));
            } else if code == OK {
                println!("{}: {shown} ({state})", setting.label);
            } else {
                eprintln!("{}: you asked for {shown}, but {state}.", setting.label);
            }
            code
        }
        Err(error) => fail(&format!("Could not change {}: {error}", setting.label.to_lowercase())),
    }
}

fn set(options: &Options, device: &str, snapshot: &Snapshot) -> i32 {
    let [name, text] = match options.positional.as_slice() {
        [name, text] => [name, text],
        _ => return usage_error("set needs a setting and a value, for example: set exposure-mode manual"),
    };
    let setting = match resolve(snapshot, name) { Ok(s) => s, Err(e) => return usage_error(&e) };
    match parse_value(setting, snapshot, text) {
        Ok(value) => apply(options, device, snapshot, setting, value),
        Err(error) => usage_error(&error),
    }
}

fn toggle(options: &Options, device: &str, snapshot: &Snapshot) -> i32 {
    let Some(name) = options.positional.first() else { return usage_error("toggle needs a setting name") };
    let setting = match resolve(snapshot, name) { Ok(s) => s, Err(e) => return usage_error(&e) };
    if !matches!(setting.kind, Kind::Toggle) { return usage_error(&format!("{} is not an on/off setting. Use `set`.", setting.label)); }
    match setting.read(snapshot) {
        Some(current) => apply(options, device, snapshot, setting, Value::Bool(!current.as_bool())),
        None => usage_error(&format!("The camera does not report {}, so there is nothing to flip. Use `set {} on` or `off`.", setting.label.to_lowercase(), alias(setting))),
    }
}

fn reset(options: &Options, device: &str, snapshot: &Snapshot) -> i32 {
    let Some(name) = options.positional.first() else { return usage_error("reset needs a setting name") };
    let setting = match resolve(snapshot, name) { Ok(s) => s, Err(e) => return usage_error(&e) };
    match setting.slider(snapshot).and_then(|s| s.default.map(|d| (d, s))) {
        Some((default, spec)) => apply(options, device, snapshot, setting, if spec.step < 1.0 { Value::Float(default) } else { Value::Int(default.round() as i64) }),
        None => usage_error(&format!("{} has no default value to go back to.", setting.label)),
    }
}

fn status(options: &Options, device: &str, snapshot: &Snapshot) -> i32 {
    let ids = ["obsbot:sleep", "v4l2:auto_exposure", "v4l2:focus_automatic_continuous", "v4l2:zoom_absolute", "obsbot:ai_tracking_mode", "obsbot:tracking_speed"];
    let position = backend::get_gimbal_position(device).ok();
    if options.json {
        let values: Vec<Json> = ids.iter().filter_map(|id| registry::find(id)).filter(|s| s.present(snapshot)).map(|s| setting_json(s, snapshot)).collect();
        println!("{}", serde_json::to_string_pretty(&json!({
            "device": device, "settings": values,
            "gimbal": position.map(|(pan, tilt)| json!({ "pan_degrees": pan, "tilt_degrees": tilt })),
        })).unwrap_or_default());
        return OK;
    }
    println!("Camera   {device}");
    for id in ids {
        if let Some(setting) = registry::find(id).filter(|s| s.present(snapshot)) {
            println!("{:<9}{}", setting.label.split_whitespace().next().unwrap_or(""), show(setting, snapshot, setting.read(snapshot)));
        }
    }
    match position { Some((pan, tilt)) => println!("Gimbal   pan {}, tilt {}", backend::angles::format(pan), backend::angles::format(tilt)), None => println!("Gimbal   position not available") }
    OK
}

fn gimbal(options: &Options, device: &str, snapshot: &Snapshot) -> i32 {
    match options.positional.first().map(String::as_str) {
        Some("position") => match backend::get_gimbal_position(device) {
            Ok((pan, tilt)) => {
                if options.json { println!("{}", json!({ "pan_degrees": pan, "tilt_degrees": tilt })); }
                else { println!("pan {}, tilt {}", backend::angles::format(pan), backend::angles::format(tilt)); }
                OK
            }
            Err(error) => fail(&format!("Could not read the gimbal position: {error}")),
        },
        Some("centre") | Some("center") => {
            if !options.yes {
                eprintln!("This turns the camera to pan 0°, tilt 0°.\nClear the space around it, then run the command again with --yes.");
                return FAILED;
            }
            match backend::centre_gimbal(device) { Ok(()) => { println!("Centring the camera (pan 0°, tilt 0°)"); OK } Err(error) => fail(&format!("Could not centre the camera: {error}")) }
        }
        Some("stop") => stop_gimbal(device),
        Some("step") => {
            let degrees = |name: &str| options.flag(name).map(|v| v.parse::<i64>().map_err(|_| format!("{name} needs a whole number of degrees, for example 1 or -2."))).transpose();
            let (pan, tilt) = match (degrees("--pan"), degrees("--tilt")) { (Ok(p), Ok(t)) => (p, t), (Err(e), _) | (_, Err(e)) => return usage_error(&e) };
            if pan.is_none() && tilt.is_none() {
                return usage_error(&format!("gimbal step needs --pan and/or --tilt in whole degrees (1 to {} each).", backend::MAX_STEP_DEGREES));
            }
            for (name, value) in [("--pan", pan), ("--tilt", tilt)] {
                if value.is_some_and(|v| v == 0 || v.abs() > backend::MAX_STEP_DEGREES) {
                    return usage_error(&format!("{name} must be between 1 and {} degrees, in either direction.", backend::MAX_STEP_DEGREES));
                }
            }
            let mut code = OK;
            for (axis, control, value) in [("pan", "pan_absolute", pan), ("tilt", "tilt_absolute", tilt)] {
                let Some(value) = value else { continue };
                let Some(range) = snapshot.control(control).filter(|c| !c.disabled).map(|c| (c.min, c.max)) else {
                    eprintln!("This video node has no working absolute {axis} control.");
                    code = FAILED;
                    continue;
                };
                match backend::step_gimbal(device, control, value, range.0, range.1) {
                    Ok(target) => println!("{axis} target {}", backend::angles::format(target)),
                    Err(error) => { eprintln!("Could not turn {axis}: {error}"); code = FAILED; }
                }
            }
            code
        }
        Some("move") => {
            let percent = |name: &str| options.flag(name).map(|v| v.parse::<f64>().map_err(|_| format!("{name} needs a number from -100 to 100."))).transpose();
            let (pan, tilt) = match (percent("--pan"), percent("--tilt")) { (Ok(p), Ok(t)) => (p.unwrap_or(0.0), t.unwrap_or(0.0)), (Err(e), _) | (_, Err(e)) => return usage_error(&e) };
            if pan == 0.0 && tilt == 0.0 { return usage_error("gimbal move needs --pan and/or --tilt, as a percentage of full speed (-100 to 100). Positive is right and up."); }
            let seconds = match options.flag("--seconds").map(str::parse::<f64>).transpose() { Ok(s) => s.unwrap_or(1.0), Err(_) => return usage_error("--seconds needs a number.") };
            if !(0.1..=10.0).contains(&seconds) { return usage_error("--seconds must be between 0.1 and 10."); }
            let (Some(pan_control), Some(tilt_control)) = (snapshot.control("pan_speed"), snapshot.control("tilt_speed")) else { return fail("This video node has no pan/tilt speed controls.") };
            let axis_pan = crate::ui::joystick::to_axis((pan / 100.0).clamp(-1.0, 1.0), pan_control.min, pan_control.max);
            let axis_tilt = crate::ui::joystick::to_axis((tilt / 100.0).clamp(-1.0, 1.0), tilt_control.min, tilt_control.max);
            // Whatever happens next, stop on Ctrl-C.
            let stop_device = device.to_string();
            let _ = ctrlc::set_handler(move || { let _ = stop_gimbal(&stop_device); std::process::exit(130); });
            eprintln!("Moving the gimbal for {seconds:.1} s (pan {axis_pan}, tilt {axis_tilt}). Press Ctrl-C to stop.");
            if let Err(error) = backend::run_v4l2(device, &format!("--set-ctrl=pan_speed={axis_pan},tilt_speed={axis_tilt}")) {
                let _ = stop_gimbal(device);
                return fail(&format!("Could not start the move: {error}"));
            }
            std::thread::sleep(Duration::from_millis((seconds * 1000.0) as u64));
            stop_gimbal(device)
        }
        _ => usage_error("gimbal needs one of: position, stop, step --pan N --tilt N, move --pan N --tilt N [--seconds S]"),
    }
}

fn stop_gimbal(device: &str) -> i32 {
    let mut last = String::new();
    for attempt in 0..4 {
        match backend::run_v4l2(device, "--set-ctrl=pan_speed=0,tilt_speed=0") {
            Ok(_) => { eprintln!("Gimbal stopped."); return OK; }
            Err(error) => { last = error; std::thread::sleep(Duration::from_millis(40 * (attempt + 1))); }
        }
    }
    fail(&format!("Could not confirm the gimbal stopped: {last}"))
}

fn preset(options: &Options, device: &str, snapshot: &Snapshot) -> i32 {
    let _ = snapshot;
    let name = options.positional.get(1);
    match (options.positional.first().map(String::as_str), name) {
        (Some("list"), _) => {
            let saved = current_config().presets;
            if options.json { println!("{}", json!(saved.iter().map(|p| json!({ "name": p.name, "settings": p.settings.len() })).collect::<Vec<_>>())); }
            else if saved.is_empty() { println!("No presets yet. Save one with: preset save NAME"); }
            else { for p in saved { println!("{}  ({} settings)", p.name, p.settings.len()); } }
            OK
        }
        (Some("apply"), Some(name)) => match current_config().presets.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
            Some(found) if presets::moves_gimbal(found) && !options.yes => {
                eprintln!("This preset includes a gimbal position, so the camera will turn.\nClear the space around it, then run the command again with --yes.");
                NEEDS_YES
            }
            Some(found) => match presets::apply(device, found) {
                Ok(message) => { println!("{}", if message.is_empty() { format!("Applied {}", found.name) } else { message }); OK }
                Err(error) => fail(&format!("Some settings in {} were not applied: {error}", found.name)),
            },
            None => fail(&format!("There is no preset called `{name}`.")),
        },
        (Some("save"), Some(name)) => match presets::capture(device, name.clone()) {
            Ok(captured) => {
                let result = update_config_checked(|c| {
                    if let Some(existing) = c.presets.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) { *existing = captured.clone(); }
                    else if c.presets.len() < 10 { c.presets.push(captured.clone()); }
                });
                if current_config().presets.iter().all(|p| !p.name.eq_ignore_ascii_case(name)) { return fail("You can keep up to 10 presets. Delete one first."); }
                match result { Ok(()) => { println!("Saved {name}"); OK } Err(error) => fail(&format!("Could not save {name}: {error}")) }
            }
            Err(error) => fail(&format!("Could not read the camera's settings: {error}")),
        },
        (Some("delete"), Some(name)) => {
            let before = current_config().presets.len();
            let result = update_config_checked(|c| c.presets.retain(|p| !p.name.eq_ignore_ascii_case(name)));
            if current_config().presets.len() == before { return fail(&format!("There is no preset called `{name}`.")); }
            match result { Ok(()) => { println!("Deleted {name}"); OK } Err(error) => fail(&format!("Could not delete {name}: {error}")) }
        }
        _ => usage_error("preset needs one of: list, apply NAME, save NAME, delete NAME"),
    }
}

fn slot_number(text: Option<&String>) -> Result<u8, String> {
    text.and_then(|t| t.parse::<u8>().ok()).filter(|n| (1..=3).contains(n)).map(|n| n - 1).ok_or_else(|| "Give a position number from 1 to 3.".to_string())
}

fn position(options: &Options, device: &str) -> i32 {
    let confirm = |what: &str| -> Option<i32> {
        if options.yes { None } else { eprintln!("{what}\nRun the command again with --yes to go ahead."); Some(NEEDS_YES) }
    };
    match options.positional.first().map(String::as_str) {
        Some("list") => match backend::read_camera_presets(device) {
            Ok(entries) => {
                if options.json {
                    println!("{}", json!(entries.iter().map(|e| json!({ "position": e.slot + 1, "name": e.name, "pan_degrees": e.pan_degrees, "tilt_degrees": e.tilt_degrees, "zoom": e.zoom })).collect::<Vec<_>>()));
                } else if entries.is_empty() { println!("The camera has no saved positions."); }
                else { for e in entries { println!("{}  {}  pan {}, tilt {}, zoom {:.2}×", e.slot + 1, e.name, backend::angles::format(e.pan_degrees), backend::angles::format(e.tilt_degrees), e.zoom); } }
                OK
            }
            Err(error) => fail(&format!("Could not read the camera's saved positions: {error}")),
        },
        Some("go") => {
            let slot = match slot_number(options.positional.get(1)) { Ok(s) => s, Err(e) => return usage_error(&e) };
            if let Some(code) = confirm(&format!("This turns the gimbal to position {}. Clear the space around the camera first.", slot + 1)) { return code; }
            match backend::recall_camera_preset(device, slot) { Ok(()) => { println!("Moving to position {}", slot + 1); OK } Err(e) => fail(&format!("Could not move: {e}")) }
        }
        Some("save") => {
            let slot = match slot_number(options.positional.get(1)) { Ok(s) => s, Err(e) => return usage_error(&e) };
            let Some(name) = options.positional.get(2) else { return usage_error("position save needs a number and a name: position save 1 Desk") };
            let exists = backend::read_camera_presets(device).map(|e| e.iter().any(|x| x.slot == slot)).unwrap_or(false);
            match backend::save_camera_preset(device, slot, name, exists) { Ok(()) => { println!("Saved position {}", slot + 1); OK } Err(e) => fail(&format!("Could not save: {e}")) }
        }
        Some("delete") => {
            let slot = match slot_number(options.positional.get(1)) { Ok(s) => s, Err(e) => return usage_error(&e) };
            if let Some(code) = confirm(&format!("This removes position {} from the camera. It cannot be undone.", slot + 1)) { return code; }
            match backend::delete_camera_preset(device, slot) { Ok(()) => { println!("Deleted position {}", slot + 1); OK } Err(e) => fail(&format!("Could not delete: {e}")) }
        }
        _ => usage_error("position needs one of: list, go N, save N NAME, delete N"),
    }
}

fn startup(options: &Options, device: &str) -> i32 {
    match options.positional.first().map(String::as_str) {
        Some("show") | None => match backend::read_initial_camera_state(device) {
            Ok(s) => {
                if options.json { println!("{}", json!({ "pan_degrees": s.pan, "tilt_degrees": s.tilt, "zoom": s.zoom, "source": s.source })); }
                else { println!("pan {}, tilt {}, zoom {:.2}×", backend::angles::format(s.pan), backend::angles::format(s.tilt), s.zoom); }
                OK
            }
            Err(e) => fail(&format!("Could not read the start-up position: {e}")),
        },
        Some("set") => {
            if !options.yes { eprintln!("From now on the camera will point where it is pointing now each time it powers on.\nRun the command again with --yes to go ahead."); return NEEDS_YES; }
            match backend::set_initial_camera_state(device) { Ok(_) => { println!("Start-up position saved"); OK } Err(e) => fail(&format!("Could not save the start-up position: {e}")) }
        }
        _ => usage_error("startup needs one of: show, set"),
    }
}

fn capture(options: &Options, device: &str) -> i32 {
    match options.positional.first().map(String::as_str) {
        Some("list") => match video::capabilities(device) {
            Ok(caps) => {
                if options.json {
                    println!("{}", json!(caps.iter().map(|c| json!({ "format": c.pixel_format, "width": c.width, "height": c.height, "fps": c.frame_rates_milli.iter().map(|r| f64::from(*r) / 1000.0).collect::<Vec<_>>() })).collect::<Vec<_>>()));
                } else {
                    for c in caps { println!("{}  {}x{}  {}", c.pixel_format, c.width, c.height, c.frame_rates_milli.iter().map(|r| format!("{:.0} fps", f64::from(*r) / 1000.0)).collect::<Vec<_>>().join(", ")); }
                }
                OK
            }
            Err(e) => fail(&format!("Could not read the capture modes: {e}")),
        },
        Some("get") | None => match video::current_mode(device) {
            Some(m) => {
                if options.json { println!("{}", json!({ "format": m.pixel_format, "width": m.width, "height": m.height, "fps": f64::from(m.frame_rate_milli) / 1000.0 })); }
                else { println!("{}x{}  {}  {:.0} fps", m.width, m.height, m.pixel_format, f64::from(m.frame_rate_milli) / 1000.0); }
                OK
            }
            None => fail("The camera did not report a capture mode."),
        },
        Some("set") => {
            let [size, format, fps] = match &options.positional[1..] { [a, b, c] => [a, b, c], _ => return usage_error("capture set needs a size, a format and a frame rate: capture set 1920x1080 MJPG 30") };
            let Some((w, h)) = size.to_lowercase().split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))) else { return usage_error("The size looks like 1920x1080.") };
            let Ok(rate) = fps.parse::<f64>() else { return usage_error("The frame rate is a number, for example 30.") };
            let mode = video::VideoMode { width: w, height: h, pixel_format: format.to_uppercase(), frame_rate_milli: (rate * 1000.0).round() as u32 };
            let supported = video::capabilities(device).map(|caps| caps.iter().any(|c| c.pixel_format == mode.pixel_format && c.width == mode.width && c.height == mode.height && c.frame_rates_milli.contains(&mode.frame_rate_milli))).unwrap_or(false);
            if !supported { return fail("The camera does not list that mode. Run `capture list` to see what it offers."); }
            match video::set_mode(device, &mode) {
                Ok(()) => {
                    let saved = config::VideoModeConfig { width: mode.width, height: mode.height, pixel_format: mode.pixel_format.clone(), frame_rate_milli: mode.frame_rate_milli };
                    let _ = update_config_checked(|c| c.video_mode = Some(saved));
                    println!("Capture mode {}x{} {} at {:.0} fps", mode.width, mode.height, mode.pixel_format, rate);
                    OK
                }
                Err(e) => fail(&format!("The camera did not accept that mode: {e}")),
            }
        }
        _ => usage_error("capture needs one of: list, get, set SIZE FORMAT FPS"),
    }
}

// ── Help and completion ───────────────────────────────────────────────────

fn help(topic: Option<&str>) -> String {
    let _ = topic;
    let groups: Vec<String> = Group::ALL.iter().map(|g| group_slug(*g)).collect();
    format!("\
tiny4lite-control: control the OBSBOT Tiny 2 Lite from a window or a terminal

With no command, a window opens (--full, --slim or --preview choose which).

Settings (every setting the windows show):
  list [TAB|GROUP]           Show all settings and their values (tabs: camera, tracking, audio, device; groups: {groups})
  get NAME                   Print one value
  set NAME VALUE             Change a setting
  toggle NAME                Flip an on/off setting
  reset NAME                 Return a slider to its default
  info NAME                  Explain a setting: values, range, what it needs
  deps                       Print the dependency map (what blocks what) as Markdown
  service run [--once]       The background service: restores the camera when it is plugged in (needs no window)
  service status | config    Is it running; what it is set to do
  service set KEY VALUE      restore on|off, preset NAME|none, gimbal on|off, sleep on|off, sleep-after MINUTES
  service install [TARGET] [--write]   Start it with your session: systemd, xdg, hyprland, niri, sway, i3, river, mango, labwc, openbox, bspwm, shell, openrc, runit, s6, dinit, cron, udev
  service targets | uninstall          List the targets; remove what --write made

Other things the windows do:
  status                     Power, exposure, focus, tracking and gimbal position
  gimbal position|stop       Read the gimbal position, or stop it
  gimbal centre --yes            Turn to pan 0°, tilt 0° (the camera moves, so it needs --yes)
  gimbal step --pan N --tilt N   Turn by N whole degrees (1 to 5 each, negative for the other way)
  gimbal move --pan N --tilt N [--seconds S]
                             Turn at N% of full speed (-100 to 100) for S seconds (default 1, up to 10)
  preset list|apply|save|delete NAME     Presets kept by this app (a preset with a saved position turns the gimbal: apply needs --yes)
  position list|go N|save N NAME|delete N    The camera's three saved positions
  startup show|set           Where the camera points when it powers on
  capture list|get|set SIZE FORMAT FPS      Format, size and frame rate sent to apps
  completions bash|zsh|fish  Print a shell completion script

Options:
  --json         Machine-readable output
  --device PATH  Use this video device (or set TINY4LITE_DEVICE); default: find the Tiny 2 Lite
  --yes, -y      Confirm actions that move the gimbal or change what the camera stores
  --force        Send a setting even though its dependency says it is unavailable

NAME is the short name from `list` (for example exposure-mode, shutter-speed, autofocus), the full id
(v4l2:auto_exposure) or the label. Values: on/off for switches, an option name for choices, and for
sliders a number in the unit shown (3200 for kelvin, +1.5 for EV, 1/60 for shutter speed, 2.5 for zoom ratio).

Examples:
  tiny4lite-control set exposure-mode manual
  tiny4lite-control set shutter-speed 1/60
  tiny4lite-control toggle autofocus
  tiny4lite-control set tracking-mode upper-body
  tiny4lite-control gimbal move --pan 30 --seconds 2
  tiny4lite-control list white-balance --json

Exit codes: 0 done, 1 failed, 2 wrong usage, 3 unavailable because of another setting,
4 needs --yes, 5 no camera found.

Nothing is sent to the camera except what you ask for. Moving the gimbal or sleeping the camera
physically moves it: clear the space around it first.
", groups = groups.join(", "))
}

/// Names the shell completion scripts ask for.
fn complete(args: &[String]) -> i32 {
    let device = backend::discover_tiny4lite().into_iter().next();
    let snapshot = device.as_deref().map(Snapshot::read).unwrap_or_default();
    // Without a camera, complete from the whole catalogue.
    let settings: Vec<&'static Setting> = if snapshot.controls.is_empty() { registry::all().iter().collect() } else { all_settings(&snapshot) };
    match args.first().map(String::as_str) {
        Some("settings") => { for s in settings { println!("{}", alias(s)); } }
        Some("groups") => {
            for tab in ["camera", "tracking", "audio", "device"] { println!("{tab}"); }
            for g in Group::ALL { println!("{}", group_slug(g)); }
        }
        Some("values") => if let Some(setting) = args.get(1).and_then(|n| resolve_any(&settings, n)) {
            match &setting.kind {
                Kind::Toggle => { println!("on\noff"); }
                Kind::Choice(_) | Kind::DriverMenu => for o in setting.options(&snapshot) { println!("{}", normalise_hyphen(o.label)); },
                Kind::Slider(_) => {}
            }
        },
        Some("presets") => for p in current_config().presets { println!("{}", p.name); },
        _ => {}
    }
    OK
}

fn resolve_any(settings: &[&'static Setting], name: &str) -> Option<&'static Setting> {
    let wanted = normalise(name);
    settings.iter().find(|s| normalise(&alias(s)) == wanted || normalise(s.id) == wanted || normalise(&technical_name(s)) == wanted).copied()
}

fn normalise_hyphen(label: &str) -> String {
    label.to_lowercase().replace(' ', "-")
}

fn completions(shell: Option<&str>) -> i32 {
    let commands = COMMANDS.iter().filter(|c| !c.starts_with("__") && **c != "help").copied().collect::<Vec<_>>().join(" ");
    let script = match shell {
        Some("bash") => format!(r#"# tiny4lite-control completion for bash: source this file
_tiny4lite_control() {{
    local cur prev cmd
    cur="${{COMP_WORDS[COMP_CWORD]}}"; prev="${{COMP_WORDS[COMP_CWORD-1]}}"; cmd="${{COMP_WORDS[1]}}"
    if [ "$COMP_CWORD" -eq 1 ]; then COMPREPLY=($(compgen -W "{commands} --slim --full --preview --help --version" -- "$cur")); return; fi
    case "$cmd" in
        get|set|toggle|reset|info)
            if [ "$COMP_CWORD" -eq 2 ]; then COMPREPLY=($(compgen -W "$(tiny4lite-control __complete settings 2>/dev/null)" -- "$cur"))
            elif [ "$cmd" = set ] && [ "$COMP_CWORD" -eq 3 ]; then COMPREPLY=($(compgen -W "$(tiny4lite-control __complete values "$prev" 2>/dev/null)" -- "$cur")); fi ;;
        list) COMPREPLY=($(compgen -W "$(tiny4lite-control __complete groups 2>/dev/null) --json" -- "$cur")) ;;
        preset) if [ "$COMP_CWORD" -eq 2 ]; then COMPREPLY=($(compgen -W "list apply save delete" -- "$cur")); else COMPREPLY=($(compgen -W "$(tiny4lite-control __complete presets 2>/dev/null)" -- "$cur")); fi ;;
        gimbal) COMPREPLY=($(compgen -W "position stop move step centre --pan --tilt --seconds" -- "$cur")) ;;
        position) COMPREPLY=($(compgen -W "list go save delete --yes" -- "$cur")) ;;
        startup) COMPREPLY=($(compgen -W "show set --yes" -- "$cur")) ;;
        capture) COMPREPLY=($(compgen -W "list get set" -- "$cur")) ;;
        completions) COMPREPLY=($(compgen -W "bash zsh fish" -- "$cur")) ;;
    esac
}}
complete -F _tiny4lite_control tiny4lite-control
"#),
        Some("zsh") => format!(r#"#compdef tiny4lite-control
# tiny4lite-control completion for zsh: put this file in your fpath as _tiny4lite-control
_tiny4lite_control() {{
    local -a cmds; cmds=({commands})
    if (( CURRENT == 2 )); then compadd -- $cmds --slim --full --preview --help --version; return; fi
    case "$words[2]" in
        get|set|toggle|reset|info)
            if (( CURRENT == 3 )); then compadd -- ${{(f)"$(tiny4lite-control __complete settings 2>/dev/null)"}}
            elif [[ "$words[2]" == set && CURRENT == 4 ]]; then compadd -- ${{(f)"$(tiny4lite-control __complete values "$words[3]" 2>/dev/null)"}}; fi ;;
        list) compadd -- ${{(f)"$(tiny4lite-control __complete groups 2>/dev/null)"}} --json ;;
        preset) if (( CURRENT == 3 )); then compadd list apply save delete; else compadd -- ${{(f)"$(tiny4lite-control __complete presets 2>/dev/null)"}}; fi ;;
        gimbal) compadd position stop move step centre --pan --tilt --seconds ;;
        position) compadd list go save delete --yes ;;
        startup) compadd show set --yes ;;
        capture) compadd list get set ;;
        completions) compadd bash zsh fish ;;
    esac
}}
_tiny4lite_control "$@"
"#),
        Some("fish") => format!(r#"# tiny4lite-control completion for fish: save as ~/.config/fish/completions/tiny4lite-control.fish
complete -c tiny4lite-control -f
complete -c tiny4lite-control -n '__fish_use_subcommand' -a '{commands}'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from get set toggle reset info; and test (count (commandline -opc)) -eq 2' -a '(tiny4lite-control __complete settings 2>/dev/null)'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from set; and test (count (commandline -opc)) -eq 3' -a '(tiny4lite-control __complete values (commandline -opc)[3] 2>/dev/null)'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from list' -a '(tiny4lite-control __complete groups 2>/dev/null) --json'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from preset; and test (count (commandline -opc)) -eq 2' -a 'list apply save delete'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from preset; and test (count (commandline -opc)) -ge 3' -a '(tiny4lite-control __complete presets 2>/dev/null)'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from gimbal' -a 'position stop move step centre'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from position' -a 'list go save delete'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from startup' -a 'show set'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from capture' -a 'list get set'
complete -c tiny4lite-control -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish'
"#),
        _ => return usage_error("completions needs a shell: bash, zsh or fish"),
    };
    print!("{script}");
    OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_has_a_unique_command_name() {
        let mut names = std::collections::HashSet::new();
        for setting in registry::all() {
            assert!(names.insert(normalise(&alias(setting))), "two settings are both called `{}`", alias(setting));
        }
        // The old driver-style names must not collide with another setting's new name.
        for setting in registry::all() {
            let old = normalise(&technical_name(setting));
            let clash = registry::all().iter().any(|other| other.id != setting.id && normalise(&alias(other)) == old);
            assert!(!clash, "{} clashes with another setting's name", technical_name(setting));
        }
    }

    #[test]
    fn names_follow_the_labels_in_the_windows() {
        assert_eq!(alias(registry::find("v4l2:exposure_time_absolute").unwrap()), "shutter-speed");
        assert_eq!(alias(registry::find("v4l2:focus_automatic_continuous").unwrap()), "autofocus");
        assert_eq!(alias(registry::find("obsbot:auto_exposure_mode").unwrap()), "metering-priority");
        assert_eq!(alias(registry::find("obsbot:ai_tracking_mode").unwrap()), "tracking-mode");
    }

    #[test]
    fn on_off_words_are_understood() {
        for yes in ["on", "ON", "true", "yes", "1", "enable"] { assert_eq!(parse_bool(yes), Ok(true)); }
        for no in ["off", "False", "no", "0", "disabled"] { assert_eq!(parse_bool(no), Ok(false)); }
        assert!(parse_bool("maybe").is_err());
    }

    #[test]
    fn sliders_accept_the_units_the_windows_show() {
        assert_eq!(parse_slider(Fmt::Shutter, "1/60").unwrap(), 167.0);
        assert_eq!(parse_slider(Fmt::Shutter, "1/30 s").unwrap(), 333.0);
        assert_eq!(parse_slider(Fmt::Shutter, "0.5s").unwrap(), 5000.0);
        assert_eq!(parse_slider(Fmt::Shutter, "330").unwrap(), 330.0);
        assert_eq!(parse_slider(Fmt::Ev, "0").unwrap(), 9.0);
        assert_eq!(parse_slider(Fmt::Ev, "+3 EV").unwrap(), 18.0);
        assert_eq!(parse_slider(Fmt::Ev, "-3").unwrap(), 0.0);
        assert!(parse_slider(Fmt::Ev, "5").is_err());
        assert_eq!(parse_slider(Fmt::Kelvin, "5600K").unwrap(), 5600.0);
        assert_eq!(parse_slider(Fmt::Ratio, "2.5x").unwrap(), 2.5);
        assert_eq!(parse_slider(Fmt::Level, "off").unwrap(), 0.0);
        assert_eq!(parse_slider(Fmt::Plain, "40%").unwrap(), 40.0);
        assert!(parse_slider(Fmt::Plain, "loud").is_err());
    }

    #[test]
    fn choices_match_labels_loosely_and_by_number() {
        let snapshot = Snapshot::default();
        let mode = registry::find("obsbot:ai_tracking_mode").unwrap();
        assert_eq!(parse_value(mode, &snapshot, "upper-body").unwrap(), Value::Int(2));
        assert_eq!(parse_value(mode, &snapshot, "Upper body").unwrap(), Value::Int(2));
        assert_eq!(parse_value(mode, &snapshot, "close_up").unwrap(), Value::Int(3));
        assert!(parse_value(mode, &snapshot, "sideways").unwrap_err().contains("No tracking, Normal, Upper body"));
        assert!(parse_value(mode, &snapshot, "hand").is_err(), "hand tracking is not offered");
        assert_eq!(parse_value(mode, &snapshot, "group").unwrap(), Value::Int(6));
        let fov = registry::find("obsbot:fov").unwrap();
        assert_eq!(parse_value(fov, &snapshot, "60").unwrap(), Value::Int(5));
    }

    #[test]
    fn names_resolve_by_short_name_id_or_label() {
        let snapshot = Snapshot::default();
        // Vendor commands are always offered; standard controls only once the camera reports them.
        assert_eq!(resolve(&snapshot, "mirror-image").unwrap().id, "obsbot:mirror_image");
        assert_eq!(resolve(&snapshot, "obsbot:mirror_image").unwrap().id, "obsbot:mirror_image");
        assert_eq!(resolve(&snapshot, "Pop filter").unwrap().id, "obsbot:pop_filter");
        assert!(resolve(&snapshot, "no-such-thing").err().unwrap().contains("no setting called"));
    }

    #[test]
    fn negative_numbers_are_values_not_flags() {
        let args: Vec<String> = ["--tilt", "-20", "--pan", "30", "--json"].iter().map(|s| s.to_string()).collect();
        let options = Options::parse(&args).unwrap();
        assert_eq!(options.flag("--tilt"), Some("-20"));
        assert_eq!(options.flag("--pan"), Some("30"));
        assert!(options.json);
    }

    #[test]
    fn window_flags_do_not_trigger_the_command_line() {
        assert!(!is_cli(&["--slim".to_string()]));
        assert!(!is_cli(&[]));
        assert!(is_cli(&["list".to_string()]));
        assert!(is_cli(&["--help".to_string()]));
    }
}
