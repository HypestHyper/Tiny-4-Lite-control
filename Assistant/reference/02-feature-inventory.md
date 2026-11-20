## 4. Tiny 2 Lite feature inventory

### 4.1 Power and sleep/wake

Sleep timeout is an app-managed automation setting. The app should
monitor camera use and issue the existing camera sleep command after
the configured idle period. It must be disabled by default, avoid
repeated sleep commands, and handle disconnect/reconnect safely.

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

The app now has a read-only Camera Terminal PanTilt Absolute GET_CUR
implementation (entity 1, selector 0x0D), based on confirmed Windows
DirectShow live physical feedback. The Linux request and UI polling path
are implemented, but direct Linux ioctl success and raw-unit conversion
still require hardware validation. Do not substitute cached V4L2
pan_absolute/tilt_absolute values for this read-back. See
[reference 12](12-2026-11-19-directshow-pan-tilt-position-readback.md).

### 4.3 Zoom and field-of-view presets

**Status:** Physically confirmed; standard V4L2 control only.

Use `zoom_absolute` for all manual zoom and FOV preset operations. The
Tiny 2 Lite reports a range of 0–100. The user confirmed smooth
adjustment across the range and these preset mappings:

| Preset | `zoom_absolute` | Observed zoom factor |
|---|---:|---:|
| Wide | 0 | 1.00× |
| Medium | 5 | 1.15× |
| Narrow | 15 | 1.47× |
| Maximum | 100 | Maximum zoom |

The GTK app exposes the device-reported `zoom_absolute` slider and
three preset buttons. Presets write the same V4L2 control and read its
value back. Do not use `zoom_continuous` or the separate vendor FOV
setter. Gesture zoom remains a separate camera-side gesture and does
not update `zoom_absolute`.


### 4.4 AI tracking / framing modes

**Current app scope:** Off, Normal, Upper Body, Close-up, Headless,
Lower Body, and Group. Desk, Whiteboard, and Hand are intentionally
excluded from the app. Do not expose them as selectable modes.

Use the exact enum values from the current Tiny 2 Lite protocol
implementation/source. Do not invent enum numbers from the display
order.

### 4.5 Tracking speed

**Status:** Both Standard and Sport have been physically confirmed working on the user's Tiny 2 Lite. Firmware compatibility remains a separate concern.

-   Standard: physically confirmed.
-   Sport: physically confirmed.

On firmware 6.2.5.x, Sport value is `0x02`. Older firmware may use
`0x21`. The current app sends the documented `0x02` value and does not
automatically identify firmware or select the legacy encoding. The
hardware test confirms both modes on the tested camera/firmware; it
does not establish compatibility across older firmware.

### 4.6 Gestures

**Status:** Target selection, Zoom/L-pose, Dynamic Zoom, and Direction Flip have all been physically verified on the user's Tiny 2 Lite. Target selection is triggered by an open palm/fingers spread and enables normal tracking. Zoom uses the L-pose gesture. Dynamic Zoom uses both hands in L shapes moved apart/together. Direction Flip changes the direction in which the Dynamic Zoom gesture responds. These two controls work together: enable Dynamic Zoom to use the two-hand gesture, then use Direction Flip to reverse its direction. Direction Flip is not a separate gesture and does not affect Zoom/L-pose or Global/Face autofocus priority.

Available controls: - Target selection - Zoom - Dynamic zoom - Dynamic
zoom direction (Direction Flip)

The selector `0x02` framed messages are used. Function groups:

| Gesture | Function group |
|---|---|
| Target selection | `0a 04 c4 30 01 00` |
| Zoom/L-pose | `0a 04 44 31 01 00` |
| Dynamic Zoom | `0a 04 44 33 01 00` |
| Dynamic Zoom Direction | `0a 04 c4 33 01 00` |

For the ordinary gesture groups, captured payloads are Off =
`e6 3f 00 00 00 00` and On = `27 ff 01 00 00 00`.
**Dynamic Zoom Direction is inverted:** UI Off sends
`27 ff 01 00 00 00`; UI On sends `e6 3f 00 00 00 00`.

