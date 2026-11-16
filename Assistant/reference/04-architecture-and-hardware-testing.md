## 8. Suggested test application architecture

A native Rust + GTK4/libadwaita app is a strong fit, especially because
the user wants a polished GTK desktop application.

Suggested modules:

``` text
src/
  main.rs
  app.rs
  ui/
    window.rs
    camera_page.rs
    ptz_page.rs
    tracking_page.rs
    presets_page.rs
    image_page.rs
    diagnostics_page.rs
  device/
    discovery.rs
    identity.rs
    session.rs
  v4l2/
    device.rs
    controls.rs
    metadata.rs
  obsbot/
    xu.rs
    framing.rs
    crc.rs
    commands.rs
    status.rs
    presets.rs
    gestures.rs
  media/
    preview.rs
    snapshot.rs
    recording.rs
  settings/
    config.rs
```

Architecture rules: - UI must not encode protocol bytes. - V4L2
operations must be separate from vendor XU operations. - Keep command
encoders pure and unit-testable. - Add protocol tests using known byte
vectors from the repositories. - Keep raw XU access behind a narrow,
auditable API. - Discover the camera dynamically; never hard-code
`/dev/video0`. - Read control metadata and firmware/status before
enabling model-specific features. - Keep experimental features visibly
separate from verified controls. - Log raw requests/responses in a
diagnostics panel, with secrets absent (none expected). - Use a single
serialized device command queue to avoid overlapping XU operations. - Do
not block the GTK main thread; use async tasks or worker threads and
marshal results back to the UI. - Persist user settings locally, but do
not persist unsafe or stale device state as if it were current. -
Include a "Stop / cancel movement" mechanism where the underlying API
supports it.

### Suggested first milestone

Build a **test bench**, not a full-featured polished app:

1.  Detect Tiny 2 Lite by USB identity and associated V4L2 nodes.
2.  Show device model, firmware if discoverable, node paths and
    available controls.
3.  Read V4L2 control values/ranges without changing anything.
4.  Read the 60-byte XU status block and show raw hex.
5.  Add read-only status decoding only for confirmed fields.
6.  Add one carefully bounded pan/tilt test.
7.  Add zoom and basic image controls.
8.  Add sleep/wake only after explicit UI confirmation because it moves
    the gimbal.
9.  Add AI modes, tracking speed, gestures and presets after validating
    command encoders.

Do not begin with preview, recording, or a full UI. Prove device
discovery and safe control first.

------------------------------------------------------------------------

## 9. Hardware test plan and safety

### Phase A: Read-only inspection

Run:

``` bash
lsusb -d 3564:
v4l2-ctl --list-devices
v4l2-ctl --all -d /dev/videoX
v4l2-ctl --list-ctrls-menus -d /dev/videoX
```

Record: - Exact USB VID:PID. - All associated video nodes. - Driver and
card name. - Firmware version if available. - Full V4L2 control list and
ranges. - Whether selector `0x06` status read works.

### Phase B: Low-risk controls

Test one at a time: - Brightness/contrast/saturation. - White balance. -
Exposure mode. - Zoom.

Record original value, requested value, ioctl result, read-back value,
and visible result. Restore original values after each test.

### Phase C: Movement

-   Warn the user first.
-   Clear the camera's movement area.
-   Use small 1° or 2° changes initially.
-   Never send max-range pan/tilt as a first test.
-   Do not run multiple movement commands concurrently.
-   Record requested and read-back/cached values accurately.

### Phase D: Vendor commands

Test in this order: 1. Status read. 2. Tracking speed. 3. AI mode. 4.
One gesture toggle. 5. Sleep/wake (explicit confirmation; physical
movement). 6. Preset create/list/recall/update/delete, verifying after
every mutation.

Do not run a whole test suite blindly. Stop if the camera disconnects or
starts returning USB errors.

### Phase E: Preview

-   Start one stream.
-   Keep it open.
-   Avoid cycling the stream while issuing XU commands.
-   Test snapshot capture while the stream is active if supported.
-   Monitor kernel logs for USB errors.

Useful monitoring:

``` bash
journalctl -kf
```

------------------------------------------------------------------------

