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
| Full | default, or `--full` | Live preview and gimbal joystick beside tabbed settings (Camera, Presets, Device). |
| Slim | `--slim` | A narrow panel with only the controls you pick (the tune button chooses them), plus an optional preview and joystick. |
| Preview | `--preview`, or the pop-out button | The preview in its own resizable window. |

The app remembers which of full or slim you used last, the size of the pop-out, and the split between
preview and settings. Dragging the divider in the full window resizes the preview; the slim preview
follows the window width.

## Icon

`assets/tiny4lite-control.svg` is the app icon: the camera from the front as a plain outline on a rounded tile, with a red ring round the lens. `assets/tiny4lite-control-symbolic.svg` is the one-colour version for panels and trays. Install both into the icon theme (`share/icons/hicolor/scalable/apps/` and `share/icons/hicolor/symbolic/apps/`), as the AppImage does; the window asks for the icon by the name `tiny4lite-control`.

## Background service

`tiny4lite-control service run` needs no window and no display. It watches for the camera being plugged in and, once the camera has settled, puts the saved capture mode back. That is all it does by default.

| Setting (`service set KEY VALUE`) | Default | What it does |
|---|---|---|
| `restore on` | off | Also puts the camera settings back: the preset named by `preset`, or the last values this app set |
| `preset NAME` | none | Which preset a restore applies |
| `gimbal on` | off | Lets a restore turn the gimbal to a saved position. Off, positions are left out |
| `sleep on` | off | Puts an idle camera to sleep (the gimbal moves when it sleeps) |
| `shutdown on` / `suspend on` | off | Also sleeps the camera when the computer shuts down or suspends |
| `sleep-after MINUTES` | 10 | How long it must be idle first |

Idle means no other program has the camera open (a call, a recording, the preview) and the control window is not running. A program that keeps the camera open all the time (some desktop camera monitors do) means it is never idle. Idle sleep, shutdown and suspend have not been tried on a real camera.

**Sleep timer, shutdown and suspend.** All three are on the Device tab under **Sleep timer**, and the service picks up a change within a couple of seconds. The sleep command is sent whether or not the camera already sleeps (sending it again does no harm), and the service never sends wake: the camera is expected to wake by itself when a program opens it, which still has to be confirmed (see `docs/troubleshooting.md`).

* *Sleep when nothing is using the camera* watches the camera's video node. When no program other than this app's own preview and commands has it open, and the control window is closed, for the time you set (type it in the box or use its arrows), the service sends sleep once, and again only after the camera has been used.
* *Sleep when the computer shuts down* works from the stop signal: a shutdown, a reboot, a logout and `systemctl stop` all end a user program with SIGTERM or SIGHUP in every init system, and the service answers by sending sleep first and then exiting.
* *Sleep when the computer suspends* holds a logind **delay inhibitor lock** and watches the `PrepareForSleep` signal, which is how a program runs something just before a suspend on systemd and on elogind (Void, Artix, Alpine, Gentoo). It needs `dbus-monitor` and `systemd-inhibit` or `elogind-inhibit`; with `service set suspend on` and a missing tool the service says what is missing instead of failing. A system with neither is not covered.

**Installing and running the service from the window.** The **Background service** card on the Device tab shows its status, refreshed every two seconds (process, uptime, camera, last event), and has **Install…**, then **Start**, **Stop** and **Remove**. Install opens a terminal (your `$TERMINAL`, or the first of the common ones found) that runs `tiny4lite-control service setup`: it lists what is needed, offers to install what is missing with your package manager (apt, dnf, pacman, zypper, apk, xbps, emerge) asking for your password in that terminal, then installs the service as a systemd user service, or as a desktop autostart entry where there is no systemd user manager, and starts it. The same steps are commands: `requirements [--install]`, `service setup|start|stop|restart|remove`.

How the service is installed and run on systemd and non-systemd systems, the window managers and init systems it has lines for, and what is not covered, is in `docs/service-and-distros.md`.

**Capture mode and other programs.** Changing resolution or frame rate breaks the picture of any other program using the camera, and the camera may refuse. The window now checks first, names the programs that have the camera open (a browser, OBS), and leaves the mode alone until they are closed.

