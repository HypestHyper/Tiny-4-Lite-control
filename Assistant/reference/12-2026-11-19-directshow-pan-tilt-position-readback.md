# 2026-11-19 physical gimbal position read-back

**Device:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Windows API:** DirectShow `IAMCameraControl`  
**Linux transport:** UVC `UVCIOC_CTRL_QUERY`, Camera Terminal entity 1, PanTilt Absolute selector `0x0D`  
**Evidence status:** Live physical position is confirmed through Windows DirectShow. The equivalent standard UVC GET_CUR path is implemented in the Linux app; Linux hardware validation and raw-unit scale confirmation remain pending.

## 1. Result

The Tiny 2 Lite exposes actual physical pan and tilt position through its standard camera-control interface. A Windows test enumerated the OBSBOT video capture device, bound its DirectShow filter, queried `IAMCameraControl`, read the advertised ranges, and repeatedly called `IAMCameraControl::Get()` for Pan, Tilt, and Roll.

The user physically moved the gimbal head by hand. Pan and tilt readings changed continuously with that physical movement, settled at the mechanical endpoints, and returned near their center readings when released. This is independent position feedback, not a reconstruction from joystick commands.

Confirmed Windows calls:

- `IAMCameraControl::Get(CameraControl_Pan, ...)`
- `IAMCameraControl::Get(CameraControl_Tilt, ...)`
- `IAMCameraControl::Get(CameraControl_Roll, ...)`

The Linux app targets the corresponding standard UVC Camera Terminal PanTilt Absolute control directly. It does not use vendor Extension Unit selectors 0x02 or 0x06 for position.

## 2. Windows DirectShow test

The test enumerated the video input device whose friendly name contains `OBSBOT`, bound its `IBaseFilter`, queried `IAMCameraControl`, and read the control ranges with `GetRange()`. It then polled `Get()` every 250 ms.

### Advertised ranges

```text
PAN:  min=-130 max=130 step=1 default=0 caps=2
TILT: min=-90  max=90  step=1 default=0 caps=2
ROLL: min=-180 max=180 step=90 default=0 caps=2
```

The user chose the advertised Pan and Tilt ranges as the app's software display/clamp bounds:

| Axis | Software minimum | Software maximum |
|---|---:|---:|
| Pan | -130° | +130° |
| Tilt | -90° | +90° |

Do not expand these bounds to match the slightly larger observed mechanical readings.

### DirectShow flags clarification

The observed `caps=2` from `GetRange()` and `flags=2` from `Get()` are `CameraControl_Flags_Manual` values. They indicate manual-control capability/current manual mode. They do **not** mean the controls are read-only. An earlier interpretation that treated 2 as a read-only flag was incorrect.

### Physical movement observations

The user manually moved the gimbal head. Representative readings from the continuous log:

| Physical movement | Observed PAN | Observed TILT |
|---|---:|---:|
| Near initial center | +3 | +7 |
| Pan fully left | approximately -146 | approximately +5 to +10 |
| Pan fully right | approximately +150 | approximately +7 to +12 |
| Tilt fully down | approximately 0 | -89 |
| Tilt fully up | approximately +1 to +2 | +90 to +91 |
| Returned near center | approximately +1 to +2 | +6 to +7 |

Intermediate readings changed during movement. Pan traversed approximately -146 through 0 to +150, while tilt traversed -89 through 0 to +91. Roll remained 0 throughout this test.

The observed mechanical endpoints exceed the advertised Pan range by up to 20 degrees and Tilt by about 1 degree. The application intentionally clamps displayed values to the advertised bounds.

The test was performed by gently moving the camera head by hand, not by moving the Windows application's joystick. This distinguishes actual pose feedback from commanded movement targets. Do not force the gimbal against mechanical resistance.

## 3. USB descriptor evidence

The camera descriptors report:

- Camera Terminal entity ID: `1`
- VideoControl interface: `0`
- Camera Terminal advertises `PanTilt (Absolute)`, `PanTilt (Relative)`, and `Roll (Absolute)`.
- PanTilt Absolute selector: `0x0D`.
- PanTilt Absolute data: two signed 32-bit values, pan followed by tilt.
- Roll Absolute is a separate standard control (selector `0x0F`); changing roll was not demonstrated.

The vendor Extension Unit is a separate entity (ID 2) with 19 advertised controls. It is not the source of this confirmed position path.

## 4. Direct UVC request for Linux

The standard Camera Terminal PanTilt Absolute read is a class/interface control-IN request:

| Field | Value |
|---|---:|
| `bmRequestType` | `0xA1` |
| `bRequest` | `0x81` (`GET_CUR`) |
| `wValue` | `0x0D00` (selector 0x0D) |
| `wIndex` | `0x0100` (entity 1, interface 0) |
| `wLength` | `8` |

Equivalent Linux `UVCIOC_CTRL_QUERY` arguments:

- `unit = 1` (Camera Terminal entity)
- `selector = 0x0D` (PanTilt Absolute)
- `query = 0x81` (GET_CUR)
- `size = 8`

Response layout:

