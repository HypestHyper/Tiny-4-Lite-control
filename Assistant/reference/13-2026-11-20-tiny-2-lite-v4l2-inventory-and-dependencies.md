# OBSBOT Tiny 2 Lite: V4L2 hardware inventory and dependency matrix

**Capture date:** 2026-11-20  
**Target:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Observed hardware revision:** `0x0510` (1296; USB bcdDevice `5.10`)  
**Observed Linux driver:** `uvcvideo`, kernel/driver version `7.3.4`  
**Host used for this capture:** CachyOS / Arch Linux  
**Purpose:** Device-specific baseline for the Tiny 2 Lite control application. This is a record of observed hardware metadata and controlled dependency tests, not a substitute for querying the connected device at runtime.

## 1. Evidence scope and confidence

This note transcribes the user's direct terminal output from:

- `v4l2-ctl --all -d /dev/video1`
- `v4l2-ctl --list-ctrls-menus -d /dev/video1`
- `v4l2-ctl --list-formats-ext -d /dev/video1`
- `media-ctl -d /dev/media0 -p`
- `lsusb -v -d 3564:fef9`
- Controlled changes to `auto_exposure`, `white_balance_automatic`, and `focus_automatic_continuous`, followed by fresh `--list-ctrls-menus` queries.

Evidence labels used below:

- **Observed:** directly reported by this device/driver in the supplied output.
- **Dependency-confirmed:** the control's V4L2 `inactive` flag changed in response to a controlled parent-setting change.
- **Unverified behavior:** metadata or a successful control request does not prove a visible image/physical effect.

The USB descriptor command printed `Couldn't open device, some information will be missing`, but still returned the device/configuration/interface descriptors below. Treat the descriptor dump as useful but potentially incomplete.

## 2. Device identity and nodes

| Property | Observed value |
|---|---|
| USB VID:PID | `3564:fef9` |
| Manufacturer / product | Remo Tech Co., Ltd. / OBSBOT Tiny 2 Lite |
| USB device revision | `5.10` |
| Hardware revision | `0x00000510` |
| Negotiated USB speed | High Speed (480 Mbps) |
| USB configuration | 1 |
| Interfaces | 4 |
| UVC version | 1.00 |
| UVC Extension Unit | Unit 2 |
| Extension Unit GUID | `9a1e7291-6843-4683-6d92-39bc7906ee49` |
| Extension Unit controls | 19 |
| Processing Unit | Entity 3 |
| Camera Input Terminal | Entity 1 |
| Video Control interface | 0 |
| Video Streaming interface | 1 |
| Audio Control interface | 2 |
| Audio Streaming interface | 3 |

Observed Linux nodes:

- `/dev/video1`: Video Capture, bound to `uvcvideo`; the node used for the V4L2 control inventory.
- `/dev/video2`: separate metadata-capture node, not a second image-capture stream.
- `/dev/media0`: media-controller topology for the same physical camera.

Media topology:

`Camera 1` sensor → `Processing 3` → `Extension 2` → `OBSBOT Tiny 2 Lite` V4L2 node.

The camera's Video Capture format at the time of `--all` was MJPEG 1920×1080 at 30 FPS. The format enumeration is broader; see below.

## 3. Video formats and frame rates

### MJPEG

| Resolution | Advertised frame rates |
|---|---|
| 3840×2160 | 30, 25, 24, 20, 15 FPS |
| 1920×1080 | 60, 50, 30, 25, 24, 20, 15 FPS |
| 1280×720 | 60, 50, 30, 25, 24, 20, 15 FPS |

### YUYV 4:2:2

| Resolution | Advertised frame rates |
|---|---|
| 640×360 | 30, 25, 24, 20, 15 FPS |
| 640×480 | 30, 25, 24, 20, 15 FPS |

The USB descriptor identifies the video stream as MJPEG plus uncompressed YUYV. The selected `--all` format is a current selection, not the complete capability list.

### USB audio interface

The same USB device also advertises:

- Two-channel microphone input.
- 16-bit PCM.
- 32,000 Hz sample rate.
- Feature Unit entity 4 with mute and volume controls advertised at master channel.

This records descriptor capabilities only; it does not establish audio quality or application-level routing behavior.

## 4. Complete V4L2 control inventory

Values below are the baseline readout before the dependency test loop. The current value is a snapshot, not a recommended default or a persistent factory setting.

### 4.1 User Controls

