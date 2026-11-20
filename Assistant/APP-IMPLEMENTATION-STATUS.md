# Application implementation status

**Branch:** `test-app-v0.1`  
**Application:** `app/`  
**Target:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Stage:** Feature/protocol test bench. Core device discovery, generic V4L2 controls, selected vendor XU commands, diagnostics, and USB capture are implemented. The current GTK UI is temporary; further UI redesign is deferred.

This file records what is currently implemented in the repository and the protocol findings used by it. It is not a claim that every command has been independently validated on every firmware version.

## 1. Current implementation

### GTK family design pass

- Replaced notebook tabs with a rounded sidebar navigation and a compact stack switcher below 760 px.
- Added persisted System / Light / Dark theme selection. System reads the XDG desktop appearance portal when `gdbus` is available; fallback uses GTK's dark-theme preference.
- Bundles Google Sans Flex and Material Symbols Rounded as process-local fonts. Font files are embedded in the binary, cached under the user's cache directory, and registered through fontconfig; they are not installed system-wide. Both upstream licences are included under `app/assets/fonts/`.
- Added semantic light/dark surface and accent tokens, rounded page surfaces, consistent control shapes, and keyboard focus styling.
- Added a Saved captures browser to the Capture page. It lists saved text reports and opens the selected report or matching raw PCAPNG trace. It does not delete or modify captures.
- Remaining validation: live portal theme-change updates, reduced-motion integration, accessibility/screen-reader checks, text scaling, and visual review under Adwaita/Breeze. The System theme is currently resolved when selected or when the app starts, not live-updated after an external desktop theme change.


### Compact Controls page and headless visual validation

- The Controls page now orders standard V4L2 controls by workflow: Exposure, White balance, Focus, Zoom, then Image. Gimbal movement and its position readout are grouped first; OBSBOT camera features, AI tracking, microphone processing, presets, video capture, and power remain distinct sections.
- Mirror image is grouped under Image orientation, not Microphone processing.
- Known dependent controls remain visible while unavailable, with the blocker named below the control: manual white balance requires Auto White Balance Off; shutter speed requires Manual exposure; manual focus requires Continuous Autofocus Off; Auto Exposure priority and compensation require Auto exposure.
- Changing the relevant V4L2 mode rebuilds the page so dependent controls update immediately. Reported ranges, steps, and menu choices are retained. Invalid menu defaults are normalized to a supported choice; out-of-range numeric current values are displayed disabled with the driver-reported range.
- Mouse-wheel input is blocked on dropdowns and sliders so page scrolling does not accidentally alter settings. Dragging and keyboard operation remain available.
- A synthetic-device mode guarded by `TINY4LITE_UI_TEST=1` bypasses hardware discovery and every V4L2/UVC operation. GitHub Actions builds the GTK app, runs unit tests, launches it under Xvfb, and uploads Auto/Manual/Aperture Priority Controls, scrolled Controls, Capture, and Diagnostics screenshots. These are UI-only checks, not hardware validation.

The application is a Rust + GTK4 test bench. Current source of truth:

- `app/src/main.rs`
- `app/Cargo.toml`
- `app/README.md`

### Device discovery

- Enumerates Linux V4L2 video nodes through `/sys/class/video4linux` and matches USB vendor/product IDs `3564:fef9`.
- V4L2 controls require a video node bound to `uvcvideo`; they are unavailable while the camera's video interfaces are claimed by `usbfs` for VM passthrough.
- USB camera presence is detected separately through `/sys/bus/usb/devices`, so the app can distinguish a missing camera from a camera with no host V4L2 node.
- Excludes regular Tiny 2 (`3564:fef8`).
- Reads control metadata from the selected node at runtime using `v4l2-ctl`.

### Persistent settings and runtime architecture

- Added `app/src/config.rs`, a versioned TOML configuration module using
  Serde/TOML. The file is stored at
  `${XDG_CONFIG_HOME:-~/.config}/tiny4lite-control/config.toml`.
- The application loads the config at startup and creates a default file
  on first launch. If the legacy `settings.conf` theme file exists, its
  System/Light/Dark value is migrated without deleting the old file.
- Successful setting changes are persisted: supported V4L2 control
  values, LED brightness, HDR state, AI tracking mode, tracking speed,
  Global/Face focus and Auto Exposure priority selections, gesture toggles, gesture zoom ratio, theme, and
  joystick sensitivity.
- Transient commands are deliberately excluded: pan/tilt position and
  speed movement, joystick motion, and Sleep/Wake are actions, not
  persistent preferences.
