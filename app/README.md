# Tiny 2 Lite Control

A small GTK4 test bench for the OBSBOT Tiny 2 Lite.

- Detects video nodes by walking sysfs to the USB parent and matching **3564:fef9**. It does not match regular Tiny 2 (3564:fef8).
- Reads V4L2 control metadata from the camera at runtime.
- Exposes supported boolean, menu, and numeric controls with an explicit Apply button.
- Provides one-degree pan/tilt buttons when available. Every movement requires confirmation.
- Does not send vendor XU commands, sleep/wake, preset mutations, or undocumented state changes in this first build.

The app uses the ranges and menu entries reported by the connected camera rather than hard-coded assumptions.

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

Pan/tilt buttons move the physical gimbal by one degree and require confirmation. Keep the movement area clear. No movement, homing, sleep/wake, preset save, or vendor XU command is sent automatically.
