# Application implementation status

**Branch:** `test-app-v0.1`  
**Application:** `app/`  
**Target:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Stage:** Hardware-control test bench with an initial GTK family-design implementation. UI conformance is partial and remains subject to live desktop validation.

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
- The GTK controls now put the Autofocus switch first. Global / Face options are shown only while autofocus is enabled, but are deliberately disabled: the previous XU `0x3602` payload interpretation was not sufficiently correlated with the UI checkpoints and the user reports that the options do not appear to work. Do not re-enable writes until a controlled capture establishes the mapping.
- The same trace captured standard UVC Zoom Absolute selector `0x0B` writes. The XU selector `0x02` float record strongly correlates with the same manual-zoom changes. Domatix's Lite-specific protocol reference identifies manual zoom with the standard `zoom_absolute` control and distinguishes gesture zoom, which bypasses it. The XU record remains telemetry/secondary representation until its exact role is confirmed; the image-processing path is not established.
- Current GTK implementation exposes the device-reported `zoom_absolute` as a generic numeric slider and explicitly suppresses `zoom_continuous`. There is no zoom press/release handler or stop-on-release command in the app; the existing release-to-neutral code applies only to the pan/tilt joystick. Therefore the reported zoom continuing after release cannot be fixed safely by changing a callback that does not exist. First inspect the selected node's exact `zoom_absolute` and `zoom_continuous` descriptors (type, min/max, step, flags) and correlate them with a controlled press/hold/release trace. If the driver exposes a valid continuous control, implement a press/release interaction that sends the documented neutral value on release; otherwise keep absolute zoom as a position slider.
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
- Send enable/disable commands for target-selection gesture, Zoom/L-pose gesture, dynamic zoom, and dynamic-zoom direction. Dynamic Zoom Direction has an inverted wire-state mapping: UI enabled sends `e6 3f 00 00`; UI disabled sends `27 ff 01 00`. The 2026-11-18 capture shows the frame sequence advancing across commands (`0x000f`, `0x0010`, `0x0011`, `0x0012`), including the final, separate Dynamic Zoom disable command. All framed XU writes now use one rolling 16-bit sequence counter rather than fixed per-feature values. The counter starts at zero for each app process; the capture establishes incrementing behavior, not a required initial value. The fourth inspected packet uses group `0a 04 44 33` (Dynamic Zoom), not the direction group `0a 04 c4 33`, and is not used as direction-state evidence.

Sleep/wake is implemented through the captured CAM_SET_DEV_STATUS frame and has been physically tested by the user. Preset mutation is not implemented.

### Read-only diagnostics and device identity

- The Diagnostics tab reads the 60-byte selector `0x06` status block and selector `0x02` command-channel response using GET_CUR only.
- It shows the selector `0x08` model/stream name and USB descriptor identity, including `bcdDevice` clearly labelled as a USB device-release value, not confirmed firmware.
- Status fields are decoded only where evidence exists. FOV (`0x11`) and crop/zoom-related raw code (`0x04`) are displayed separately. Candidate position floats from selector `0x02` remain labelled unverified.
- Camera settings expose Wide / Normal / Narrow FOV commands and LED settings. The user has visually confirmed all three FOV modes. FOV changes are incompatible with certain AI tracking modes; the exact affected modes are not yet identified. The Narrow status-byte read-back mapping remains unverified. LED has four settings total: Off plus brightness levels 1, 2, and 3. The selector 0x06/tag 0x1A command uses raw values 0, 1, 2, and 3 for those four settings respectively; status byte 0x21 reports the setting. The previous implementation incorrectly subtracted one from the requested level, making level 1 send raw 0 (Off), and also used tag 0x18 as a separate power switch. User testing showed tag 0x18 On causes blinking and Off leaves the LED continuously lit, so it is not established as a power switch and is intentionally not used by the current UI.
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

Four command groups are encoded. The user has now physically confirmed the following behaviour on the Tiny 2 Lite: Target selection works and enables normal tracking when the user spreads an open palm/fingers; the Zoom/L-pose gesture works; and Dynamic Zoom works when both hands form L shapes and move apart/together. Dynamic Zoom direction gesture is functional. The 2026-11-18 capture established the inverted ON/OFF payload mapping and the rolling frame sequence behavior; both are implemented. Device read-back remains unavailable, so the UI tracks the last successfully sent state only. These are live hardware observations, not protocol read-back.

- Target selection: `0a 04 c4 30 01 00`
- Zoom/L-pose: `0a 04 44 31 01 00`
- Dynamic zoom: `0a 04 44 33 01 00`
- Dynamic zoom direction: `0a 04 c4 33 01 00`; unlike the other gesture commands, its wire-state payload is inverted: OFF = `27 ff 01 00`, ON = `e6 3f 00 00`. Unit tests compare both complete frame prefixes, including sequence and CRC, against the 2026-11-18 capture.

Enable/disable frames are built with the current frame builder. The status block does not provide reliable per-gesture read-back, so a successful ioctl only confirms that the request was accepted by the host driver, not that the camera applied the feature. Capture-based validation is still required.

### Frame construction and tests

- The current frame builder constructs a 60-byte XU frame, including sequence, header, payload length, and CRC-16.
- Unit tests in `app/src/main.rs` compare tracking-speed and gesture frame prefixes against the Lite captures recorded in the research.
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
- No reliable per-gesture state read-back. Dynamic Zoom direction ON/OFF wire payloads are mapped from the 2026-11-18 capture, but the user-visible direction reversal still needs hardware confirmation.
- No complete protocol coverage for all features listed in the research inventory.
- No UI redesign or visual-polish pass. The current GTK interface is only a functional test bench.
- GitHub Actions AppImage build passed on 2026-11-17 for commit `150ca222d3d8c10a5d48880788ce1043639b11ba` (run [44184954913](https://github.com/HypestHyper/Tiny-4-Lite-control/actions/runs/44184954913)). This confirms compilation and packaging, not live hardware validation of the new diagnostics reads.

## 5. Evidence status and next investigation

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
