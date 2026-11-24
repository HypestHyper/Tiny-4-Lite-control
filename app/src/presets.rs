use crate::*;
use std::collections::BTreeMap;

const MOVEMENT_CONTROLS: &[&str] = &["pan_speed", "tilt_speed", "zoom_continuous"];
/// Red and blue gains are never saved or written: writing either switches the camera away from the colour
/// temperature (presets saved before still have them; they are skipped).
const NOT_SAVED: &[&str] = &["red_balance", "blue_balance"];

fn ai_mode_id(status: &[u8; 60]) -> Option<i64> {
    match (status[0x18], status[0x1c]) {
        (0x00, 0x00) => Some(0),
        (0x02, 0x00) => Some(1),
        (0x02, 0x01) => Some(2),
        (0x02, 0x02) => Some(3),
        (0x02, 0x03) => Some(4),
        (0x02, 0x04) => Some(5),
        (0x01, 0x00) => Some(6),
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
        if !control.disabled && !MOVEMENT_CONTROLS.contains(&control.name.as_str()) && !NOT_SAVED.contains(&control.name.as_str()) && control.kind != "button" {
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

    // The position comes from the camera's live pose, not the V4L2 value, which can be the last
    // command sent. Pan and tilt agree in sign with it (checked on a real camera: +1 step is +1°).
    // The V4L2 controls move in whole degrees, so round to one.
    if let Ok((pan, tilt)) = get_gimbal_position(device) {
        if pan.is_finite() && tilt.is_finite() {
            for (name, degrees) in [("pan_absolute", pan), ("tilt_absolute", tilt)] {
                if let Some(control) = parse_controls(&output).into_iter().find(|c| c.name == name && !c.disabled) {
                    let raw = backend::angles::degrees_to_raw(degrees);
                    settings.insert(format!("v4l2:{name}"), raw.clamp(control.min, control.max));
                }
            }
        }
    }

    if let Ok(status) = xu_status(device) {
        if status[0x21] <= 3 {
            settings.insert("obsbot:led_brightness".into(), status[0x21] as i64);
        }
        if let Some(mode) = ai_mode_id(&status) {
            settings.insert("obsbot:ai_tracking_mode".into(), mode);
        }
        // Vertical tracking, stored as the camera's own value (0 Standard, 1 Headroom, 2 Motion).
        if let Some(mode) = vertical_tracking_from_status(status[0x24]) {
            settings.insert("obsbot:vertical_tracking".into(), i64::from(mode));
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
        // Microphone and HDR, as the status block reports them (the same bytes the settings read).
        if status[0x08] <= 3 { settings.insert("obsbot:noise_reduction".into(), i64::from(status[0x08])); }
        if status[25] <= 1 { settings.insert("obsbot:auto_gain".into(), i64::from(status[25])); }
        if status[16] <= 1 { settings.insert("obsbot:mic_during_sleep".into(), i64::from(status[16])); }
        settings.insert("obsbot:mic_disabled".into(), i64::from(status[34] & 0x10 == 0));
        if let Some(on) = hdr_state_from_status(&status) { settings.insert("obsbot:hdr".into(), i64::from(on)); }
    }
    // Hand side and the gestures, from the camera's AI status (ahead of the last-sent values below).
    if let Ok(ai) = get_ai_status(device) {
        if ai[2] <= 1 { settings.insert("obsbot:hand_side".into(), i64::from(ai[2])); }
        for (name, index) in [("zoom_l_pose", 4), ("dynamic_zoom", 5), ("dynamic_zoom_direction", 6)] {
            if ai[index] <= 1 { settings.insert(format!("obsbot:gesture:{name}"), i64::from(ai[index])); }
        }
        if (10..=40).contains(&ai[7]) { settings.insert("obsbot:gesture_zoom_ratio_milli".into(), i64::from(ai[7]) * 100); }
    }
    if let Ok(volume) = get_mic_volume(device) { settings.insert("obsbot:mic_volume".into(), i64::from(volume)); }
    if let Ok(seconds) = get_camera_sleep_time(device) { settings.insert("obsbot:camera_sleep".into(), i64::from(seconds)); }
    // The tracking area: on or off (this app's setting; its limits are the gimbal limits, which stay the
    // same for every preset), whether new people are picked up, and where the camera returns to.
    let zone = current_config();
    settings.insert("app:tracking_area".into(), i64::from(zone.tracking_area));
    for (key, degrees) in [("app:zone_pan_low", zone.zone_pan_low), ("app:zone_pan_high", zone.zone_pan_high),
                           ("app:zone_tilt_low", zone.zone_tilt_low), ("app:zone_tilt_high", zone.zone_tilt_high)] {
        settings.insert(key.into(), (degrees * 100.0).round() as i64);
    }
    if let Ok(on) = get_tracking_area_auto_select(device) { settings.insert("obsbot:zone_auto_select".into(), i64::from(on)); }
    if let Ok((pan, tilt, zoom)) = get_zone_home(device) {
        settings.insert("obsbot:return_pan_centi".into(), (pan * 100.0).round() as i64);
        settings.insert("obsbot:return_tilt_centi".into(), (tilt * 100.0).round() as i64);
        settings.insert("obsbot:return_zoom_milli".into(), (zoom * 1000.0).round() as i64);
    }

    // Autofocus priority comes from the camera only. If it does not answer, leave it out.
    if let Ok(priority) = get_face_focus(device) {
        settings.insert("obsbot:focus_mode".into(), (priority == FaceFocusPriority::Face) as i64);
    }

    let saved = current_config();
    // Settings the camera cannot report: what this app last sent.
    if let Some(disabled) = saved.audio_microphone_disabled { settings.entry("obsbot:mic_disabled".into()).or_insert(i64::from(disabled)); }
    if let Some(distance) = saved.audio_radio_distance { settings.insert("obsbot:radio_distance".into(), i64::from(distance)); }
    if let Some(on) = saved.pop_filter_enabled { settings.insert("obsbot:pop_filter".into(), i64::from(on)); }
    if let Some(on) = saved.ai_auto_zoom { settings.insert("obsbot:ai_auto_zoom".into(), i64::from(on)); }
    if let Some(ratio) = saved.gesture_zoom_ratio {
        settings.entry("obsbot:gesture_zoom_ratio_milli".into()).or_insert((ratio * 1000.0).round() as i64);
    }
    // The other three gestures have no read-back: record what this app last sent.
    for (name, enabled) in saved.gesture_states {
        settings.entry(format!("obsbot:gesture:{name}")).or_insert(enabled as i64);
    }

    // The Studio camera's look goes with it: the LUT that is on and its strength, or none.
    let studio = saved.studio.clone();
    let look = config::PresetLook { lut: studio.lut.filter(|_| studio.lut_on), strength: studio.lut_strength };
    Ok(config::CameraPreset { name: name.trim().to_string(), settings, look: Some(look) })
}

/// Apply settings in dependency order: automatic/manual modes first, ordinary
/// controls second, gimbal positions last, then vendor XU settings.
pub fn apply(device: &str, preset: &config::CameraPreset) -> Result<String, String> {
    let mut controls: Vec<(String, i64)> = preset.settings.iter()
        .filter_map(|(key, value)| key.strip_prefix("v4l2:").map(|name| (name.to_string(), *value)))
        .filter(|(name, _)| !NOT_SAVED.contains(&name.as_str()))
        .collect();
    let priority = |name: &str| match name {
        "white_balance_automatic" | "focus_automatic_continuous" | "auto_exposure" => 0,
        "pan_absolute" | "tilt_absolute" => 2,
        _ => 1,
    };
    controls.sort_by_key(|(name, _)| priority(name));

    let mut applied = 0usize;
    let mut errors = Vec::new();
    let mut notes: Vec<String> = Vec::new();
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
        // A saved position is never driven past the range the camera is driven over.
        let value = match name.as_str() {
            "pan_absolute" => backend::angles::degrees_to_raw(backend::angles::clamp_pan(backend::angles::raw_to_degrees(value))),
            "tilt_absolute" => backend::angles::degrees_to_raw(backend::angles::clamp_tilt(backend::angles::raw_to_degrees(value))),
            _ => value,
        };
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
        } else if key == "obsbot:vertical_tracking" {
            match *value {
                0..=2 => set_vertical_tracking(device, *value as u8),
                _ => Err("Unknown vertical tracking in preset.".into()),
            }
        } else if key == "obsbot:tracking_speed" {
            // Presets saved before: 0 Standard, 1 Sport (which is Motion, wire 2).
            match *value {
                0 => set_vertical_tracking(device, 0),
                1 => set_vertical_tracking(device, 2),
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
        } else if key == "app:white_balance_by" {
            // Presets saved before red and blue were taken out: nothing to do, the temperature is used.
            continue;
        } else if key == "obsbot:ai_auto_zoom" {
            set_ai_auto_zoom(device, *value == 1)
        } else if key == "obsbot:hand_side" {
            set_hand_side(device, *value == 1)
        } else if key == "obsbot:noise_reduction" {
            u8::try_from(*value).ok().filter(|v| *v <= 3).ok_or_else(|| "Unknown noise reduction.".to_string())
                .and_then(|level| set_noise_reduction(device, level))
        } else if key == "obsbot:auto_gain" {
            set_auto_gain(device, *value == 1)
        } else if key == "obsbot:mic_during_sleep" {
            set_microphone_during_sleep(device, *value == 1)
        } else if key == "obsbot:mic_disabled" {
            set_microphone_disabled(device, *value == 1)
        } else if key == "obsbot:mic_enabled" {
            // Presets saved before the switch became "Disable microphone".
            set_microphone_disabled(device, *value == 0)
        } else if key == "obsbot:radio_distance" {
            u8::try_from(*value).ok().filter(|v| *v <= 2).ok_or_else(|| "Unknown pickup distance.".to_string())
                .and_then(|distance| set_radio_distance(device, distance))
        } else if key == "obsbot:pop_filter" {
            set_pop_filter(device, *value == 1)
        } else if key == "obsbot:mic_volume" {
            set_mic_volume(device, (*value).clamp(0, 100) as i16)
        } else if key == "obsbot:hdr" {
            // Only when it changes: switching HDR takes the camera about three seconds.
            match xu_status(device).ok().and_then(|status| hdr_state_from_status(&status)) {
                Some(now) if now == (*value == 1) => Ok(()),
                _ => set_hdr_enabled(device, *value == 1),
            }
        } else if key == "obsbot:camera_sleep" {
            set_camera_sleep_time(device, (*value).clamp(0, 65535) as u32)
        } else if key == "app:tracking_area" {
            // With the preset's own area, if it has one (sent once, here).
            let end = |key: &str| preset.settings.get(key).map(|v| *v as f64 / 100.0);
            update_config_checked(|c| {
                c.tracking_area = *value == 1;
                if let (Some(a), Some(b)) = (end("app:zone_pan_low"), end("app:zone_pan_high")) { c.zone_pan_low = a; c.zone_pan_high = b; }
                if let (Some(a), Some(b)) = (end("app:zone_tilt_low"), end("app:zone_tilt_high")) { c.zone_tilt_low = a; c.zone_tilt_high = b; }
            }).and_then(|()| apply_tracking_area(device, &current_config()))
        } else if key.starts_with("app:zone_") {
            continue;
        } else if key == "obsbot:zone_auto_select" {
            set_tracking_area_auto_select(device, *value == 1)
        } else if key == "obsbot:return_pan_centi" {
            // The return position is one command: sent once, with its tilt and zoom.
            match (preset.settings.get("obsbot:return_tilt_centi"), preset.settings.get("obsbot:return_zoom_milli")) {
                (Some(tilt), Some(zoom)) => set_zone_home(device, *value as f64 / 100.0, *tilt as f64 / 100.0, *zoom as f64 / 1000.0),
                _ => Err("The return position in this preset is incomplete.".into()),
            }
        } else if key == "obsbot:return_tilt_centi" || key == "obsbot:return_zoom_milli" {
            continue;
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
            // Already in that mode: nothing to change (and nothing to fail while a stream is running).
            Some(mode) if video::current_mode(device).is_some_and(|now| now.width == mode.width && now.height == mode.height
                && now.pixel_format == mode.pixel_format && now.frame_rate_milli.abs_diff(mode.frame_rate_milli) <= 500) => { applied += 1; }
            // Another program is streaming, so the camera refuses a new mode: it is changed once it is free.
            Some(mode) if !crate::backend::usage::streaming_holders(device).is_empty() => {
                let pending = config::VideoModeConfig { width: mode.width, height: mode.height, pixel_format: mode.pixel_format.clone(), frame_rate_milli: mode.frame_rate_milli };
                match update_config_checked(|config| config.pending_video_mode = Some(pending)) {
                    Ok(()) => notes.push("The capture mode changes when no program is using the camera (the background service does that).".to_string()),
                    Err(error) => errors.push(format!("video mode: the camera is in use, and the change could not be saved for later: {error}")),
                }
            }
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

    // The Studio camera's look, if the preset has one (the service picks it up within a couple of seconds).
    if let Some(look) = &preset.look {
        let missing = look.lut.as_ref().is_some_and(|name| !crate::lut::list().contains(name));
        if missing {
            errors.push(format!("look: the LUT {} is no longer in the list", look.lut.as_deref().unwrap_or("")));
        } else {
            crate::backend::update_config(|c| {
                if look.lut.is_some() { c.studio.lut = look.lut.clone(); }
                c.studio.lut_on = look.lut.is_some();
                c.studio.set_lut_strength(look.strength);
            });
            applied += 1;
        }
    }

    if errors.is_empty() {
        let mut done = format!("Submitted {applied} settings from preset '{}'.", preset.name);
        for note in notes { done.push(' '); done.push_str(&note); }
        Ok(done)
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
        // Hand tracking (7) is offered again: 03/00, as the camera reported it to OBSBOT's SDK.
        assert_eq!(ai_mode_payload(7), Some((0x03, 0x00)));
        assert_eq!(ai_mode_payload(6), Some((0x01, 0x00)));
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