- The config is versioned, defaults missing fields, rejects config
  versions newer than the app supports, clamps joystick sensitivity to
  10–200%, rejects invalid gesture zoom ratios, and writes via a
  temporary file followed by rename. Unit tests cover safe defaults and
  round-tripping persisted settings.
- `apply_on_connect` is reserved and defaults to `false`. Loading
  settings does not send camera commands or move the gimbal. Camera
  read-back remains authoritative where available; locally saved
  gesture/focus values represent the last successfully submitted
  setting where the camera has no reliable read-back.
- This is the persistence foundation, not yet the complete service
  architecture. A dedicated in-process camera service for serialized
  device ownership, monitoring, disconnect/reconnect handling, and
  optional restore-on-connect has not been implemented. No separately
  installed system daemon is planned.

### 2026-11-20 device-specific V4L2 baseline

The user's OBSBOT Tiny 2 Lite was queried directly on CachyOS with `uvcvideo` driver version 7.3.4. The complete observed control inventory, ranges, defaults/current values, USB/media topology, video formats, invalid values, and controlled dependency matrix are recorded in [reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md](reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md).

The controlled tests confirmed:
- `auto_exposure`: exposure time is active only in Manual mode; inactive in Auto and Aperture Priority.
- `focus_automatic_continuous`: manual focus is active only when continuous autofocus is disabled.
- `white_balance_automatic`: red balance, blue balance, and white-balance temperature are active only when automatic white balance is disabled.
- The other listed controls did not acquire the V4L2 `inactive` flag in the tested states. This does not prove every control produces a visible effect.

Two important driver anomalies were observed: `power_line_frequency` advertises default 3 despite valid menu values 0–2; `zoom_continuous` reports current value 245 despite its declared -100…100 range. The app must validate defaults and current values independently, avoid writing invalid values, and continue to query live metadata rather than hard-code this snapshot.

### Capture-mode application and verification

The Video capture controls enumerate discrete format/resolution/frame-rate combinations from the selected V4L2 node. Applying a mode now reads back both the active format (`VIDIOC_G_FMT`) and frame interval (`VIDIOC_G_PARM`) and only reports success when the driver retained the requested dimensions/format and a frame rate within 0.5 fps of the requested nominal rate. A driver fallback is reported with both requested and actual modes instead of being treated as success. The UI warns that other camera viewers can reclaim the previous mode; close OBS, CameraControls, and other stream consumers before changing capture modes. This verifies the mode at the time of the read-back; it does not prevent another process from changing it afterward. Hardware validation on the user's machine is still required.

### Standard V4L2 control path

- Lists controls and menu metadata from `v4l2-ctl --list-ctrls-menus`.
- Reads and writes controls through `v4l2-ctl --get-ctrl` and `--set-ctrl`.
- Supports the camera-reported boolean, menu, and numeric controls.
- Handles continuous autofocus and manual focus availability. The 2026-11-17 Windows trace confirmed standard UVC Camera Terminal Focus Auto selector `0x08` (one-byte Boolean) and Focus Absolute selector `0x06` (two-byte little-endian value). It captured 14 Focus Absolute writes (observed values 0–100) after autofocus was disabled. These are observed values, not a validated min/max.
- When autofocus is enabled, Global / Face options send the Tiny 2 Lite framed Camera command `CAM_SET_FACE_FOCUS` (`0x3602`, receiver `0x02`) through XU unit 2 / selector `0x02`. The controlled 2026-11-18 Linux usbmon capture correlates Global with payload `bf fb 01 00` and Face with `be 07 00 00` (details in the focus capture evidence below). The previous implementation had these payloads reversed. The grouped GTK selector also dispatched from `toggled` signals on both buttons; it now dispatches from the newly clicked active segment only. The final Face-to-Global transition in the capture produced no focus SET_CUR, so the UI dispatch fix still requires a fresh hardware test. Treat Global / Face as **not yet hardware-verified**: USB transport completion does not prove the camera applied the priority, and there is no reliable camera-side read-back. Do not describe the selected local button or saved config value as actual camera state. When autofocus is disabled, Global / Face options are hidden and the manual Focus Absolute slider is shown. Keep this distinct from face-priority auto-exposure, which is selector `0x06` tag `0x03`.

