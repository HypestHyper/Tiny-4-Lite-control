# Tiny 2 Lite Control

A Rust/GTK4 Linux hardware-control test bench for the OBSBOT Tiny 2 Lite
(USB VID:PID `3564:fef9`). This is a feature/protocol development app with a compact test-bench interface,
not the final application design.

## Current capabilities

## UI layout and availability

The Controls page keeps the complete supported control set visible and groups standard V4L2 settings in this order: Exposure, White balance, Focus, Zoom, and Image. Gimbal movement, autofocus priority, Auto Exposure priority, exposure compensation, OBSBOT camera features, microphone processing, video capture, presets, and camera power are kept in separate related sections.

A setting that is unavailable because of another setting remains visible but disabled, with a note naming the blocking mode. Confirmed dependencies are:

- Red balance, blue balance, and white-balance temperature require Auto White Balance Off.
- Shutter speed requires Exposure mode Manual (menu value 1).
- Manual focus requires Continuous Autofocus Off.
- Auto Exposure priority and exposure compensation require Exposure mode Auto (menu value 0).

These controls are rebuilt after their controlling mode changes. Device-reported ranges, steps, and menu choices are used; a menu default that the driver reports outside its advertised choices is replaced with the current valid choice. A numeric current value outside its declared range is retained for diagnosis and shown disabled with the reported range. The Tiny 2 Lite's invalid `zoom_continuous` value is therefore not sent; `zoom_absolute` remains the supported zoom path.

The mouse wheel is prevented from changing dropdown selections or slider values. Pointer dragging and keyboard operation remain available. Mirror image is grouped under Image orientation rather than microphone processing.

## Headless UI validation

The application has an explicit synthetic-device mode for visual tests. It is enabled only by setting `TINY4LITE_UI_TEST=1`; this bypasses USB discovery and all V4L2/UVC I/O, disables USB capture so no TShark process can be started, and supplies fixture camera metadata instead. Optional variables select the fixture state and page:

- `TINY4LITE_UI_TEST_STATE=auto|manual`
- `TINY4LITE_UI_TEST_PAGE=controls|capture|diagnostics`

Run the screenshot harness after building:

```sh
bash app/tests/headless-screenshots.sh \
  app/target/release/tiny4lite-control \
  app/artifacts/headless-ui
```

The GitHub Actions workflow `.github/workflows/headless-ui.yml` runs unit tests, builds the GTK app, and uploads Auto/Manual/Aperture Priority screenshots at multiple scroll positions, a screenshot at the normal 900 × 680 window size, Capture, and Diagnostics. It also compares screenshots before and after wheel input over a slider and dropdown, then drags the enabled Auto exposure compensation slider and checks that its value label changes. These screenshots validate rendering and layout only; they do not validate physical camera behavior or vendor commands.


### Device discovery and standard V4L2 controls

- Discovers camera video nodes through sysfs and matches USB ID
  `3564:fef9`. It deliberately excludes regular Tiny 2 (`3564:fef8`).
- Detects USB camera presence separately from V4L2 node availability.
  Controls require a host video node bound to `uvcvideo`; USB capture
  can still work when the video interfaces are claimed by `usbfs`.
- Reads V4L2 control metadata at runtime with `v4l2-ctl`, including
  reported ranges, steps, defaults, flags, and menu entries. It does not
  hardcode camera control ranges.
- Exposes supported boolean, menu, and numeric controls. Numeric
  controls apply immediately and offer reset to the camera-reported
  default.
- Autofocus uses the standard UVC Focus Auto control. When autofocus is
  off, the manual Focus Absolute slider is shown. While autofocus is on,
  Global / Face options send the framed XU `CAM_SET_FACE_FOCUS`
  command (`0x3602`, receiver `0x02`). The 2026-11-18 controlled Linux
  capture correlates Global with `bf fb 01 00` and Face with
  `be 07 00 00`; the earlier app mapping had these reversed. The grouped
  selector now dispatches from the newly clicked active segment rather
  than both buttons' toggle-state signals. The final Face-to-Global
  transition in that capture emitted no focus SET_CUR, so this dispatch
  correction still needs a fresh hardware test. Treat Global/Face as
  not yet hardware-verified: successful USB transfer completion does not
  prove camera behavior, and camera-side read-back is unavailable.
  Selection is tracked locally as the last successfully submitted
  command, not actual camera state. This is distinct from face-priority
  auto-exposure.
