# 2026-11-17 PTZ position read-back investigation

**Target:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Purpose:** Determine whether the camera exposes current pan/tilt position through vendor UVC Extension Unit read-back, including while AI tracking moves the gimbal.  
**Evidence status:** Position-like data is strongly indicated in selector `0x02` GET_CUR responses, but continuous/live read-back during AI tracking is not yet verified.

## Executive summary

- Standard V4L2 `pan_absolute` / `tilt_absolute` controls are already used by the app for movement. Existing research warns that some Linux implementations expose cached or last-commanded values rather than guaranteed live physical position.
- A correctly paired set of 83 vendor XU selector `0x02` GET_CUR responses contains records whose float32 values at payload offset 20 track plausible pan/tilt angles.
- The strongest candidate response record is `04 0A 44 3A 18 00`. A second record, `02 0A 42 29 05 00`, returned the same position values in three observed cases.
- This is not yet proof of a continuously readable live pose. Most samples came from captures around manual movement or preset recall. A dedicated AI-tracking capture did not contain selector `0x02` GET_CUR requests because OBSBOT Center did not poll that selector during the test.
- The next decisive experiment is to issue read-only selector `0x02` GET_CUR requests at a controlled interval while AI tracking is moving the camera, then compare the decoded values with observed movement.

## USB transport

Camera was passed through directly to the Windows 11 QEMU/KVM VM. Do not attempt concurrent access from the Linux host while the VM owns the USB device.

- XU entity ID: `2`
- XU GUID: `9A1E7291-6843-4683-6D92-39BC7906EE49`
- Interface: `0`
- XU control selector: `0x02`
- GET_CUR request: `bmRequestType=0xA1`, `bRequest=0x81`
- `wValue=0x0200`
- `wIndex=0x0200` (entity 2, interface 0)
- `wLength=60`
- SET_CUR is `bRequest=0x01`; do not send SET_CUR for this read-back investigation.

The selector `0x06` status block is a different XU control. Its AI tracking state bytes must not be mistaken for pan/tilt position.

## Selector 0x02 GET_CUR: position-like response data

### Pairing and decoding rules

An earlier analysis incorrectly associated responses with requests because USB URB IDs were reused. Discard that earlier pairing. The evidence below uses the corrected pairing of submissions and completions, matched by URB ID in capture order.

Across the corrected sample set, 83 selector `0x02` GET_CUR responses were examined. The response records that appear position-related include:

- `04 0A 44 3A 18 00` (12 responses)
- `02 0A 42 29 05 00` (3 responses)

For these records, interpret two little-endian IEEE-754 float32 values beginning at response payload offset 20 as candidate pan and tilt values, respectively. This interpretation is based on plausible angle-like values and repeated correspondence across captures. It remains a candidate field mapping, not a fully decoded protocol specification.

### Observed samples

| Relative time | Response record | Candidate pan | Candidate tilt |
|---:|---|---:|---:|
| 8.383s | `04 0A 44 3A 18 00` | 3.30 | -31.10 |
| 17.726s | `04 0A 44 3A 18 00` | -38.50 | -30.20 |
| 28.395s | `04 0A 44 3A 18 00` | 92.70 | -27.70 |
| 41.666s | `04 0A 44 3A 18 00` | 3.30 | -31.10 |
| 126.585s | `04 0A 44 3A 18 00` | -38.50 | -30.20 |
| 158.174s | `04 0A 44 3A 18 00` | 92.70 | -27.70 |
| 213.374s | `04 0A 44 3A 18 00` | 1.50 | -10.50 |
| 223.322s | `02 0A 42 29 05 00` | 1.50 | -10.50 |
| 223.556s | `04 0A 44 3A 18 00` | 1.50 | -10.50 |
| 264.799s | `04 0A 44 3A 18 00` | -17.30 | -31.50 |
| 273.453s | `02 0A 42 29 05 00` | -17.30 | -31.50 |
| 273.676s | `04 0A 44 3A 18 00` | -17.30 | -31.50 |
| 308.161s | `04 0A 44 3A 18 00` | 90.90 | -13.40 |
| 313.948s | `02 0A 42 29 05 00` | 90.90 | -13.40 |
| 314.248s | `04 0A 44 3A 18 00` | 90.90 | -13.40 |

The values are consistent with pan/tilt-like angles. Matching values from both response record types strengthen the position-read-back hypothesis. They do not yet establish update rate, latency, coordinate origin, limits, sign convention, or whether values remain current during all camera operating modes.

### Request frequency and current limitation

OBSBOT Center sent selector `0x02` GET_CUR requests during some ordinary operation sequences. However, in the dedicated AI-tracking capture it sent 19 selector `0x06` GET_CUR requests over approximately 38.3 seconds and **zero selector `0x02` GET_CUR requests**.

In that AI-tracking capture, selector `0x06` response byte offset `0x18` changed from `0x00` to `0x02` at approximately 4.66 seconds and remained `0x02`. This is evidence of a tracking-state change, not position feedback.