- Auto Exposure Face / Global priority is implemented as a separate grouped control, shown only while the V4L2 `auto_exposure` menu is set to Auto (value 0). It writes a 60-byte `UVC_SET_CUR` to XU unit 2 / selector `0x06`: Face = `03 01 00`, Global = `03 01 01`, with the remaining 57 bytes zero. The selected value is persisted locally but is not replayed on startup. No reliable camera read-back has been established; the UI shows the last successfully submitted selection, not confirmed device state. This control is unrelated to autofocus priority and must remain separate.
- The 2026-11-18 Linux capture contains five selector `0x06` writes: frames 80075 (`03 01 00`), 146705 (`03 01 01`), 219369 (`03 01 00`), 297559 (`03 01 01`), and 300515 (`03 01 00`). These repeated transitions support Face = 0 and Global = 1. The final checkpoint was labelled Global, but the last actual write is Face; retain that checkpoint discrepancy rather than changing the byte mapping to fit the label.
- Exposure compensation is implemented as a UVC Processing Unit
  control, separate from V4L2 controls and the OBSBOT Extension Unit.
  The camera descriptor identifies Processing Unit entity 3; selector 1
  is Backlight Compensation. The controlled 2026-11-18 Linux capture
  mapped all 19 OBSBOT Center positions to consecutive 16-bit
  little-endian values: −3.0 = 1 (01 00), −2.7 = 2, −2.3 = 3,
  −2.0 = 4, −1.7 = 5, −1.3 = 6, −1.0 = 7, −0.7 = 8, −0.3 = 9,
  0.0 = 10 (0a 00), +0.3 = 11, +0.7 = 12, +1.0 = 13,
  +1.3 = 14, +1.7 = 15, +2.0 = 16, +2.3 = 17, +2.7 = 18,
  +3.0 = 19 (13 00). The request is host-to-device class/interface
  SET_CUR (bmRequestType 0x21, bRequest 0x01, wValue 0x0100,
  wIndex 0x0300, wLength 2). In the UVC ioctl API this is unit 3,
  selector 1, SET_CUR, with a two-byte unsigned little-endian value.
  The UI exposes 19 positions only while V4L2 Auto Exposure is Auto,
  reads the current position with GET_CUR when available, and persists
  the last successful selection. This is not a V4L2 exposure-bias control.
- Zoom and FOV now share one standard V4L2 path. The Tiny 2 Lite reports
  `zoom_absolute` from 0 to 100; the user confirmed smooth adjustment
  and preset values Wide=0 (1.00×), Medium=5 (1.15×), Narrow=15
  (1.47×), with 100 as maximum. The GTK app has three preset buttons
  that write `zoom_absolute` and read it back, plus the generic
  device-reported slider. After the preset callback was fixed to refresh
  the controls, the user confirmed that the slider displays 0, 5, and
  15 after selecting Wide, Medium, and Narrow respectively. `zoom_continuous` and the separate vendor
  FOV setter are intentionally unused.
- Uses the device-reported ranges, steps, menu entries, and defaults rather than assuming one fixed set of controls.
- Pan/tilt use standard V4L2 controls: `pan_absolute`,
  `tilt_absolute`, `pan_speed`, and `tilt_speed`. Provides speed
  joystick control and bounded absolute steps. The joystick sends
  neutral speed on release.

- **Physical position read-back:** Windows DirectShow `IAMCameraControl::Get()` confirmed live Pan/Tilt changes with hand-moved gimbal position. The current Linux app implementation follows the later selector `0x02` GET_CUR candidate: 60-byte XU response, accepted observed record signatures, float32 pan/tilt at offsets 20/24, clamped to Pan [-130°, +130°] and Tilt [-90°, +90°]. The Controls page reads immediately, then polls every 250 ms. This candidate remains unverified as a host-initiated live read: the 2026-11-19 selector `0x02` capture contained SET_CUR movement records but no GET_CUR requests. The earlier standard Camera Terminal entity 1 / selector `0x0D` GET_CUR path remains documented in reference 12 but is not the active `get_gimbal_position` implementation. Do not claim Linux hardware validation until the user confirms live values move with the physical head. The separate captured “set initial state” action is not implemented: its host-to-camera command has not yet been isolated from the surrounding `AA 29` traffic.

### Vendor UVC Extension Unit path

The XU transport is separate from V4L2. Current code uses:

- UVC ioctl: `UVCIOC_CTRL_QUERY` (`0xc0107521`).
- XU unit: `2`.
- `UVC_SET_CUR`: `0x01`.
- `UVC_GET_CUR`: `0x81`.
- Status selector: `0x06`, read as a 60-byte block.
- Command/frame selector: `0x02`.

Implemented XU operations:

- Read AI status and decode the tracking mode from offsets `0x18` and `0x1c`.
- Read the tracking-speed status byte at `0x24`.
- Set HDR through selector `0x06` using the captured 60-byte payload prefix. HDR On is `01 01 01`; HDR Off is `01 01 00`. Status byte `0x06` reports `0x01` for On and `0x00` for Off. The GTK switch initializes from this status byte and persists the last successfully submitted setting as a fallback when status is unavailable.
- Microphone controls use XU unit 2 / selector 0x06:
  - Disable microphone: byte offset 2 of its separate fixed 60-byte
    command payload, 0 = disabled, 1 = enabled.
    The controlled 2026-11-18 capture recorded Disable microphone
    On → Off → On, with SET_CUR prefixes `1c 01 00`, `1c 01 01`,
    `1c 01 00`. The camera restarted after each toggle in OBSBOT
    Center. The restart mechanism and any additional restart-related
    commands have not been investigated. The UI exposes a **Disable
    microphone** switch. This is a distinct captured 60-byte command
    payload with prefix `1c 01 <state>` and zero-filled remaining bytes,
    not the GET_CUR status block. No matching read-back is established;
    the UI displays the last successfully submitted local state only.
  - Microphone during sleep: byte offset 16 in the shared 60-byte
    GET_CUR configuration block, Off = 0, On = 1. The 2026-11-18
    controlled capture confirmed both transitions and matching read-back.
    The UI reads this value on page construction and uses GET_CUR / patch
    byte 16 / SET_CUR for changes, preserving all other bytes. It is a
    separate setting from Disable microphone.
  - Auto gain: byte offset 25 in the GET_CUR configuration block,
    Off = 0, On = 1. The capture sequence
    was Off → On → Off → On, with matching read-backs.
  - Noise reduction: byte offset 8 in the GET_CUR configuration block,
    Off = 0, Weak = 1, Medium = 2,
    Strong = 3. All four levels were captured and read back.
  Auto gain and noise reduction perform GET_CUR, modify only their
  respective status-block byte, then SET_CUR the complete block,
  preserving unrelated fields. Disable microphone sends its separately
  captured fixed command payload. App-side hardware validation remains
  pending; successful ioctl completion alone is not proof of the
  camera's applied state.
- Radio distance is implemented through XU unit 2 / selector 0x06
  using a separate fixed 60-byte SET_CUR payload. The 2026-11-19
  controlled capture maps Close = `1b 01 00`, Standard = `1b 01 01`,
  and Far = `1b 01 02`; bytes 3–59 are zero. The GET_CUR responses
  observed around all three writes were identical, so no reliable
  camera read-back is established. The UI persists the last successful
  local selection and does not claim that it reflects a queried camera
  state.
- Set the AI tracking-mode tuple through selector `0x06`, opcode `0x16`.
- Send tracking-speed commands.
- Send enable/disable commands for target-selection gesture, Zoom/L-pose gesture, dynamic zoom, and dynamic-zoom direction. Dynamic Zoom Direction has an inverted wire-state mapping: UI enabled sends `e6 3f 00 00`; UI disabled sends `27 ff 01 00`. The 2026-11-18 capture checkpoints show Direction Flip OFF at baseline, ON at 17.37s, OFF at 35.15s, then Dynamic Zoom OFF at 54.78s. The user has physically confirmed that Target Selection, Zoom/L-pose, Dynamic Zoom and Direction Flip all work on the Tiny 2 Lite. Direction Flip is the direction modifier for the Dynamic Zoom gesture and is intended to be used together with Dynamic Zoom; it does not change the separate Zoom/L-pose gesture or autofocus Global/Face priority. The corresponding SET_CUR packets are frames 13247, 72265, 170871, and 179033. The first three use direction group `0a 04 c4 33` and alternate payloads `27 ff 01 00`, `e6 3f 00 00`, `27 ff 01 00`. The fourth uses Dynamic Zoom group `0a 04 44 33` and is the separate Dynamic Zoom disable command, not a Direction Flip transition. The sequence values progress `0x000f`, `0x0010`, `0x0011`, `0x0012` across these commands. All framed XU writes now use one rolling 16-bit sequence counter rather than fixed per-feature values. The observed initial value `0x000f` is capture context; the required sequence seed on a fresh camera/app session is not established. The implementation currently initializes its process-local counter to zero and increments it for each framed XU write.

Sleep/wake is implemented through the captured CAM_SET_DEV_STATUS frame and has been physically tested by the user. Automatic idle sleep timeout is not implemented. The app has local preset capture/apply support in `app/src/presets.rs`; camera-native preset-slot create/delete/recall/read/rename/list/update operations are not implemented.

### Read-only diagnostics and device identity

