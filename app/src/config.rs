use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoModeConfig {
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    /// Frame rate multiplied by 1000 to preserve fractional rates.
    pub frame_rate_milli: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CameraPreset {
    pub name: String,
    /// Namespaced integer settings: v4l2:<control>, obsbot:<setting>.
    /// Movement commands are deliberately excluded from presets.
    pub settings: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub theme: String,
    pub v4l2_controls: BTreeMap<String, i64>,
    pub led_brightness: Option<u8>,
    pub hdr_enabled: Option<bool>,
    pub ai_tracking_mode: Option<String>,
    pub tracking_speed: Option<String>,
    pub focus_mode: Option<String>,
    pub auto_exposure_mode: Option<String>,
    pub gesture_states: BTreeMap<String, bool>,
    pub gesture_zoom_ratio: Option<f32>,
    pub joystick_sensitivity_percent: u16,
    // Service policy is persisted with the rest of the user's preferences.
    // Auto-sleep is opt-in and must remain inert unless explicitly enabled.
    pub service: ServiceConfig,
    // Reserved for a future explicit restore-on-connect feature. False is
    // deliberately the default: loading config must never move the camera.
    pub apply_on_connect: bool,
    /// Compact mode is a user-selected quick-control surface.
    pub compact_mode: bool,
    /// Stable setting identifiers shown while compact mode is active.
    pub compact_controls: Vec<String>,
    /// User presets. The UI and persistence layer enforce a maximum of ten.
    pub presets: Vec<CameraPreset>,
    /// Last selected capture mode, restored on connect only if still advertised by V4L2.
    pub video_mode: Option<VideoModeConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceConfig {
    pub auto_sleep_enabled: bool,
    pub auto_sleep_timeout_minutes: u32,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            auto_sleep_enabled: false,
            auto_sleep_timeout_minutes: 10,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            theme: "system".into(),
            v4l2_controls: BTreeMap::new(),
            led_brightness: None,
            hdr_enabled: None,
            ai_tracking_mode: None,
            tracking_speed: None,
            focus_mode: None,
            auto_exposure_mode: None,
            gesture_states: BTreeMap::new(),
            gesture_zoom_ratio: None,
            joystick_sensitivity_percent: 100,
            service: ServiceConfig::default(),
            apply_on_connect: false,
            compact_mode: false,
            compact_controls: vec![
                "v4l2:zoom_absolute".into(),
                "v4l2:brightness".into(),
                "v4l2:contrast".into(),
                "v4l2:focus_automatic_continuous".into(),
                "obsbot:focus_mode".into(),
                "obsbot:auto_exposure_mode".into(),
                "obsbot:ai_tracking_mode".into(),
                "obsbot:led_brightness".into(),
            ],
            presets: Vec::new(),
            video_mode: None,
        }
    }
}

pub fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("tiny4lite-control")
        .join("config.toml")
}

