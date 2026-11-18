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

### Standard V4L2 control path

- Lists controls and menu metadata from `v4l2-ctl --list-ctrls-menus`.
- Reads and writes controls through `v4l2-ctl --get-ctrl` and `--set-ctrl`.
- Supports the camera-reported boolean, menu, and numeric controls.
- Handles continuous autofocus and manual focus availability. The 2026-11-17 Windows trace confirmed standard UVC Camera Terminal Focus Auto selector `0x08` (one-byte Boolean) and Focus Absolute selector `0x06` (two-byte little-endian value). It captured 14 Focus Absolute writes (observed values 0–100) after autofocus was disabled. These are observed values, not a validated min/max.
- When autofocus is enabled, Global / Face options send the Tiny 2 Lite framed Camera command `CAM_SET_FACE_FOCUS` (`0x3602`, receiver `0x02`) through XU unit 2 / selector `0x02`. The previous Boolean payload (`00000000` / `01000000`) was incorrect for the Lite and has been replaced with the values seen in the user's capture: Global `be 07 00 00`, Face `bf fb 01 00`. The Face/Global assignment is inferred from the paired transitions around the 15.85s and 25.17s checkpoints; the remaining checkpoints do not correlate cleanly, so hardware confirmation is still required. The UI tracks selection locally because camera-side read-back is not implemented. When autofocus is disabled, Global / Face options are hidden and the manual Focus Absolute slider is shown. Keep this distinct from face-priority auto-exposure, which is selector `0x06` tag `0x03`.
- Zoom and FOV now share one standard V4L2 path. The Tiny 2 Lite reports
  `zoom_absolute` from 0 to 100; the user confirmed smooth adjustment
  and preset values Wide=0 (1.00×), Medium=5 (1.15×), Narrow=15
  (1.47×), with 100 as maximum. The GTK app has three preset buttons
  that write `zoom_absolute` and read it back, plus the generic
  device-reported slider. `zoom_continuous` and the separate vendor
  FOV setter are intentionally unused.
- Uses the device-reported ranges, steps, menu entries, and defaults rather than assuming one fixed set of controls.
- Provides pan/tilt speed control and bounded absolute pan/tilt steps. The joystick sends neutral speed on release.

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
- Set the AI tracking-mode tuple through selector `0x06`, opcode `0x16`.
- Send tracking-speed commands.
- Send enable/disable commands for target-selection gesture, Zoom/L-pose gesture, dynamic zoom, and dynamic-zoom direction. Dynamic Zoom Direction has an inverted wire-state mapping: UI enabled sends `e6 3f 00 00`; UI disabled sends `27 ff 01 00`. The 2026-11-18 capture checkpoints show Direction Flip OFF at baseline, ON at 17.37s, OFF at 35.15s, then Dynamic Zoom OFF at 54.78s. The user has physically confirmed that Target Selection, Zoom/L-pose, Dynamic Zoom and Direction Flip all work on the Tiny 2 Lite. Direction Flip is the direction modifier for the Dynamic Zoom gesture and is intended to be used together with Dynamic Zoom; it does not change the separate Zoom/L-pose gesture or autofocus Global/Face priority. The corresponding SET_CUR packets are frames 13247, 72265, 170871, and 179033. The first three use direction group `0a 04 c4 33` and alternate payloads `27 ff 01 00`, `e6 3f 00 00`, `27 ff 01 00`. The fourth uses Dynamic Zoom group `0a 04 44 33` and is the separate Dynamic Zoom disable command, not a Direction Flip transition. The sequence values progress `0x000f`, `0x0010`, `0x0011`, `0x0012` across these commands. All framed XU writes now use one rolling 16-bit sequence counter rather than fixed per-feature values. The observed initial value `0x000f` is capture context; the required sequence seed on a fresh camera/app session is not established. The implementation currently initializes its process-local counter to zero and increments it for each framed XU write.

Sleep/wake is implemented through the captured CAM_SET_DEV_STATUS frame and has been physically tested by the user. Preset mutation is not implemented.

### Read-only diagnostics and device identity

