# Tiny 2 Lite Control

A Rust/GTK4 Linux hardware-control test bench for the OBSBOT Tiny 2 Lite
(USB VID:PID `3564:fef9`). This is a feature/protocol development app,
not the final UI. UI redesign is deferred.

## Current capabilities

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
  command (`0x3602`, receiver `0x02`) using captured Tiny 2 Lite
  payloads: Global `be 07 00 00`, Face `bf fb 01 00`. Their assignment
  is inferred from paired transitions in the recording and still needs
  physical confirmation. Selection is tracked locally because camera-side
  read-back is unavailable. This is distinct from face-priority auto-exposure.
- Exposes the camera-reported `zoom_absolute` as a position slider.
  `zoom_continuous` is currently suppressed. A hold-to-zoom control
  with release-to-stop is not implemented; do not assume the current
  slider resolves the earlier zoom-continuation issue.
- Provides pan/tilt speed joystick control with sensitivity adjustment.
  Releasing the joystick sends neutral speed. Bounded absolute pan/tilt
  step controls are also available and move the physical gimbal
  immediately.

### OBSBOT vendor UVC Extension Unit controls

The XU transport uses unit 2, selector `0x02` for framed commands and
selector `0x06` for the 60-byte status block.

- AI tracking modes: Off, Normal, Upper Body, Close-up, Headless, Lower
  Body, Group, Desk, Whiteboard, and Hand. The Hand setter uses the
  Lite-tested tuple `03 00`; the status decoder accepts both `03 00`
  and `06 00` because the observed representations differ.
- Tracking speed: Standard and Sport. Sport encoding is
  firmware-sensitive; the current implementation sends the documented
  `0x02` value and does not automatically choose the older reported
  `0x21` encoding.
- Gestures: Target selection, Zoom/L-pose, Dynamic Zoom, and Dynamic
  Zoom Direction. All four gesture behaviours have been physically confirmed. Dynamic
  Zoom Direction reverses the direction of the Dynamic Zoom gesture, so
  these two settings work together: enable Dynamic Zoom for the two-hand
  L-shaped gesture, then use Direction Flip to reverse its direction.
  Direction Flip does not affect the separate Zoom/L-pose gesture or
  autofocus Global/Face priority.
- Framed XU writes use a rolling process-local 16-bit sequence counter.
  The 2026-11-18 capture shows consecutive sequence values across
  multiple command groups. The required initial seed after reconnect or
  process restart is not established; the current implementation
  initializes it to zero.
- Sleep and wake are explicit user actions. Both have been physically
  tested. The separate power-status read-back has also been tested.
- FOV: Wide, Normal, and Narrow have all been visually confirmed. Some
  AI tracking modes are incompatible with FOV changes; the exact
  combinations are not yet mapped.
- LED: one setting control with Off and brightness levels 1, 2, and 3.
  Selector `0x06`, tag `0x1A`, uses raw values 0 through 3 directly.
  Status byte `0x21` reports the selected setting. Tag `0x18` is not
  treated as LED power: observed On makes the indicator blink, while
  Off leaves it continuously lit.
- Preset mutation is not implemented.

The selector `0x06` status block provides AI tracking state/mode and
selected settings where documented, but it does not provide reliable
per-gesture read-back. A successful ioctl only confirms host-side
request completion; use observable device behaviour or independent
read-back to validate state changes.

## Diagnostics and USB capture

The Diagnostics page provides read-only XU reads for selectors `0x02`
and `0x06`, plus the model/stream name from selector `0x08` and USB
descriptor identity. USB `bcdDevice` is labelled as the device-release
field, not as installed firmware. Candidate pan/tilt float values from
selector `0x02` remain unverified as live position telemetry.

The Capture page:

- Finds the camera's USB bus/device address directly through sysfs.
- Records continuous `usbmonN` traffic using `tshark`; elevated capture
  access is requested through `pkexec` when needed.
- Supports timestamped baseline/change checkpoints.
- Produces a raw `.pcapng` and a text report with checkpoint times,
  transfer summaries, decoded control-transfer fields, URB IDs/status,
  and payload bytes when TShark exposes them.
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
  sleep/wake service. The longer-term service/config architecture is
  still a design direction, not an implemented feature.
- Zoom hold/release stop behaviour, Global / Face focus mode hardware confirmation/read-back, reliable
  gesture read-back, automatic firmware detection, live pan/tilt
  telemetry, and preset mutation remain unimplemented or unverified.
- Do not add undocumented writes or firmware-specific fallbacks without
  capture evidence and explicit hardware validation.

## Project documentation

- [Implementation status and protocol evidence](../Assistant/APP-IMPLEMENTATION-STATUS.md)
- [Feature inventory](../Assistant/reference/02-feature-inventory.md)
- [Project index](../Assistant/INDEX.md)
- [Agent and hardware-safety rules](../Assistant/AGENTS.md)
