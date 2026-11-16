## 4. Tiny 2 Lite feature inventory

### 4.1 Power and sleep/wake

**Status:** Lite-confirmed.

Domatix documents `CAM_SET_DEV_STATUS`, opcode `0xA0C2`: - Wake/run
payload: `00 00 00 00` - Sleep payload: `01 00 00 00`

The MCP encoder uses the same payload convention (`encodeSetRunStatus`).

Expected physical behavior: - Sleep stows the gimbal face-down,
approximately 84° pitch. - Wake returns the gimbal toward level.

Tiny4Linux implements a different fixed-frame encoding on unit 2,
selector 2, using 36-byte frames padded to a 60-byte transfer buffer:

Sleep frame (36 bytes):

``` text
AA 25 42 00 0C 00 EA 63 0A 02 C2 A0 04 00 BF FB
01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00
```

Wake frame (36 bytes):

``` text
AA 25 A5 00 0C 00 5F EF 0A 02 C2 A0 04 00 BE 07
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00
```

These are different implementations/encodings of the operation. Do not
mix the fixed frames with the other framed-command encoder.

Tiny4Linux does not verify a device acknowledgement; successful ioctl
return does not prove that the camera changed state. Prefer status
read-back if reliable.

**Safety:** Sleep/wake physically moves the gimbal. Warn the user before
the first test and keep the area around the camera clear.

### 4.2 Gimbal movement / PTZ

**Status:** Standard V4L2 controls; Tiny 2 Lite control ranges
documented.

Use: - `pan_absolute` - `tilt_absolute` - `pan_speed` - `tilt_speed`

Research ranges: - Pan: `-468000` to `+468000`, step `3600`. - Tilt:
`-324000` to `+324000`, step `3600`.

A first test app should expose: - Recenter/home (only if a reliable
method is verified). - Small relative pan/tilt buttons. - Absolute
pan/tilt sliders. - Stop/cancel behavior if supported by the API.

Do not assume a position query gives live physical feedback. The MCP
Linux implementation documents gimbal position as cached/last-commanded
rather than live. Display that distinction clearly.

### 4.3 Zoom

**Status:** Standard V4L2 zoom controls available; gesture zoom
separately confirmed.

Use V4L2: - `zoom_absolute` - `zoom_continuous`

Gesture-based zoom is applied internally by the camera and does not
update V4L2 `zoom_absolute`.

### 4.4 AI tracking / framing modes

**Status:** Ten modes validated on Tiny 2 Lite in Domatix protocol
notes.

Modes: 1. Normal 2. Upper Body 3. Close-up 4. Headless 5. Lower Body 6.
Group 7. Desk 8. Whiteboard 9. Hand 10. Off

Use the exact enum values from the current Domatix protocol
document/source. Do not invent enum numbers from the display order.

### 4.5 Tracking speed

**Status:** Lite-confirmed, firmware-sensitive.

-   Standard
-   Sport

On firmware 6.2.5.x, Sport value is `0x02`. Older firmware may use
`0x21`.

Read device firmware/version where possible and verify the accepted
value before sending. Do not assume a single enum mapping works across
all firmware.

### 4.6 Gestures

**Status:** Gesture controls discovered and verified on Tiny 2 Lite.

Available controls: - Target selection - Zoom - Dynamic zoom - Dynamic
zoom direction

The selector `0x02` framed messages are used. Function groups
documented:

  Gesture                  Function group
  ------------------------ ---------------------
  Target selection         `0a 04 c4 30 01 00`
  Zoom                     `0a 04 44 31 01 00`
  Dynamic zoom             `0a 04 44 33 01 00`
  Dynamic zoom direction   `0a 04 c4 33 01 00`

Gesture on/off payload bytes: - Off: `e6 3f 00 00 00 00` - On:
`27 ff 01 00 00 00`

**Naming trap:** The L-pose gesture is called **Zoom**, not Dynamic
Zoom. Disabling Zoom is the action that stops the L-shaped gesture.
Disabling Dynamic Zoom appears to do nothing for that gesture.

Do not present the gesture controls as a single generic "gesture mode";
expose each function separately and document the naming.

### 4.7 Presets

**Status:** Tiny 2 Lite preset operations verified in Domatix protocol
notes dated 2026-10-20.

