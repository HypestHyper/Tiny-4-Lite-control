# Tiny 2 Lite Control

A small GTK4 test bench for the OBSBOT Tiny 2 Lite.

- Detects video nodes by walking sysfs to the USB parent and matching **3564:fef9**. It does not match regular Tiny 2 (3564:fef8).
- Reads V4L2 control metadata from the camera at runtime.
- Exposes supported boolean, menu, and numeric controls. Numeric sliders apply changes immediately, mark the camera-reported default value, and include a Reset button. Binary 0/1 controls are instant toggles. Manual focus remains visible and becomes adjustable when continuous autofocus is switched off; the controls are refreshed after changing autofocus.
- Provides a drag joystick for signed pan/tilt speed controls, mapped against the widget's actual size, with an adjustable 10–200% sensitivity slider. Releasing the joystick sends zero speed to stop the gimbal.
- Provides one-degree absolute pan/tilt buttons when available. These buttons move immediately without a confirmation dialog.
- Includes OBSBOT vendor XU controls for all ten documented AI tracking modes, Standard/Sport tracking speed, and four gesture settings (target selection, Zoom/L-pose, dynamic zoom, and dynamic zoom direction). AI status is read from the 60-byte XU status block. XU commands are sent only when the user presses Apply, Enable, or Disable.
- Includes a Capture tab for continuous Linux usbmon recording, timestamped baseline/change checkpoints, and a decoded report of the selected camera's USB control transfers. It preserves both the raw `.pcapng` and shareable `.txt` report.
- Does not send sleep/wake commands or mutate presets.

The app uses the ranges and menu entries reported by the connected camera rather than hard-coded assumptions. AI features use the Tiny 2 Lite vendor XU (unit 2), not V4L2. The Sport command encoding is documented for firmware 6.2.5.x; older firmware may use a different value, so confirm the result in the AI status read-back. The Hand mode setter uses the Lite-tested `(0x03, 0x00)` tuple. Gesture controls expose explicit Enable/Disable actions because the Lite status block does not provide reliable per-gesture read-back.

## Runtime

The AppImage bundles GTK runtime dependencies and `v4l2-ctl`. The host still supplies the Linux V4L2 kernel driver and access to the camera device nodes. On Arch/CachyOS:

```sh
sudo pacman -S v4l-utils
sudo usermod -aG video "$USER"
```

Log out and back in after adding the user to the `video` group. Skip that step if your system already grants access through udev/logind.

```sh
chmod +x Tiny4LiteControl-x86_64.AppImage
./Tiny4LiteControl-x86_64.AppImage
```

## Local build

```sh
sudo pacman -S --needed base-devel rust gtk4 v4l-utils wireshark-cli polkit
cd app
cargo build --release
```

Absolute pan/tilt buttons move the physical gimbal by one degree immediately. The speed joystick sends live signed speed values while dragged and sends zero on release. Keep the movement area clear. No movement is initiated automatically at startup. AI status reads are read-only; AI mode, tracking speed, and gesture commands are sent only after an explicit user action. No homing, sleep/wake, or preset mutation is included.


## USB capture workflow

The Capture tab uses the selected camera video node to discover its USB bus and device address. It starts one continuous `tshark` recording on the matching `usbmonN` interface. On systems where usbmon access is restricted, it launches `tshark` through `pkexec` for a one-time authorization prompt; the capture stream is written back to a user-owned file.

Use **Start capture + baseline**, change one setting in OBSBOT Center, enter a short description, and press **Add checkpoint** after each change. Press **Finish session** to stop the trace and generate a report. Files are saved under:

```text
${XDG_DATA_HOME:-~/.local/share}/tiny4lite-control/captures/
```

Each session produces a raw `.pcapng` file and a `.txt` report. The report includes checkpoint timestamps and decoded control-transfer fields, including request metadata, response data, and captured payload bytes. Attach both files when sharing a trace. This records traffic; it does not automatically infer the meaning of unknown protocol fields.
