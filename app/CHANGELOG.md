# Changelog

## Unreleased

### Fixed

- The camera model's view cone stays visible at −30° tilt; it used to disappear there.
- The camera is put to sleep at shutdown again. The background service used to crash with a bus error
  when systemd stopped it, if its AppImage had been replaced while it was running. It now sleeps the camera
  as soon as the shutdown is announced (logind `PrepareForShutdown`). Stopping the service is kept as a fallback.
- Sliders no longer stay held after the pointer leaves the window mid-drag (Hyprland with focus on hover).
- The Studio camera at 4K30: the camera's mode is read again when it changes, and the picture is scaled
  to the size an app already holds. Before, 4K frames came out as tiles in a 1080p Studio camera.
- The frame rate shown for the Studio camera is averaged over about ten seconds, so it no longer flickers
  between 29 and 30.

### Changed

- The zoom gesture, dynamic zoom, reverse dynamic zoom and the gesture zoom amount are read from the camera
  (no more Unknown); presets save them from the camera too.

- **Tracking area** is its own card with its own pan and tilt ends (OBSBOT's default: 100° each way, 30° up and
  down), separate from the gimbal limits; presets save the ends too.
- **Tracking speed: Standard / Sport** again (OBSBOT's SDK calls them Standard and Motion; its third mode,
  Headroom, behaves like Standard and is not offered).
- Manual white balance is the colour temperature only: red and blue balance are gone (they turned the picture
  green), and presets no longer save or write them.
- The Zoom read-out under the camera model shows the camera's live zoom (gestures and tracking included); the
  zoom slider stays what the app sent.
- The desktop's GTK theme can no longer move or resize buttons and other controls on hover.
- The camera's own sleep timer is on the Sleep timer card, above this app's timer.
- Return position: each change is read back and checked; **Go there** turns the camera to it.

- Less text on the cards: card descriptions and long explanations are behind info buttons; rows keep one short line.
- Tooltips follow the colour theme (the notices at the bottom stay light, to stand out).
- Choosing an LED brightness also turns on the light's own switch.

- Commands checked against OBSBOT's own SDK on a Tiny 2 Lite; where they differed, the SDK's are used now
  (old versions kept in `docs/sdk-alignment-backup.md`):
  - audio auto gain sends the SDK's own command;
  - "Tracking speed" is now **Vertical tracking**: Standard, Headroom (new) and Motion (was Sport). Presets
    saved before keep working.
  - Metering priority Face / Global and the Reverse dynamic zoom gesture follow the SDK's values, which are
    the other way round from our earlier captures.
- Manual white balance is done either by temperature or by red & blue gains, chosen on the White balance
  card. The camera uses whichever was changed last, and touching red or blue alone turned the picture green.
- Hue is described and drawn as the magenta–green tint it is (measured), not blue–red.
- Shutter speed moves through the usual stops and ends at the longest time the frame rate allows (1/30 s
  at 30 fps, 1/60 s at 60 fps).
- Focus peaking uses the Canny edge detector (imageproc) on its own thread, so the preview keeps its speed.
- One card for the app and the background service, at the top of the Device tab.
- Updating needs no removing and reinstalling. A new AppImage opened from `~/Applications` becomes the
  installed one. A replaced AppImage restarts the service on the new version. Installing again brings back a
  service that Remove took away.
- New icon, drawn from a photo of the camera, also shown at the top left of the main window.
- Gesture pictures traced from the reference drawings.

### Added

- **Hand to follow** (Right / Left) under Tracking mode, shown while Hand tracking is on; read from the camera.
- **Picture while the camera sleeps** on the Studio camera card: shown on the Studio camera instead of the
  camera's dark picture while the camera is asleep.

- **Hand** tracking mode again, with its own tracking area kept to the gimbal limits.
- Click on the preview to focus there or to follow that person (a button on the preview picks which). While
  another app uses the camera, this works only if that app uses the Studio camera; the button's menu says so.
- Turning the microphone on or off asks first: it restarts the camera's audio.
- Microphone **Volume** and the **Camera's own sleep timer**, both read from the camera.
- Presets also save the microphone settings, HDR, the camera's sleep timer and the tracking area (on or off,
  Auto-track new targets, return position).

- **Keep AI tracking inside these limits** (Gimbal limits card): the camera's own limited-zone tracking,
  using the same pan and tilt limits as the stick, re-sent whenever they change and when the camera
  reconnects; plus **Auto-track new targets** (OBSBOT Center's name for it), a read-out of the area the camera holds,
  and the **Return position** the camera goes back to when the person leaves the area (use the current position,
  or centre).
- HDR switch, with a 3-second rest between changes (OBSBOT's guidance).
- Fine steps on every slider: the value sits in a pill with up and down arrows; hold an arrow to repeat.
- Cards fold away with an arrow in all three tabs; folded cards stay folded.
- LUTs for the Studio camera: favourites, a strength remembered for each LUT, and looks saved in presets.
  A LUTs button sits beside search, in the quick-controls header, and on the popped-out preview.
- `tools/probe-colour-controls.py`, which measures what the colour controls do, and `tools/sdk-probe/`, which
  records the USB commands of OBSBOT's SDK using a local copy of the SDK.
