# Tiny 2 Lite Control

A GTK 4 control panel for the OBSBOT Tiny 2 Lite (USB `3564:fef9`) on Linux, written in Rust.
It talks to the camera through standard V4L2 controls and OBSBOT's vendor UVC extension unit.

> **Status:** the interface was built and checked against a synthetic camera (see *Testing*).
> It has **not** yet been exercised against real hardware in this form. The camera protocol code
> is unchanged from the test bench it grew out of; follow the hardware-safety notes in
> `Assistant/AGENTS.md` before trying commands that move the gimbal.

## Windows

| Window | Opens with | Purpose |
|---|---|---|
| Full | default, or `--full` | Live preview and gimbal joystick beside tabbed settings (Camera, Tracking, Audio, Presets, Device). |
| Slim | `--slim` | A narrow panel with only the controls you pick (the tune button chooses them), plus an optional preview and joystick. |
| Preview | `--preview`, or the pop-out button | The preview in its own resizable window. |

The app remembers which of full or slim you used last, the size of the pop-out, and the split between
preview and settings. Dragging the divider in the full window resizes the preview; the slim preview
follows the window width.

Keyboard: `Alt+1`…`Alt+5` switch tabs, `Ctrl+F` or just typing opens search, `Esc` closes search or a
dialog. With the joystick focused the arrow keys move the gimbal and releasing them stops it; `Space` or
`Esc` stops it at once.

### Window-manager rules

Each window has its own title and class, so rules can target one window without catching the others.
On Wayland the class is the `app_id`; on X11 it is `WM_CLASS`. Both were checked on a headless sway and Xvfb.

| Window | Class / app id | Title |
|---|---|---|
| Full | `io.github.hypesthyper.Tiny4LiteControl` | `Tiny 2 Lite Control` |
| Slim | `io.github.hypesthyper.Tiny4LiteControl.Slim` | `Tiny 2 Lite Control - Quick controls` |
| Preview | `io.github.hypesthyper.Tiny4LiteControl.Preview` | `Tiny 2 Lite Control - Preview` |