The 2026-11-18 controlled capture has three Direction Flip transitions:
OFF at baseline (frame 13247), ON (frame 72265), then OFF (frame
170871). A fourth command (frame 179033) uses the separate Dynamic
Zoom group `0a 04 44 33 01 00` and disables Dynamic Zoom. It is not a
fourth Direction Flip transition. The sequence values progress
`0x000f`, `0x0010`, `0x0011`, `0x0012` across those four framed
commands. The app now allocates a rolling process-local 16-bit sequence
number to every framed XU write. Its initial value of zero is an
implementation choice; the capture does not establish the required
sequence seed after a fresh connection or app restart.

**Naming and relationship:** The L-pose gesture is called **Zoom**, not Dynamic
Zoom. Disabling Zoom is the action that stops the L-shaped gesture.
Dynamic Zoom is a separate two-hand L-shape gesture. Direction Flip is
not an independent gesture: it modifies the direction of the Dynamic
Zoom gesture, so the two settings work together. Direction Flip does
not change the Zoom/L-pose gesture or Global/Face autofocus priority.

The selector 0x06 status block does not provide reliable read-back for
Target Selection, Dynamic Zoom, or Direction Flip. In the app these
controls therefore start as **Unknown** unless a previously successful
local setting was saved; the first click from Unknown sends Enable.
Zoom/L-pose has a status-byte read-back. The Unknown label means the app
cannot determine the initial state, not that the camera reports an error.
A successful ioctl confirms host-side request completion, not that the
camera applied the feature. Keep the controls separate and preserve this
evidence distinction.

### 4.7 Presets

**Current app:** Local application presets can capture and apply
supported V4L2, selected OBSBOT, gesture, and video-mode settings.
These are app-managed snapshots, not camera-native preset slots.

**Camera-native presets:** The Tiny 2 Lite command family is documented
in the Domatix protocol notes, but native slot create/delete/recall/read/
rename/list/update operations are not implemented in the current app.
Validate the read/list/recall path before adding native slot mutations.

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

FOV is implemented as the three `zoom_absolute` presets documented in Section 4.3. There is no separate FOV protocol command or independent FOV state in the app.

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

**Global / Face autofocus priority (separate vendor command):**

- Current app mapping: Global = `bf fb 01 00`; Face = `be 07 00 00`.
- The earlier capture checkpoints were imperfectly correlated and the original mapping was reversed in a later correction. The user has tested both app options and reports that neither produces the expected focus behavior. Treat this feature as not working / unverified until a fresh action-correlated capture and visible behavior test establish the correct mapping.
- Transport: framed V3 command on XU unit 2, selector `0x02`.
- This is face-priority **autofocus**, not face-priority auto-exposure. Face-priority AE is a different selector `0x06` tag `0x03` setting and status byte `0x07`.
- The app exposes Global / Face controls while autofocus is on and sends the capture-derived Tiny 2 Lite payloads above. The user has tested both options and reports that neither currently produces the expected focus behavior. Treat this path as **not working / unverified**. The payload-to-label assignment is inferred from imperfectly correlated checkpoints, and no reliable camera-side read-back is implemented. The UI's local selection or successful host-side submission is not proof that focus behavior changed.


Expose the standard controls reported by V4L2, including auto/manual
exposure, exposure time, autofocus, manual focus, auto/manual white
balance, white-balance temperature, mirror, brightness, contrast,
saturation, sharpness, gain, hue, red/blue balance, backlight
compensation, and power-line frequency. Query each selected camera
node's metadata at runtime; do not hardcode ranges or assume every
firmware exposes every control.

**Important quirk:** Tiny 2 XU exposure-mode labels are swapped relative
to V4L2. Domatix removed the XU exposure-mode GUI control and uses
standard V4L2 `auto_exposure` as the sole UI control.

Face-AE GUI control was retired in Domatix because it meters correctly
only in the XU auto-exposure path and silently no-ops when the V4L2
exposure route is used. Treat Face AE as experimental, not a reliable
basic feature.

**2026-11-18 Tiny 2 Lite Linux capture: Auto Exposure Face / Global**

The controlled capture contains five 60-byte `UVC_SET_CUR` writes to
XU unit 2, selector `0x06`, with these payload prefixes:

| Frame | Payload | Correlated selection |
|---:|---|---|
| 80075 | `03 01 00` | Face |
| 146705 | `03 01 01` | Global |
| 219369 | `03 01 00` | Face |
| 297559 | `03 01 01` | Global |
| 300515 | `03 01 00` | Face |

