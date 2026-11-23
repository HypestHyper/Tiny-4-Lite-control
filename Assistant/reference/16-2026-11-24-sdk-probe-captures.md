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
