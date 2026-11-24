# 2026-11-24 SDK probe captures (Tiny 2 Lite, firmware 6.2.8.1)

**Source:** `app/tools/sdk-probe` run on the user's machine: OBSBOT's Linux SDK (libdev v2.1.0_8 beta, a local
copy, not in this repository) called one function at a time with timestamped markers, while usbmon recorded
the bus (`usbmon`, bus of the camera, device address 10). The report was produced by `report.py` from the raw
pcapng. The SDK is only the means of producing traffic; the evidence is the captured frames and the camera's
replies.

**Confidence:** High for the frames below (exact bytes captured, and every header and payload CRC reproduces
with `crc16_usb` in `app/src/backend/device.rs`). What each command does to the picture or the gimbal was not
observed visually in this run, except that the camera accepted each write (the URB completed with status 0).

## Framing

The same framing as the commands already implemented (`obsbot_frame_with_payload`): XU unit 2, selector `0x02`,
60-byte SET_CUR.

```
aa KK SS SS 0c 00 HH HH 0a DD CC CC LL LL PP PP <payload> <stale bytes>
```

- `KK`: `0x25` write, `0x01` read request, `0x05` reset to default. Replies start `aa 29` (or `aa 09`, see the
  preset list below).
- `SS SS`: rolling sequence (LE). The reply echoes it.
- `HH HH`: CRC16 (`crc16_usb`) over the first 12 bytes with `HH HH` zeroed.
- `0a DD`: from host (`0a`) to module `DD`: `04` for AI/gimbal settings, `03` for the attitude read. Replies
  swap the two bytes.
- `CC CC`: command (LE in the frame; written below as a 16-bit number).
- `LL LL`: payload length; `PP PP`: CRC16 over bytes 12.. with `PP PP` zeroed.
- The SDK reuses its buffer, so bytes after the payload (and after the header in read and reset requests) are
  left over from earlier frames. They are not part of the command; this app sends zeros there.

Reads: a read request (`aa 01`, header only) is followed by a GET_CUR on selector `0x02`, whose reply carries the
value with the same layout.

## Commands

| What (SDK function) | Command | Payload | Captured |
|---|---|---|---|
| AI auto zoom on / off (`aiSetAiAutoZoomR`) | `0x0a44` | u8: 1 on, 0 off | `aa 25 10 00 0c 00 4a f6 0a 04 44 0a 01 00 27 ff 01` / `… 1b 33 … 01 00 e6 3f 00` |
| Pan direction reverse on / off (`aiSetGimbalYawDirReverseR`) | `0x3b84` | u8 | `aa 25 12 00 0c 00 7a e8 0a 04 84 3b 01 00 27 ff 01` |
| Limited-zone tracking on / off (`aiSetLimitedZoneTrackEnabledR`) | `0x5084` | u8 | `aa 25 1c 00 0c 00 5a f2 0a 04 84 50 01 00 27 ff 01` |
| Limited zone, yaw min: set / read / reset | `0x4c04` / `0x4c44` / `0x4f84` | f32 LE degrees | set −30: `aa 25 14 00 0c 00 bb 11 0a 04 04 4c 04 00 3b 97 00 00 f0 c1`; read reply −13.6 |
| Limited zone, yaw max: set / read / reset | `0x4c84` / `0x4cc4` / `0x4fc4` | f32 | set 30: `… 84 4c 04 00 3a 37 00 00 f0 41`; read reply 100.0 |
| Limited zone, pitch min: set / read / reset | `0x4d04` / `0x4d44` / `0x5004` | f32 | set −10: `… 04 4d 04 00 66 57 00 00 20 c1`; read reply −30.0 |
| Limited zone, pitch max: set / read / reset | `0x4d84` / `0x4dc4` / `0x5044` | f32 | set 10: `… 84 4d 04 00 67 f7 00 00 20 41`; read reply 30.0 |
| Limited zone, auto select: read | `0x4e44` | — | reply u8 0 |
| Limited zone enabled: read (`aiGetLimitedZoneTrackEnabledR`) | `0x50c4` | — | **no valid reply**: about 70 one-byte `00` GET replies, then the SDK gave up (ret −1). Not usable on this firmware. |
| Gimbal preset list: read | `0x3b44` | — | reply length 3: `02 00 01` (preset ids 2, 0, 1) |
| Zone preset list: read | `0x1344` | — | reply `aa 09 …`: none (the SDK returned an empty list) |
| Start-up position: read (`aiGetGimbalBootPosR`) | `0x3884` | — | reply 24 bytes: `03 00 00 00`, yaw 27.64, pitch −22.45, 0.0, zoom 1.0, 0.0 (f32) |
| AI status: read | `0x0104` | — | 12-byte reply |
| Gimbal state: read | `0x6604` | — | reply on module `04` |
| Gimbal attitude: read (`gimbalGetAttitudeInfoR`) | `0x0043`, module `03` | — | the SDK reported pitch −25.24, yaw 3.43 |