Only one service runs at a time; `service status` shows it. Start it with your session using `service install`, which prints what to add, or writes it with `--write` for systemd and desktop autostart:

```bash
tiny4lite-control service install systemd --write
tiny4lite-control service install hyprland        # Hyprland 0.55+ (Lua): hl.on("hyprland.start", ...); hyprland-legacy prints exec-once
tiny4lite-control service targets                 # systemd, xdg, hyprland, hyprland-legacy, niri, sway, i3, river, mango, labwc, openbox, bspwm, shell, openrc, runit, s6, dinit, cron, udev
```

Window manager configurations are never edited for you. For the AppImage the unit points at the AppImage file itself. The `udev` target prints a rule that starts the systemd user service when the camera is plugged in.

**Angles.** Pan and tilt are shown in whole degrees everywhere. The live position wobbles, sometimes lands between two degrees, and now and then gives one odd value, so the shown degree is the middle of the last five readings and only changes once that is 0.8° or more away from it; a real move (two readings in a row well away) is followed at once. Bumping the desk by a degree still shows.

If the model swings the wrong way when the camera pans, turn on **Reverse pan on the camera model** in Settings (Device tab, Appearance). It is a one-time choice, and the gimbal info button says where it is.

**The stick and the picture.** Pushing the stick right moves the picture you see to the right. On this camera a positive pan speed shows more of the scene's left, so the stick sends the opposite; Mirror image flips the picture, so with it on the stick sends the plain direction. Turn the mirror part off with **Flip the joystick with the mirror** in Settings (Device tab, Appearance). `gimbal move` follows the same rule, so right means right. The 1° step buttons are angles and keep the camera's own direction, and the camera model follows the real camera.

**Tracking and the stick.** While Tracking mode is anything but No tracking the camera moves itself, so the stick, the 1° steps and Centre are greyed out, with a hint under the stick. Presets and Wake/Sleep stay usable.

**Joystick safety.** The camera is driven over OBSBOT's published controllable range, pan ±140° and tilt from 30° down to 70° up. Near an end the speed tapers off (from 25° away) and stops 2° short of it; turning back is always allowed. Speed rises over about a fifth of a second but stops at once. If the live position is unknown, the stick is held to 35% speed. If the window freezes while the stick is held, a watchdog stops the gimbal after 0.7 s. A failed send is repeated, and the 1° steps, Centre and presets never go past the range.

**Layout.** There are three tabs. **Camera** starts with tracking, then exposure, white balance, focus and picture. **Presets** holds the saved positions and the app's presets. **Device** has gestures, microphone, light, power, capture mode, the start-up position, appearance and developer tools. Next to the preview is the **Camera controls** card: the model, the joystick with four preset buttons beside it (they apply the first four presets), the 1° steps, Centre, zoom, field of view, speed and Wake/Sleep. Wide, the preview and card sit in a column on the left (drag the divider to size the preview) and the tabs on the right. When the right-hand panel would get narrower than about 470 px, whatever the window size, the page stacks and scrolls as one: preview and controls, then the tabs, then the content.

**Live camera model.** The Camera controls card starts with a wireframe camera on its gimbal that follows the camera's
real pan and tilt, with the numbers on top. It is drawn straight on from the front and a little above, like a product photo: the lens and the front of the base
face you. Click it to see it from behind instead (the choice is remembered). From the front, positive pan turns toward
your left, because it is the camera's right. A blue cone shows the view and follows the zoom: OBSBOT's published 67.2° horizontal at zoom 0 (79.4° diagonal at 4:3), narrowed by the documented zoom factors (1.15× at 5, 1.47× at 15) up to the published 4× at 100; the line between 15 and 100 is an assumption, and so is that the 16:9 picture keeps the full sensor width. The inner lens glass sits deeper as you zoom in. It is a drawing of the angles the camera reports (the same position
query as the readout), never a command. It glides between readings rather than jumping, asks for the position
about six times a second while the camera is moving and about once a second when it is still, and stops asking when the
window is in the background or the card is hidden. With no reading it dims and shows no view cone.