| Control | Type | Minimum | Maximum | Step | Driver default | Baseline current | Baseline flags |
|---|---|---:|---:|---:|---:|---:|---|
| `brightness` | Integer | 0 | 100 | 1 | 50 | 50 | — |
| `contrast` | Integer | 0 | 100 | 1 | 50 | 54 | — |
| `saturation` | Integer | 0 | 100 | 1 | 50 | 50 | — |
| `hue` | Integer | 0 | 100 | 1 | 50 | 50 | — |
| `white_balance_automatic` | Boolean | — | — | — | 1 | 0 | — |
| `red_balance` | Integer | 0 | 2048 | 1 | 1024 | 1043 | — |
| `blue_balance` | Integer | 0 | 2048 | 1 | 1024 | 1312 | — |
| `gain` | Integer | 1 | 64 | 1 | 1 | 1 | — |
| `power_line_frequency` | Menu | 0 | 2 | — | **3 (invalid)** | 2 | — |
| `white_balance_temperature` | Integer | 2000 | 10000 | 100 | 5000 | 3000 | — |
| `sharpness` | Integer | 0 | 100 | 1 | 50 | 50 | — |
| `backlight_compensation` | Integer | 0 | 18 | 1 | 9 | 18 | — |

Menu entries for `power_line_frequency`:

| Value | Meaning |
|---:|---|
| 0 | Disabled |
| 1 | 50 Hz |
| 2 | 60 Hz |

**Driver quirk:** the reported default is 3, which is outside the valid menu range 0–2. The current value was 2 (60 Hz). An application must validate the default against the menu entries and must not select or display 3 as a valid option. Prefer a valid read-back value; if no valid current value exists, represent the state as unknown or choose an explicitly documented fallback rather than trusting the invalid default.

### 4.2 Camera Controls

| Control | Type | Minimum | Maximum | Step | Driver default | Baseline current | Baseline flags |
|---|---|---:|---:|---:|---:|---:|---|
| `auto_exposure` | Menu | 0 | 3 | — | 0 | 0 (Auto) | — |
| `exposure_time_absolute` | Integer | 1 | 2500 | 1 | 330 | 330 | inactive |
| `pan_absolute` | Integer | -468000 | 468000 | 3600 | 0 | -50400 | — |
| `tilt_absolute` | Integer | -324000 | 324000 | 3600 | 0 | -50400 | — |
| `focus_absolute` | Integer | 0 | 100 | 1 | 0 | 100 | inactive |
| `focus_automatic_continuous` | Boolean | — | — | — | 1 | 1 | — |
| `zoom_absolute` | Integer | 0 | 100 | 1 | 0 | 15 | — |
| `zoom_continuous` | Integer | -100 | 100 | 1 | 100 | **245 (out of range)** | — |
| `pan_speed` | Integer | -80 | 80 | 1 | 10 | 0 | — |
| `tilt_speed` | Integer | -120 | 120 | 1 | 20 | 0 | — |

Menu entries for `auto_exposure`:

| Value | Meaning |
|---:|---|
| 0 | Auto Mode |
| 1 | Manual Mode |
| 3 | Aperture Priority Mode |

Menu value 2 is not listed as a valid entry.

**Out-of-range current value:** `zoom_continuous` reports current value 245 despite a declared range of -100 to 100. Do not clamp this value and write it back, infer a valid state from it, or expose it as a normal interactive control without further investigation. The app should mark the current value invalid/out-of-range and avoid writing to this control. Use `zoom_absolute` for the verified manual zoom path.

Pan/tilt range and step correspond to ±130° pan and ±90° tilt in 1° increments when interpreted using the established 3600 raw-units-per-degree scale. The user has separately confirmed the physical hand-movement range as Pan -130°…+130°, Tilt -90°…+90°, Roll -180°…+180° (Roll is not exposed in this V4L2 control list).

## 5. Experimentally confirmed V4L2 dependencies

The test changed one parent control at a time, then queried `--list-ctrls-menus` again. The relevant driver flag transitions are:

### 5.1 Exposure mode → shutter time

| `auto_exposure` | Value | `exposure_time_absolute` |
|---|---:|---|
| Auto Mode | 0 | inactive |
| Manual Mode | 1 | active |
| Aperture Priority Mode | 3 | inactive |

This is directly confirmed by the V4L2 `inactive` flag changing only in Manual Mode. The app should keep shutter time visible but disable it in Auto and Aperture Priority, with a reason tied to the selected mode.