Values the SDK returned for the reads match the replies (for example yaw min `9a 99 59 c1` = −13.6).

## Not yet captured

Moves (`gimbalSetSpeedPositionR`, `cameraSetZoomWithSpeedAbsoluteR`), presets with names, the camera's own sleep
timer, voice control, and the remaining SDK functions. The probe can be extended for each, with the same
safety rules (`app/tools/sdk-probe/probe.cpp`).

## 2026-11-24 full run (`run.sh --toggles --moves`)

Every SDK function for the Tiny 2 series was called, except factory reset, preset and boot-position changes,
background images, the boot AI mode, privacy and firmware. Policy from here: **where this app and the SDK differ,
the SDK is taken as true**; replaced implementations are kept in `app/docs/sdk-alignment-backup.md`.

### Writes

| Feature (SDK function) | SDK bytes | This app | Result |
|---|---|---|---|
| HDR (`cameraSetWdrR`) | sel 0x06 `01 01 01` / `01 01 00` | same | identical |
| Face / Global exposure (`cameraSetFaceAER`) | sel 0x06 `03 01 xx`; status 0x07 follows | same bytes | identical. The SDK's `face_ae = 1` sends `01`, which our labelled capture of OBSBOT Center shows as Global: the SDK's parameter name is misleading; the bytes agree. |
| Autofocus priority (`cameraSetFaceFocusR`) | module 02 cmd 0x3602, u32 1 / 0 | same (Face 1) | identical |
| Mirror (`cameraSetImageFlipHorizonU`) | sel 0x06 `14 01 xx` | same | identical |
| Listen while asleep (`cameraSetMicrophoneDuringSleepU`) | sel 0x06 `13 01 xx` | same | identical |
| **Audio auto gain** (`cameraSetAudioAutoGainU`) | sel 0x06 `17 01 xx` | rewrote status byte 25 in the whole block | **changed to the SDK's** |
| Gestures 0 / 1 / 2 / 3 (`aiSetGestureCtrlIndividualR`) | cmd 0x30c4 / 0x3144 / 0x3344 / 0x33c4, u8 1 on / 0 off | same commands | identical; for 3 ("dynamic zoom direction") the SDK's on is `01`, while our "Reverse dynamic zoom" on is `00` from a labelled capture of OBSBOT Center: same bytes, opposite naming. |
| **Vertical tracking** (`aiSetTrackingModeR`) | cmd 0x0cc4, u8 0 Standard / 1 Headroom / 2 Motion | "Tracking speed" Standard 0 / Sport 2 | **renamed to the SDK's meaning; Headroom added** |
| AI mode (`cameraSetAiModeU`) | sel 0x06 `16 02 main sub` | same | identical. Status after each (no one in front of the camera): human modes read 0/0, group 1/0, hand 3/0; whiteboard (4) and desk (5) left the status at 3/0: not taken on this firmware. |
| Sleep / wake (`cameraSetDevRunStatusR`) | module 02 cmd 0xa0c2, u32 1 / 0 | same | identical |
| Recall preset (`aiTrgGimbalPresetR`) | cmd 0x39c4, u32 id + 4 × f32 1.0 | same | identical |
| Brightness, contrast, hue, saturation, sharpness, white balance, anti-flicker, exposure, focus, zoom | standard UVC controls (processing unit 3, camera terminal 1) | same standard controls through V4L2 | identical |
| AI auto zoom (`aiSetAiAutoZoomR`) | cmd 0x0a44, u8 | — | new |
| Pan reverse (`aiSetGimbalYawDirReverseR`) | cmd 0x3b84, u8 | — | new |
| Limited zone: enable / auto select / limits | cmd 0x5084 / 0x4e04 / 0x4c04, 0x4c84, 0x4d04, 0x4d84 (f32) | — | new |
| Camera's own sleep timer (`cameraSetSuspendTimeU`) | sel 0x06 `0b 02` + u16 seconds (600 = `58 02`; 0 off) | — | new |
| Voice commands (`cameraSetAudioCtrlStateU`) | sel 0x06 `15 02 cmd state` (cmd 0–6) | — | new (the camera accepted them; whether the Lite has voice control is unverified) |
| Zone-tracking LED pattern (`cameraSetLedCtrlU`) | sel 0x06 `18 01 xx` | — | new |
| AI on / off (`aiSetEnabledR`) | cmd 0x0244, u8 | — | new |
| Zone tracking (`aiSetZoneTrackStateR`) | cmd 0x08c4, u8 | — | new |
| AI track mode (`aiSetAiTrackModeEnabledR`) | cmd 0x0584 (8 bytes) / reset 0x0504 | — | new (a Tail Air oriented call) |

