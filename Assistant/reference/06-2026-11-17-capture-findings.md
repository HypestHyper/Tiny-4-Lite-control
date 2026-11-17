# 2026-11-17 Windows OBSBOT Center capture findings

**Target:** OBSBOT Tiny 2 Lite, VID:PID `3564:fef9`  
**Capture path:** Windows 11 VM with camera passed through; Linux `usbmon3`  
**USB device address:** 9  
**Original capture:** `~/.local/share/tiny4lite-control/captures/obsbot-capture-1794935483-056861901.pcapng` (about 868 MiB; 3,674,198 frames)  
**Text export:** `/tmp/obsbot-control-all.hex` (997,894 bytes)

## Parsing notes

The export contains 1,236 control-transfer submissions and 1,236
completions for camera address 9. Pair submissions and completions by
URB ID in capture order; URB IDs are reused. The selector `0x02`
Extension Unit traffic contains 93 SET_CUR submissions and 57 GET_CUR
requests. The 93 SET_CUR frames use message types `0x01` (40),
`0x21` (17), and `0x25` (36), with sequence values `0x23` through
`0x7F`.

The Capture tab's Finish operation hung during this large capture, so
its checkpoint/report metadata was not recovered. Packet timestamps
are reliable; do not assign them to individual OBSBOT Center UI actions
unless independently correlated below.

## Standard UVC Camera Terminal controls

### Focus

| Control | UVC selector | Transfer length | Observed behavior |
|---|---:|---:|---|
| Focus Absolute | `0x06` | 2 bytes | GET_CUR returned 16; 14 SET_CUR writes followed |
| Focus Auto | `0x08` | 1 byte | GET_CUR 0, SET_CUR 1, GET_CUR 1, SET_CUR 0 |

Focus Absolute SET_CUR values, little-endian unsigned:
`16, 16, 13, 0, 0, 0, 2, 18, 30, 43, 51, 72, 87, 100`.

All these SET_CUR requests had matching completions with USB status 0.
The manual-focus sequence occurred after Focus Auto was set to 0.
The observed values do not establish the device's supported range;
query V4L2 control metadata at runtime.

Linux UVC maps Camera Terminal Focus Absolute to selector `0x06`
and Focus Auto to `0x08`. The V4L2 API describes Focus Absolute's
unit as undefined and says manual focus while continuous autofocus is
enabled has undefined effect:
- [V4L2 camera controls](https://www.kernel.org/doc/html/latest/userspace-api/media/v4l/ext-ctrls-camera.html)
- [Linux UVC control mapping](https://github.com/torvalds/linux/blob/master/drivers/media/usb/uvc/uvc_ctrl.c)

### Zoom

Five standard Camera Terminal Zoom Absolute writes (selector
`0x0B`, two-byte little-endian values) were observed:

| Relative time | UVC value | Nearby XU float |
|---:|---:|---:|
| 422.824s | 66 | 2.98 at 422.848s |
| 422.931s | 63 | 2.89 at 422.933s |
| 450.217s | 0 | 1.00 at 450.238s |
| 450.329s | 2 | 1.06 at 450.346s |
| 450.540s | 64 | 2.92 at 450.556s |

This close timing and value correspondence strongly associates the XU
float parameter below with zoom. It does not establish the image
processing location or distinguish digital crop from any other
camera-side implementation. Do not describe it as optical, digital,
or hardware zoom without further evidence.

## Extension Unit selector 0x02

### Zoom-associated float record

Seven outgoing SET_CUR frames use message type `0x25` and function
record `0A 04 44 32 04 00`. A float32 little-endian value begins at
payload offset 16:

| Relative time | Sequence | Float |
|---:|---:|---:|
| 422.848s | `0x66` | 2.98 |
| 422.933s | `0x67` | 2.89 |
| 423.502s | `0x69` | 1.00 |
| 450.238s | `0x6C` | 1.00 |
| 450.346s | `0x6D` | 1.06 |
| 450.556s | `0x6E` | 2.92 |
| 451.127s | `0x70` | 4.00 |

GET_CUR responses contain the corresponding status record
`0A 04 68 04 00` (response byte 8 is a length prefix; the five-byte
record begins at byte 9). The float32 little-endian value is at offset
16. Sixteen responses were observed, with values:
`1.00, 3.00, 3.00, 1.00, 1.00, 3.00, 1.00, 1.00, 1.15, 1.47,
1.00, 1.00, 1.00, 1.00, 4.00, 4.00`.

This is a strong zoom association because of the adjacent standard
Zoom Absolute writes, but the exact semantic/processing path is not
proven. Keep the XU record labelled **zoom-associated float** until a
controlled single-feature test or independent implementation confirms
its role.

### Other identified command groups

| Function record | Writes | Evidence / interpretation |
|---|---:|---|
| `0A 04 C4 0C 01 00` | 4 | Existing tracking-speed group; two Sport-pattern and two Standard-pattern writes |
| `0A 04 C4 30 01 00` | 2 | Existing target-selection gesture group; enable and disable patterns both observed |
| `0A 04 44 31 01 00` | 2 | Existing Zoom/L-pose gesture group; enable and disable patterns both observed |
| `0A 02 C2 A0 04 00` | 5 | Matches the documented sleep/wake command family; this trace alone does not assign each write to a UI action |
| `0A 04 04 3B 04 00` | 15 | Recurring preset-related opcode family; exact action/slot mapping not inferred from this trace |
| `0A 04 44 3B FE 7F` | 5 | Matches the known preset-list opcode family; exact UI action not independently isolated |

Other observed records remain unidentified. Notable examples:
`0A 04 04 68 00 00`, `0A 04 04 01 FE 7F`,
`0A 04 04 68 4A 59`, `0A 04 C4 26 01 00`,
`0A 04 C4 08 01 00`, `0A 04 84 50 01 00`,
`0A 04 84 68 08 00`, `0A 04 44 22 01 00`,
`0A 02 02 36 04 00`, and `0A 02 42 19 08 00`.
Keep them unknown; do not add UI labels or setters from identifier
patterns alone.

## Implementation consequences

- Focus Auto and Focus Absolute already fit the app's generic
  `v4l2-ctl --list-ctrls-menus` / get / set path. No special XU
  implementation is needed. Keep device-reported range, step, default,
  and inactive flags authoritative.
- Zoom Absolute is likewise a standard V4L2 control. The XU
  zoom-associated float is analysis evidence only; do not add a second
  user-facing zoom control until its relationship to the standard
  control is understood.
- The capture does not justify implementing any additional unknown XU
  records.
