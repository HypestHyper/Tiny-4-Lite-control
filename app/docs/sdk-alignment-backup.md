# Backup: implementations replaced to match OBSBOT's SDK

Policy (2026-11-24): where this app and OBSBOT's SDK (libdev v2.1.0_8, captured with `tools/sdk-probe`) send
different bytes for the same feature, the SDK's version is taken as true. Every replaced implementation is kept
here, so it can be restored. Evidence: `Assistant/reference/16-2026-11-24-sdk-probe-captures.md`.

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