- The Diagnostics tab reads the 60-byte selector `0x06` status block and selector `0x02` command-channel response using GET_CUR only.
- It shows the selector `0x08` model/stream name and USB descriptor identity, including `bcdDevice` clearly labelled as a USB device-release value, not confirmed firmware.
- Status fields are decoded only where evidence exists. The legacy FOV byte (`0x11`) is retained only as raw diagnostic data; it is not used to control zoom. Candidate position floats from selector `0x02` remain labelled unverified.
- Camera settings expose V4L2 zoom presets (Wide / Medium / Narrow) and LED settings. The old vendor FOV command and status-byte interpretation are superseded by the V4L2 zoom mapping. The Narrow status-byte read-back mapping remains unverified. LED has four settings total: Off plus brightness levels 1, 2, and 3. The selector 0x06/tag 0x1A command uses raw values 0, 1, 2, and 3 for those four settings respectively; status byte 0x21 reports the setting. The previous implementation incorrectly subtracted one from the requested level, making level 1 send raw 0 (Off), and also used tag 0x18 as a separate power switch. User testing showed tag 0x18 On causes blinking and Off leaves the LED continuously lit, so it is not established as a power switch and is intentionally not used by the current UI.
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

- Standard and Sport commands are implemented.
- Sport encoding `0x02` is associated with firmware 6.2.5.x in the research.
- An older/legacy value `0x21` has been observed/reported for older firmware.
- Current app sends the documented Sport command but does not automatically identify firmware or choose the legacy encoding.
- Always verify the resulting status byte after a command. Firmware compatibility remains an open validation item.

### Gesture commands

Four command groups are encoded. The user has physically confirmed Target selection (open palm/fingers spread enables normal tracking), Zoom/L-pose, and Dynamic Zoom (both hands form L shapes and move apart/together). The 2026-11-18 capture establishes the Direction Flip wire payload mapping, and the user has now physically confirmed that the corrected direction reversal works on the camera. Direction Flip modifies the direction of the Dynamic Zoom gesture, so the two settings work together. It is not related to Zoom/L-pose or Global/Face autofocus priority. Device read-back remains unavailable, so the UI tracks the last successfully sent state only. These are live hardware observations, not protocol read-back.

- Target selection: `0a 04 c4 30 01 00`
- Zoom/L-pose: `0a 04 44 31 01 00`
- Dynamic zoom: `0a 04 44 33 01 00`
- Dynamic zoom direction: `0a 04 c4 33 01 00`; unlike the other gesture commands, its wire-state payload is inverted: OFF = `27 ff 01 00`, ON = `e6 3f 00 00`. Unit tests check both payload choices and rolling sequence increments. The capture-derived full frame prefixes are recorded in the evidence section below.

Enable/disable frames are built with the current frame builder. The status block does not provide reliable per-gesture read-back, so a successful ioctl only confirms that the request was accepted by the host driver, not that the camera applied the feature. Capture-based validation is still required.

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
- No reliable per-gesture state read-back. The user has physically confirmed all four exposed gesture behaviours, including Direction Flip reversing Dynamic Zoom.
- No complete protocol coverage for all features listed in the research inventory.
- No UI redesign or visual-polish pass. The current GTK interface is only a functional test bench.
- GitHub Actions AppImage build passed on 2026-11-17 for commit `150ca222d3d8c10a5d48880788ce1043639b11ba` (run [44184954913](https://github.com/HypestHyper/Tiny-4-Lite-control/actions/runs/44184954913)). This confirms compilation and packaging for that earlier revision only. The later sequence-counter code commit `dbe37f0` has no confirmed CI result in the available status check; do not treat the latest source as build-verified until its workflow succeeds.

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
- Existing tracking-speed, target-selection gesture, Zoom/L-pose gesture, sleep/wake, and preset opcode families also appear in the trace.\n- Correctly paired selector 0x02 GET_CUR responses contain candidate pan/tilt float32 values at payload offset 20. The evidence is strong enough to justify a controlled read-only polling test, but not yet enough to claim live position telemetry. OBSBOT Center did not poll selector 0x02 during the dedicated AI-tracking capture.
- Several XU records remain unknown. The capture's Finish operation hung, so timestamped UI checkpoint descriptions were not recovered. Do not infer individual UI actions from timestamps alone.

Next work: test host-initiated selector 0x02 GET_CUR polling during idle, manual movement, and AI tracking. Record all 60-byte responses and correlate the offset-20 float32 pair with visible gimbal movement. See [reference/07-2026-11-17-ptz-position-readback.md](reference/07-2026-11-17-ptz-position-readback.md) for the transport tuple, samples, Windows KS topology investigation, and test plan. Until that experiment succeeds, do not expose the candidate values as verified live position. Continue offline decoding of unresolved XU records against Lite-specific references, preserving unknown fields. Any new setter must have a known payload and independent read-back/observable validation before implementation.

Use the capture workflow and template linked from the [tracing documentation on main](https://github.com/HypestHyper/Tiny-4-Lite-control/tree/main/Assistant/tracing). Important: that generic guide is HID-focused. Tiny 2 Lite controls are V4L2/UVC XU; reuse its evidence discipline, not HID-specific assumptions.

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
