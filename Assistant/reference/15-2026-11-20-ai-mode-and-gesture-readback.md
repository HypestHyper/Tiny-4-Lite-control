# OBSBOT Tiny 2 Lite: AI mode and gesture status read-back

**Test date:** 2026-11-20  
**Device:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Host:** CachyOS / Arch Linux  
**Interface:** UVC Video Control interface 0, Extension Unit 2

## Summary

Controlled Linux UVC Extension Unit tests established:

- All eight application AI tracking modes can be set with selector `0x06`, opcode `0x16`, and read back exactly from selector `0x06` status offsets `0x18` and `0x1C`.
- Target Selection gesture state is readable from selector `0x06` status byte `0x01`: `0x00` = disabled and `0x01` = enabled.
- Zoom/L-pose, Dynamic Zoom, and Dynamic Zoom Direction SET_CUR commands were accepted, but their ON/OFF tests did not produce a corresponding change in selectors `0x03`, `0x06`, or `0x09`. Their state must remain saved/local or Unknown; do not claim camera read-back.
- Selector `0x03` byte 0 and selector `0x09` byte 0 correlate with some AI mode changes, but their meanings remain unknown.
- Selectors `0x07`, `0x0C`, and `0x0E` were static across repeated one-second reads while the camera was moved. No live telemetry meaning is established.

## Extension Unit descriptor and request shape

| Property | Observed |
|---|---|
| Unit ID | `2` |
| GUID | `9a1e7291-6843-4683-6d92-39bc7906ee49` |
| `bNumControls` | `19` |
| Selectors | `0x01` through `0x13` |
| GET_LEN | `60` bytes for every selector queried |
| GET_INFO | `0x03` for every selector queried |

GET_INFO `0x03` indicates selector-level GET/SET support. It does not prove every selector is semantically meaningful or that each command has a useful state read-back.

The Linux UVC ioctl used for the 60-byte reads/writes is `UVCIOC_CTRL_QUERY = 0xC0107521`. The application uses unit 2, selector `0x06`, and 60-byte buffers for the AI mode status/setter path.

### Selector GET_CUR inventory

| Selector | Observed GET_CUR |
|---|---|
| `0x01`–`0x03` | Initially zero-filled; `0x03` byte 0 correlates with tracking/mode changes |
| `0x04` | `01` followed by zeros |
| `0x05` | Zero-filled |
| `0x06` | Structured 60-byte global status block |
| `0x07` | `06000000e4fb9c006400010000bcf33ef96400020000cefa78236400020000cefa78236400020000cefa78236400020000cefa782364000000000000` |
| `0x08` | ASCII `Tiny 2 Lite StreamCamera`, then zero padding |
| `0x09` | Initially zero; byte 0 varies with AI mode |
| `0x0A` | `ff` followed by zeros |
| `0x0B` | `01` followed by zeros |
| `0x0C` | `03 02 00 01` followed by zeros |
| `0x0D` | Not read; known consuming preset cursor |
| `0x0E` | `fe 20 04 00` followed by zeros |
| `0x0F`–`0x13` | Zero-filled |

The selector `0x07`, `0x0C`, and `0x0E` blocks were each read ten times at one-second intervals while the camera was physically moved. All samples were identical. This does not prove the selectors are unused; it only means no changing telemetry was observed in that test.

## AI tracking mode setter and status

The setter uses selector `0x06`, SET_CUR, with a 60-byte buffer:

```text
16 02 <mode> <submode> 00 00 ... (60 bytes total)
```

The following eight tuples were each set and then read from selector `0x06` GET_CUR. The status tuple at offsets `0x18` and `0x1C` matched the requested tuple in every test.

| Application mode | Mode byte | Submode byte | Status read-back |
|---|---:|---:|---|
| No Tracking | `00` | `00` | Exact |
| Normal Tracking | `02` | `00` | Exact |
| Upper Body | `02` | `01` | Exact |
| Close-up | `02` | `02` | Exact |
| Headless | `02` | `03` | Exact |
| Lower Body | `02` | `04` | Exact |
| Hand | `03` | `00` | Exact |
| Group | `01` | `00` | Exact |

