# Tiny 2 Lite Control

A small GTK4 test bench for the OBSBOT Tiny 2 Lite.

- Detects video nodes by walking sysfs to the USB parent and matching **3564:fef9**. It does not match regular Tiny 2 (3564:fef8).
- Reads V4L2 control metadata from the camera at runtime.
- Exposes supported boolean, menu, and numeric controls. Numeric sliders apply changes immediately, mark the camera-reported default value, and include a Reset button. Binary 0/1 controls are instant toggles. Manual focus remains visible and becomes adjustable when continuous autofocus is switched off; the controls are refreshed after changing autofocus.
- Provides a drag joystick for signed pan/tilt speed controls, mapped against the widget's actual size, with an adjustable 10–200% sensitivity slider. Releasing the joystick sends zero speed to stop the gimbal.
- Provides one-degree absolute pan/tilt buttons when available. These buttons move immediately without a confirmation dialog.
- Includes OBSBOT vendor XU controls for all ten documented AI tracking modes, Standard/Sport tracking speed, and four gesture settings (target selection, Zoom/L-pose, dynamic zoom, and dynamic zoom direction). AI status is read from the 60-byte XU status block. XU commands are sent only when the user presses Apply, Enable, or Disable.
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
sudo pacman -S --needed base-devel rust gtk4 v4l-utils
cd app
cargo build --release
```

Absolute pan/tilt buttons move the physical gimbal by one degree immediately. The speed joystick sends live signed speed values while dragged and sends zero on release. Keep the movement area clear. No movement is initiated automatically at startup. AI status reads are read-only; AI mode, tracking speed, and gesture commands are sent only after an explicit user action. No homing, sleep/wake, or preset mutation is included.
