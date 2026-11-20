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

## 6. Linux app implementation

**Current source update (2026-11-20):** the active `get_gimbal_position()` path was changed to the candidate XU unit 2 / selector `0x02` GET_CUR response, using the two observed six-byte record signatures and float32 fields at offsets 20 and 24. The Controls page initializes the readout immediately and polls every 250 ms. This is a candidate read path, not a confirmed replacement for the standard Camera Terminal request described above: the 2026-11-19 motion capture contains selector `0x02` SET_CUR traffic but no GET_CUR requests. Linux hardware validation is still required. The standard Camera Terminal helper remains in source but is not the active read path.

The separate **Set initial state** operation is not implemented. The 2026-11-19 capture's save-action window (epoch 1795052741.355522–1795052764.996286) includes camera-to-host `AA 29` response records, but the corresponding host-to-camera command has not been isolated from unrelated traffic. Do not synthesize a setter or substitute a local preset for this camera operation.

The app implements the current read-only candidate path in `app/src/main.rs`:

- Camera Terminal entity 1, selector 0x0D, GET_CUR, 8-byte buffer.
- Decode signed little-endian pan and tilt values.
- Convert using the existing 3600-raw-units-per-degree app scale.
- Clamp displayed values to Pan [-130°, +130°] and Tilt [-90°, +90°].

The Controls page adds a **Gimbal position** readout. It polls at 4 Hz (every 250 ms), displays signed degree values, and shows **Unavailable** with the ioctl error as a tooltip if the read fails. The timer ends when the readout widgets are destroyed. It performs no writes and does not move the camera.

The readout is created only when the selected video node advertises both enabled `pan_absolute` and `tilt_absolute` V4L2 controls.

## 7. Validation status and next hardware check

**Confirmed:**
- Windows DirectShow returns live physical Pan and Tilt values while the gimbal is manually moved.
- Advertised Windows ranges and selected software clamp values.
- Camera Terminal descriptor advertises PanTilt Absolute.
- Linux app request tuple and decoder are implemented as read-only code.

**Not yet confirmed on Linux:**
- Whether `UVCIOC_CTRL_QUERY` for entity 1 / selector 0x0D succeeds on the Tiny 2 Lite Linux V4L2 node.
- Whether the Linux response uses the 3600-raw-units-per-degree scale assumed from existing app controls.
- Whether values update smoothly at the 4 Hz UI polling interval.

The camera was owned by the Windows VM during the Windows test. Linux cannot query it concurrently while USB passthrough assigns it to the VM. To validate, return USB ownership to the Linux host, ensure the camera has a V4L2 node, launch the app, and gently move the gimbal by hand. Displayed values should change with physical movement and settle when stationary. If the ioctl fails, preserve the exact error; do not silently fall back to the old XU status block or cached V4L2 values.

## 8. Safety and scope

- This feature performs only Camera Terminal GET_CUR reads.
- No SET_CUR movement requests are added.
- No undocumented Extension Unit writes are added.
- The app does not restore position or move the camera on startup.
- Software display bounds remain Pan ±130° and Tilt ±90°.
- Roll remains outside this feature until its live behaviour and encoding are separately validated.