- Auto Exposure priority has its own grouped Global / Face control, shown
  only when V4L2 `auto_exposure` is Auto (menu value 0). The 2026-11-18
  Linux capture identified selector `0x06` SET_CUR payloads:
  Face = `03 01 00`, Global = `03 01 01`, padded to 60 bytes with
  zeros. This is not the autofocus command. The last submitted selection
  is persisted locally, but no reliable device read-back is known.
  Exposure compensation (−3.0 to +3.0) is implemented through UVC
  Processing Unit 3, selector 1 (Backlight Compensation). The 19
  OBSBOT Center positions map to unsigned little-endian values 1–19:
  −3.0 = 1 (01 00), 0.0 = 10 (0a 00), and +3.0 = 19 (13 00).
  It is shown only while V4L2 Auto Exposure is Auto. The current value
  is initialized with UVC GET_CUR when supported, and successful changes
  are persisted locally.
- Uses `zoom_absolute` as the only manual zoom/FOV path. The user
  confirmed smooth adjustment from 0 to 100 and these presets: Wide=0
  (1.00×), Medium=5 (1.15×), Narrow=15 (1.47×). Preset buttons write
  this same V4L2 control and read its value back. The slider is refreshed
  after each preset; the user confirmed it displays 0, 5, and 15 for the
  three presets. `zoom_continuous` and
  the separate vendor FOV setter are not used.
- Pan and tilt use standard V4L2 controls: `pan_absolute`,
  `tilt_absolute`, `pan_speed`, and `tilt_speed`. Provides speed
  joystick control with sensitivity adjustment; releasing the joystick
  sends neutral speed. Bounded absolute pan/tilt steps are also available
  and move the physical gimbal immediately.

### OBSBOT vendor UVC Extension Unit controls

The XU transport uses unit 2, selector `0x02` for framed commands and
selector `0x06` for the 60-byte status block.

- AI tracking modes: Off, Normal, Upper Body, Close-up, Headless, Lower
  Body, and Group. Desk, Whiteboard and Hand are intentionally excluded
  from the supported app scope.
- Tracking speed: Standard and Sport. Both modes have been physically
  confirmed working on the user's Tiny 2 Lite. Sport encoding is
  firmware-sensitive; the current implementation sends the documented
  `0x02` value and does not automatically choose the older reported
  `0x21` encoding. Compatibility with older firmware remains unverified.
- Gestures: Target selection, Zoom/L-pose, Dynamic Zoom, and Dynamic
  Zoom Direction. All four gesture behaviours have been physically confirmed. Dynamic
  Zoom Direction reverses the direction of the Dynamic Zoom gesture, so
  these two settings work together: enable Dynamic Zoom for the two-hand
  L-shaped gesture, then use Direction Flip to reverse its direction.
  Direction Flip does not affect the separate Zoom/L-pose gesture or
  autofocus Global/Face priority.
- Framed XU writes and queries use one rolling process-local 16-bit
  sequence counter. It is seeded from the current millisecond clock on
  first use to reduce collisions with earlier camera sessions and other
  short-lived control processes. Every query reply is matched to its
  request sequence before decoding.
- Sleep and wake are explicit user actions. Both have been physically
  tested. The separate power-status read-back has also been tested.
- FOV presets are implemented through V4L2 `zoom_absolute`, not a
  separate vendor command: Wide=0, Medium=5, Narrow=15. The generic
  V4L2 slider covers 0–100.
- HDR: one switch. The 2026-11-18 controlled Linux capture identified the selector `0x06` SET_CUR payload prefix: `01 01 01` enables HDR and `01 01 00` disables it. Status byte `0x06` reads `0x01` for On and `0x00` for Off. The switch initializes from camera status and falls back to the last successfully submitted local setting only when status is unavailable. The capture showed three matching state transitions; the final checkpoint had no corresponding command and is treated as an incomplete recording, not evidence against the mapping.
- Microphone controls use XU unit 2 / selector 0x06. **Microphone
  during sleep** reads the shared status block at byte offset 0x10
  (Off=0, On=1). Its write is a separate tagged command:
  `13 01 <state>` (0=Off, 1=On), not a rewrite of the status block.
  This matches the documented Tiny 2 Lite command mapping. It is separate
  from **Disable microphone**, which uses its own fixed command
  `1c 01 <state>` (0=disabled, 1=enabled) and has no established GET_CUR
  read-back. Auto gain and noise reduction continue to read their
  respective status bytes. OBSBOT Center restarted the camera after each
  Disable microphone toggle; the restart mechanism is unknown.