### 5.2 Continuous autofocus → manual focus

| `focus_automatic_continuous` | Value | `focus_absolute` |
|---|---:|---|
| Enabled | 1 | inactive |
| Disabled | 0 | active |

Manual focus is only available when continuous autofocus is disabled. Keep the manual focus control visible but disabled while continuous AF is on.

### 5.3 Automatic white balance → manual white-balance controls

| `white_balance_automatic` | Value | `red_balance` | `blue_balance` | `white_balance_temperature` |
|---|---:|---|---|---|
| Disabled | 0 | active | active | active |
| Enabled | 1 | inactive | inactive | inactive |

The driver marks all three manual white-balance controls inactive when automatic white balance is enabled.

### 5.4 Controls not observed to become inactive

Across the captured exposure, white-balance, and autofocus mode changes, these controls remained reported without the `inactive` flag:

- `brightness`
- `contrast`
- `saturation`
- `hue`
- `gain`
- `power_line_frequency`
- `sharpness`
- `backlight_compensation`
- `pan_absolute`
- `tilt_absolute`
- `zoom_absolute`
- `zoom_continuous` (although its baseline current value is invalid/out-of-range)
- `pan_speed`
- `tilt_speed`

This means the driver did not report these controls as inactive in the tested states. It does **not** prove every write has a visible effect or that every combination is meaningful.

### 5.5 Test-sequence note

The exposure snapshots were captured while iterating Auto → Manual → Aperture Priority. The later white-balance and autofocus snapshots were captured with exposure left in Aperture Priority. This does not affect the specific dependency conclusions above, but it is important context if comparing full snapshots byte-for-byte. The script restored the camera to Auto exposure, manual white balance, and continuous autofocus enabled after the tests.

## 6. Application implementation rules derived from this hardware

1. **Runtime metadata remains authoritative.** This document is the reference baseline for the tested device/revision and kernel driver, not permission to hard-code control availability. Query the selected camera node's live control IDs, types, ranges, steps, menu entries, defaults, current values, and flags.
2. **Use reported inactive flags.** Also retain the known dependency explanations in the UI so disabled controls are understandable. Do not infer additional inactive relationships from unrelated controls.
3. **Validate defaults and current values.** A driver default can be invalid (`power_line_frequency=3`); a current value can be outside its own range (`zoom_continuous=245`). Validate both independently.
4. **Never silently clamp invalid current values and write them.** Show an invalid/unknown state and avoid sending an unintended value.
5. **Use `zoom_absolute` for manual zoom.** Do not use `zoom_continuous` based on this baseline.
6. **Keep standard V4L2 autofocus separate from OBSBOT Face/Global focus priority.** `focus_automatic_continuous` controls whether continuous AF is enabled; `focus_absolute` is manual focus. Face/Global priority is a separate vendor XU command path, with imperfectly correlated payload-label evidence and no reliable device-side read-back. Do not conflate the two or claim the priority mode is confirmed by the V4L2 state.
7. **Do not equate successful ioctl completion with a visible result.** Image appearance, physical movement, and vendor feature behavior require separate hardware validation.

## 7. Raw capture provenance

The source output was supplied directly in the project conversation on 2026-11-20. The original terminal text is not embedded verbatim here; the exact reported values, menu entries, formats, node topology, and relevant inactive-flag transitions are transcribed into this structured reference. The source commands were:

```bash
v4l2-ctl -d /dev/video1 --all
v4l2-ctl -d /dev/video1 --list-ctrls-menus
v4l2-ctl -d /dev/video1 --list-formats-ext
media-ctl -d /dev/media0 -p
lsusb -v -d 3564:fef9
```

The controlled dependency test used `auto_exposure=0,1,3`, `white_balance_automatic=0,1`, and `focus_automatic_continuous=0,1`, querying `--list-ctrls-menus` after each change.

## 8. Remaining questions

- Re-run `lsusb -v -d 3564:fef9` with sufficient USB permissions to ensure no descriptor sections were omitted.
- Investigate why `zoom_continuous` reads 245 while declaring -100…100. Do not probe by writing speculative values.
- Test the actual visual effect of gain and backlight compensation; a reported writable range is not proof of visible effect.
- Keep Face/Global autofocus priority as a separate XU investigation. The payload-to-label mapping is capture-derived and behavior remains unverified.
- Re-capture the inventory after a firmware update or significant kernel/uvcvideo change and compare it with this baseline.

