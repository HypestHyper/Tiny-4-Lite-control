use crate::*;
use std::collections::BTreeMap;

const MOVEMENT_CONTROLS: &[&str] = &["pan_speed", "tilt_speed", "zoom_continuous"];

fn ai_mode_id(status: &[u8; 60]) -> Option<i64> {
    match (status[0x18], status[0x1c]) {
        (0x00, 0x00) => Some(0),
        (0x02, 0x00) => Some(1),
        (0x02, 0x01) => Some(2),
        (0x02, 0x02) => Some(3),
        (0x02, 0x03) => Some(4),
        (0x02, 0x04) => Some(5),
        (0x01, 0x00) => Some(6),
        // Added after the first presets were saved, so it takes the next number and
        // older presets keep their meaning.
        (0x03, 0x00) => Some(7),
        _ => None,
    }
}

fn ai_mode_payload(id: i64) -> Option<(u8, u8)> {
    match id {
        0 => Some((0x00, 0x00)),
        1 => Some((0x02, 0x00)),
        2 => Some((0x02, 0x01)),
        3 => Some((0x02, 0x02)),
        4 => Some((0x02, 0x03)),
        5 => Some((0x02, 0x04)),
        6 => Some((0x01, 0x00)),
        7 => Some((0x03, 0x00)),
        _ => None,
    }
}

fn fourcc_to_i64(value: &str) -> Option<i64> {
    let bytes: [u8; 4] = value.as_bytes().try_into().ok()?;
    Some(u32::from_le_bytes(bytes) as i64)
}

fn fourcc_from_i64(value: i64) -> Option<String> {
    let bytes = u32::try_from(value).ok()?.to_le_bytes();
    if !bytes.iter().all(|byte| byte.is_ascii_graphic()) { return None; }
    Some(String::from_utf8(bytes.to_vec()).ok()?)
}

fn gesture_group(name: &str) -> Option<[u8; 6]> {
    match name {
        "target_selection" => Some([0x0a, 0x04, 0xc4, 0x30, 0x01, 0x00]),
        "zoom_l_pose" => Some([0x0a, 0x04, 0x44, 0x31, 0x01, 0x00]),
        "dynamic_zoom" => Some([0x0a, 0x04, 0x44, 0x33, 0x01, 0x00]),
        "dynamic_zoom_direction" => Some([0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00]),
        _ => None,
    }
}

/// Snapshot every readable, persistent camera setting. Transient speed controls
/// are intentionally excluded; absolute pan/tilt positions are included.
pub fn capture(device: &str, name: String) -> Result<config::CameraPreset, String> {
    let output = run_v4l2(device, "--list-ctrls-menus")?;
    let mut settings = BTreeMap::new();
    for control in parse_controls(&output) {
        if !control.disabled && !MOVEMENT_CONTROLS.contains(&control.name.as_str()) && control.kind != "button" {
            settings.insert(format!("v4l2:{}", control.name), control.value);
        }
    }

    if let Some(mode) = video::current_mode(device) {
        settings.insert("video:width".into(), mode.width as i64);
        settings.insert("video:height".into(), mode.height as i64);
        settings.insert("video:frame_rate_milli".into(), mode.frame_rate_milli as i64);
        if let Some(code) = fourcc_to_i64(&mode.pixel_format) {
            settings.insert("video:pixel_format".into(), code);
        }
    }

    if let Ok(status) = xu_status(device) {
        if status[0x21] <= 3 {
            settings.insert("obsbot:led_brightness".into(), status[0x21] as i64);
        }
        if let Some(mode) = ai_mode_id(&status) {
            settings.insert("obsbot:ai_tracking_mode".into(), mode);
        }
        match status[0x24] {
            0x00 => { settings.insert("obsbot:tracking_speed".into(), 0); }
            0x02 | 0x21 => { settings.insert("obsbot:tracking_speed".into(), 1); }
            _ => {}
        }
        // Auto Exposure priority: status byte 7 (0 = Face, 1 = Global) is authoritative.
        if let Some(face) = auto_exposure_mode_from_status(&status) {
            settings.insert("obsbot:auto_exposure_mode".into(), face as i64);
        }
        // Target Selection is the only gesture the camera reports (status byte 1).
        // Selector 0x06 byte 0x07 is NOT the zoom gesture: that assumption was disproven.
        if let Some(enabled) = match status[0x01] { 0 => Some(0), 1 => Some(1), _ => None } {
            settings.insert("obsbot:gesture:target_selection".into(), enabled);
        }
    }

    // Autofocus priority comes from the camera only. If it does not answer, leave it out.
    if let Ok(priority) = get_face_focus(device) {
        settings.insert("obsbot:focus_mode".into(), (priority == FaceFocusPriority::Face) as i64);
    }

    let saved = current_config();
    if let Some(ratio) = saved.gesture_zoom_ratio {
        settings.insert("obsbot:gesture_zoom_ratio_milli".into(), (ratio * 1000.0).round() as i64);
    }
    // The other three gestures have no read-back: record what this app last sent.
    for (name, enabled) in saved.gesture_states {
        settings.entry(format!("obsbot:gesture:{name}")).or_insert(enabled as i64);
    }

    Ok(config::CameraPreset { name: name.trim().to_string(), settings })
}

