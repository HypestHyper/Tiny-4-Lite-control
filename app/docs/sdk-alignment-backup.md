# Backup: implementations replaced to match OBSBOT's SDK

Policy (2026-11-24): where this app and OBSBOT's SDK (libdev v2.1.0_8, captured with `tools/sdk-probe`) send
different bytes for the same feature, the SDK's version is taken as true. Every replaced implementation is kept
here, so it can be restored.

The last commit before these changes: `c21d688` (branch `claude/upbeat-rubin-gnae3f`). To see or restore a
file as it was: `git show c21d688:app/src/backend/device.rs`.

## Audio auto gain

**Before** (from our own controlled capture of selector 0x06 status, byte 25): read the whole 60-byte status
block, change byte 25, write the block back with selector 0x06 SET_CUR.

```rust
pub(crate) fn set_auto_gain(device: &str, enabled: bool) -> Result<(), String> {
    set_xu_status_byte(device, XU_AUDIO_AUTO_GAIN_OFFSET, u8::from(enabled))?;
    update_config(|config| config.audio_auto_gain = Some(enabled));
    Ok(())
}
```

**Now** (SDK `cameraSetAudioAutoGainU`): the tagged selector 0x06 command `17 01 01` (on) / `17 01 00` (off), like
mirror (`14 01 xx`) and listen while asleep (`13 01 xx`). Reading it back is unchanged (status byte 25).

## Tracking speed → Vertical tracking (command 0x0cc4)

**Before**: called "Tracking speed", two choices, Standard (wire 0) and Sport (wire 2), read from status byte
0x24 (non-zero = Sport); presets stored `obsbot:tracking_speed` = 0 (Standard) or 1 (Sport).

```rust
pub(crate) fn set_tracking_speed(device: &str, sport: bool) -> Result<(), String> {
    let group = [0x0a, 0x04, 0xc4, 0x0c, 0x01, 0x00];
    let command = if sport { [0x67, 0xfe, 0x02, 0x00, 0x00, 0x00] } else { [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00] };
    xu_send_frame(device, group, command)?;
    update_config(|config| config.tracking_speed = Some(if sport { "sport" } else { "standard" }.into()));
    Ok(())
}
```

```rust
Setting::custom("obsbot:tracking_speed", Tracking, "Tracking speed",
    "How quickly the camera follows movement.",
    Kind::Choice(vec![opt(0, "Standard", "Smooth and steady"), opt(1, "Sport", "Faster, for quick movement")]),
    |s| from_camera(s.byte(0x24).map(|b| Value::Int(i64::from(b != 0))))
        .or_else(|| if s.status.is_none() { from_saved(s.config.tracking_speed.as_deref().map(|v| Value::Int(i64::from(v == "sport")))) } else { None }),
    |d, v| sent(set_tracking_speed(d, v.as_i64() == 1))),
```

**Now** (SDK `aiSetTrackingModeR(AiVerticalTrackType)`): the same command and wire values, which the SDK names
vertical tracking: Standard 0, Headroom 1, Motion 2 (our "Sport" is Motion; Headroom is new). Presets store
`obsbot:vertical_tracking` with the wire value; old presets' `obsbot:tracking_speed` 1 is read as Motion.

## Metering priority (Face / Global exposure, selector 0x06 tag 0x03) and Reverse dynamic zoom

Both were taken from earlier captures of OBSBOT Center that the user describes as unreliable, and both are the
other way round from what OBSBOT's SDK sends (`cameraSetFaceAER`, `aiSetGestureCtrlIndividualR(3, …)`), so the
SDK's values are used now (2026-11-24, from commit `0d730dc`):

- Metering priority: Face sends `03 01 01` and reads status byte 7 = `01`; Global `03 01 00` / `00`.
  It was Face `03 01 00` / `00`, Global `03 01 01` / `01`.
- Reverse dynamic zoom (gesture command 0x33c4): on sends `01`, off `00`, like every other gesture. It was
  inverted (on `00`, off `01`).

The replaced code:

```rust
pub(crate) fn auto_exposure_mode_payload(face_priority: bool) -> [u8; 60] {
    let mut data = [0u8; 60];
    data[..3].copy_from_slice(&[0x03, 0x01, if face_priority { 0x00 } else { 0x01 }]);
    data
}

pub(crate) fn auto_exposure_mode_from_status(status: &[u8; 60]) -> Option<bool> {
    match status[XU_AUTO_EXPOSURE_PRIORITY_OFFSET] {
        0x00 => Some(true),  // Face
        0x01 => Some(false), // Global
        _ => None,
    }
}


pub(crate) fn gesture_command(group: [u8; 6], enabled: bool) -> [u8; 6] {
    // Dynamic Zoom Direction is the exception: its captured ON/OFF payloads
    // are inverted relative to the other gesture commands.
    let wire_enabled = if group == [0x0a, 0x04, 0xc4, 0x33, 0x01, 0x00] {
        !enabled
    } else {
        enabled
    };

    if wire_enabled {
        [0x27, 0xff, 0x01, 0x00, 0x00, 0x00]
    } else {
        [0xe6, 0x3f, 0x00, 0x00, 0x00, 0x00]
    }
}

```


## Red and blue balance (removed 2026-11-24, at the user's request)

The camera uses whichever of the colour temperature and the red/blue gains was written last, and writing red or
blue alone turned the picture green. Only the colour temperature is offered now; red_balance and blue_balance are
in `IGNORED_V4L2` and presets neither save nor write them. The settings as they were (registry.rs), with the
helper `set_white_balance_by_gains` still in device.rs:

```rust
        Setting::custom("app:white_balance_by", WhiteBalance, "Manual white balance by",
            "The camera uses one or the other, not both.",
            Kind::Choice(vec![opt(0, "Temperature", "Set the colour of your light; the camera works out the red and blue"), opt(1, "Red & blue", "Set the red and blue gains yourself")]),
            |s| from_saved(Some(Value::Int(i64::from(s.config.white_balance_by_gains)))),
            |d, v| set_white_balance_by_gains(d, v.as_i64() == 1).map(|()| Some(v)))
            .needs(awb, OFF, "Turn off Auto white balance to set white balance by hand.")
            .info("Measured on the camera: it uses whichever of Colour temperature and Red/Blue balance was changed last. Changing red or blue switches it to raw red and blue gains, where the middle (1024) is no correction at all, which looks green; changing the temperature switches back. So the app keeps to the one you choose here, and switching sends that one's values again."),
        Setting::v4l2("red_balance", "v4l2:red_balance", WhiteBalance, "Red balance",
            "Less red (towards cyan) or more red. The middle leaves it as it is.", ticked(0.0, 2048.0, 1.0, Fmt::Plain, vec![(1024.0, "●")]))
            .needs(awb, OFF, "Turn off Auto white balance to adjust red.")
            .needs(WB_BY, Cond::Is(1), "Choose Red & blue under Manual white balance by.")
            .info("A raw gain on the red channel, used instead of Colour temperature (the camera uses one or the other). 1024 is no gain. It is steep above the middle: measured, from 1024 to 1536 red nearly doubles."),
        Setting::v4l2("blue_balance", "v4l2:blue_balance", WhiteBalance, "Blue balance",
            "Less blue (towards yellow) or more blue. The middle leaves it as it is.", ticked(0.0, 2048.0, 1.0, Fmt::Plain, vec![(1024.0, "●")]))
            .needs(awb, OFF, "Turn off Auto white balance to adjust blue.")
            .needs(WB_BY, Cond::Is(1), "Choose Red & blue under Manual white balance by.")
            .info("A raw gain on the blue channel, used instead of Colour temperature (the camera uses one or the other). 1024 is no gain; under warm light, blue needs raising well above it."),

```

## Tracking speed names (2026-11-24)

Shown as "Vertical tracking: Standard / Headroom / Motion" (the SDK's names) for one build; back to
**Tracking speed: Standard / Sport** at the user's request. The wire values are unchanged (0 Standard, 2 Sport,
which the SDK calls Motion); Headroom (1) behaves like Standard on the user's camera and is not offered.


## Start-up position and saved positions (aligned with the SDK, 2026-11-25)

Probe round 5 captured the SDK's own writes: start-up position `aiSetGimbalBootPosR` is one write, 04 `0x3844`
(24 bytes: id 3, yaw, pitch, 0, zoom, 0); a saved position is added with 04 `0x3944` (same 24 bytes, last value 0),
updated with 04 `0x3a04`, named with 04 `0x3a84` and deleted with 04 `0x3984` (u32 id) alone. The code before:

```rust
pub(crate) fn set_initial_camera_state(device: &str) -> Result<InitialCameraState, String> {
    let (pan, tilt) = get_gimbal_position(device)?;
    let zoom = get_live_zoom_factor(device)?;
    let pose = initial_pose_payload(3, pan as f32, tilt as f32, zoom)?;
    // Read and preserve the existing initial-state picture/action block. If it
    // cannot be read, abort before changing the boot pose.
    let actions = query_obsbot(device, 0x04, 0x3e84)?;
    if actions.len() != 40 {
        return Err(format!("Initial-state settings reply is {} bytes; expected 40. Nothing was changed.", actions.len()));
    }
    send_camera_preset_command(device, 0x3ec4, &pose)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    send_camera_preset_command(device, 0x3e44, &actions)?;
    let saved = read_initial_camera_state(device)?;
    if (saved.pan - pan as f32).abs() > 0.2 || (saved.tilt - tilt as f32).abs() > 0.2 {
        return Err(format!("Initial-state write was sent, but read-back differs: requested Pan {pan:+.1}° / Tilt {tilt:+.1}°, camera reports Pan {:+.1}° / Tilt {:+.1}°.", saved.pan, saved.tilt));
    }
    Ok(saved)
}


// save_camera_preset, payload and command:
    for value in [pan as f32, -tilt as f32, 0.0f32, zoom, -1000.0f32] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    send_camera_preset_command(device, if update { 0x3e04 } else { 0x3944 }, &payload)?;
    std::thread::sleep(std::time::Duration::from_millis(100));

pub(crate) fn delete_camera_preset(device: &str, slot: u8) -> Result<(), String> {
    let index = preset_slot_index(slot)?.to_le_bytes();
    send_camera_preset_command(device, 0x3984, &index)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    const CLEARED: [u8; 44] = [
        0x00, 0x00, 0x00, 0x00, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    send_camera_preset_command(device, 0x3d84, &CLEARED)
}

```

## Removed: AI auto zoom (2026-11-24)

The user found it did nothing on their camera, so the switch was removed. What it sent:

```rust
/// AI auto zoom (aiSetAiAutoZoomR): write 04 0x0a44, u8, as captured from the SDK. The camera does not report it.
pub(crate) fn set_ai_auto_zoom(device: &str, on: bool) -> Result<(), String> {
    send_camera_preset_command(device, 0x0a44, &[u8::from(on)])?;
    update_config(|c| c.ai_auto_zoom = Some(on));
    Ok(())
}
```