The app should expose exactly these eight modes. The mode selector must initialize from the camera's current status tuple, retain Unknown for unrecognized tuples, and refresh status from GET_CUR after an explicit refresh.

### Other status-byte correlations (not decoded)

Selector `0x03` byte 0 was `0x00` for No Tracking and Group, and `0x01` for Normal Tracking, Upper Body, Close-up, Headless, Lower Body, and Hand.

Selector `0x09` byte 0 was `0x00` for No Tracking, `0x01` for Normal Tracking, `0x02` for Upper Body, `0x03` for Close-up, and `0xFF` for Headless, Lower Body, Hand, and Group.

These are observed correlations only. Do not label either byte as an AI mode ID, tracking-enabled flag, or other semantic field without additional controlled evidence. After the tests, the original No Tracking tuple was restored. Other global status bytes did not all return to their original baseline values, and selector `0x09` remained `0xFF`; therefore the test restored the AI mode tuple, not the entire camera state.

## Gesture commands and status read-back

The existing framed selector `0x02` command groups are:

| Gesture | Function group |
|---|---|
| Target Selection | `0a 04 c4 30 01 00` |
| Zoom (L-pose) | `0a 04 44 31 01 00` |
| Dynamic Zoom | `0a 04 44 33 01 00` |
| Dynamic Zoom Direction | `0a 04 c4 33 01 00` |

Full captured ON/OFF frames:

| Gesture | OFF frame | ON frame |
|---|---|---|
| Target Selection | `aa2508000c002b5a0a04c4300100e63f00` | `aa2509000c007a9f0a04c430010027ff01` |
| Zoom | `aa250a000c002a900a0444310100e63f00` | `aa250b000c007b550a044431010027ff01` |
| Dynamic Zoom | `aa250c000c004b4e0a0444330100e63f00` | `aa250d000c001a8b0a044433010027ff01` |
| Dynamic Zoom Direction | `aa250e000c008b440a04c4330100e63f00` | `aa250f000c00da810a04c433010027ff01` |

Dynamic Zoom Direction has inverted ON/OFF wire payload semantics relative to the other three commands. Preserve the existing command builder and rolling frame sequence implementation.

### Controlled read-back results

| Gesture test | Selector `0x06` byte 1 | Other queried status |
|---|---|---|
| Target Selection ON | `00 → 01` | — |
| Target Selection OFF | `01 → 00` | — |
| Zoom ON/OFF | Unchanged | Selectors `0x03`, `0x06`, `0x09` unchanged |
| Dynamic Zoom ON/OFF | Unchanged | Selectors `0x03`, `0x06`, `0x09` unchanged |
| Dynamic Zoom Direction ON/OFF | Unchanged | Selectors `0x03`, `0x06`, `0x09` unchanged |

The Target Selection UI state is initialized from selector `0x06` byte offset `0x01`, with direct mapping: zero = disabled, one = enabled. Camera read-back takes precedence over a saved local value. If the read fails or returns an unexpected byte, the app may fall back to the last saved value; otherwise show Unknown.

The other three gesture controls have no confirmed device-side state read-back. Initialize them from the last successfully saved local command state or Unknown. Never infer their state from selector `0x06` byte `0x07`; the earlier assumption that this byte represented Zoom state was disproven by the controlled tests.

A successful SET_CUR/USB ioctl only confirms host-driver request completion. The user's separate physical tests confirmed the visible behavior of all four gesture features; the lack of status change for three gestures is a read-back limitation, not evidence of failed gesture functionality.

## Implementation contract for the GTK app and future UI work

1. Keep the eight AI mode tuples exactly as listed.
2. Initialize the AI mode selector from selector `0x06` offsets `0x18` and `0x1C`.
3. Read Target Selection from selector `0x06` byte `0x01` (0 = Off, 1 = On).
4. Do not use selector `0x06` byte `0x07` as Zoom gesture state.
5. For Zoom, Dynamic Zoom, and Dynamic Zoom Direction, show saved/local state or Unknown; do not present it as camera-confirmed.
6. Keep gesture controls and AI tracking modes distinct. Target Selection is a gesture command, not an AI mode.
7. Preserve all existing selector `0x02` frame payloads and sequence/CRC construction.
8. Keep unknown selector fields unknown. A descriptor advertising GET/SET support is not semantic documentation.
