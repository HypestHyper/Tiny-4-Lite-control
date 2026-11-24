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

- Commands checked against OBSBOT's own SDK on a Tiny 2 Lite; where they differed, the SDK's are used now
  (old versions kept in `docs/sdk-alignment-backup.md`):
  - audio auto gain sends the SDK's own command;
  - "Tracking speed" is now **Vertical tracking**: Standard, Headroom (new) and Motion (was Sport). Presets
    saved before keep working.
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

- HDR switch, with a 3-second rest between changes (OBSBOT's guidance).
- Fine steps on every slider: the value sits in a pill with up and down arrows; hold an arrow to repeat.
- Cards fold away with an arrow in all three tabs; folded cards stay folded.
- LUTs for the Studio camera: favourites, a strength remembered for each LUT, and looks saved in presets.
  A LUTs button sits beside search, in the quick-controls header, and on the popped-out preview.
- `tools/probe-colour-controls.py`, which measures what the colour controls do, and `tools/sdk-probe/`, which
  records the USB commands of OBSBOT's SDK using a local copy of the SDK.
