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
    /// OBSBOT exposure compensation slider position, 0..=18 (-3.0..=+3.0).
    pub exposure_compensation_position: Option<u8>,
    /// Last successfully submitted microphone auto-gain state; camera GET_CUR is authoritative.
    pub audio_auto_gain: Option<bool>,
    /// Last successfully submitted microphone-disabled state; no reliable GET_CUR read-back is established.
    pub audio_microphone_disabled: Option<bool>,
    /// Last successfully submitted microphone-during-sleep setting; camera GET_CUR is authoritative.
    pub audio_microphone_during_sleep: Option<bool>,
    /// Last successfully submitted microphone noise-reduction level: 0 Off, 1 Weak, 2 Medium, 3 Strong.
    pub audio_noise_reduction: Option<u8>,
    /// Last successfully submitted radio distance: 0 Close, 1 Standard, 2 Far; no camera read-back is established.
    pub audio_radio_distance: Option<u8>,
    /// Last successfully submitted Pop filter state; camera read-back is not established.
    pub pop_filter_enabled: Option<bool>,
    /// Last successfully submitted Mirror image state; camera read-back is not established.
    pub mirror_image_enabled: Option<bool>,
    pub gesture_states: BTreeMap<String, bool>,
    pub gesture_zoom_ratio: Option<f32>,
    pub joystick_sensitivity_percent: u16,
    // Service policy is persisted with the rest of the user's preferences.
    // Auto-sleep is opt-in and must remain inert unless explicitly enabled.
    pub service: ServiceConfig,
    /// The denoised virtual camera.
    pub denoise: DenoiseConfig,
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
    /// A capture mode chosen while another program was using the camera; the service applies it when
    /// the camera is free.
    pub pending_video_mode: Option<VideoModeConfig>,
    /// Accent seed: a family name (mint, sky, lilac, rose, amber, leaf) or `#rrggbb`.
    pub accent: String,
    /// `auto`, `compact`, `default` or `large`.
    pub ui_scale: String,
    pub reduce_motion: bool,
    /// Which window opens at start: `full` or `slim`.
    pub last_window: String,
    pub slim_show_preview: bool,
    pub slim_show_joystick: bool,
    /// Width in pixels of the controls column in the full window; 0 picks a default.
    pub split_position: i32,
    pub popup_width: i32,
    pub popup_height: i32,
    pub preview_popped_out: bool,
    /// Draw minimise, maximise and close buttons in the header.
    pub window_buttons: bool,
    /// The view of the gimbal model, turned about the vertical in degrees (180 looks at the front).
    pub model_view_yaw: f64,
    /// Draw the model's pan the other way round (for a camera that is mounted or read the other way).
    pub model_pan_reversed: bool,
    /// With Mirror image on, turn the stick the other way, so pushing it right still moves the picture
    /// you see to the right.
    pub joystick_follows_mirror: bool,
    /// The live preview was paused with its button; it stays paused until it is resumed.
    pub preview_paused: bool,
    /// Start the live preview when the app opens. Off, it waits for its play button, so opening the app
    /// does not turn the camera on.
    pub preview_on_start: bool,
    /// Outline what is in focus in green on the preview.
    pub preview_focus_peaking: bool,
    /// How strict focus peaking is, 0–100: higher marks only the very sharpest edges.
    pub preview_peaking_strictness: u8,
    /// Keep the gimbal inside OBSBOT's published tilt range (30° down to 70° up). Off, it can be driven to
    /// the mechanical ends.
    pub tilt_limit: bool,
    /// The tilt range used while the limit is on, in degrees (up is positive). OBSBOT's published
    /// range, −30 to +70, unless changed.
    pub tilt_limit_low: f64,
    pub tilt_limit_high: f64,
    /// What the six shortcut buttons beside the joystick do. None: the first six app presets.
    pub quick_presets: Option<Vec<QuickPreset>>,
    /// Keep pan inside a range: OBSBOT's published ±140° unless changed. Off, the mechanical ±150°.
    pub pan_limit: bool,
    pub pan_limit_low: f64,
    pub pan_limit_high: f64,
    /// "Not now" was chosen on the banner that offers to set up the background service.
    pub service_setup_dismissed: bool,
}

/// The denoised virtual camera, run by the background service.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DenoiseConfig {
    pub enabled: bool,
    /// 1 to 100.
    pub strength: u8,
    /// Denoise on the graphics chip (VA-API) when there is one.
    pub use_gpu: bool,
}