- Pop filter: experimental On/Off switch under Microphone processing.
  It replays fixed session-specific data through XU unit 2 / selector
  0x02 (OFF uses byte-2 value 0x0f; ON uses 0x10). Sequence/check fields
  are not regenerated, and reliable camera read-back is unavailable.
  The source currently writes 0x01 at payload offset 32, while the
  investigation's written capture summary says offsets 32–59 are zero.
  Reconcile the complete raw 60-byte vectors before changing this payload.
  Physical ON/OFF validation is still required.
- Mirror image: experimental On/Off switch under Image orientation.
  It sends the captured 60-byte XU unit 2 / selector 0x06 payload:
  `14 01 01` for ON and `14 01 00` for OFF, followed by zeroes.
  The three controlled OBSBOT Center writes differed only at payload byte 2.
  The last successfully submitted state is saved locally; camera read-back
  is not established. Physical ON/OFF validation is still required.
- Radio distance: Close, Standard, and Far. XU unit 2 / selector 0x06
  sends a separate fixed 60-byte command with prefix
  `1b 01 <value>` (Close=0, Standard=1, Far=2), zero-filled
  remainder. No GET_CUR read-back is established; the app remembers the
  last successful local selection.
- LED: one setting control with Off and brightness levels 1, 2, and 3.
  Selector `0x06`, tag `0x1A`, uses raw values 0 through 3 directly.
  Status byte `0x21` reports the selected setting. Tag `0x18` is not
  treated as LED power: observed On makes the indicator blink, while
  Off leaves it continuously lit.
- Local application presets remain available for app-managed settings.
- Camera-native presets are now exposed separately. The Controls page
  reads the Tiny 2 Lite selector 0x0C occupancy list and selector 0x0D
  entry cursor, then supports create/update, rename, recall, and the
  two-frame delete sequence. Presets store the camera's live pose and
  zoom, not stale V4L2 target values.
- **Power-on initial state** is a separate camera feature. The Controls
  page reads its stored pose from query 0x3884 and can set the current
  live pose as the initial state using 0x3EC4 followed by 0x3E44. The
  existing 40-byte startup picture-settings block is read from 0x3E84
  and preserved.

The selector `0x06` status block provides AI tracking state/mode and
selected settings where documented, but it does not provide reliable
per-gesture read-back for Target Selection, Dynamic Zoom, or Direction
Flip. These controls show **Unknown** on startup unless a previously
successful local setting is saved; the first click from Unknown sends
Enable. Zoom/L-pose has a status-byte read-back. Unknown describes the
app's inability to determine the starting state, not a camera-reported
error. A successful ioctl only confirms host-side request completion;
use observable device behaviour or independent read-back to validate
state changes.

## Persistent configuration

Settings are stored outside the AppImage in:

```text
${XDG_CONFIG_HOME:-~/.config}/tiny4lite-control/config.toml
```

The versioned TOML config is loaded when the application starts and
written after successful changes. It records supported V4L2 control
values (excluding transient pan/tilt position and speed commands), LED
brightness, AI tracking mode and speed, HDR state, Global/Face focus and Auto Exposure priority selections,
gesture toggles, gesture zoom ratio, theme, joystick sensitivity, and
background-service policy.

The service policy is stored under `[service]`:
- `auto_sleep_enabled = false` by default. The future service must not
  start idle monitoring or send automatic sleep commands unless this is
  explicitly enabled.
- `auto_sleep_timeout_minutes = 10` is the default idle timeout and is
  validated to the range 1–1440 minutes.

The previous `settings.conf` theme preference is migrated on first load.
Missing keys in older config files receive safe defaults, so adding these
service fields does not invalidate existing user settings.

Loading the file does not send camera commands. In particular,
`apply_on_connect` defaults to `false`; saved camera values are retained
for the future in-process service, but are not replayed to the camera at
startup. Camera read-back remains authoritative for settings that the
device exposes. Locally remembered gesture/focus states are only the last
successfully submitted commands where no reliable read-back exists.

The config module provides version checking, defaults for missing keys,
safe value validation, and write-to-temporary-file followed by rename.
The settings module is implemented and loaded at application startup.
The dedicated in-process camera service that will consume its service
policy, own device monitoring, and implement optional idle sleep/wake
behavior is not implemented yet. The service should receive a config
snapshot at startup and reload policy only after an explicit settings
change, rather than independently guessing defaults.

## Diagnostics and USB capture