Hyprland (check your version's rule syntax; newer releases changed it):

```ini
windowrulev2 = float, class:^(io\.github\.hypesthyper\.Tiny4LiteControl\.Slim)$
windowrulev2 = size 380 640, class:^(io\.github\.hypesthyper\.Tiny4LiteControl\.Slim)$
windowrulev2 = float, class:^(io\.github\.hypesthyper\.Tiny4LiteControl\.Preview)$
windowrulev2 = pin, class:^(io\.github\.hypesthyper\.Tiny4LiteControl\.Preview)$
windowrulev2 = size 640 360, title:^(Tiny 2 Lite Control - Preview)$
```

sway: `for_window [app_id="io.github.hypesthyper.Tiny4LiteControl.Preview"] floating enable, sticky enable`

GTK cannot ask for "always on top" itself, so use your compositor's pin or sticky rule for the preview.
The header window buttons can be hidden under Device → Appearance if your compositor draws its own.

## How it is organised

```
src/backend/      Camera access, no GTK.
  device.rs         V4L2 through v4l2-ctl; the UVC extension unit; OBSBOT framed commands.
  registry.rs       The catalogue of settings (see below).
  worker.rs         One thread that owns every camera transaction.
  state.rs          Shared settings, written to disk after a short delay.
src/ui/           GTK 4 only; knows nothing about individual settings.
  theme.rs          Palette from one accent seed, and the stylesheet.
  style.rs          Fonts, theme, scale, reduced motion, desktop dark mode.
  store.rs          Latest camera snapshot, worker bridge, toasts, confirmations.
  widgets.rs        Controls, and the generic row built from a registry entry.
  settings.rs       Tab pages and cards generated from the registry.
  joystick.rs       The gimbal pad.
  preview.rs        One capture stream shared by every place the preview is shown.
  full.rs slim.rs popup.rs app.rs   The windows and start-up.
```

**Camera work never runs on the UI thread.** Jobs go to the worker in order. A keyed job replaces one
still waiting with the same key, so a slider or the joystick can fire faster than the camera answers
and only the newest value is sent. An urgent job (the gimbal stop) runs first.

**Dependencies are data.** A setting lists what it needs, with the sentence shown to the user:

```rust
Setting::v4l2("red_balance", "v4l2:red_balance", WhiteBalance, "Red balance",
    "Fine-tune the red channel.", slider(0.0, 2048.0, 1.0, Fmt::Plain))
    .needs("v4l2:white_balance_automatic", OFF, "Turn off Auto white balance to adjust red."),
```

The row stays visible, is dimmed, and shows that sentence. Unknown state never blocks a setting: only
what the camera reports does. Confirmed dependencies today: manual white balance needs Auto white
balance off; shutter speed needs Manual exposure; exposure compensation and metering priority need Auto
exposure; manual focus needs Autofocus off; autofocus priority needs Autofocus on.

### Adding or changing a setting

1. **A standard V4L2 control:** add one `Setting::v4l2(...)` entry in `registry.rs`. Range, step, default
   and menu come from the driver at run time. Controls with no entry still appear under *More controls*.
2. **A vendor command:** write the payload and sender in `device.rs` as before (keep captured bytes in
   documented constants), then add a `Setting::custom(...)` entry with a reader and a writer.
3. Say whether the camera reports it back (`.last_sent()` if it does not), add `.info(...)` for the info
   dialog, `.needs(...)` for dependencies, `.experimental()` if it is unverified, `.not_quick()` to keep it
   out of the slim window's list.

The full window, slim window, search, customise list and info dialogs pick it up with no UI code.

## Appearance

Material 3 Expressive on plain GTK 4, following the GTK app family design language: one accent seed
produces every colour role (contrast is measured and corrected in `theme.rs`, and a test covers all six
family accents plus extreme custom seeds), Google Sans Flex text, Material Symbols Rounded icons, shapes
that tighten when pressed, and no libadwaita. System, light and dark follow the desktop through the
settings portal. Size can be Auto (compact below 900 px), Compact, Default or Large. Reduce motion turns
off every transition.

**Exception to the family baseline:** the design language specifies GTK 4 through PyGObject. This app is
Rust (gtk4-rs) so it can keep the existing, hardware-verified protocol code. Tokens, type, shape, motion,
spacing, accessibility rules and writing style follow the design language; the code is Rust.

The icon font is a subset of Material Symbols Rounded. After adding an icon name to
`assets/icon-names.txt`, run `python3 tools/subset-fonts.py` (needs `pip install fonttools`; it downloads
the full font). A unit test fails if the UI asks for an icon that is not in the font.

## Building

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libfontconfig1-dev v4l-utils   # Debian/Ubuntu
cd app && cargo build --release
```

GTK 4.8 or newer is required. `v4l2-ctl` (from v4l-utils) must be on `PATH`. The user needs access to the
camera's `/dev/video*` node.

## Testing

```sh
cd app && cargo test
bash tests/headless-screenshots.sh target/release/tiny4lite-control artifacts/headless-ui
```

`TINY4LITE_UI_TEST=1` switches to a synthetic camera: no USB or V4L2 access happens, and nothing can be
sent to a real device. The screenshot script (needs Xvfb, xdotool, imagemagick, x11-utils) renders each
window, tab and state, then uses real pointer events to check that:

* each window has the expected class and title;
* dragging the joystick sends speeds in the right direction, never more than 30 updates a second, and
  always ends with a stop;
* putting the camera to sleep asks for confirmation first.

Optional variables for the synthetic camera: `TINY4LITE_UI_TEST_STATE=auto|manual|aperture`,
`_PAGE=camera|tracking|audio|presets|device|customise`, `_THEME`, `_SCALE`, `_WIDTH`, `_HEIGHT`,
`_SEARCH`, and `_LOG=<file>` to record every V4L2 `--set` the UI issued.

## What is not verified yet

* Everything above against a real Tiny 2 Lite.
* How the stop-on-release behaves if the USB device disconnects mid-drag.
* The Hyprland rules (the class names were read back from a headless sway, not from Hyprland itself).
* Light/dark switching from a live desktop portal (the portal code path is untested here).