impl Default for DenoiseConfig {
    fn default() -> DenoiseConfig { DenoiseConfig { enabled: false, strength: 40, use_gpu: true } }
}

/// One shortcut button: an app preset by name, a position in the camera's memory (0–2), or nothing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct QuickPreset {
    pub app: Option<String>,
    pub camera: Option<u8>,
    /// What the button shows instead of its number: a short word, a letter or an emoji (up to 8 characters).
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceConfig {
    pub auto_sleep_enabled: bool,
    pub auto_sleep_timeout_minutes: u32,
    /// The preset to restore when the camera is plugged in (with `apply_on_connect`). None restores
    /// the last settings this app set.
    pub connect_preset: Option<String>,
    /// Let a restore turn the gimbal to a saved position. Off, positions are left out.
    pub allow_gimbal_move: bool,
    /// Put the camera to sleep when the computer shuts down or the session ends, so it does not stay on.
    pub sleep_on_shutdown: bool,
    /// Put the camera to sleep when the computer suspends.
    pub sleep_on_suspend: bool,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            auto_sleep_enabled: false,
            auto_sleep_timeout_minutes: 10,
            connect_preset: None,
            allow_gimbal_move: false,
            sleep_on_shutdown: false,
            sleep_on_suspend: false,
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
            exposure_compensation_position: None,
            audio_auto_gain: None,
            audio_microphone_disabled: None,
            audio_microphone_during_sleep: None,
            audio_noise_reduction: None,
            audio_radio_distance: None,
            pop_filter_enabled: None,
            mirror_image_enabled: None,
            gesture_states: BTreeMap::new(),
            gesture_zoom_ratio: None,
            joystick_sensitivity_percent: 100,
            service: ServiceConfig::default(),
            denoise: DenoiseConfig::default(),
            apply_on_connect: false,
            compact_mode: false,
            compact_controls: vec![
                "v4l2:zoom_absolute".into(),
                "v4l2:brightness".into(),
                "v4l2:contrast".into(),
                "v4l2:focus_automatic_continuous".into(),
                "obsbot:focus_mode".into(),
                "obsbot:auto_exposure_mode".into(),
                "obsbot:exposure_compensation".into(),
                "obsbot:ai_tracking_mode".into(),
                "obsbot:led_brightness".into(),
                "obsbot:fov".into(),
                "camera:preview".into(),
            ],
            presets: Vec::new(),
            video_mode: None,
            pending_video_mode: None,
            accent: "sky".into(),
            ui_scale: "auto".into(),
            reduce_motion: false,
            last_window: "full".into(),
            slim_show_preview: true,
            slim_show_joystick: false,
            split_position: 0,
            popup_width: 640,
            popup_height: 360,
            preview_popped_out: false,
            window_buttons: true,
            model_view_yaw: 180.0,
            model_pan_reversed: false,
            joystick_follows_mirror: true,
            preview_paused: false,
            preview_on_start: false,
            preview_focus_peaking: false,
            preview_peaking_strictness: 60,
            tilt_limit: true,
            tilt_limit_low: -30.0,
            tilt_limit_high: 70.0,
            service_setup_dismissed: false,
            quick_presets: None,
            pan_limit: true,
            pan_limit_low: -140.0,
            pan_limit_high: 140.0,
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
            if config.exposure_compensation_position.is_some_and(|position| position > 18) {
                config.exposure_compensation_position = None;
            }
            if config.audio_noise_reduction.is_some_and(|level| level > 3) {
                config.audio_noise_reduction = None;
            }
            if config.audio_radio_distance.is_some_and(|distance| distance > 2) {
                config.audio_radio_distance = None;
            }
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
        assert!(config.exposure_compensation_position.is_none());
        assert!(config.audio_auto_gain.is_none());
        assert!(config.audio_microphone_during_sleep.is_none());
        assert!(config.audio_noise_reduction.is_none());
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
        config.exposure_compensation_position = Some(18);
        config.audio_auto_gain = Some(true);
        config.audio_microphone_disabled = Some(true);
        config.audio_microphone_during_sleep = Some(true);
        config.audio_noise_reduction = Some(3);
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
        assert_eq!(decoded.exposure_compensation_position, Some(18));
        assert_eq!(decoded.audio_auto_gain, Some(true));
        assert_eq!(decoded.audio_microphone_disabled, Some(true));
        assert_eq!(decoded.audio_microphone_during_sleep, Some(true));
        assert_eq!(decoded.audio_noise_reduction, Some(3));
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
