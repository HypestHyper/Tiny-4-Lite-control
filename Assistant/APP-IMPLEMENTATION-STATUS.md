# Application implementation status

**Branch:** `test-app-v0.1`  
**Application:** `app/`  
**Target:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Stage:** Hardware-control framework / test bench. UI design and visual polish are explicitly deferred.

This file records what is currently implemented in the repository and the protocol findings used by it. It is not a claim that every command has been independently validated on every firmware version.

## 1. Current implementation

The application is a Rust + GTK4 test bench. Current source of truth:

- `app/src/main.rs`
- `app/Cargo.toml`
- `app/README.md`

### Device discovery

- Enumerates Linux V4L2 video nodes through `/sys/class/video4linux`.
- Walks each node's sysfs device ancestry and matches USB vendor/product IDs `3564:fef9`.
- Excludes regular Tiny 2 (`3564:fef8`).
- Reads control metadata from the selected node at runtime using `v4l2-ctl`.

### Standard V4L2 control path

- Lists controls and menu metadata from `v4l2-ctl --list-ctrls-menus`.
- Reads and writes controls through `v4l2-ctl --get-ctrl` and `--set-ctrl`.
- Supports the camera-reported boolean, menu, and numeric controls.
- Handles continuous autofocus and manual focus availability.
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
- Send enable/disable commands for target-selection gesture, Zoom/L-pose gesture, dynamic zoom, and dynamic-zoom direction.

The current app does not implement sleep/wake or preset mutation.

### USB capture workflow

- A dedicated Capture tab records a continuous `usbmonN` trace for the selected Tiny 2 Lite USB bus.
- The app discovers the bus and device address from the selected `/dev/videoN` node's sysfs ancestry.
- Capture runs through `pkexec tshark` where elevated usbmon access is required; the pcapng stream is written to a user-owned file.
- The user marks a baseline and subsequent state changes with timestamped text checkpoints.
- Finishing a session creates a text report containing the checkpoint list and TShark-decoded control transfers filtered to the camera's USB address. It includes `usb.control.Response` and `usb.capdata` fields as available.
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

Four command groups are encoded:

- Target selection: `0a 04 c4 30 01 00`
- Zoom/L-pose: `0a 04 44 31 01 00`
- Dynamic zoom: `0a 04 44 33 01 00`
- Dynamic zoom direction: `0a 04 c4 33 01 00`

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
- Do not add guessed commands, firmware-specific fallbacks, sleep/wake, or preset mutation without documenting evidence and obtaining explicit approval.

## 4. What is not implemented

- No automatic byte-diff view or protocol-semantic inference in the application; capture sessions export raw traces and timestamped decoded control-transfer reports.
- No automatic firmware identification.
- No reliable per-gesture state read-back.
- No complete protocol coverage for all features listed in the research inventory.
- No UI redesign or visual-polish pass. The current GTK interface is only a functional test bench.
- No claim of a completed release or verified AppImage build is made by this status note.

## 5. Evidence status and next investigation

The protocol work so far is based on the organized research, Lite-specific repository material, and captured byte examples already recorded in that research. Controlled OBSBOT Center traces have now been collected from the Windows 11 VM and from Linux `usbmon3` while the camera was passed through to the VM. HDR On/Off testing confirmed a reversible status change at byte offset 5 of the selector `0x06` 60-byte status response (`00` Off, `01` On). The identical write prefix `01 01 01` was observed for both directions; its internal semantics remain unknown.

Next investigation is a controlled capture campaign:

1. Establish the camera's USB identity and interfaces in the Windows VM.
2. Capture idle/start-up behaviour from OBSBOT Center.
3. Change exactly one camera setting or invoke one action.
4. Preserve the original capture and record the action, before/after state, software/firmware versions, and timestamps.
5. Compare only the relevant camera control transfers against the baseline.
6. Repeat with multiple values/states before assigning field meanings.
7. Record raw bytes, request metadata, timing, and confidence. Keep unknown bytes unknown.
8. Validate read-back independently before treating a decoded field as state.
9. Add implementation only after the command is understood and its physical/device effect is clear.

Use the capture workflow and template linked from the [tracing documentation on main](https://github.com/HypestHyper/Tiny-4-Lite-control/tree/main/Assistant/tracing). Important: that generic guide is HID-focused. Tiny 2 Lite controls are V4L2/UVC XU; reuse its evidence discipline, not HID-specific assumptions.

## 6. Related project documentation

- [Research index](INDEX.md)
- [Agent rules](AGENTS.md)
- [Original feature research](source/obsbot_tiny_2_lite_linux_research_handoff.md)
- [Feature inventory](reference/02-feature-inventory.md)
- [Architecture and hardware test plan](reference/04-architecture-and-hardware-testing.md)
- [Application source](../app/src/main.rs)
- [Application build/runtime notes](../app/README.md)
