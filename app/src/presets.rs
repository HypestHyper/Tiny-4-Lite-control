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
        _ => None,
    }
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
        if !MOVEMENT_CONTROLS.contains(&control.name.as_str()) && control.kind != "button" {
            settings.insert(format!("v4l2:{}", control.name), control.value);
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
        // Status byte 0x07 is inverted: zero means the L-pose gesture is enabled.
        settings.insert("obsbot:gesture:zoom_l_pose".into(), (status[0x07] == 0) as i64);
    }

    let saved = current_config();
    if let Some(mode) = saved.focus_mode.as_deref() {
        settings.insert("obsbot:focus_mode".into(), (mode == "face") as i64);
    }
    if let Some(ratio) = saved.gesture_zoom_ratio {
        settings.insert("obsbot:gesture_zoom_ratio_milli".into(), (ratio * 1000.0).round() as i64);
    }
    for (name, enabled) in saved.gesture_states {
        if name != "zoom_l_pose" {
            settings.insert(format!("obsbot:gesture:{name}"), enabled as i64);
        }
    }

    Ok(config::CameraPreset { name: name.trim().to_string(), settings })
}

/// Apply settings in dependency order: automatic/manual modes first, ordinary
/// controls second, gimbal positions last, then vendor XU settings.
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
    for (name, value) in controls {
        match set_control(device, &name, value) {
            Ok(()) => applied += 1,
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }

    for (key, value) in &preset.settings {
        let result = if key == "obsbot:led_brightness" {
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
                0 => set_face_focus(device, false),
                1 => set_face_focus(device, true),
                _ => Err("Unknown autofocus priority in preset.".into()),
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
        } else {
            continue;
        };
        match result {
            Ok(()) => applied += 1,
            Err(error) => errors.push(format!("{key}: {error}")),
        }
    }

    if errors.is_empty() {
        Ok(format!("Applied {applied} settings from preset '{}'.", preset.name))
    } else {
        Err(format!("Applied {applied} settings from '{}'; {} setting(s) failed: {}",
            preset.name, errors.len(), errors.join(" · ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_mode_preset_ids_round_trip() {
        for id in 0..=6 {
            assert_eq!(ai_mode_id_from_payload(ai_mode_payload(id).unwrap()), Some(id));
        }
        assert!(ai_mode_payload(7).is_none());
    }

    fn ai_mode_id_from_payload(payload: (u8, u8)) -> Option<i64> {
        match payload {
            (0x00, 0x00) => Some(0), (0x02, 0x00) => Some(1),
            (0x02, 0x01) => Some(2), (0x02, 0x02) => Some(3),
            (0x02, 0x03) => Some(4), (0x02, 0x04) => Some(5),
            (0x01, 0x00) => Some(6), _ => None,
        }
    }

    #[test]
    fn movement_speed_is_not_a_persistent_preset_value() {
        assert!(MOVEMENT_CONTROLS.contains(&"pan_speed"));
        assert!(MOVEMENT_CONTROLS.contains(&"tilt_speed"));
        assert!(!MOVEMENT_CONTROLS.contains(&"pan_absolute"));
        assert!(!MOVEMENT_CONTROLS.contains(&"tilt_absolute"));
    }
}