The drawing is traced from OBSBOT's product photos (front, back, both sides and an angled view) and calibrated to the
published overall size (48.37 × 46.46 × 64.2 mm); the view cone uses the published 79.4° diagonal field of view.
Tests check both. As in the real camera, the tilt motor (a cylinder) and the arm are on the camera's left, the
head sits off to the right of the pan axis, and the turntable is a flat round disc. It draws the rounded base with
its mic holes, USB-C slot and clip groove, the head with its front plate, ridged lens recess and status light, and the
perforated panel on the back. It leaves out the OBSBOT logo and the small fillets where the arm meets the disc and
the motor, and it assumes the head, arm and base are centred front to back (they are in the side photos). Every
dimension is a named constant in the `shape` module of `src/ui/gimbal_model.rs`, so correcting a detail is a
one-number change. Positive pan
turns right and positive tilt looks up, matching the joystick; whether those signs match your camera's own
convention has not been checked on hardware. The slim window shows it with the joystick.

The card also has **Pan −1° / +1°** and **Tilt −1° / +1°** buttons: small, bounded turns for precise
moves and careful first tests (a step is at most 5°, and stays inside the range the driver reports). They use
the standard absolute pan and tilt controls. Which way "+" turns has not been checked on a camera; the
position readout above the buttons shows the result.

Keyboard: `Alt+1`…`Alt+3` switch tabs, `Ctrl+F` or just typing opens search, `Esc` closes search or a
dialogue. With the joystick focused the arrow keys move the gimbal and releasing them stops it; `Space` or
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

## Command line

Every setting the windows show can also be read and changed from a terminal, with no display needed.
The commands are generated from the same catalogue as the windows, so a setting added there appears
here too, with completion and JSON output.

```sh
tiny4lite-control list                      # every setting, grouped, with its value
tiny4lite-control list white-balance        # one group
tiny4lite-control get exposure-mode         # just the value, for scripts
tiny4lite-control set exposure-mode manual
tiny4lite-control set shutter-speed 1/60    # units as shown: 1/60, 5600K, +1.5 (EV), 2.5 (zoom ratio)
tiny4lite-control toggle autofocus
tiny4lite-control set tracking-mode upper-body
tiny4lite-control reset zoom                # back to the camera's default
tiny4lite-control info shutter-speed        # what it does, its range, what it needs
tiny4lite-control status
tiny4lite-control gimbal move --pan 30 --seconds 2    # percent of full speed; always stops
tiny4lite-control gimbal step --pan 1 --tilt -1        # whole degrees, 1 to 5 each, either direction
tiny4lite-control deps                                   # what blocks what, as Markdown (also in docs/dependencies.md)
tiny4lite-control gimbal centre                         # turn to the physical centre, pan 0° tilt 0°
tiny4lite-control gimbal position
tiny4lite-control preset list | apply NAME | save NAME | delete NAME
tiny4lite-control position list | go 1 | save 1 Desk | delete 1    # the camera's three saved positions
tiny4lite-control startup show | set
tiny4lite-control capture list | get | set 1920x1080 MJPG 30
tiny4lite-control list --json               # machine-readable; also works on get, info, status
```

Names are the labels from the windows (`shutter-speed`, `autofocus`, `led-brightness`); the older
driver-style names (`exposure-time-absolute`) and full ids (`v4l2:auto_exposure`) still work. Choices
ignore case, spaces and hyphens. `--device /dev/videoN` (or `TINY4LITE_DEVICE`) picks a camera.

**Dependencies apply here too.** `set red-balance 1000` while Auto white balance is on exits with 3 and
says what to change; `--force` sends it anyway. Anything that physically moves the camera or changes
what it stores needs `--yes`: turning the light off, `position delete` and `startup set`. Moving the
gimbal, sleep and wake, `position go` and applying a preset never ask; without `--yes` the commands that do
explain and exit 4.

Exit codes: 0 done, 1 failed, 2 wrong usage, 3 unavailable because of another setting, 4 needs
`--yes`, 5 no camera found.

Shell completion (settings, groups, values and preset names):

```sh
tiny4lite-control completions bash > ~/.local/share/bash-completion/completions/tiny4lite-control
tiny4lite-control completions zsh  > ~/.zfunc/_tiny4lite-control
tiny4lite-control completions fish > ~/.config/fish/completions/tiny4lite-control.fish
```

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
src/cli.rs        The command line, built from the same registry.
tests/cli.rs      Runs the real binary against the synthetic camera.
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

### What the labels mean

