# Tiny 2 Lite controls and protocol notes

> **About this file.** These are the control, protocol and configuration notes written for the original
> test bench (branch `test-app-v0.1`), kept because they record verified protocol behaviour and the
> evidence behind it. The window layout they describe has been replaced: see [../README.md](../README.md).
> The protocol code that these notes call `main.rs` now lives in `src/backend/device.rs`, and the list of
> settings (labels, dependencies, which are camera-confirmed) is `src/backend/registry.rs`.
> Where this file and `Assistant/reference/` disagree, the newer dated evidence note wins.


A Rust/GTK4 Linux hardware-control test bench for the OBSBOT Tiny 2 Lite
(USB VID:PID `3564:fef9`). This is a feature/protocol development app with a compact test-bench interface,
not the final application design.

## Current capabilities

## Previous test-bench UI layout (superseded)

The Controls page keeps the complete supported control set visible and groups standard V4L2 settings in this order: Exposure, White balance, Focus, Zoom, and Image. Gimbal movement, autofocus priority, Auto Exposure priority, exposure compensation, OBSBOT camera features, microphone processing, video capture, presets, and camera power are kept in separate related sections.

A setting that is unavailable because of another setting remains visible but disabled, with a note naming the blocking mode. Confirmed dependencies are:

- Red balance, blue balance, and white-balance temperature require Auto White Balance Off.
- Shutter speed requires Exposure mode Manual (menu value 1).
- Manual focus requires Continuous Autofocus Off.
- Auto Exposure priority and exposure compensation require Exposure mode Auto (menu value 0).

These controls are rebuilt after their controlling mode changes. Device-reported ranges, steps, and menu choices are used; a menu default that the driver reports outside its advertised choices is replaced with the current valid choice. A numeric current value outside its declared range is retained for diagnosis and shown disabled with the reported range. The Tiny 2 Lite's invalid `zoom_continuous` value is therefore not sent; `zoom_absolute` remains the supported zoom path.

The mouse wheel is prevented from changing dropdown selections or slider values. Pointer dragging and keyboard operation remain available. Mirror image is grouped under Image orientation rather than microphone processing.

## Previous headless validation (superseded; see ../README.md)

The application has an explicit synthetic-device mode for visual tests. It is enabled only by setting `TINY4LITE_UI_TEST=1`; this bypasses USB discovery and all V4L2/UVC I/O, disables USB capture so no TShark process can be started, and supplies fixture camera metadata instead. Optional variables select the fixture state and page:

- `TINY4LITE_UI_TEST_STATE=auto|manual`
- `TINY4LITE_UI_TEST_PAGE=controls|capture|diagnostics`

Run the screenshot harness after building:

```sh
bash app/tests/headless-screenshots.sh \
  app/target/release/tiny4lite-control \
  app/artifacts/headless-ui
```

The GitHub Actions workflow `.github/workflows/headless-ui.yml` runs unit tests, builds the GTK app, and uploads Auto/Manual/Aperture Priority screenshots at multiple scroll positions, a screenshot at the normal 900 × 680 window size, Capture, and Diagnostics. It also compares screenshots before and after wheel input over a slider and dropdown, then drags the enabled Auto exposure compensation slider and checks that its value label changes. These screenshots validate rendering and layout only; they do not validate physical camera behaviour or vendor commands.


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
  the Global / Face priority selector sends framed XU
  `CAM_SET_FACE_FOCUS` (`0x3602`, receiver `0x02`) and reads the
  camera state with `CAM_GET_FACE_FOCUS` (`0x35C2`, receiver `0x02`).
  Both commands use XU unit 2 / selector `0x02`. Hardware read-back on
  2026-11-20 corrected the original capture labels: Face is
  `bf fb 01 00`; Global is `be 07 00 00`. The getter returns this
  four-byte value, not a Boolean. It sends a header-only GET request,
  polls GET_CUR up to eight times at 30 ms intervals, and accepts only
  a response matching the command and sequence with a valid 12-byte
  header CRC and four-byte payload. It ignores empty or nonmatching mailbox
  polls; the observed one-byte empty-mailbox response cannot be mistaken for
  the framed reply. The UI initialises from camera read-back, offers an explicit
  **Read camera state** action, verifies each change after SET_CUR, and
  displays Unknown on query failure or an unrecognised value. A successful
  setter transfer alone is never shown as confirmed state. This is
  autofocus priority, separate from Auto Exposure Face / Global.
