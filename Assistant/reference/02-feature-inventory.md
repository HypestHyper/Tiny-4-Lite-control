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

**2026-11-17 capture evidence:** Five standard UVC Camera Terminal
Zoom Absolute writes (selector `0x0B`, two-byte little-endian values
66, 63, 0, 2, 64) were closely followed by XU selector `0x02`
messages carrying a float32 little-endian value at payload offset 16:
2.98, 2.89, 1.00, 1.06, 2.92. The same XU record later carried 4.00.
Its GET_CUR counterpart returned values including 1.00, 1.15, 1.47,
3.00 and 4.00. This strongly associates the XU parameter with zoom,
but the trace does **not** establish whether the crop is performed by
the camera's image processor or elsewhere. Do not label it optical,
digital, or hardware zoom based on this trace alone.

Gesture-based zoom remains a separate feature and does not update
V4L2 `zoom_absolute`. See
[2026-11-17 capture findings](06-2026-11-17-capture-findings.md)
for packet details.

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

**Status:** Gesture controls physically verified on Tiny 2 Lite. The user confirmed Target selection (open palm/fingers spread enables normal tracking), Zoom/L-pose, and Dynamic Zoom (both hands form L shapes and move apart/together). Dynamic Zoom direction remains unknown; do not infer which hand movement increases or decreases zoom.

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

**Status:** Wide, Normal, and Narrow visually confirmed working by the user.

-   Wide: 86°
-   Normal: 78°
-   Narrow: 65°

**Compatibility limitation:** FOV changes do not work with certain AI tracking modes. The specific incompatible modes have not yet been identified, so do not imply that FOV is available in every tracking mode. If a change is ignored, switch tracking off or to another mode and retry. The Narrow status-byte read-back mapping is still unverified; visual operation is confirmed.

### 4.9 Exposure, focus, white balance and image settings

**Status:** Standard V4L2 controls available; individual behavior should
be queried and verified.

Focus uses standard UVC Camera Terminal controls:

- Focus Absolute: selector `0x06`, two-byte little-endian unsigned
  value; exposed by V4L2 as `focus_absolute`.
- Focus Auto: selector `0x08`, one-byte Boolean; exposed by V4L2 as
  `focus_automatic_continuous`.

**2026-11-17 capture evidence:** OBSBOT Center read Focus Absolute as
16, then wrote 14 values: 16, 16, 13, 0, 0, 0, 2, 18, 30, 43, 51,
72, 87, 100. It read Focus Auto as 0, enabled it (1), read back 1,
then disabled it (0) before the manual-focus sequence. All observed
writes completed with USB status 0. These are observed values, **not**
proof that the device's supported minimum/maximum are 0 and 100.
Query the selected V4L2 node's control metadata at runtime; do not
hardcode a range. Manual focus while continuous autofocus is enabled
has undefined effect in the V4L2 API.

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

See [2026-11-17 capture findings](06-2026-11-17-capture-findings.md)
for raw request details and the accompanying zoom correlation.

### 4.10 LED power and brightness

**Status:** Four settings are confirmed: Off plus brightness levels 1–3. The tag 0x1A setting command is implemented with raw values 0=Off and 1–3=brightness levels. Tag 0x18 behaviour remains unresolved and is excluded from the current UI.

- Selector `0x06`, tag `0x1A`, payload `[0x1A, 0x01, raw]` selects the LED setting.
- Raw `0` = Off; raw `1`, `2`, and `3` = brightness levels 1, 2, and 3.
- Status byte `0x21` reports the selected setting.
- Tag `0x18` has separate 0/1 writes in the capture, but its semantics are unresolved. The user's live test found that the On command makes the LED blink and the Off command leaves it continuously lit. It is not verified as LED power and is not used by the current UI.

The earlier app implementation subtracted one from the selected brightness level, so level 1 sent raw `0` and switched the LED Off. It also sent tag `0x18` as if it were a power switch. The corrected UI exposes a single Off/1/2/3 setting control and sends the selected raw value directly. Do not reinterpret tag `0x18` without evidence.

### 4.11 Status block and read-back

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

  `0x21`                  LED brightness          Captured raw values
                                                  0–3 map to user levels
                                                  1–4; power is separate

  `0x24`                  Tracking speed          Lite-confirmed
  -----------------------------------------------------------------------

Do not label unknown bytes as facts. Build a diagnostic page that
displays raw status bytes in hex and decoded fields with a confidence
indicator.

------------------------------------------------------------------------

## 5. Features found in other projects but not yet confirmed for Tiny 2 Lite

These are useful research leads, not promises:

-   LED blink/solid mode details beyond the captured tag 0x18 command.
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