The remaining 57 bytes are zero. Repeated transitions support the
mapping **Face = 0** and **Global = 1** in the third payload byte.
The final checkpoint was labelled Global, but the final actual write
was Face; preserve this mismatch as a checkpoint discrepancy. The app
implements the setting only while V4L2 `auto_exposure` is Auto and
keeps it separate from the framed `CAM_SET_FACE_FOCUS` autofocus
priority command. No reliable GET_CUR read-back mapping has been
established, so the app remembers only the last successful local write.

**2026-11-18 Tiny 2 Lite Linux capture: exposure compensation**

Exposure compensation is a UVC Processing Unit control, not an OBSBOT
Extension Unit command and not a V4L2 exposure-bias control. The camera
descriptor identifies Processing Unit entity 3; selector 1 is
Backlight Compensation. The controlled capture correlates the 19
OBSBOT Center positions with these two-byte little-endian values:

| OBSBOT Center | PU value | Payload |
|---:|---:|---:|
| −3.0 | 1 | 01 00 |
| −2.7 | 2 | 02 00 |
| −2.3 | 3 | 03 00 |
| −2.0 | 4 | 04 00 |
| −1.7 | 5 | 05 00 |
| −1.3 | 6 | 06 00 |
| −1.0 | 7 | 07 00 |
| −0.7 | 8 | 08 00 |
| −0.3 | 9 | 09 00 |
| 0.0 | 10 | 0a 00 |
| +0.3 | 11 | 0b 00 |
| +0.7 | 12 | 0c 00 |
| +1.0 | 13 | 0d 00 |
| +1.3 | 14 | 0e 00 |
| +1.7 | 15 | 0f 00 |
| +2.0 | 16 | 10 00 |
| +2.3 | 17 | 11 00 |
| +2.7 | 18 | 12 00 |
| +3.0 | 19 | 13 00 |

The captured USB setup fields are bmRequestType=0x21,
bRequest=0x01 (SET_CUR), wValue=0x0100, wIndex=0x0300,
wLength=2. The high byte of wIndex selects entity 3 and the low
byte selects VideoControl interface 0. In the UVC ioctl query, use
unit 3, selector 1, SET_CUR, and a two-byte little-endian payload.
The app exposes this slider only when V4L2 Auto Exposure is Auto and
uses GET_CUR to initialize it when the driver supports read-back.
The last successful slider selection is persisted locally; it is not
automatically replayed to the camera on startup.

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
This is a legacy research map of selector 0x06 offsets, not a
statement that every field is implemented or authoritative. The current
app reads the raw 60-byte block and only decodes fields with supporting
Lite-specific evidence.

| Offset | Meaning | Current confidence / app treatment |
|---|---|---|
| `0x02` | Sleep state | Tiny4Linux mapping only; not used as authoritative sleep read-back. |
| `0x06` | HDR | Lite capture supports 0 = Off / 1 = On; app reads this byte. |
| `0x07` | Face AE / gesture-related | Mixed/uncertain field; the L-pose gesture uses an inverted state interpretation. Do not infer Face AE state from it. |
| `0x09` | Device status | Partly inherited from regular Tiny 2; unverified on Lite. |
| `0x0A` | Auto-sleep seconds | Unverified; automatic sleep timeout is not implemented. |
| `0x11` | FOV | Legacy diagnostic byte only. Manual zoom/FOV uses standard V4L2 `zoom_absolute`; no separate FOV control. |
| `0x13` | Mirror | Partial/uncertain status mapping; app uses the captured selector 0x06 command and does not claim read-back. |
| `0x18` / `0x1C` | AI mode tuple | Current app decodes the captured Tiny 2 Lite tuple; retain unknown tuples as Unknown. |
| `0x1F` | FPS | XU status mapping unverified. App capture mode uses standard V4L2 format/frame-interval negotiation instead. |
| `0x21` | LED brightness | Confirmed raw 0 = Off, 1–3 = brightness levels 1–3. |
| `0x24` | Tracking speed | Tiny 2 Lite mapping confirmed; firmware-specific encoding compatibility remains open. |

Do not label unknown bytes as facts. The Diagnostics page displays
the raw status block and only decodes fields for which the app has
supporting evidence.

### 4.12 Microphone processing

**Status:** Disable microphone, auto gain, noise reduction, and radio
distance are protocol-mapped from controlled Tiny 2 Lite Linux
captures. App-side hardware validation is pending.