- The Diagnostics tab reads the 60-byte selector `0x06` status block and selector `0x02` command-channel response using GET_CUR only.
- It shows the selector `0x08` model/stream name and USB descriptor identity, including `bcdDevice` clearly labelled as a USB device-release value, not confirmed firmware.
- Status fields are decoded only where evidence exists. The legacy FOV byte (`0x11`) is retained only as raw diagnostic data; it is not used to control zoom. Candidate position floats from selector `0x02` remain labelled unverified.
- Camera settings expose V4L2 zoom presets (86° / 78° / 65°), an HDR switch, and LED settings. The old vendor FOV command and status-byte interpretation are superseded by the V4L2 zoom mapping. The separate legacy status-byte read-back mapping remains unverified. LED has four settings total: Off plus brightness levels 1, 2, and 3. The selector 0x06/tag 0x1A command uses raw values 0, 1, 2, and 3 for those four settings respectively; status byte 0x21 reports the setting. The previous implementation incorrectly subtracted one from the requested level, making level 1 send raw 0 (Off), and also used tag 0x18 as a separate power switch. User testing showed tag 0x18 On causes blinking and Off leaves the LED continuously lit, so it is not established as a power switch and is intentionally not used by the current UI.
- Sleep and wake controls send the captured Tiny 2 Lite power command. The command action and power-status read-back are separate; the user physically confirmed both sleep and wake behaviour.

### USB capture workflow

- A dedicated Capture tab records a continuous `usbmonN` trace for the Tiny 2 Lite USB bus.
- The app discovers the bus and device address directly by scanning `/sys/bus/usb/devices` for VID:PID `3564:fef9`; capture does not require a `/dev/videoN` node and works when interfaces are claimed by `usbfs` for VM passthrough.
- Capture runs through `pkexec tshark` where elevated usbmon access is required; the pcapng stream is written to a user-owned file.
- The user marks a baseline and subsequent state changes with timestamped text checkpoints.
- Finishing a session creates a text report containing the checkpoint list, a compact transfer summary grouped by transfer type and `bRequest/wValue/wIndex`, and TShark-decoded control transfers filtered to the camera's USB address. URB IDs/status, `usb.control.Response`, and `usb.capdata` are included as available.
- Raw `.pcapng` and `.txt` files are saved under `$XDG_DATA_HOME/tiny4lite-control/captures/` (or `~/.local/share/tiny4lite-control/captures/`).
- This is a recorder and evidence exporter, not an automatic protocol decoder or payload diff engine. Unknown bytes remain uninterpreted.

## 2. Protocol findings currently encoded

### AI tracking modes

The application currently maps these status/setter tuples:

| Mode | Tuple |
|---|---|
| Off | `00 00` |
| Normal tracking | `02 00` |
| Upper body | `02 01` |
| Close-up | `02 02` |
| Headless | `02 03` |
| Lower body | `02 04` |
| Group | `01 00` |
| Desk mode | `05 00` |
| Whiteboard | `04 00` |
| Hand | setter `03 00`; status decoder accepts `03 00` or `06 00` |

The Hand tuple discrepancy is intentionally visible in code: the Lite-tested setter uses `(0x03, 0x00)`, while the status decoder accepts both reported forms. Do not normalize this without new capture/read-back evidence.

### Tracking speed

- Standard and Sport commands are implemented and both modes have now been physically confirmed working on the user's Tiny 2 Lite.
- Sport encoding `0x02` is associated with firmware 6.2.5.x in the research.
- An older/legacy value `0x21` has been observed/reported for older firmware.
- Current app sends the documented Sport command but does not automatically identify firmware or choose the legacy encoding.
- Hardware behaviour is confirmed on the tested camera/firmware. Cross-firmware compatibility remains an open research item, not a pending basic hardware test.

### Gesture commands

Four command groups are encoded. The user has physically confirmed Target selection (open palm/fingers spread enables normal tracking), Zoom/L-pose, and Dynamic Zoom (both hands form L shapes and move apart/together). The 2026-11-18 capture establishes the Direction Flip wire payload mapping, and the user has now physically confirmed that the corrected direction reversal works on the camera. Direction Flip modifies the direction of the Dynamic Zoom gesture, so the two settings work together. It is not related to Zoom/L-pose or Global/Face autofocus priority. Device read-back remains unavailable, so the UI tracks the last successfully sent state only. These are live hardware observations, not protocol read-back.