pub fn load() -> Result<AppConfig, String> {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let mut config: AppConfig = toml::from_str(&contents)
                .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
            if config.version > CONFIG_VERSION {
                return Err(format!("{} uses a newer config version ({}) than this app supports ({CONFIG_VERSION}).", path.display(), config.version));
            }
            config.version = CONFIG_VERSION;
            if config.presets.len() > 10 {
                return Err(format!("{} contains more than 10 camera presets; edit the file before opening it.", path.display()));
            }
            config.joystick_sensitivity_percent = config.joystick_sensitivity_percent.clamp(10, 200);
            config.service.auto_sleep_timeout_minutes = config.service.auto_sleep_timeout_minutes.clamp(1, 1440);
            if config.gesture_zoom_ratio.is_some_and(|ratio| !ratio.is_finite() || !(1.0..=4.0).contains(&ratio)) {
                config.gesture_zoom_ratio = None;
            }
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Migrate the old one-line theme preference without deleting it.
            let legacy = path.with_file_name("settings.conf");
            let mut config = AppConfig::default();
            if let Ok(contents) = fs::read_to_string(legacy) {
                if let Some(theme) = contents.lines().find_map(|line| line.strip_prefix("theme=")) {
                    if matches!(theme, "system" | "light" | "dark") {
                        config.theme = theme.to_string();
                    }
                }
            }
            // Create the first config file on first launch, preserving any
            // migrated theme. Failure to persist defaults must not block startup.
            if let Err(error) = save(&config) {
                eprintln!("Could not create initial settings file: {error}");
            }
            Ok(config)
        }
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    if config.presets.len() > 10 {
        return Err("A maximum of 10 camera presets is supported.".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for preset in &config.presets {
        let name = preset.name.trim();
        if name.is_empty() {
            return Err("Camera preset names cannot be empty.".into());
        }
        if !names.insert(name.to_lowercase()) {
            return Err(format!("Camera preset name '{name}' is already in use."));
        }
    }
    let path = config_path();
    let parent = path.parent().ok_or_else(|| "Config path has no parent directory.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    let contents = toml::to_string_pretty(config)
        .map_err(|error| format!("Could not serialize settings: {error}"))?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, contents)
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("Could not replace {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_have_no_camera_commands() {
        let config = AppConfig::default();
        assert!(!config.apply_on_connect);
        assert!(!config.service.auto_sleep_enabled);
        assert_eq!(config.service.auto_sleep_timeout_minutes, 10);
        assert!(config.v4l2_controls.is_empty());
        assert!(config.hdr_enabled.is_none());
        assert!(config.auto_exposure_mode.is_none());
        assert!(config.gesture_states.is_empty());
        assert_eq!(config.theme, "system");
        assert!(!config.compact_mode);
        assert!(config.presets.is_empty());
        assert!(config.video_mode.is_none());
        assert_eq!(config.compact_controls.first().map(String::as_str), Some("v4l2:zoom_absolute"));
    }

    #[test]
    fn older_config_without_service_section_gets_safe_defaults() {
        let config: AppConfig = toml::from_str("version = 1\ntheme = \"dark\"\n").unwrap();
        assert_eq!(config.theme, "dark");
        assert!(!config.service.auto_sleep_enabled);
        assert_eq!(config.service.auto_sleep_timeout_minutes, 10);
    }

    #[test]
    fn config_round_trips_all_persistent_settings() {
        let mut config = AppConfig::default();
        config.theme = "dark".into();
        config.v4l2_controls.insert("zoom_absolute".into(), 15);
        config.led_brightness = Some(2);
        config.hdr_enabled = Some(true);
        config.ai_tracking_mode = Some("upper".into());
        config.tracking_speed = Some("sport".into());
        config.focus_mode = Some("face".into());
        config.auto_exposure_mode = Some("global".into());
        config.gesture_states.insert("dynamic_zoom".into(), true);
        config.gesture_zoom_ratio = Some(2.25);
        config.joystick_sensitivity_percent = 135;
        config.service.auto_sleep_enabled = true;
        config.service.auto_sleep_timeout_minutes = 25;
        config.compact_mode = true;
        config.compact_controls = vec!["v4l2:zoom_absolute".into(), "obsbot:focus_mode".into()];
        config.video_mode = Some(VideoModeConfig {
            width: 1920, height: 1080, pixel_format: "MJPG".into(), frame_rate_milli: 30000,
        });
        config.presets.push(CameraPreset {
            name: "Studio".into(),
            settings: BTreeMap::from([
                ("v4l2:brightness".into(), 55),
                ("obsbot:led_brightness".into(), 2),
            ]),
        });

        let encoded = toml::to_string(&config).unwrap();
        let decoded: AppConfig = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.v4l2_controls.get("zoom_absolute"), Some(&15));
        assert_eq!(decoded.led_brightness, Some(2));
        assert_eq!(decoded.hdr_enabled, Some(true));
        assert_eq!(decoded.ai_tracking_mode.as_deref(), Some("upper"));
        assert_eq!(decoded.tracking_speed.as_deref(), Some("sport"));
        assert_eq!(decoded.focus_mode.as_deref(), Some("face"));
        assert_eq!(decoded.auto_exposure_mode.as_deref(), Some("global"));
        assert_eq!(decoded.gesture_states.get("dynamic_zoom"), Some(&true));
        assert_eq!(decoded.gesture_zoom_ratio, Some(2.25));
        assert_eq!(decoded.joystick_sensitivity_percent, 135);
        assert!(decoded.service.auto_sleep_enabled);
        assert_eq!(decoded.service.auto_sleep_timeout_minutes, 25);
        assert!(decoded.compact_mode);
        assert_eq!(decoded.compact_controls.len(), 2);
        assert_eq!(decoded.video_mode.as_ref().map(|mode| mode.pixel_format.as_str()), Some("MJPG"));
        assert_eq!(decoded.presets.len(), 1);
        assert_eq!(decoded.presets[0].settings.get("obsbot:led_brightness"), Some(&2));
    }
}
