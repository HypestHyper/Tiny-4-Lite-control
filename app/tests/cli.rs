//! Runs the real binary as a terminal tool against the synthetic camera.
//! No display and no hardware are involved.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Run {
    config: PathBuf,
}

impl Run {
    fn new(name: &str) -> Run {
        let config = std::env::temp_dir().join(format!("t4-cli-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&config).unwrap();
        Run { config }
    }

    fn command(&self, state: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
            .args(args)
            .env("TINY4LITE_UI_TEST", "1")
            .env("TINY4LITE_UI_TEST_STATE", state)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_CACHE_HOME", &self.config)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .expect("the binary runs")
    }

    fn run(&self, args: &[&str]) -> Output { self.command("auto", args) }
}

fn text(bytes: &[u8]) -> String { String::from_utf8_lossy(bytes).to_string() }

#[test]
fn works_without_a_display_and_lists_every_group() {
    let run = Run::new("list");
    let out = run.run(&["list"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let listing = text(&out.stdout);
    for group in ["Exposure", "White balance", "Focus", "Picture", "AI tracking", "Gestures", "Microphone", "Light", "Power"] {
        assert!(listing.contains(group), "missing group {group}:\n{listing}");
    }
}

#[test]
fn every_setting_can_be_read_and_set_from_the_terminal() {
    let run = Run::new("all");
    let out = run.run(&["list", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let json: Value = serde_json::from_slice(&out.stdout).expect("list --json is valid JSON");
    let settings = json["settings"].as_array().expect("settings array");
    assert!(settings.len() >= 36, "only {} settings listed", settings.len());
    // Every vendor command the windows offer is reachable from the terminal.
    let ids: Vec<&str> = settings.iter().filter_map(|s| s["id"].as_str()).collect();
    for id in ["obsbot:hdr", "obsbot:mirror_image", "obsbot:pop_filter", "obsbot:ai_tracking_mode", "obsbot:tracking_speed",
               "obsbot:sleep", "obsbot:led_brightness", "obsbot:fov", "obsbot:focus_mode", "obsbot:auto_exposure_mode",
               "obsbot:exposure_compensation", "obsbot:mic_enabled", "obsbot:noise_reduction", "obsbot:radio_distance",
               "obsbot:auto_gain", "obsbot:mic_during_sleep", "obsbot:gesture_zoom", "obsbot:gesture_zoom_ratio",
               "obsbot:gesture_target", "obsbot:gesture_dynamic_zoom", "obsbot:gesture_dynamic_zoom_direction"] {
        assert!(ids.contains(&id), "{id} cannot be reached from the command line");
    }
    for setting in settings {
        let name = setting["name"].as_str().unwrap();
        let get = run.run(&["get", name]);
        assert_eq!(get.status.code(), Some(0), "get {name}: {}", text(&get.stderr));

        // A value the setting accepts.
        let value: String = match setting["kind"].as_str().unwrap() {
            "toggle" => "on".into(),
            "choice" => setting["options"][0].as_str().unwrap().to_string(),
            _ => {
                let range = &setting["range"];
                let default = range["default"].as_f64().unwrap_or_else(|| range["min"].as_f64().unwrap());
                // Express it in the unit the setting uses, as a person would type it.
                let display = run.run(&["info", name]);
                assert_eq!(display.status.code(), Some(0));
                match setting["id"].as_str().unwrap() {
                    "obsbot:exposure_compensation" => "0".into(),
                    "v4l2:exposure_time_absolute" => "1/30".into(),
                    "v4l2:white_balance_temperature" => "5000".into(),
                    _ => format!("{default}"),
                }
            }
        };
        let set = run.run(&["set", name, &value, "--yes"]);
        let code = set.status.code().unwrap();
        // 0 done, 3 unavailable because of another setting. Never "wrong usage" or a crash.
        assert!(code == 0 || code == 3, "set {name} {value} exited {code}: {}", text(&set.stderr));
    }
}

#[test]
fn a_blocked_setting_explains_what_to_change_and_exits_3() {
    let run = Run::new("blocked");
    let out = run.run(&["set", "red-balance", "1000"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(text(&out.stderr).contains("Turn off Auto white balance"), "{}", text(&out.stderr));
    // The same setting works once its dependency is met (manual white balance fixture).
    let manual = run.command("manual", &["set", "red-balance", "1000"]);
    assert_eq!(manual.status.code(), Some(0), "{}", text(&manual.stderr));
    // --force sends it anyway.
    let forced = run.run(&["set", "red-balance", "1000", "--force"]);
    assert_eq!(forced.status.code(), Some(0));
}

#[test]
fn bad_input_exits_2_and_says_what_is_valid() {
    let run = Run::new("usage");
    let out = run.run(&["set", "tracking-mode", "sideways"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("Upper body"));
    let out = run.run(&["set", "exposure-mode"]);
    assert_eq!(out.status.code(), Some(2));
    let out = run.run(&["get", "exposur"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("Did you mean"));
    let out = run.run(&["set", "zoom", "9000"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("must be between"));
}

#[test]
fn sleeping_the_camera_needs_yes() {
    let run = Run::new("sleep");
    let out = run.run(&["set", "camera-power", "asleep"]);
    assert_eq!(out.status.code(), Some(4));
    assert!(text(&out.stderr).contains("--yes"));
    let out = run.run(&["set", "camera-power", "asleep", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[test]
fn toggle_reset_and_info_work() {
    let run = Run::new("misc");
    let out = run.run(&["toggle", "autofocus"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("off"));
    let out = run.run(&["reset", "zoom"]);
    assert_eq!(out.status.code(), Some(0));
    let out = run.run(&["info", "shutter-speed"]);
    assert!(text(&out.stdout).contains("Needs:"));
    // The camera does not report mirror image, so there is nothing to flip.
    let out = run.run(&["toggle", "mirror-image"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn gimbal_move_always_ends_with_a_stop() {
    let run = Run::new("gimbal");
    let log = run.config.join("commands.log");
    let out = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["gimbal", "move", "--pan", "50", "--tilt", "-25", "--seconds", "0.2"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let commands = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = commands.lines().collect();
    assert!(lines.first().unwrap().contains("pan_speed=40,tilt_speed=-30"), "{commands}");
    assert!(lines.last().unwrap().ends_with("--set-ctrl=pan_speed=0,tilt_speed=0"), "{commands}");
    // Limits are enforced before anything is sent.
    let out = run.run(&["gimbal", "move", "--pan", "50", "--seconds", "60"]);
    assert_eq!(out.status.code(), Some(2));
    let out = run.run(&["gimbal", "move"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn presets_positions_startup_and_capture_commands_run() {
    let run = Run::new("rest");
    assert_eq!(run.run(&["preset", "list"]).status.code(), Some(0));
    assert_eq!(run.run(&["preset", "save", "Desk"]).status.code(), Some(0));
    assert!(text(&run.run(&["preset", "list"]).stdout).contains("Desk"));
    // The synthetic camera has absolute pan/tilt, so this preset turns the gimbal and asks first.
    assert_eq!(run.run(&["preset", "apply", "desk"]).status.code(), Some(4));
    let applied = run.run(&["preset", "apply", "desk", "--yes"]);
    assert_eq!(applied.status.code(), Some(0), "{}", text(&applied.stderr));
    assert_eq!(run.run(&["preset", "delete", "Desk"]).status.code(), Some(0));
    assert_eq!(run.run(&["preset", "apply", "Desk"]).status.code(), Some(1));
    assert_eq!(run.run(&["position", "go", "1"]).status.code(), Some(4));
    assert_eq!(run.run(&["position", "delete", "2"]).status.code(), Some(4));
    assert_eq!(run.run(&["startup", "set"]).status.code(), Some(4));
    assert_eq!(run.run(&["startup", "show"]).status.code(), Some(0));
    let capture = run.run(&["capture", "list"]);
    assert!(text(&capture.stdout).contains("MJPG"));
    assert_eq!(run.run(&["capture", "set", "1920x1080", "MJPG", "30"]).status.code(), Some(0));
    assert_eq!(run.run(&["capture", "set", "123x45", "MJPG", "30"]).status.code(), Some(1));
    assert_eq!(run.run(&["status"]).status.code(), Some(0));
}

#[test]
fn help_and_completions_describe_the_settings() {
    let run = Run::new("help");
    let help = text(&run.run(&["help"]).stdout);
    for word in ["set NAME VALUE", "gimbal move", "--json", "Exit codes"] { assert!(help.contains(word), "help lacks {word}"); }
    let names = text(&run.run(&["__complete", "settings"]).stdout);
    assert!(names.contains("shutter-speed") && names.contains("tracking-mode"));
    let values = text(&run.run(&["__complete", "values", "tracking-mode"]).stdout);
    assert!(values.contains("upper-body"));
    for shell in ["bash", "zsh", "fish"] {
        let script = text(&run.run(&["completions", shell]).stdout);
        assert!(script.contains("tiny4lite-control"), "{shell} script is empty");
    }
    assert_eq!(run.run(&["completions", "powershell"]).status.code(), Some(2));
}