Operations/opcodes: - Create: `0x3944` - Delete: `0x3984` - Recall:
`0x39C4` - Read pose: `0x3A44` - Rename: `0x3A84` - Read name:
`0x3B04` - List: `0x3B44` - Update: `0x3E04`

Preset command family is derived as `(index << 6) | 0x04`.

Important behavior: - Delete may ACK even if a slot does not exist.
Verify the preset list after deletion. - Preset names use padded
Base64. - Preset list selector is `0x0C`. - Preset entry cursor is
`0x0D`. - The cursor is consuming; reset it by echo-writing the selector
`0x0C` block before a new enumeration. - Recall payload requires four
`1.0f32` floats. - Pose uses raw V4L2 pan/tilt/zoom integer values
converted to `f32` (scale 1.0).

**Documentation discrepancy:** Older README text may say only preset
recall is supported. The newer protocol document reports
create/update/delete/rename/read-back verified on Tiny 2 Lite. Prefer
the newer protocol notes and source implementation, and validate each
operation on hardware.

### 4.8 Field of view (FOV)

**Status:** Wide and Normal confirmed; Narrow is a no-op on Tiny 2 Lite
firmware 5.10.

-   Wide: 86°
-   Normal: 78°
-   Narrow: 65° (does not change state on the tested Lite firmware;
    read-back remains Normal)

Do not expose Narrow as working on all Lite firmware without retesting.

### 4.9 Exposure, focus, white balance and image settings

**Status:** Standard V4L2 controls available; individual behavior should
be queried and verified.

Potential UI: - Auto/manual exposure. - Exposure time. - Autofocus
toggle. - Manual focus. - Auto/manual white balance. - White-balance
temperature. - Brightness, contrast, saturation, sharpness, gain, hue,
red/blue balance, backlight compensation, power-line frequency.

**Important quirk:** Tiny 2 XU exposure-mode labels are swapped relative
to V4L2. Domatix removed the XU exposure-mode GUI control and uses
standard V4L2 `auto_exposure` as the sole UI control.

Face-AE GUI control was retired in Domatix because it meters correctly
only in the XU auto-exposure path and silently no-ops when the V4L2
exposure route is used. Treat Face AE as experimental, not a reliable
basic feature.

### 4.10 Status block and read-back

Selector `0x06` `GET_CUR` returns a 60-byte status block.

Documented offsets (confidence varies):

  -----------------------------------------------------------------------
  Offset                  Meaning                 Confidence / notes
  ----------------------- ----------------------- -----------------------
  `0x02`                  Sleep state             Tiny4Linux: 0 awake, 1
                                                  sleep

  `0x06`                  HDR                     Listed in protocol
                                                  notes

  `0x07`                  Face AE /               Uncertain
                          gesture-related state   

  `0x09`                  Device status           Field map partly based
                                                  on regular Tiny 2

  `0x0A`                  Auto-sleep seconds      Needs further
                                                  validation

  `0x11`                  FOV                     Lite-confirmed: Wide=1,
                                                  Normal=2; Narrow
                                                  remains 2

  `0x13`                  Mirror                  Partial

  `0x18` / `0x1C`         AI mode / framing       Mapping requires source
                                                  verification

  `0x1F`                  FPS                     Needs validation

  `0x21`                  LED brightness          Needs validation

  `0x24`                  Tracking speed          Lite-confirmed
  -----------------------------------------------------------------------

Do not label unknown bytes as facts. Build a diagnostic page that
displays raw status bytes in hex and decoded fields with a confidence
indicator.

------------------------------------------------------------------------

## 5. Features found in other projects but not yet confirmed for Tiny 2 Lite

These are useful research leads, not promises:

-   LED brightness.
-   Mirror mode.
-   Auto-sleep timeout.
-   HDR control details.
-   Face AE.
-   Additional AI framing/status states.
-   Video FPS settings.
-   Audio capture/recording and microphone controls.
-   Background replacement, blur, noise reduction, and visual effects
    (historical Meet 4K work).
-   Voice-command toggles and microphone pickup distance (proprietary
    SDK app).
-   Virtual camera via `v4l2loopback`.
-   GPU filters and advanced preview processing.

Keep these behind an **Experimental** or **Diagnostics** section until
tested on the Lite.

------------------------------------------------------------------------