/// Apply settings in dependency order: automatic/manual modes first, ordinary
/// controls second, gimbal positions last, then vendor XU settings.
/// Applying this preset turns the gimbal, because it stores absolute pan/tilt.
pub fn moves_gimbal(preset: &config::CameraPreset) -> bool {
    preset.settings.contains_key("v4l2:pan_absolute") || preset.settings.contains_key("v4l2:tilt_absolute")
}

pub fn apply(device: &str, preset: &config::CameraPreset) -> Result<String, String> {
    let mut controls: Vec<(String, i64)> = preset.settings.iter()
        .filter_map(|(key, value)| key.strip_prefix("v4l2:").map(|name| (name.to_string(), *value)))
        .collect();
    let priority = |name: &str| match name {
        "white_balance_automatic" | "focus_automatic_continuous" | "auto_exposure" => 0,
        "pan_absolute" | "tilt_absolute" => 2,
        _ => 1,
    };
    controls.sort_by_key(|(name, _)| priority(name));

    let mut applied = 0usize;
    let mut errors = Vec::new();
    // Tracking would pull the camera away from a saved position, so a preset that turns tracking
    // off does it first; one that turns tracking on does it last, after the gimbal has moved.
    let tracking = preset.settings.get("obsbot:ai_tracking_mode").copied();
    let tracking_first = tracking == Some(0);
    if tracking_first {
        match ai_mode_payload(0).ok_or_else(|| "Unknown AI tracking mode in preset.".to_string()).and_then(|mode| set_ai_mode(device, mode)) {
            Ok(()) => applied += 1,
            Err(error) => errors.push(format!("obsbot:ai_tracking_mode: {error}")),
        }
    }
    for (name, value) in controls {
        match set_control(device, &name, value) {
            Ok(()) => applied += 1,
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }

    for (key, value) in &preset.settings {
        let result = if key == "obsbot:ai_tracking_mode" && tracking_first {
            continue;
        } else if key == "obsbot:led_brightness" {
            u8::try_from(*value).ok().filter(|v| *v <= 3)
                .ok_or_else(|| "LED brightness must be between 0 and 3.".to_string())
                .and_then(|level| set_led_brightness(device, level))
        } else if key == "obsbot:ai_tracking_mode" {
            ai_mode_payload(*value).ok_or_else(|| "Unknown AI tracking mode in preset.".to_string())
                .and_then(|mode| set_ai_mode(device, mode))
        } else if key == "obsbot:tracking_speed" {
            match *value {
                0 => set_tracking_speed(device, false),
                1 => set_tracking_speed(device, true),
                _ => Err("Unknown tracking speed in preset.".into()),
            }
        } else if key == "obsbot:focus_mode" {
            match *value {
                0 => set_face_focus(device, FaceFocusPriority::Global).map(|_| ()),
                1 => set_face_focus(device, FaceFocusPriority::Face).map(|_| ()),
                _ => Err("Unknown autofocus priority in preset.".into()),
            }
        } else if key == "obsbot:auto_exposure_mode" {
            match *value {
                0 => set_auto_exposure_mode(device, false).map(|_| ()),
                1 => set_auto_exposure_mode(device, true).map(|_| ()),
                _ => Err("Unknown Auto Exposure priority in preset.".into()),
            }
        } else if key == "obsbot:gesture_zoom_ratio_milli" {
            set_gesture_zoom_ratio(device, *value as f32 / 1000.0)
        } else if let Some(name) = key.strip_prefix("obsbot:gesture:") {
            match (gesture_group(name), *value) {
                (Some(group), 0) => set_gesture(device, group, false),
                (Some(group), 1) => set_gesture(device, group, true),
                (None, _) => Err(format!("Unknown gesture setting '{name}'.")),
                _ => Err(format!("Invalid value for gesture '{name}'.")),
            }
        } else if key.starts_with("video:") || key.starts_with("v4l2:") {
            // Video modes are handled below and standard controls were applied above.
            continue;
        } else {
            Err(format!("Unsupported preset setting '{key}'."))
        };
        match result {
            Ok(()) => applied += 1,
            Err(error) => errors.push(format!("{key}: {error}")),
        }
    }

    let video_keys = [
        preset.settings.get("video:width"),
        preset.settings.get("video:height"),
        preset.settings.get("video:frame_rate_milli"),
        preset.settings.get("video:pixel_format"),
    ];
    let has_video_settings = preset.settings.keys().any(|key| key.starts_with("video:"));
    if has_video_settings && video_keys.iter().any(Option::is_none) {
        errors.push("video mode: preset is missing one or more capture-mode fields".into());
    }
    if let [Some(width), Some(height), Some(rate), Some(format)] = video_keys {
        let mode = (|| Some(video::VideoMode {
            width: u32::try_from(*width).ok()?,
            height: u32::try_from(*height).ok()?,
            frame_rate_milli: u32::try_from(*rate).ok()?,
            pixel_format: fourcc_from_i64(*format)?,
        }))();
        match mode {
            Some(mode) => {
                match video::set_mode(device, &mode) {
                Ok(()) => {
                    applied += 1;
                    if let Err(error) = update_config_checked(|config| config.video_mode = Some(config::VideoModeConfig {
                        width: mode.width, height: mode.height,
                        pixel_format: mode.pixel_format.clone(),
                        frame_rate_milli: mode.frame_rate_milli,
                    })) {
                        errors.push(format!("video mode applied but could not be saved: {error}"));
                    }
                }
                    Err(error) => {
                        errors.push(format!("video mode: {error}"));
                    }
                }
            },
            None => errors.push("video mode: invalid preset capture settings".into()),
        }
    }

    if errors.is_empty() {
        Ok(format!("Submitted {applied} settings from preset '{}'.", preset.name))
    } else {
        Err(format!("Submitted {applied} settings from '{}'; {} setting(s) failed: {}",
            preset.name, errors.len(), errors.join(" · ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_mode_preset_ids_round_trip() {
        for id in 0..=7 {
            assert_eq!(ai_mode_id_from_payload(ai_mode_payload(id).unwrap()), Some(id));
        }
        assert!(ai_mode_payload(8).is_none());
        // Presets saved before Hand existed keep their meaning: 6 is still Group.
        assert_eq!(ai_mode_payload(6), Some((0x01, 0x00)));
        assert_eq!(ai_mode_payload(7), Some((0x03, 0x00)));
    }

    fn ai_mode_id_from_payload(payload: (u8, u8)) -> Option<i64> {
        match payload {
            (0x00, 0x00) => Some(0), (0x02, 0x00) => Some(1),
            (0x02, 0x01) => Some(2), (0x02, 0x02) => Some(3),
            (0x02, 0x03) => Some(4), (0x02, 0x04) => Some(5),
            (0x01, 0x00) => Some(6), (0x03, 0x00) => Some(7), _ => None,
        }
    }

    #[test]
    fn movement_speed_is_not_a_persistent_preset_value() {
        assert!(MOVEMENT_CONTROLS.contains(&"pan_speed"));
        assert!(MOVEMENT_CONTROLS.contains(&"tilt_speed"));
        assert!(!MOVEMENT_CONTROLS.contains(&"pan_absolute"));
        assert!(!MOVEMENT_CONTROLS.contains(&"tilt_absolute"));
    }

    #[test]
    fn pixel_format_round_trips_as_a_fourcc() {
        assert_eq!(fourcc_from_i64(fourcc_to_i64("MJPG").unwrap()).as_deref(), Some("MJPG"));
        assert_eq!(fourcc_from_i64(fourcc_to_i64("YUYV").unwrap()).as_deref(), Some("YUYV"));
        assert!(fourcc_to_i64("MJ").is_none());
    }
}