- Target selection: `0a 04 c4 30 01 00`
- Zoom/L-pose: `0a 04 44 31 01 00`
- Dynamic zoom: `0a 04 44 33 01 00`
- Dynamic zoom direction: `0a 04 c4 33 01 00`; unlike the other gesture commands, its wire-state payload is inverted: OFF = `27 ff 01 00`, ON = `e6 3f 00 00`. Unit tests check both payload choices and rolling sequence increments. The capture-derived full frame prefixes are recorded in the evidence section below.

Enable/disable frames are built with the current frame builder. The status block does not provide reliable per-gesture read-back for Target Selection, Dynamic Zoom, or Direction Flip, so a successful ioctl only confirms host-driver request completion. The user has separately confirmed the visible behavior of all four gesture controls on the camera; the app still cannot reconstruct the initial state of most gestures from device read-back.

### Frame construction and tests

- The current frame builder constructs a 60-byte XU frame, including sequence, header, payload length, and CRC-16.
- Unit tests in `app/src/main.rs` cover the tracking-speed frame builder, generic gesture frame construction, Dynamic Zoom Direction payload selection, rolling sequence increment behavior, zoom-ratio bounds/frame construction, sleep-frame construction, LED payload mapping, and AI-mode payload shape. The Dynamic Zoom Direction test validates the payload mapping and counter progression; it does not reproduce the entire captured transaction stream.
- A separate unit test checks the AI-mode setter payload.
- These tests validate local byte construction against stored examples. They do not prove live-device acceptance, firmware compatibility, or command semantics beyond the evidence behind those examples.

## 3. Safety and command behaviour

- XU status reads are read-only.
- AI mode, tracking speed, and gesture writes occur only after an explicit action in the test app.
- Pan/tilt speed is live while the control is held/dragged and returns to zero on release.
- Absolute pan/tilt steps cause immediate physical movement.
- Commands should remain serialized. Stop testing on USB errors, disconnects, unexpected movement, or inconsistent status.
- Keep the camera's movement area clear before any movement test.
- Do not add guessed commands, firmware-specific fallbacks, or preset mutation without documenting evidence and obtaining explicit approval.

## 4. What is not implemented