### Moves (all undone in the run)

| SDK function | Bytes |
|---|---|
| `aiSetGimbalMotorAngleR(pitch, yaw)` | cmd 0x6444, 3 × f32: roll (−1000 = leave), pitch, yaw |
| `aiSetGimbalSpeedCtrlR` / `aiSetGimbalStop` | cmd 0x6484, 3 × f32 / reset 0x6704 |
| `gimbalSpeedCtrlR(pitch, pan)` | module 03 cmd 0x0103, 3 × i16 degrees/s × 100 (pan 10 = `e8 03`) |
| `gimbalSetSpeedPositionR` | module 03 cmd 0x0283, 12 bytes |
| `gimbalRstPosR` | module 03 cmd 0x00c3, 6 zero bytes |
| `aiTrgGimbalBootPosR` | reset 0x3904 |
| `aiTrgLimitedZoneTrackInitPosR` | reset 0x4f44 |
| `cameraSetZoomAbsoluteR(2.0)` | standard UVC zoom 100 |
| `cameraSetZoomWithSpeedAbsoluteR(150, 5)` | module 02 cmd 0x1942, u32 speed, u32 ratio |

This app moves the gimbal with the standard UVC pan/tilt controls (speed and absolute), which also work; it does
not use these vendor commands yet.

### Reads with values

Presets: id 0 pan 1.56 / tilt −10.52, id 1 −17.30 / −31.40, id 2 90.80 / −13.30 (names of 5 bytes; info cmd
0x3a44, name cmd 0x3b04). Limited zone defaults (id −1): yaw −100…100, pitch −30…30; start position (cmd 0x4ec4)
0 / 0, zoom 1. The first probe run reset yaw min from −13.6 to its default −100; later runs restore what they read.

## 2026-11-24 decision: the SDK's values are used where the naming differed

The earlier OBSBOT Center captures behind Face/Global exposure and Reverse dynamic zoom are considered less
reliable than the SDK. The app now follows the SDK for both: Face exposure = `03 01 01` (status byte 7 = 1),
Reverse dynamic zoom on = `01`. This supersedes the Face = `00` mapping in
[02-feature-inventory.md](02-feature-inventory.md) and the 2026-11-20 capture notes. Old code:
`app/docs/sdk-alignment-backup.md`.

## Round 3: functions the headers leave out (`run.sh --hidden --toggles --moves`, firmware 6.2.8.1)

Called by demangled signature from libdev v2.1.0_8. Frames: `aa KK seq 0c 00 hdrCRC 0a DD cmdLE len payloadCRC
payload`; KK 25 write, 01 read, 05 reset, 21 default read; replies `aa 29`, or `aa 89` when the camera does not
answer that read.

