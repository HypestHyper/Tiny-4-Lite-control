use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub theme: String,
    pub v4l2_controls: BTreeMap<String, i64>,
    pub led_brightness: Option<u8>,
    pub ai_tracking_mode: Option<String>,
    pub tracking_speed: Option<String>,
    pub focus_mode: Option<String>,
    pub gesture_states: BTreeMap<String, bool>,
    pub gesture_zoom_ratio: Option<f32>,
    pub joystick_sensitivity_percent: u16,
    // Reserved for a future explicit restore-on-connect feature. False is
    // deliberately the default: loading config must never move the camera.
    pub apply_on_connect: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            theme: "system".into(),
            v4l2_controls: BTreeMap::new(),
            led_brightness: None,
            ai_tracking_mode: None,
            tracking_speed: None,
            focus_mode: None,
            gesture_states: BTreeMap::new(),
            gesture_zoom_ratio: None,
            joystick_sensitivity_percent: 100,
            apply_on_connect: false,
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
            config.joystick_sensitivity_percent = config.joystick_sensitivity_percent.clamp(10, 200);
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
            Ok(config)
        }
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

pub fn save(config: &AppConfig) -> Result<(), String> {
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
        assert!(config.v4l2_controls.is_empty());
        assert!(config.gesture_states.is_empty());
        assert_eq!(config.theme, "system");
    }

    #[test]
    fn config_round_trips_all_persistent_settings() {
        let mut config = AppConfig::default();
        config.theme = "dark".into();
        config.v4l2_controls.insert("zoom_absolute".into(), 15);
        config.led_brightness = Some(2);
        config.ai_tracking_mode = Some("upper".into());
        config.tracking_speed = Some("sport".into());
        config.focus_mode = Some("face".into());
        config.gesture_states.insert("dynamic_zoom".into(), true);
        config.gesture_zoom_ratio = Some(2.25);
        config.joystick_sensitivity_percent = 135;

        let encoded = toml::to_string(&config).unwrap();
        let decoded: AppConfig = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.v4l2_controls.get("zoom_absolute"), Some(&15));
        assert_eq!(decoded.led_brightness, Some(2));
        assert_eq!(decoded.ai_tracking_mode.as_deref(), Some("upper"));
        assert_eq!(decoded.tracking_speed.as_deref(), Some("sport"));
        assert_eq!(decoded.focus_mode.as_deref(), Some("face"));
        assert_eq!(decoded.gesture_states.get("dynamic_zoom"), Some(&true));
        assert_eq!(decoded.gesture_zoom_ratio, Some(2.25));
        assert_eq!(decoded.joystick_sensitivity_percent, 135);
    }
}