- Auto Exposure priority has its own grouped Global / Face control,
  shown only when V4L2 `auto_exposure` is Auto (menu value 0). The
  2026-11-20 controlled Linux capture verified XU unit 2 / selector
  `0x06`, using full 60-byte SET_CUR and GET_CUR transfers. SET payload:
  Face = `03 01 00` + 57 zero bytes; Global = `03 01 01` + 57 zero
  bytes. GET_CUR status byte 7 is authoritative: `00` = Face,
  `01` = Global. The app initialises from camera read-back and performs
  a read-back after every change; the saved local value is only a
  fallback if the camera query fails. UI and persisted state reflect
  the actual read-back value, not the requested value. This is distinct
  from autofocus priority. Protocol read-back is capture-verified;
  integrated app behaviour still needs physical camera validation.
  Exposure compensation (−3.0 to +3.0) is the camera's standard V4L2
  control `backlight_compensation`, 0 to 18, read and written through
  `v4l2-ctl`. (The earlier raw Processing Unit ioctl failed with ENOENT on
  this camera and kernel.) Direct hardware tests cross-checked OBSBOT
  Center +2.0 with 15 and +2.3 with 16. The slider sends its zero-based
  position 0–18 directly. The UI reads the value on startup and reads it
  back after every write, and displays and persists the returned value,
  not merely the requested value. The
  control is available only while Auto Exposure is Auto. The current value
  is initialised with UVC GET_CUR when supported, and successful changes
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

- AI tracking modes: No Tracking, Normal Tracking, Upper Body,
  Close-up, Headless, Lower Body and Group: seven of the eight
  mode/submode tuples verified by selector 0x06 GET_CUR read-back. Hand
  tracking was taken out of the app at the user's request; a preset saved
  with it skips that setting with a note and applies the rest.
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
- HDR: one switch. The 2026-11-18 controlled Linux capture identified the selector `0x06` SET_CUR payload prefix: `01 01 01` enables HDR and `01 01 00` disables it. Status byte `0x06` reads `0x01` for On and `0x00` for Off. The switch initialises from camera status and falls back to the last successfully submitted local setting only when status is unavailable. The capture showed three matching state transitions; the final checkpoint had no corresponding command and is treated as an incomplete recording, not evidence against the mapping.
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
selected settings where documented. Target Selection is the only gesture
with confirmed camera-side state read-back: selector `0x06` byte `0x01`
is 0 when disabled and 1 when enabled. Zoom/L-pose, Dynamic Zoom, and
Dynamic Zoom Direction have no confirmed status read-back; they show the
last successfully saved local state or **Unknown**. The first click from
Unknown sends Enable. Unknown describes the app's inability to determine
the starting state, not a camera-reported error. A successful ioctl only
confirms host-side request completion; use observable device behaviour
or independent read-back to validate state changes.

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
- `auto_sleep_enabled = false` by default. The service does not start idle
  monitoring or send automatic sleep commands unless this is enabled (Device tab, Sleep timer).
- `auto_sleep_timeout_minutes = 10` is the default idle timeout and is
  validated to the range 1–1440 minutes.

The previous `settings.conf` theme preference is migrated on first load.
Missing keys in older config files receive safe defaults, so adding these
service fields does not invalidate existing user settings.

Loading the file does not send camera commands. In particular,
`apply_on_connect` defaults to `false`; saved camera values are retained
for the background service, which replays them only when `apply_on_connect` is on, but they are not replayed to the camera at
startup. Camera read-back remains authoritative for settings that the
device exposes. Autofocus priority is queried from the camera when the
Controls page is populated, after each setter, and on explicit refresh;
the persisted `focus_mode` is not trusted to initialise the UI. Gesture
states without reliable camera read-back remain locally remembered as the
last successfully submitted commands.

The config module provides version checking, defaults for missing keys,
safe value validation, and write-to-temporary-file followed by rename.
The settings module is implemented and loaded at application startup.
The background service (`src/service.rs`) reloads this file whenever it changes (within a couple of seconds), so a change in the window or from `service set` needs no restart. `apply_on_connect` is the restore switch.

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
- Sleep/wake physically move the gimbal. The window's buttons always send the command. The service sends sleep itself only when the sleep timer, shutdown or suspend options are on.
- No camera movement or sleep action runs automatically at startup.
- The background service (`tiny4lite-control service run`) restores the capture
  mode, optionally the settings, and optionally sleeps an idle camera, at shutdown
  and at suspend. It never wakes the camera. It is described in the README and
  in `service-and-distros.md`, and has not been run against a real camera.