| Function | Command | Payload | Result on the Tiny 2 Lite |
|---|---|---|---|
| `cameraGet/SetAutoSleepTimeR` | read 0xa182, set 0xa1c2 (module 02) | u32 LE seconds | read 0; set 600 `58 02 00 00` and back. Separate from the status-block sleep timer (`0b 02`) |
| `cameraGet/SetAudioVolumeR` | read 0x80c2, set 0x8102 (02) | i16 LE | read 50; 49 and back |
| `cameraGet/SetAudioAGCR` | read 0x8242 → `aa 89`; set = status block `17 01 xx` | u8 | read not answered; set is the auto gain we use |
| `cameraGet/SetAudioNoiseReduceR` | read 0x82c2 (02); set = status block `0a 01 level` | read: u8 on, u32 level | read on, level 2; set off = `0a 01 00`, back = `0a 01 02` (our existing noise reduction) |
| `cameraGet/SetAudioSourceMuteR` | read 0x89c2 → `aa 89`; set 0x8a02 (02) | u8 | mute on `01` / off `00`; no read-back |
| `cameraGet/SetGammaModeR`, `NightModeR`, `ImageStyleModeR` | reads 0x2f42 / 0x3002 / 0x3802 → `aa 89`; sets 0x2f82 / 0x3042 / 0x3842 | u32 | reads not answered (not on the Lite, probably); the sets wrote 0 |
| `cameraGetAutoCompensationR`, `cameraGetWhiteBalanceAreaR` | the SDK sent an `aa 41` frame, `aa 89` reply; returned −1 | — | not on the Lite |
| `cameraGet/SetWhiteBalanceComponentR` | no traffic | — | not on the Lite |
| `cameraGetFieldView` / `cameraSetFieldView` | read 0xab02 → `aa 89`; set = status block `21 10` + 4 × u32 | — | read not answered; the set wrote four zeros |
| `cameraGetUvcModeR` | read 0xc442 (02) | u32 | 0 |
| `cameraGet/SetVirtualTrackEnabledR` | read 0xaec2 → `aa 89`; set 0xae82 | u8 | not answered |
| `sysMgGet/SetLedEnabledR` | module 0x12: read 0x758d → `aa 89`; set 0x75cd | u8 | read not answered |
| `sysMgGet/SetLedBrightnessR` | read module 0x12 cmd 0x754d; set = status block `1a 01 xx` | u8 | read 3 (matches our LED brightness levels) |
| `aiGet/SetLimitedZoneTrackInitPosR` (return position) | read 0x4ec4, default read KK 21, set 0x4e84 (04) | 24 bytes: i32 id, f32 yaw, f32 pitch, f32 0, f32 zoom, f32 0 | read and default both 0 / 0 / zoom 1; written back byte for byte. Field order from the start-up position reply (0x3884), which the SDK decodes the same way |
| `aiSetHandZone{Yaw,Pitch}{Min,Max}R` / `aiRstHandZone…R` | set 0x2044 / 0x20c4 / 0x2144 / 0x21c4; reset (KK 05) 0x2604 / 0x25c4 / 0x2684 / 0x2644 | f32 | hand-tracking area; reset to default after |
| `aiSetGestureZoomR` / `TargetSelectR` / `DynamicZoomR` / `DynamicZoomMirrorR` | 0x3144 / 0x30c4 / 0x3344 / 0x33c4 | u8 | on/off; left off |
| `aiSetHandTrackGimbalEnabledR` | 0x26c4 | u8 | on/off |
| `aiSetZoneTrackGimbalEnabledR` | 0x0844 | u8 | off then on |
| `cameraSetUserFocusPos(x, y)` | 0x3582 (02) | f32 x, f32 y, 8 zero bytes, u32 100 | tap-to-focus at (0.5, 0.5) |
| `aiSetCameraZoomRatioR(2.0, -1)` | 0x6884 (04) | i32 −1, f32 2.0 | zoom read 2.5 s later was still 1.0 |
| `aiSetSelectTargetByPos(x, y, 0)` | 0x0404 (04) | f32 x, f32 y, u8 | click-to-track at (0.5, 0.5) |
| `aiTrgLimitedZoneTrackInitPosR` (round 2) | KK 05, 0x4f44 | 12 bytes left over from the previous command | the camera goes to the return position |

Effects on the camera to check: gamma, night mode and image style were written as 0 and the field of view block
as zeros because their reads were not answered; the status light was written off (0x75cd `00`) after its read
failed.