Every value is shown with where it came from, in the windows and on the command line:

* **No label: read from the camera.** Refreshing reads it again. For some settings the app also reads the
  camera straight after every change and shows what the camera reports, which may differ from what you asked
  for. Exposure compensation, metering priority, autofocus priority and tracking mode work this way.
* **Saved:** the last value this app sent. The camera cannot report it, so it may differ if you changed it
  elsewhere. The command line writes `(saved)` after the value.
* **Unknown:** the camera did not report it and there is no saved value. The control shows a starting
  position, not the camera's real state.

A matching read-back confirms the camera holds the value. It does not prove how the picture or the focus
behaves. Autofocus priority never falls back to a saved value; the settings that can (for example
metering priority) only do so if the camera's status block cannot be read at all, and they are then labelled Saved.

Two controls that look alike are different: **Metering priority** (Auto Exposure Face/Global) and **Autofocus
priority** (Face/Global focus) use different commands. **Exposure compensation** is the camera's standard
Backlight Compensation control; there is no separate backlight slider because it is the same control.
**Tracking mode** has seven choices; Target selection is a gesture, not a tracking mode.

### Adding or changing a setting

1. **A standard V4L2 control:** add one `Setting::v4l2(...)` entry in `registry.rs`. Range, step, default
   and menu come from the driver at run time. Controls with no entry still appear under *More controls*.
2. **A vendor command:** write the payload and sender in `device.rs` as before (keep captured bytes in
   documented constants), then add a `Setting::custom(...)` entry with a reader and a writer.
3. Return where each value came from in the reader (`from_camera(...)`, `from_saved(...)`; nothing means
   Unknown) and, if the camera can confirm a write, return the value it reports from the writer. Mark
   `.last_sent()` if the camera cannot report it, add `.info(...)` for the info
   dialogue, `.needs(...)` for dependencies, `.experimental()` if it is unverified, `.not_quick()` to keep it
   out of the slim window's list.

The full window, slim window, search, customise list and info dialogues pick it up with no UI code.

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
* turning the light off asks for confirmation first;
* the 1° step buttons send one-degree absolute pan and tilt moves;
* Developer tools opens and a diagnostic read works.

`tests/cli.rs` (part of `cargo test`) runs the binary with no display and checks that every vendor command
is reachable from the terminal, that every setting can be read and set, that dependencies and
confirmations produce the right exit codes, and that `gimbal move` always ends with a stop.

Optional variables for the synthetic camera: `TINY4LITE_UI_TEST_STATE=auto|manual|aperture`,
`_PAGE=camera|tracking|audio|presets|device|customise`, `_THEME`, `_SCALE`, `_WIDTH`, `_HEIGHT`,
`_SEARCH`, and `_LOG=<file>` to record every V4L2 `--set` the UI issued.

## More documentation

* [docs/controls-and-protocol.md](docs/controls-and-protocol.md): control, protocol and configuration notes from the test bench, with the evidence behind them.
* [../Assistant/APP-IMPLEMENTATION-STATUS.md](../Assistant/APP-IMPLEMENTATION-STATUS.md): implementation status; section 0 describes this rebuild.
* [../Assistant/reference/](../Assistant/reference/): dated hardware evidence.
* [../CLAUDE_HANDOFF.md](../CLAUDE_HANDOFF.md): the state-handling contract the UI follows, and how it was met.

## What is not verified yet

* Everything above against a real Tiny 2 Lite. In particular the read-back paths (autofocus priority, metering
  priority, exposure compensation, tracking mode, Target selection) have only been exercised against a synthetic camera.
* Whether Face and Global autofocus produce a visibly different focus result; a matching read-back confirms the
  setting, not the behaviour.
* Zoom, Dynamic zoom and Reverse dynamic zoom: the commands work on the camera but there is no state to read, so they stay Saved or Unknown.
* Running the command line while the window's joystick or position readout is active: both use the same
  command mailbox, so avoid doing both at once.
* How the stop-on-release behaves if the USB device disconnects mid-drag.
* The model's left/right and up/down against the real camera (it follows the signs the camera reports).
* The Hyprland rules (the class names were read back from a headless sway, not from Hyprland itself).
* Light/dark switching from a live desktop portal (the portal code path is untested here).