- No automatic byte-diff view or semantic inference for unknown XU records; capture reports now include a transfer summary and pairing aids, while preserving raw decoded rows.
- No automatic firmware-version identification. USB `bcdDevice` and XU model identity are displayed, but neither is treated as the installed OBSBOT firmware revision.
- No reliable per-gesture state read-back for Target Selection, Dynamic Zoom, or Direction Flip. Their controls therefore start as **Unknown** unless a previous successful local setting is saved; the first click from Unknown sends Enable. Zoom/L-pose has a status-byte read-back. All four gesture behaviours have been physically confirmed, including Direction Flip reversing Dynamic Zoom. Unknown is a read-back limitation, not a claim that the gesture feature itself is unsupported.
- Global / Face autofocus priority remains **not yet hardware-verified**. The 2026-11-18 capture establishes the likely payload mapping (Global `bf fb 01 00`, Face `be 07 00 00`) and shows that the final Face-to-Global UI transition emitted no focus SET_CUR. The app payload mapping and grouped-toggle dispatch were corrected in commit `8fd0878`; a fresh capture and visible camera-behavior test are still required. The camera exposes no reliable read-back, and successful USB completion must not be presented as proof of applied focus behavior.
- No complete protocol coverage for all features listed in the research inventory.
- Pop filter has an experimental fixed-payload prototype in the Controls page. It is not a complete protocol implementation: sequence/session fields remain unresolved, GET_CUR read-back is unavailable, and focused ON/OFF hardware validation is pending. The source currently sets payload byte 32 to `0x01`; [reference/09](reference/09-pop-filter-protocol-investigation.md) describes bytes 32–59 as zero for the captured prototype. Resolve this discrepancy against the raw fresh-session capture before changing the payload.
- A first GTK family styling and compact-layout pass is implemented, but the final UI redesign is not complete. Remaining validation includes live theme updates, reduced motion, accessibility/screen-reader behavior, text scaling, and visual review under Adwaita/Breeze (see the GTK family design pass above).
- GitHub Actions AppImage build passed for zoom-slider refresh commit
  `e9df89705798070d7409f3d98d2268b2e8ceccc7` (run
  [44226690186](https://github.com/HypestHyper/Tiny-4-Lite-control/actions/runs/44226690186)).
  The config wiring commits `ee85464` and `f7cc6cb` also passed
  AppImage builds (runs
  [44227385377](https://github.com/HypestHyper/Tiny-4-Lite-control/actions/runs/44227385377)
  and
  [44227463033](https://github.com/HypestHyper/Tiny-4-Lite-control/actions/runs/44227463033)).
  The latest config-initialization change `27af898` and subsequent
  documentation commits still require their own final build result.
  Do not treat the newest branch head as build-verified until its
  workflow succeeds.

## 5. Evidence status and next investigation

### 2026-11-18 Tiny 2 Lite Dynamic Zoom Direction capture

The user supplied a 54.78-second controlled capture with timestamped checkpoints. The relevant command requests were extracted from the original PCAPNG using TShark `-V`, which exposed the 60-byte `Data Fragment` field even though `usb.capdata` was empty for these usbmon submission records.

| Frame | Relative time | Checkpoint interpretation | Function group | 60-byte payload prefix |
|---:|---:|---|---|---|
| 13247 | 2.964235s | Baseline: Direction Flip OFF | `0a 04 c4 33 01 00` | `aa250f000c00da810a04c433010027ff0100` |
| 72265 | 16.141447s | Direction Flip ON | `0a 04 c4 33 01 00` | `aa2510000c00eb240a04c4330100e63f0000` |
| 170871 | 38.165125s | Direction Flip OFF | `0a 04 c4 33 01 00` | `aa2511000c00bae10a04c433010027ff0100` |
| 179033 | 40.003863s | Dynamic Zoom OFF | `0a 04 44 33 01 00` | `aa2512000c002b2e0a0444330100e63f0000` |

The fourth request is not a Direction Flip transition. The checkpoint at 54.78s records the final state marker after the Dynamic Zoom-off action; no additional SET_CUR packet was among the four extracted requests. The observed sequence values increment by one across the three Direction Flip writes and the separate Dynamic Zoom disable write. The payload values establish the Direction Flip inversion relative to the other gesture toggles. They do not establish the sequence counter's required starting value after reconnect/restart, nor do they independently prove that the corrected app implementation reverses the visible gesture direction.

For this capture, the useful extraction was:

```sh
tshark -r "$HOME/.local/share/tiny4lite-control/captures/obsbot-capture-1794965274-629799401.pcapng" \\
  -Y 'frame.number == 13247 || frame.number == 13252 || frame.number == 72265 || frame.number == 72268 || frame.number == 170871 || frame.number == 170872 || frame.number == 179033 || frame.number == 179058' \\
  -V
```

This is the source evidence for the current Direction Flip payload mapping and rolling-sequence investigation. The raw capture remains on the user's machine; the repository records the relevant extracted bytes and interpretation.

The 2026-11-17 Windows OBSBOT Center capture has now been parsed from the original PCAPNG. It contains 3,674,198 frames (about 868 MiB), 1,236 camera control submissions and 1,236 completions. See [reference/06-2026-11-17-capture-findings.md](reference/06-2026-11-17-capture-findings.md) for exact packet evidence.

New findings:

- Standard UVC Focus Auto (Camera Terminal selector `0x08`) and Focus Absolute (selector `0x06`) were written and read back. The app already exposes these through its generic V4L2 control discovery; no special implementation is needed.
- Standard UVC Zoom Absolute (selector `0x0B`) writes closely correlate with an XU selector `0x02` float record and matching GET_CUR values. Treat the XU record as zoom-associated, but do not claim digital/optical processing or add a duplicate control.
- Existing tracking-speed, target-selection gesture, Zoom/L-pose gesture, sleep/wake, and preset opcode families also appear in the trace.
- Correctly paired selector 0x02 GET_CUR responses contain candidate pan/tilt float32 values at payload offset 20. The evidence is strong enough to justify a controlled read-only polling test, but not yet enough to claim live position telemetry. OBSBOT Center did not poll selector 0x02 during the dedicated AI-tracking capture.
- Several XU records remain unknown. The capture's Finish operation hung, so timestamped UI checkpoint descriptions were not recovered. Do not infer individual UI actions from timestamps alone.

Next work: test host-initiated selector 0x02 GET_CUR polling during idle, manual movement, and AI tracking. Record all 60-byte responses and correlate the offset-20 float32 pair with visible gimbal movement. See [reference/07-2026-11-17-ptz-position-readback.md](reference/07-2026-11-17-ptz-position-readback.md) for the transport tuple, samples, Windows KS topology investigation, and test plan. Until that experiment succeeds, do not expose the candidate values as verified live position. Continue offline decoding of unresolved XU records against Lite-specific references, preserving unknown fields. Any new setter must have a known payload and independent read-back/observable validation before implementation.

Use the capture workflow and template linked from the [tracing documentation on main](https://github.com/HypestHyper/Tiny-4-Lite-control/tree/main/Assistant/tracing). Important: that generic guide is HID-focused. Tiny 2 Lite controls are V4L2/UVC XU; reuse its evidence discipline, not HID-specific assumptions.

## Mirror image (experimental implementation)

A controlled Linux OBSBOT Center capture on 2026-11-19 identified three XU unit 2 /
selector 0x06 SET_CUR writes (60-byte payloads). Mirror ON appeared twice and
OFF once. All three payloads were zero-filled except the first three bytes:
`14 01 01` for ON and `14 01 00` for OFF. The repeated ON command matched
exactly. This supports a simple payload builder with byte 2 representing the
observed state.

The app now exposes an experimental **Mirror image** switch alongside the
microphone processing controls. It sends the captured 60-byte payload and
remembers the last successfully submitted state locally. No reliable camera
read-back is established. The payload mapping is capture-supported, but
physical ON/OFF behaviour from this app has not yet been validated. Do not
describe a successful ioctl as proof that the camera applied the setting.

See [reference/10-mirror-image-protocol.md](reference/10-mirror-image-protocol.md)
for the packet frames, action windows, decoded request, and exact payloads.

## Selector 0x02 motion-associated SET_CUR stream (2026-11-19)

A controlled usbmon3 capture (`obsbot-capture-1795096511-064087666.pcapng`) contains 4,686 USB records: endpoint counts `0x81`=2,916, `0x80`=1,102, `0x00`=668, and no `0x86` interrupt-IN traffic. It recorded 207 host-to-camera selector `0x02` SET_CUR writes with 60-byte `AA 25` payloads, plus 207 GET_LEN requests returning `3C 00`. Two little-endian float32 fields at payload offsets 20 and 24 vary during the controlled pan/tilt windows and return to zero when movement stops. Frame 2897 independently confirms the extraction (`30 8f ad be` at offset 20 ≈ -0.33898; `fc f2 58 40` at offset 24 ≈ 3.38983).

The capture contains no selector `0x02` GET_CUR requests. Selector `0x06` GET_LEN returned `3C 00` (69 requests), and its 66 GET_CUR responses were byte-for-byte identical throughout the movement sequence. Thus this capture supports classifying the changing selector `0x02` floats as movement-associated **outgoing command data**, not position feedback. Their exact command semantics (velocity, relative displacement, target, or other) remain unknown. Do not label them confirmed pan/tilt telemetry or reuse the packets as a new movement-command template. Keep this evidence separate from the earlier selector `0x02` GET_CUR position hypothesis.

See [reference/11-2026-11-19-selector-02-motion-payloads.md](reference/11-2026-11-19-selector-02-motion-payloads.md) for the request tuple, payload layout, sample values, capture checkpoints, extraction commands, and next investigation.

## Pop filter (experimental fixed-payload prototype)

A dedicated evidence record lives at [reference/09-pop-filter-protocol-investigation.md](reference/09-pop-filter-protocol-investigation.md). It contains the three controlled captures, nine 60-byte SET_CUR payloads, action-window timestamps, frame numbers, capture procedure, TShark commands, and unresolved payload-generation details.

The Controls page contains an experimental On/Off switch. The current builder replays captured session-specific command data; it does not generate a fresh sequence/check field. No reliable GET_CUR read-back exists, and the fixed replay may become stale after another application advances the camera's command sequence.

**Source/evidence discrepancy:** the builder writes `0x01` at payload offset 32, while reference 09 section 7 says offsets 32–59 are zero and places the seven-byte session-stable sequence at offsets 25–31. Re-extract the complete 60-byte vectors from the fresh-session raw capture and reconcile every offset before changing the builder. The next test should compare the app's actual ON/OFF writes against those verified vectors, then validate the visible effect. Do not claim the prototype is hardware-verified.

## 6. Related project documentation

- [Firmware identity and FOV/zoom evidence](reference/08-firmware-identification-and-fov.md)

- [Research index](INDEX.md)
- [Agent rules](AGENTS.md)
- [Original feature research](source/obsbot_tiny_2_lite_linux_research_handoff.md)
- [Feature inventory](reference/02-feature-inventory.md)
- [2026-11-17 Windows capture findings](reference/06-2026-11-17-capture-findings.md)\n- [2026-11-17 PTZ position read-back investigation](reference/07-2026-11-17-ptz-position-readback.md)
- [Architecture and hardware test plan](reference/04-architecture-and-hardware-testing.md)
- [Application source](../app/src/main.rs)
- [Application build/runtime notes](../app/README.md)