- Bytes 0–3: signed little-endian 32-bit pan.
- Bytes 4–7: signed little-endian 32-bit tilt.

The existing app's V4L2 movement controls use 3600 raw units for a one-degree step and ranges of ±468000 for pan and ±324000 for tilt. The new decoder uses that established app scale (3600 raw units per degree), then clamps the display to Pan ±130° and Tilt ±90°. The raw-unit scale must be confirmed against a live Linux GET_CUR response before treating the degree conversion as hardware-validated.

This is a read-only request. It must not send SET_CUR, alter camera state, or move the gimbal.

## 5. Why earlier USB captures did not show this

The dedicated 2026-11-19 USB capture showed:

- Selector 0x02 GET_LEN and SET_CUR traffic associated with proprietary movement command data.
- Selector 0x06 GET_LEN and GET_CUR traffic for a static status/configuration block.
- No Camera Terminal selectors 0x0D or 0x0E.
- No observed traffic on the advertised interrupt endpoint 0x86.

That capture records what OBSBOT Center chose to request. It does not establish that the Camera Terminal control is unavailable. The Windows DirectShow test subsequently demonstrated that the Windows camera stack can read live Pan and Tilt values.

The selector 0x02 float32 values at payload offsets 20 and 24 remain movement-associated outgoing command data, not position feedback. Selector 0x06 remains status/configuration data, not pose.

## 6. Current Linux app implementation

The current app no longer treats the selector-0x02 movement command payload as telemetry. The active read path uses the framed gimbal-controller query:

- Query command: `0x0043`, receiver `0x03`, through XU unit 2 / selector `0x02`.
- The reply contains signed little-endian 16-bit pitch at payload offsets 8–9 and yaw at 10–11, in hundredths of a degree.
- App display convention: pan = yaw / 100; tilt = -(pitch / 100).
- Current zoom factor is read separately through command `0x6804`, receiver `0x04`.

The Controls page initializes the **Gimbal position** readout immediately and polls at 4 Hz (every 250 ms). It displays signed degree values and reports the exact query error when unavailable. This query is read-only and does not move the camera. The live pose path is also used when saving camera-native presets and setting the startup pose, avoiding stale V4L2 movement targets.

**Capture clarification:** the repeated `0x6484` frames in the user's 2026-11-19 capture are gimbal rate/joystick commands carrying three float32 speeds (roll, pitch, yaw). They are not a live-position reply. The float32 values previously decoded at offsets 20/24 were movement-command payloads and must not be presented as telemetry.

The standard Camera Terminal entity 1 / selector `0x0D` path described in Sections 3–4 remains a separate documented interface. It is not the active Linux app position source.

## 7. Captured Set Initial State operation

The saved USB capture `obsbot-capture-1795052714-353354509.pcapng` does contain the camera-native initial-state write sequence. The earlier interpretation that the action was unmapped was wrong.

The capture contains:

- A host-to-camera framed SET_CUR command `0x3EC4`, payload length 24. The first four bytes are source index 3 (an independent pose); the remaining fields are pan, pitch, roll, zoom and trailing float.
- A host-to-camera framed SET_CUR command `0x3E44`, payload length 40. This is the startup picture/action settings block.

The camera's stored startup pose is read with query `0x3884`. Its 24-byte reply starts with the source index (0–2 means a preset slot; 3 means an independent pose), followed by the pose values. The 40-byte action block is read with query `0x3E84`.

The app now exposes **Power-on initial state** with:
- A read-only display of the camera's stored startup pose from `0x3884`.
- **Set current position as initial state**, which reads live pose via `0x0043`, zoom via `0x6804`, and the existing 40-byte settings block via `0x3E84`.
- The captured `0x3EC4` own-pose frame (source index 3), followed by `0x3E44` carrying the existing settings block unchanged.
- A `0x3884` read-back to verify that the startup pose was stored.

Preserving the existing `0x3E84` block avoids silently clearing the camera's startup AI/picture settings while changing only the initial pose.

## 8. Validation status and next hardware check

**Implemented in source:**
- Framed query builder and sequence-matched reply polling.
- Live pose decode for `0x0043` and live zoom decode for `0x6804`.
- 250 ms UI refresh.
- Startup pose read-back through `0x3884`.
- Startup pose write through `0x3EC4` + `0x3E44`, preserving the current 40-byte settings block.
- Camera-native preset list/read/create/update/rename/recall/delete.

**Still requires physical validation on the user's Linux camera:**
- Confirm the `0x0043` values follow physical hand movement in the new build.
- Confirm **Set current position as initial state** survives a camera power cycle.
- Confirm camera-native preset create, rename, recall and delete on the user's current firmware. The command family is documented as Tiny 2 Lite hardware-tested, but the app's integration still needs a live pass.

## 8. Safety and scope

- The live-position readout sends only a framed GET query (`0x0043`) and does not issue movement commands.
- Setting the startup pose is a separate explicit user action that writes `0x3EC4` and `0x3E44`.
- The app does not move the camera or change its startup pose automatically on launch.
- Software display bounds remain Pan ±130° and Tilt ±90°.
- Roll remains outside this feature until its live behaviour and encoding are separately validated.
