# Exposure Compensation: Verified UVC Backlight Compensation Implementation

**Device:** OBSBOT Tiny 2 Lite (`3564:fef9`)  
**Validation host:** CachyOS / Arch Linux  
**Date:** 2026-11-20  
**Implementation:** `app/src/main.rs` on `test-app-v0.1`

## Summary

OBSBOT Center's Exposure Compensation slider is implemented by the camera's standard UVC Processing Unit Backlight Compensation control:

- Processing Unit entity: `3`
- Interface: `0`
- Selector: `0x01` (Backlight Compensation)
- Control width: 2 bytes, unsigned little-endian
- Advertised Tiny 2 Lite BLC range: `0..=18`

This is a standard UVC Processing Unit request. It is not an OBSBOT vendor Extension Unit command and must not use the XU encoder.

## Exact UVC requests

Both requests use endpoint zero and the Video Control interface.

### SET_CUR

| Field | Value |
|---|---:|
| `bmRequestType` | `0x21` |
| `bRequest` | `0x01` (SET_CUR) |
| `wValue` | `0x0100` |
| `wIndex` | `0x0300` |
| `wLength` | `2` |
| Data | unsigned 16-bit little-endian BLC value |

### GET_CUR

| Field | Value |
|---|---:|
| `bmRequestType` | `0xA1` |
| `bRequest` | `0x81` (GET_CUR) |
| `wValue` | `0x0100` |
| `wIndex` | `0x0300` |
| `wLength` | `2` |

The Linux app uses `UVCIOC_CTRL_QUERY` with unit `3`, selector `0x01`, and the corresponding request. In `wIndex=0x0300`, the high byte is entity ID 3 and the low byte is interface 0.

## OBSBOT Center label cross-checks

Two adjacent slider positions were tested directly against the physical camera:

| OBSBOT Center label | BLC integer | SET_CUR payload | GET_CUR payload | Result |
|---:|---:|---|---|---|
| `+2.0` | 15 | `0f 00` | `0f 00` | Center displayed `+2.0` |
| `+2.3` | 16 | `10 00` | `10 00` | Center displayed `+2.3` |

For each test:
1. The camera was attached to Linux, not passed through to the Windows VM.
2. Only Video Control interface `3-3.3:1.0` was temporarily unbound from `uvcvideo`.
3. A direct PyUSB SET_CUR was issued to entity 3 / selector 1.
4. GET_CUR immediately returned the same two-byte value.
5. The interface was rebound to `uvcvideo`.
6. The camera was passed to Windows and OBSBOT Center's displayed Exposure Compensation value was checked.

The +2.0 and +2.3 cross-checks establish the zero-based mapping at two adjacent positions. The app's 19-position slider is linear from −3.0 to +3.0 in 1/3-stop increments, so the implementation maps slider position `0..=18` directly to BLC value `0..=18`. Every remaining fractional label has not been individually checked against OBSBOT Center.

### Earlier capture discrepancy

An earlier Windows capture summary recorded BLC payloads `1..18` while stepping through the slider. That capture did not establish a reliable one-to-one label/value mapping for all positions. The former app implementation interpreted the slider position as a zero-based index and added one before writing, producing values `1..19`. The later direct Linux/Center checks show that this offset is wrong for the tested +2.0 and +2.3 positions. The implementation now sends the slider position directly. Do not use the old one-based mapping.

## App implementation

The Controls page exposes Exposure Compensation only while V4L2 Auto Exposure is Auto.

- Startup reads the current BLC value using GET_CUR.
- The slider position is the raw BLC value, from 0 through 18.
- A change sends SET_CUR, then immediately issues GET_CUR.
- The returned camera value is decoded and validated against 0–18.
- The displayed label and persisted configuration use the returned value, not the requested value.
- If the camera reports a different valid value, the slider follows the read-back and the status message reports the mismatch.
- A USB error, malformed/unsupported read-back, or GET_CUR failure is reported as a failed operation; the requested value is not saved as if confirmed.
- The UI test fixture simulates the BLC SET/GET state so headless interaction tests exercise the read-back path without touching hardware.

The implementation remains in the standard Processing Unit helper path (`pu_query`), separate from vendor XU transport.

## Regression coverage

The Rust unit test `exposure_compensation_maps_all_nineteen_positions` checks:

- The direct 0–18 little-endian mapping.
- Explicit hardware vectors `15 -> 0f 00` and `16 -> 10 00`.
- Round-trip decoding for all 19 values.
- Rejection of out-of-range requests and read-back values.
- Slider labels at −3.0, 0.0, +2.0, +2.3, and +3.0.

The synthetic UI test path now stores a SET value and returns it from GET_CUR, allowing the existing headless slider interaction test to verify the read-back-driven label update.

## Confidence and limitations

- **High:** UVC entity/interface/selector and SET_CUR/GET_CUR request fields.
- **High:** direct hardware SET_CUR and matching GET_CUR for BLC values 0, 15, 16, and 18 on the tested camera.
- **High:** OBSBOT Center displays +2.0 for BLC 15 and +2.3 for BLC 16 on the tested camera.
- **Implementation assumption:** the complete 19-position scale is linear and maps position 0–18 directly to BLC 0–18. Only the +2.0 and +2.3 labels have been individually cross-checked with OBSBOT Center.
- **Not established:** identical behavior across all firmware revisions or other OBSBOT models.

The Linux hardware tests used a direct USB path because V4L2 setter success alone was not sufficient evidence of the hardware value on this host. The application path itself uses the kernel UVC ioctl interface and still requires a fresh on-camera validation after this code change.