The Diagnostics page provides read-only XU reads for selectors `0x02`
and `0x06`, plus the model/stream name from selector `0x08` and USB
descriptor identity. USB `bcdDevice` is labelled as the device-release
field, not as installed firmware. The Controls page polls the framed gimbal-controller query
`0x0043` (receiver 3) every 250 ms for live physical pan/tilt. The reply
contains signed hundredths-of-a-degree pitch and yaw at offsets 8 and 10.
Zoom is read separately through `0x6804`. The `0x6484` command seen in
the captures is gimbal rate/speed control, not a position read-back.
Power-on initial state is read with `0x3884`; setting it uses the
captured `0x3EC4` pose frame followed by `0x3E44` startup settings.
The original 2026-11-19 capture contains both writes in the save-action
sequence. Linux physical position read-back still needs validation on
the user's camera.

The Capture page:

- Finds the camera's USB bus/device address directly through sysfs.
- Records continuous `usbmonN` traffic using `tshark`; elevated capture
  access is requested through `pkexec` when needed.
- Supports a baseline plus bracketed action windows. Enter the intended
  state, click **Begin action**, switch to OBSBOT Center and perform the
  change, then return and click **End action**. This records the interval
  around the action rather than requiring a simultaneous checkpoint click.
- Each action window stores both monotonic elapsed time and Unix epoch
  timestamps. The decoded packet rows include `frame.time_epoch`, allowing
  USB requests to be matched against the action window even after switching
  between applications. `frame.time_relative` is retained as well.
- Produces a raw `.pcapng` and a text report with baseline/action windows,
  transfer summaries, decoded control-transfer fields, URB IDs/status,
  absolute and relative packet timestamps, and payload bytes when TShark
  exposes them. The report also lists decoded control-transfer rows whose
  absolute timestamps fall inside each completed action window.
- Includes a browser for saved reports and their matching raw captures.

Capture output is saved under:

```text
${XDG_DATA_HOME:-~/.local/share}/tiny4lite-control/captures/
```

The capture tool records traffic. It does not infer unknown command
semantics automatically. Preserve original captures and keep unknown
bytes uninterpreted until isolated by controlled tests.

## Runtime

The AppImage bundles the GTK runtime dependencies and `v4l2-ctl`.
The host still supplies the Linux V4L2 kernel driver and access to the
camera device nodes. On Arch/CachyOS:

```sh
sudo pacman -S v4l-utils
sudo usermod -aG video "$USER"
```

Log out and back in after adding the user to the `video` group. Skip
that step if udev/logind already grants device access.

```sh
chmod +x Tiny4LiteControl-x86_64.AppImage
./Tiny4LiteControl-x86_64.AppImage
```

## Local build

```sh
sudo pacman -S --needed base-devel rust gtk4 v4l-utils wireshark-cli polkit
cd app
cargo test --release
cargo build --release
```

GitHub Actions builds the x86_64 AppImage from branch
`test-app-v0.1`. Check the workflow result before treating a new source
revision as build-verified.

## Safety and limitations

- Pan/tilt commands physically move the gimbal. Keep its movement area
  clear. The joystick sends neutral speed on release; absolute steps
  move immediately.
- Sleep/wake physically move the gimbal and remain explicit actions.
- No camera movement or sleep action runs automatically at startup.
- There is no separately installed background daemon or automatic
  sleep/wake service. Persistent TOML configuration is implemented;
  the in-process camera service and optional restore-on-connect behavior
  are still future work.
- Global / Face autofocus priority remains not yet hardware-verified.
  The current app mapping is Global `bf fb 01 00`, Face `be 07 00 00`.
  The 2026-11-18 capture had a missing final transition, and the user
  reports neither option currently produces the expected behavior. A
  fresh action-correlated capture and visible behavior test are required.
  Reliable read-back for most gesture states, automatic firmware
  detection, Linux physical validation of framed live gimbal pose query `0x0043`,
  HDR visible-effect validation, physical Mirror image validation,
  pop-filter prototype validation, and camera-native preset-slot
  operations remain unimplemented or unverified.
- Do not add undocumented writes or firmware-specific fallbacks without
  capture evidence and explicit hardware validation.

## Project documentation

- [Implementation status and protocol evidence](../Assistant/APP-IMPLEMENTATION-STATUS.md)
- [Feature inventory](../Assistant/reference/02-feature-inventory.md)
- [Project index](../Assistant/INDEX.md)
- [Agent and hardware-safety rules](../Assistant/AGENTS.md)