Therefore, the absence of position updates in that capture says only that OBSBOT Center did not poll selector `0x02` during that test. It does not show that the camera cannot answer a host-initiated GET_CUR request while tracking.

## Selector 0x06: tracking state, not pose

Selector `0x06` GET_CUR returns a 60-byte status block. In the AI-tracking test, byte offset `0x18` changed from `0x00` to `0x02`; this corresponds to tracking becoming active in the observed test. The related mode/framing byte at `0x1C` is used by the current app's tracking-mode status decoder.

Do not decode selector `0x06` offsets `0x18` or `0x1C` as pan/tilt coordinates. Do not reuse the selector `0x02` offset-20 float interpretation on selector `0x06` data.

## Windows KS topology investigation

The Windows VM was used to inspect the camera's Kernel Streaming topology and attempt to access the XU through KS interfaces.

### Enumerated topology

For `OBSBOT Tiny 2 Lite StreamCamera`, `IKsTopologyInfo` reported four nodes:

| Node | GUID | Reported name |
|---:|---|---|
| 0 | `dff229e6-f70f-11d0-b917-00a0c9223196` | Video Camera Terminal |
| 1 | `dff229e5-f70f-11d0-b917-00a0c9223196` | Video Processing |
| 2 | `941c7ac0-c559-11d0-8a2b-00a0c9255ac1` | Device Specific |
| 3 | `dff229e1-f70f-11d0-b917-00a0c9223196` | Video Streaming |

Node 2 is a plausible vendor Extension Unit node based on its Device Specific type, but that mapping has not yet been proven by a successful property query.

### KS property attempts

- Filter-level `IKsControl` QueryInterface succeeded.
- `IKsTopologyInfo.CreateNodeInstance(2, IID_IUnknown, ...)` succeeded.
- QueryInterface for `IKsControl` on that node instance also succeeded.
- Attempts to issue a `KSP_NODE` property request using the OBSBOT XU GUID and node ID 2 returned `0x80070057` (`E_INVALIDARG`), including a zero-length-buffer test.
- A related attempt using filter-level `IKsControl` also returned `E_INVALIDARG`.

These failures do not disprove the XU's existence. The exact KS property set, property ID, flags, node routing, and request layout still need to be verified. Do not assume that the USB XU selector number maps directly to a KS property ID.

## Current implementation implications

- Keep standard V4L2 pan/tilt controls for issuing movement commands.
- Do not yet display V4L2 `pan_absolute` / `tilt_absolute` read values as guaranteed live physical position.
- Treat selector `0x02` GET_CUR as the leading candidate for independent position telemetry.
- Do not implement polling in the app until the host-initiated GET_CUR has been tested during AI tracking and during idle/manual movement.
- Do not infer or add a selector `0x02` SET_CUR movement command from the read-back evidence alone.
- Preset names and settings are intended to be stored locally by this project. Do not assume OBSBOT Center's preset storage or undocumented preset APIs are needed for the app's local preset workflow.

## Next experiment: host-initiated position polling during AI tracking

1. Keep the camera passed through to Windows; do not access it from Linux concurrently.
2. Ensure the gimbal movement area is clear and use normal, low-risk AI tracking conditions.
3. Capture USB traffic while the camera is idle, then while AI tracking visibly moves the gimbal.
4. From a single process, issue only read-only selector `0x02` GET_CUR requests using the transport tuple above, at a modest fixed interval (for example, 2–5 requests per second). Do not send SET_CUR.
5. Record request timestamps, completion status, all 60 response bytes, and the decoded float32 values at offset 20.
6. Compare the values against visible pan/tilt movement. Check whether values change smoothly, stop when movement stops, and remain coherent across tracking start/stop.
7. Repeat a short manual movement test and an idle test as controls.
8. Stop immediately on USB errors, device disconnect, unexpected physical movement, or inconsistent responses.

A successful GET_CUR completion alone is not enough. Confirm that the values change with physical movement and are stable when the camera is stationary before treating them as live current-position telemetry.

## Evidence boundaries

**Observed:**
- Selector `0x02` GET_CUR requests and 60-byte responses.
- Correctly paired response records and float32 values at offset 20.
- Repeated angle-like pan/tilt pairs across multiple samples.
- Selector `0x06` tracking-state change during the dedicated tracking capture.
- Windows KS topology and the HRESULTs from the attempted property queries.

**Strong hypothesis, not yet verified:**
- Selector `0x02` GET_CUR offset-20 float32 pair represents current pan and tilt.
- The response may be usable for live position telemetry if actively polled during movement.

**Not established:**
- Live position updates during AI tracking.
- Polling interval, latency, accuracy, coordinate conventions, or supported range.
- Exact KS property mechanism for accessing the XU.
- Any safe or documented selector `0x02` SET_CUR method for moving to a requested pose.
