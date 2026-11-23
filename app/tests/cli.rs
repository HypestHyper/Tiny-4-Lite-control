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
    // A whole window tab, or a single group, can be listed.
    let tracking = text(&run.run(&["list", "tracking"]).stdout);
    assert!(tracking.contains("AI tracking") && tracking.contains("Gestures") && !tracking.contains("Exposure"), "{tracking}");
    assert!(text(&run.run(&["list", "focus"]).stdout).contains("autofocus-priority"));
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
    for id in ["obsbot:mirror_image", "obsbot:pop_filter", "obsbot:ai_tracking_mode", "obsbot:tracking_speed",
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
                    "v4l2:zoom_absolute" => "1x".into(),
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
    // With manual white balance by temperature, red is still unavailable, and says why.
    let by_temperature = run.command("manual", &["set", "red-balance", "1000"]);
    assert_eq!(by_temperature.status.code(), Some(3));
    assert!(text(&by_temperature.stderr).contains("Choose Red & blue"), "{}", text(&by_temperature.stderr));
    // It works once white balance is done by red and blue.
    let choose = run.command("manual", &["set", "manual-white-balance-by", "Red & blue"]);
    assert_eq!(choose.status.code(), Some(0), "{}", text(&choose.stderr));
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
fn only_the_light_going_off_asks_first() {
    let run = Run::new("sleep");
    // Sleep moves the gimbal, and asks for nothing: it is a command like any other.
    let out = run.run(&["set", "camera-power", "sleep"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    // Turning the camera light off is the one setting that still asks, because nobody nearby can then
    // tell when the camera is on.
    let out = run.run(&["set", "led-brightness", "off"]);
    assert_eq!(out.status.code(), Some(4));
    assert!(text(&out.stderr).contains("--yes"));
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
    assert!(lines.first().unwrap().contains("pan_speed=-40,tilt_speed=-30"), "{commands}");
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
    let applied = run.run(&["preset", "apply", "desk"]);
    assert_eq!(applied.status.code(), Some(0), "{}", text(&applied.stderr));
    assert_eq!(run.run(&["preset", "delete", "Desk"]).status.code(), Some(0));
    assert_eq!(run.run(&["preset", "apply", "Desk"]).status.code(), Some(1));
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

#[test]
fn every_setting_named_in_the_help_text_exists() {
    let run = Run::new("help-names");
    let help = text(&run.run(&["help"]).stdout);
    // Pull each `tiny4lite-control get|set|toggle|reset|info NAME` example and the "for example" names.
    let mut names: Vec<String> = Vec::new();
    for line in help.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(at) = words.iter().position(|w| matches!(*w, "get" | "set" | "toggle" | "reset" | "info")) {
            if words.first() == Some(&"tiny4lite-control") { if let Some(name) = words.get(at + 1) { names.push(name.to_string()); } }
        }
    }
    let example = help.lines().find(|l| l.contains("for example exposure-mode")).expect("the naming sentence");
    let inside = example.split("for example").nth(1).unwrap().split(')').next().unwrap();
    names.extend(inside.split(',').map(|n| n.trim().to_string()));
    assert!(names.len() >= 6, "found only {names:?}");
    for name in names {
        let out = run.run(&["get", &name]);
        assert_eq!(out.status.code(), Some(0), "the help names `{name}` but it is not a setting: {}", text(&out.stderr));
    }
}

#[test]
fn gimbal_steps_are_small_signed_and_bounded() {
    let run = Run::new("step");
    let log = run.config.join("step.log");
    let step = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(args)
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    let out = step(&["gimbal", "step", "--pan", "1", "--tilt", "-2"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let sent = std::fs::read_to_string(&log).unwrap();
    // 3600 driver units are one degree.
    assert!(sent.contains("--set-ctrl=pan_absolute=3600"), "{sent}");
    assert!(sent.contains("--set-ctrl=tilt_absolute=-7200"), "{sent}");
    // Nothing is sent for a step that is too big, zero, or missing.
    std::fs::remove_file(&log).unwrap();
    for args in [&["gimbal", "step", "--pan", "6"][..], &["gimbal", "step", "--tilt", "0"], &["gimbal", "step"], &["gimbal", "step", "--pan", "x"]] {
        assert_eq!(step(args).status.code(), Some(2), "{args:?}");
    }
    assert!(!log.exists(), "a rejected step must not reach the camera");
}

#[test]
fn camera_positions_and_start_up_position_flow() {
    let run = Run::new("positions");
    assert_eq!(run.run(&["position", "list"]).status.code(), Some(0));
    assert_eq!(run.run(&["position", "save", "1", "Desk"]).status.code(), Some(0));
    assert_eq!(run.run(&["position", "save", "4", "Nope"]).status.code(), Some(2));
    assert_eq!(run.run(&["position", "save", "1", "Bad \u{00e9}"]).status.code(), Some(1), "names must be plain ASCII");
    // Moving the gimbal asks for nothing; deleting a position still needs --yes.
    assert_eq!(run.run(&["position", "go", "1"]).status.code(), Some(0));
    assert_eq!(run.run(&["position", "delete", "1"]).status.code(), Some(4));
    assert_eq!(run.run(&["position", "delete", "1", "--yes"]).status.code(), Some(0));
    assert_eq!(run.run(&["startup", "set", "--yes"]).status.code(), Some(0));
    let shown = text(&run.run(&["startup", "show"]).stdout);
    assert!(!shown.contains("-0.0"), "negative zero in {shown}");
}

#[test]
fn centre_sends_one_absolute_move() {
    let run = Run::new("centre");
    let log = run.config.join("commands.log");
    let go = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(args)
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    let out = go(&["gimbal", "centre"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let commands = std::fs::read_to_string(&log).unwrap();
    assert!(commands.contains("--set-ctrl=pan_absolute=0,tilt_absolute=0"), "{commands}");
}

fn service_run(run: &Run, runtime: &std::path::Path, log: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(args)
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", runtime)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap()
}

#[test]
fn the_service_restores_settings_but_never_turns_the_gimbal() {
    let run = Run::new("service");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    // Off by default: a restore sends nothing but the saved capture mode.
    let out = service_run(&run, &runtime, &log, &["service", "run", "--once"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(!std::fs::read_to_string(&log).unwrap_or_default().contains("brightness"));
    // Remember some settings, including a position, and turn restore on.
    assert_eq!(service_run(&run, &runtime, &log, &["set", "brightness", "41"]).status.code(), Some(0));
    assert_eq!(service_run(&run, &runtime, &log, &["service", "set", "restore", "on"]).status.code(), Some(0));
    std::fs::write(&log, "").unwrap();
    let out = service_run(&run, &runtime, &log, &["service", "run", "--once"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let commands = std::fs::read_to_string(&log).unwrap();
    assert!(commands.contains("--set-ctrl=brightness=41"), "{commands}");
    assert!(!commands.contains("pan_absolute") && !commands.contains("tilt_absolute"), "the gimbal must not move:\n{commands}");
    // Nothing is left behind.
    assert!(!runtime.join("tiny4lite-control/service.lock").exists());
}

#[test]
fn service_settings_are_checked_and_listed() {
    let run = Run::new("service-config");
    let (log, runtime) = (run.config.join("c.log"), run.config.join("runtime"));
    let bad = service_run(&run, &runtime, &log, &["service", "set", "sleep-after", "0"]);
    assert_eq!(bad.status.code(), Some(2));
    let bad = service_run(&run, &runtime, &log, &["service", "set", "preset", "nonexistent"]);
    assert_eq!(bad.status.code(), Some(1));
    assert_eq!(service_run(&run, &runtime, &log, &["service", "set", "sleep", "on"]).status.code(), Some(0));
    let config = text(&service_run(&run, &runtime, &log, &["service", "config"]).stdout);
    assert!(config.contains("sleep            on"), "{config}");
    let json = text(&service_run(&run, &runtime, &log, &["service", "config", "--json"]).stdout);
    assert!(json.contains("\"sleep\":true"), "{json}");
}

#[test]
fn install_writes_a_systemd_unit_and_an_autostart_entry_only_when_asked() {
    let run = Run::new("service-install");
    let (log, runtime) = (run.config.join("i.log"), run.config.join("runtime"));
    let shown = service_run(&run, &runtime, &log, &["service", "install", "systemd"]);
    assert_eq!(shown.status.code(), Some(0));
    assert!(text(&shown.stdout).contains("ExecStart="));
    assert!(!run.config.join("systemd/user/tiny4lite-control.service").exists(), "printing must not write");
    assert_eq!(service_run(&run, &runtime, &log, &["service", "install", "systemd", "--write"]).status.code(), Some(0));
    let unit = std::fs::read_to_string(run.config.join("systemd/user/tiny4lite-control.service")).unwrap();
    assert!(unit.contains("service run") && unit.contains("Restart=on-failure"), "{unit}");
    assert_eq!(service_run(&run, &runtime, &log, &["service", "install", "xdg", "--write"]).status.code(), Some(0));
    assert!(run.config.join("autostart/tiny4lite-control-service.desktop").exists());
    // A window manager's own config is never written for you.
    let refused = service_run(&run, &runtime, &log, &["service", "install", "hyprland", "--write"]);
    assert_ne!(refused.status.code(), Some(0));
    assert!(text(&service_run(&run, &runtime, &log, &["service", "install", "niri"]).stdout).contains("spawn-sh-at-startup"));
    assert_eq!(service_run(&run, &runtime, &log, &["service", "uninstall", "systemd"]).status.code(), Some(0));
    assert!(!run.config.join("systemd/user/tiny4lite-control.service").exists());
}

#[test]
fn exposure_compensation_is_sent_as_the_standard_backlight_control() {
    let run = Run::new("ev");
    let log = run.config.join("commands.log");
    let out = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["set", "exposure-compensation", "+2"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let commands = std::fs::read_to_string(&log).unwrap();
    assert!(commands.contains("--set-ctrl=backlight_compensation=15"), "{commands}");
}

#[test]
fn the_service_puts_the_camera_to_sleep_when_it_is_told_to_stop() {
    use std::time::{Duration, Instant};
    let run = Run::new("service-shutdown");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    assert_eq!(service_run(&run, &runtime, &log, &["service", "set", "shutdown", "on"]).status.code(), Some(0));
    let mut child = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["service", "run"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DISPLAY")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Wait until it has found the camera, then stop it as a shutdown would: SIGTERM.
    let state = runtime.join("tiny4lite-control/service.json");
    let started = Instant::now();
    while !state.exists() && started.elapsed() < Duration::from_secs(10) { std::thread::sleep(Duration::from_millis(100)); }
    std::thread::sleep(Duration::from_millis(2600));
    assert!(Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap().success());
    let waited = Instant::now();
    let code = loop {
        if let Some(status) = child.try_wait().unwrap() { break status.code(); }
        assert!(waited.elapsed() < Duration::from_secs(8), "the service did not stop");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(code, Some(0));
    let commands = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(commands.contains("sleep=on"), "no sleep command at shutdown:\n{commands}");
    assert!(!runtime.join("tiny4lite-control/service.lock").exists(), "the lock was left behind");
}

#[test]
fn without_the_setting_a_stopped_service_leaves_the_camera_alone() {
    use std::time::{Duration, Instant};
    let run = Run::new("service-no-shutdown");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["service", "run"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DISPLAY")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let state = runtime.join("tiny4lite-control/service.json");
    let started = Instant::now();
    while !state.exists() && started.elapsed() < Duration::from_secs(10) { std::thread::sleep(Duration::from_millis(100)); }
    std::thread::sleep(Duration::from_millis(2600));
    assert!(Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap().success());
    assert_eq!(child.wait().unwrap().code(), Some(0));
    assert!(!std::fs::read_to_string(&log).unwrap_or_default().contains("sleep="));
}

#[test]
fn the_service_puts_the_camera_to_sleep_before_a_suspend() {
    use std::time::{Duration, Instant};
    let run = Run::new("service-suspend");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    assert_eq!(service_run(&run, &runtime, &log, &["service", "set", "suspend", "on"]).status.code(), Some(0));
    // A stand-in for dbus-monitor: after a few seconds it prints what logind prints before a suspend.
    let monitor = "sleep 4; echo 'signal time=1.0 sender=:1.4 -> destination=(null destination) serial=9 path=/org/freedesktop/login1; interface=org.freedesktop.login1.Manager; member=PrepareForSleep'; echo '   boolean true'; sleep 2; echo 'signal time=2.0 sender=:1.4 -> destination=(null destination) serial=10 path=/org/freedesktop/login1; interface=org.freedesktop.login1.Manager; member=PrepareForSleep'; echo '   boolean false'; sleep 30";
    let mut child = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["service", "run"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("TINY4LITE_TEST_SUSPEND_MONITOR", monitor)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DISPLAY")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    // Once before the suspend, and once more a few seconds after waking (the camera starts up awake).
    let count = || std::fs::read_to_string(&log).unwrap_or_default().matches("sleep=on").count();
    let slept = loop {
        if count() >= 2 { break true; }
        if started.elapsed() > Duration::from_secs(25) { break false; }
        std::thread::sleep(Duration::from_millis(100));
    };
    let seen = count();
    let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
    let _ = child.wait();
    assert!(slept, "expected sleep before the suspend and again after waking; saw {seen}");
}

#[test]
fn the_service_puts_the_camera_to_sleep_as_soon_as_a_shutdown_is_announced() {
    use std::time::{Duration, Instant};
    let run = Run::new("service-shutdown-announced");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    assert_eq!(service_run(&run, &runtime, &log, &["service", "set", "shutdown", "on"]).status.code(), Some(0));
    // A stand-in for dbus-monitor: after a few seconds it prints what logind prints before a shutdown.
    let monitor = "sleep 4; echo 'signal time=1.0 sender=:1.4 -> destination=(null destination) serial=9 path=/org/freedesktop/login1; interface=org.freedesktop.login1.Manager; member=PrepareForShutdown'; echo '   boolean true'; sleep 30";
    let mut child = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["service", "run"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("TINY4LITE_TEST_SUSPEND_MONITOR", monitor)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DISPLAY")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    // Put to sleep on the announcement itself, while the service is still running (not only when stopped).
    let slept = loop {
        if std::fs::read_to_string(&log).unwrap_or_default().contains("sleep=on") { break true; }
        if started.elapsed() > Duration::from_secs(20) { break false; }
        std::thread::sleep(Duration::from_millis(100));
    };
    let still_running = child.try_wait().unwrap().is_none();
    let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
    let _ = child.wait();
    assert!(slept && still_running, "expected sleep when the shutdown was announced (slept {slept}, running {still_running})");
}

#[test]
fn the_service_starts_again_when_its_appimage_is_replaced() {
    use std::time::{Duration, Instant};
    let run = Run::new("service-replaced");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    // A stand-in for the AppImage file the service runs from.
    let appimage = run.config.join("Tiny4LiteControl.AppImage");
    std::fs::write(&appimage, "version one").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["service", "run"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("APPIMAGE", &appimage)
        .env("INVOCATION_ID", "test")
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DISPLAY")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert!(child.try_wait().unwrap().is_none(), "it keeps running while the file is unchanged");
    // A new version saved over it, in place.
    std::fs::write(&appimage, "version two, longer").unwrap();
    let started = Instant::now();
    let code = loop {
        if let Some(status) = child.try_wait().unwrap() { break status.code(); }
        if started.elapsed() > Duration::from_secs(20) { let _ = child.kill(); break None; }
        std::thread::sleep(Duration::from_millis(100));
    };
    // Under systemd it leaves with the code systemd restarts on.
    assert_eq!(code, Some(75));
}

#[test]
fn a_capture_mode_waiting_for_a_free_camera_is_applied_by_the_service() {
    let run = Run::new("service-pending");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    let dir = run.config.join("tiny4lite-control");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[pending_video_mode]\nwidth = 1280\nheight = 720\npixel_format = \"MJPG\"\nframe_rate_milli = 30000\n").unwrap();
    let out = service_run(&run, &runtime, &log, &["service", "run", "--once"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let commands = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(commands.contains("--set-fmt-video=width=1280,height=720,pixelformat=MJPG"), "{commands}");
    // It is done once: the waiting choice is cleared and becomes the saved mode.
    let saved = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(!saved.contains("pending_video_mode"), "{saved}");
}

#[test]
fn the_service_can_be_set_up_started_stopped_and_removed_from_the_command_line() {
    let run = Run::new("service-lifecycle");
    let (log, runtime) = (run.config.join("c.log"), run.config.join("runtime"));
    // A PATH with only what the service needs, and no systemctl.
    let bin = run.config.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for program in ["kill", "sh", "sleep"] {
        let found = ["/usr/bin", "/bin"].iter().map(|d| std::path::Path::new(d).join(program)).find(|p| p.exists()).unwrap();
        let _ = std::os::unix::fs::symlink(found, bin.join(program));
    }
    let go = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(args)
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        // No systemd user manager here, whatever the machine has: the autostart route is the one tested.
        .env("PATH", &bin)
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    assert_eq!(go(&["requirements"]).status.code(), Some(0));
    let setup = go(&["service", "setup", "--yes"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let status = go(&["service", "status"]);
    // Setup either started it (autostart) or the machine's systemd did; either way it is running now.
    assert_eq!(status.status.code(), Some(0), "{}{}", text(&status.stdout), text(&status.stderr));
    assert!(text(&status.stdout).contains("Running"));
    assert_eq!(go(&["service", "stop"]).status.code(), Some(0));
    assert_eq!(go(&["service", "status"]).status.code(), Some(1));
    assert_eq!(go(&["service", "start"]).status.code(), Some(0));
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert_eq!(go(&["service", "status"]).status.code(), Some(0));
    assert_eq!(go(&["service", "remove"]).status.code(), Some(0));
    assert_eq!(go(&["service", "status"]).status.code(), Some(1));
    assert!(!run.config.join("autostart/tiny4lite-control-service.desktop").exists());
    assert!(!run.config.join("systemd/user/tiny4lite-control.service").exists());
}

#[test]
fn a_second_service_exits_6_so_systemd_does_not_keep_restarting_it() {
    use std::time::{Duration, Instant};
    let run = Run::new("service-twice");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    let mut first = Command::new(env!("CARGO_BIN_EXE_tiny4lite-control"))
        .args(["service", "run"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DISPLAY")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let state = runtime.join("tiny4lite-control/service.json");
    let started = Instant::now();
    while !state.exists() && started.elapsed() < Duration::from_secs(10) { std::thread::sleep(Duration::from_millis(100)); }
    let second = service_run(&run, &runtime, &log, &["service", "run"]);
    let _ = Command::new("kill").args(["-TERM", &first.id().to_string()]).status();
    let _ = first.wait();
    assert_eq!(second.status.code(), Some(6), "{}", text(&second.stderr));
    assert!(text(&second.stderr).contains("already running"));
    let unit = service_run(&run, &runtime, &log, &["service", "install", "systemd"]);
    assert!(text(&unit.stdout).contains("RestartPreventExitStatus=6"), "{}", text(&unit.stdout));
}

#[test]
fn a_service_started_through_an_appimage_apprun_link_is_seen_as_running() {
    use std::time::{Duration, Instant};
    let run = Run::new("service-apprun");
    let (log, runtime) = (run.config.join("commands.log"), run.config.join("runtime"));
    // An AppImage runs the program as /tmp/.mount_XXXX/AppRun, a link with no "tiny4lite" in its name.
    let mount = run.config.join(".mount_Tiny4LX");
    std::fs::create_dir_all(&mount).unwrap();
    let apprun = mount.join("AppRun");
    let _ = std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tiny4lite-control"), &apprun);
    let mut child = Command::new(&apprun)
        .args(["service", "run"])
        .env("TINY4LITE_UI_TEST", "1")
        .env("TINY4LITE_UI_TEST_LOG", &log)
        .env("XDG_CONFIG_HOME", &run.config)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DISPLAY")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let state = runtime.join("tiny4lite-control/service.json");
    let started = Instant::now();
    while !state.exists() && started.elapsed() < Duration::from_secs(10) { std::thread::sleep(Duration::from_millis(100)); }
    let status = service_run(&run, &runtime, &log, &["service", "status"]);
    let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
    let _ = child.wait();
    assert_eq!(status.status.code(), Some(0), "{}", text(&status.stdout));
    assert!(text(&status.stdout).contains("Running"));
}