- Global / Face autofocus-priority setter and camera-side GET read-back
  are hardware-verified on 2026-11-20 and implemented in the backend and
  Controls test UI. Face = `bf fb 01 00`; Global = `be 07 00 00`.
  The remaining validation is the visible/perceptual autofocus result
  under both modes; protocol state read-back is not proof of image-quality
  improvement.
- Reliable read-back for most gesture states, automatic firmware
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

## Centre

The Centre button in the Camera controls card, and `tiny4lite-control gimbal centre`, send pan 0 and tilt 0
in one V4L2 command (`pan_absolute=0,tilt_absolute=0`). The user confirmed on a real camera that this is
the physical centre. The camera turns, and neither the window nor the command line asks first.

## Autofocus priority: what the four bytes are

The notes call Face `bf fb 01 00` and Global `be 07 00 00` "the payload". They are not. The set frame has a
payload length of 4 at offset 12, a CRC16/USB of that payload at offsets 14 and 15, and a 32-bit number at
offsets 16 to 19. The bytes written down are offsets 14 to 17: the CRC (`bf fb` for the number 1, `be 07` for
0) and the first two bytes of the number. The sleep and wake frames show the same pairs in front of
`01 00 00 00` and `00 00 00 00`, and a unit test checks that the CRC of a frame carrying 1 is `bf fb`.

So the number sent is 1 for Face and 0 for Global. The first versions of this app sent `0x0001FBBF` and
`0x000007BE`, which the camera did not take as either state, so switching priority left the camera
where it was. The camera's reply is read by its number (1 or 0), with the old byte pattern as a second
opinion. Run with `TINY4LITE_DEBUG_XU=1` to see what is sent and what the camera answers.

## 2026-11-23 measured colour controls (white wall, warm room lighting)

Measured with `tools/probe-colour-controls.py` on a Tiny 2 Lite: Auto white balance off, Auto exposure on, each
value held 1.5 s, the mean R, G, B (0–255) of one frame. Values before and after: Auto white balance on,
5000 K, red 1024, blue 1024, hue 50.

| Setting | R | G | B | Notes |
|---|---:|---:|---:|---|
| 2000 K | 124.8 | 122.2 | 97.6 | most neutral under warm light |
| 3000 K | 144.2 | 105.7 | 46.4 | |
| 4000 K | 173.1 | 89.2 | 19.6 | |
| 5000 K | 184.4 | 91.2 | 0.1 | blue at zero from here up |
| 10000 K | 201.4 | 87.1 | 0.0 | |
| red 0 / 512 / 1024 / 1536 / 2048 (at 5000 K) | 0 / 83.5 / 130.1 / 252.6 / 255 | 134 / 131 / 124 / 55 / 85 | 0 / 0 / 0 / 29 / 55 | steep above 1024; exposure reacts |
| blue 0 / 1024 / 1536 / 2048 (at 5000 K) | 132 / 131 / 128 / 119 | 127 / 124 / 107 / 87 | 0 / 0 / 118 / 174 | nothing below 1024 while blue is already 0 |
| hue 0 / 25 / 50 / 75 / 100 | 130 / 131 / 131 / 126 / 114 | 86 / 108 / 124 / 132 / 132 | 0 | magenta (0) to green (100) |

Findings:

- Colour temperature: lower is bluer, higher is redder; set it to the light's colour for neutral white.
- Red and blue balance never changed when the temperature was changed, and the temperature never changed when
  they were: they are independent gains that stack on top of it, neutral at 1024.
- Hue is a magenta–green tint, not a blue–red one.
- Auto exposure stays on during this test, so boosting one channel can darken the others.

### Follow-up: the camera uses temperature or red/blue gains, whichever was written last

Reported on the camera (2026-11-23): with Auto white balance off, any change to Blue balance or Red balance,
even back to the middle, turns the picture yellow-green; any small change to Colour temperature brings it
back. So the camera does not combine them: writing red or blue switches it to raw channel gains (1024 = no
gain, and an uncorrected sensor looks green), and writing the temperature switches it back to gains worked out
from the temperature. The red/blue values it reports do not follow the temperature (see the table above). The
app therefore keeps manual white balance to one of the two (Manual white balance by: Temperature or Red &
blue) and writes that one's values again when switching, and presets record and re-apply the choice last.