All four controls use Extension Unit 2, selector `0x06`, but they do
not all share one payload format. Disable microphone and radio distance
use distinct fixed command payloads; auto gain and noise reduction are
fields in the 60-byte GET_CUR configuration block.

| Setting | Payload byte offset (zero-based) | Values |
|---|---:|---|
| Disable microphone | 2 | 0 = Disabled, 1 = Enabled |
| Noise reduction | 8 | 0 = Off, 1 = Weak, 2 = Medium, 3 = Strong |
| Auto gain | 25 | 0 = Off, 1 = On |

Disable microphone was captured through UI On → Off → On transitions.
The corresponding SET_CUR payload prefixes were `1c 01 00`,
`1c 01 01`, and `1c 01 00`. The camera restarted after each toggle
in OBSBOT Center. The restart mechanism and any additional commands
associated with it have not yet been investigated. The app's switch is
labelled **Disable microphone**, so active means byte value 0.

Noise reduction was captured through Off → Weak → Medium → Strong →
Medium, with the corresponding byte values read back. Auto gain was
captured through Off → On → Off → On, also with matching read-backs.
These mappings are independent in the observed traces.

Auto gain and noise reduction use read-modify-write: the app reads
the current status block, changes only the selected setting's byte, and
sends the full block with SET_CUR. This preserves unrelated fields.
Disable microphone uses a distinct captured 60-byte command payload
with prefix `1c 01 <state>` (0 = disabled, 1 = enabled) and zero-filled
remaining bytes. It is not the GET_CUR status block. No matching
read-back has been established, so the app displays only the last
successfully submitted local state. A successful ioctl is host
transport evidence, not independent proof that the camera applied the
change.

### Radio distance

A controlled 2026-11-19 capture isolated three OBSBOT Center selections.
Each selection sent one 60-byte SET_CUR payload to Extension Unit 2,
selector `0x06`. The payload begins with `1b 01`; byte 2 (zero-based)
encodes the distance:

| OBSBOT Center setting | Byte 2 | Payload prefix |
|---|---:|---|
| Close | `00` | `1b 01 00` |
| Standard | `01` | `1b 01 01` |
| Far | `02` | `1b 01 02` |

Bytes 3–59 were zero in all three captured writes. The GET_CUR
responses surrounding the writes were identical, so this is a separate
fixed command payload, not a verified field in the shared status block.
No reliable camera read-back is established. The app exposes a
Close / Standard / Far selector and remembers only the last successful
local submission. A successful ioctl is transport evidence, not
independent proof that the camera applied the setting.

**Microphone during sleep:** Not implemented. No isolated action-correlated mapping has been recovered. Keep this separate from the ordinary microphone-disable command.

**Pop filter:** An experimental fixed-payload prototype is present in the app. The protocol investigation remains incomplete: XU unit 2 / selector `0x02`, 60-byte SET_CUR writes, with byte 17 On=`01` / Off=`00` and bytes 14–15 On=`ef 4f` / Off=`be 8f`. Byte 2 behaves as a sequence counter; bytes 6–7 repeat for matching counter values, but their generation rule is unknown. The prototype uses captured session-specific payload data and is not a general payload builder. Its current source sets byte 32 to `01`, while the investigation's section 7 describes offsets 32–59 as zero. Resolve this source/evidence discrepancy against the raw fresh-session capture before changing the payload. No reliable GET_CUR read-back exists. The prototype still needs a focused ON/OFF hardware test; do not infer success from ioctl completion.

------------------------------------------------------------------------

## 5. Remaining research leads and out-of-scope features

Do not treat this list as a list of unimplemented controls. Confirmed
Tiny 2 Lite mappings and current app implementation are described in
the sections above and in `APP-IMPLEMENTATION-STATUS.md`.

- Automatic sleep timeout: command exists, idle automation does not.
- LED blink/solid mode and tag `0x18` semantics remain unresolved.
- Additional AI framing/status states beyond the modes currently exposed.
- Firmware-specific tracking-speed compatibility, especially older
  firmware encodings.
- Host-side audio capture/recording and routing.
- Background replacement, video blur/effects, virtual camera via
  `v4l2loopback`, and GPU filters are outside the current control-app
  milestone.
- Voice-command toggles and other proprietary SDK-only functions remain
  outside scope.

Microphone processing controls, mirror image, HDR, and video capture
mode selection are not generic unconfirmed research leads: their
specific evidence and validation status are recorded above and in the
implementation-status document.

------------------------------------------------------------------------

