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
`0x7F`. All 93 SET_CUR submissions have matching completions with
USB status 0.

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
and Focus Auto to `0x08`. Domatix's Lite-specific protocol reference
also documents `focus_absolute` as 0..100, step 1, and
`focus_automatic_continuous` as Boolean; keep runtime V4L2 metadata
authoritative. The V4L2 API describes Focus Absolute's
unit as undefined and says manual focus while continuous autofocus is
enabled has undefined effect:
- [V4L2 camera controls](https://www.kernel.org/doc/html/latest/userspace-api/media/v4l/ext-ctrls-camera.html)
- [Linux UVC control mapping](https://github.com/torvalds/linux/blob/master/drivers/media/usb/uvc/uvc_ctrl.c)

### Zoom

Five standard Camera Terminal Zoom Absolute writes (selector
`0x0B`, two-byte little-endian values) were observed. Domatix's
Lite-specific protocol reference documents V4L2 `zoom_absolute` as
0..100, step 1, and identifies selector `0x0B` as the Camera Terminal
Zoom Absolute control:

| Relative time | UVC value | Nearby XU float |
|---:|---:|---:|
| 422.824s | 66 | 2.98 at 422.848s |
| 422.931s | 63 | 2.89 at 422.933s |
| 450.217s | 0 | 1.00 at 450.238s |
| 450.329s | 2 | 1.06 at 450.346s |
| 450.540s | 64 | 2.92 at 450.556s |

This close timing and value correspondence strongly associates the XU
float parameter below with the same manual-zoom activity. Domatix's
Lite-specific protocol reference independently states that manual zoom
uses the standard UVC `zoom_absolute` path, while on-device gesture zoom
bypasses it and leaves `zoom_absolute` unchanged. This capture does not
establish the image-processing location or distinguish digital crop
from another camera-side implementation. Do not describe the processing
path as optical, digital, or hardware zoom based on this trace alone.
See [Domatix OBSBOT control protocol](https://github.com/Domatix/obsbot-control/blob/main/docs/PROTOCOL.md)
(sections 2.2 and Q10).

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
| `0A 04 04 3B 04 00` | 15 | Opcode-shaped like the known `0x3B04` preset-name family; appears in recurring groups of three, but the request/reply path is not proven to be the usable preset-read API |
| `0A 04 44 3B FE 7F` | 5 | Matches the known `0x3B44` preset-list opcode; exact meaning of this captured payload form is not independently isolated |

### Preset/name-like GET_CUR data

The GET_CUR response stream also contains readable strings:

- Response record `0A 04 3B 07 00` returned `Preset1`, `Preset2`,
  and `Preset3` in five repeated groups of three.
- Response record `0A 44 3B 03 00` returned strings including
  `set1`, `set2B`, and `set3B`.

These are observed response strings, not a verified mapping from UI
slots to the preset APIs. The Lite-specific Domatix protocol reference
reports that the documented `0x3B04` RPC name-read request is not
usable on its tested firmware; it documents selector `0x0D` as the
working entry-cursor read path. Do not treat the similar-looking
selector-`0x02` records above as interchangeable with that API without
a controlled read-back test.

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


---

## 2026-11-18 Linux Tiny 2 Lite Global / Face autofocus-priority capture

**Target:** Tiny 2 Lite, VID:PID `3564:fef9`  
**USB bus/device:** `3/9`  
**Monitor:** `usbmon3`  
**Capture:** `~/.local/share/tiny4lite-control/captures/obsbot-capture-1795008410-698968467.pcapng`  
**Capture size:** 59,918,028 bytes

The user recorded autofocus enabled, with Global selected at baseline, then changed priorities in this order:

| Checkpoint | Relative time | User action |
|---:|---:|---|
| 1 | 0.000001s | Baseline: autofocus on, Global |
| 2 | 16.214066s | Global → Face |
| 3 | 25.260190s | Face → Global |
| 4 | 32.175261s | Global → Face |
| 5 | 39.067326s | Face → Global |

Four outgoing 60-byte XU SET_CUR command frames were found. Their vendor function record is `0a 02 02 36 04 00` (Tiny 2 Lite `CAM_SET_FACE_FOCUS`, command `0x3602`, receiver `0x02`). The four-byte focus-priority value is at frame payload offset 16:

| Frame | Relative time | Correlated action | Sequence | Payload |
|---:|---:|---|---:|---|
| 31244 | 6.051968s | Initial/baseline Global command candidate | `0x22` | `bf fb 01 00` |
| 78444 | 15.217506s | Global → Face | `0x23` | `be 07 00 00` |
| 119472 | 23.167025s | Face → Global | `0x24` | `bf fb 01 00` |
| 153688 | 29.833731s | Global → Face | `0x25` | `be 07 00 00` |

The 15.217s, 23.167s, and 29.833s writes occur shortly before their corresponding user checkpoints, consistent with the user action preceding the checkpoint marker. The first write at 6.052s occurs well after the baseline marker and before the first transition; it is consistent with the initial Global state but is not independently tied to a specific click. The three subsequent correlated transitions establish the working payload assignment:

- **Global:** `bf fb 01 00`
- **Face:** `be 07 00 00`

This reverses the earlier app implementation's assignment. The previous mapping in the implementation-status document and app README is superseded by this controlled capture.

### Missing final transition

No focus-priority SET_CUR was found for the final Face → Global checkpoint at 39.067s. Inspection of all control transfers from 34s through 41s found only interface alternate-setting changes, periodic UVC GET requests, and a separate three-byte endpoint control request (`00 7d 00`). None is the 60-byte focus-priority command. The final UI transition therefore did not produce a focus command in this recording.

The focus command submissions that were present had matching USB completions with status 0. This establishes successful host USB transfer completion only; it does not prove that the camera applied the priority. There is no reliable camera-side read-back for this setting.

### App correction prompted by this capture

The app's Global/Face payload mapping was corrected to the values above. The GTK selector was also changed to dispatch from the newly clicked active button instead of handling both buttons' `toggled` signals. A grouped ToggleButton pair emits state changes for both the deselected and selected segments; the old handler depended on signal ordering and could fail to send the intended command. The corrected click handler sends only for the active selected segment and restores the previous local selection if submission fails.

These changes are committed on `test-app-v0.1` in commit `8fd0878acb2de0da444bdfb0ae9f6769d3073107`. A new hardware capture is still required to verify all five transitions, especially the previously missing final Face → Global command. Do not mark Global/Face as hardware-working until the camera's visible focus behavior is confirmed.


---

## 2026-11-18 Tiny 2 Lite HDR control capture

**Target:** Tiny 2 Lite, VID:PID `3564:fef9`  
**USB bus/device:** `3/9`  
**Monitor:** `usbmon3`  
**Capture:** `~/.local/share/tiny4lite-control/captures/obsbot-capture-1795009114-271074329.pcapng`

The user recorded HDR Off at baseline and toggled HDR repeatedly. Four outgoing 60-byte Extension Unit SET_CUR submissions were identified. All use request type `0x21`, request `0x01`, selector `0x0600`, and index `0x0200` (unit 2, selector `0x06`). The captured data-fragment prefix is:

| Frame | Relative time | 60-byte payload prefix | State indicated |
|---:|---:|---|---|
| 46349 | 9.001861s | `01 01 01` | On |
| 104351 | 20.272726s | `01 01 00` | Off |
| 138985 | 26.969341s | `01 01 01` | Off |
| 174385 | 33.842670s | `01 01 00` | On |

The selector `0x06` status response independently tracks the same state at byte offset `0x06`:

| Relative time | Status byte `0x06` | State |
|---:|---:|---|
| 1.008366s | `00` | Off |
| 9.161446s | `01` | On |
| 21.410601s | `00` | Off |
| 27.535419s | `01` | On |
| 35.685629s | `00` | Off |

The outgoing command's third byte and status byte `0x06` agree: `00` means HDR Off and `01` means HDR On. The packet/status sequence establishes the command mapping used by the app:

- HDR On: 60-byte selector `0x06` SET_CUR buffer beginning `01 01 01`.
- HDR Off: 60-byte selector `0x06` SET_CUR buffer beginning `01 01 00`.

The last user checkpoint recorded HDR Off, but no corresponding outgoing command appears after the final HDR-On write. The user considers that last click likely not recorded. The repeated command alternation and matching status responses provide sufficient evidence for the mapping; the absent final write is retained as a capture omission, not used to reject the mapping.

### App implementation

The test app now exposes an HDR switch in Camera settings. It reads status byte `0x06` on startup to initialize the switch, falls back to the last successfully submitted local setting if the status read is unavailable, sends the captured 60-byte selector `0x06` payload on change, and persists the preference in `config.toml`. Unit tests cover both command prefixes and the status-byte interpretation. A successful ioctl confirms host-side request completion; the independent status read-back is the stronger state verification.
