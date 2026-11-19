# 2026-11-19 selector 0x02 motion-associated SET_CUR payloads

**Target:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Capture:** `obsbot-capture-1795096511-064087666.pcapng`, Linux usbmon3, camera USB address 19  
**Capture size:** 489,552 bytes  
**Evidence status:** A recurring host-to-camera selector `0x02` SET_CUR frame contains two little-endian float32 fields at payload offsets 20 and 24. Both fields vary during the controlled pan/tilt sequence. Their semantic meaning is not yet established. These are **outgoing writes**, not camera-to-host position read-back.

## Summary

The controlled 2026-11-19 capture exposed a useful distinction:

- Selector `0x02` GET_CUR responses had previously yielded a candidate pair of float32 values at payload offsets 20 and 24 (see [the PTZ position read-back investigation](07-2026-11-17-ptz-position-readback.md)).
- This capture also contains repeated selector `0x02` SET_CUR writes. Their 60-byte payloads begin with `AA 25`; the float32 values at offsets 20 and 24 change over time.
- The writes occur in bursts around the controlled movement sequence. Their values are correlated with the test's pan/tilt activity, but the direction of transfer means they cannot, by themselves, establish the camera's actual current physical position.

Do not label these fields as confirmed pan/tilt angles, position telemetry, velocity, or target coordinates yet. Do not use these captured SET_CUR payloads as a template for a new movement command.

## USB request

The relevant records are host-to-device class/interface control transfers:

| Field | Value |
|---|---:|
| `bmRequestType` | `0x21` |
| `bRequest` | `0x01` (SET_CUR) |
| `wValue` | `0x0200` (selector `0x02`) |
| `wIndex` | `0x0200` (Extension Unit 2, interface 0) |
| `wLength` | `60` |

The `AA 25` bytes are the beginning of the 60-byte XU payload, not the USB setup header. TShark's `usb.capdata` and `data.data` fields were empty for these submit records; the payload was recovered from the raw usbmon packet using `tshark -x`.

## Payload layout observed

Offsets below are relative to the 60-byte XU data payload.

| Offset | Observed bytes / type | Interpretation |
|---:|---|---|
| 0–1 | `AA 25` | Repeated frame signature |
| 2 | Varies | Sequence-like byte; increments across adjacent frames and wraps |
| 3–5 | `01 0C 00` | Constant in the inspected records |
| 6–7 | Varies | Unknown sequence-associated bytes |
| 8–13 | `0A 04 84 64 0C 00` | Constant in the inspected records |
| 14–15 | Varies | Unknown |
| 16–19 | `00 00 00 00` | Zero in the inspected records |
| 20–23 | Little-endian IEEE-754 float32 | Changing value, named `f20` in extraction |
| 24–27 | Little-endian IEEE-754 float32 | Changing value, named `f24` in extraction |
| 28–59 | Mostly zero in supplied examples | Not decoded |

The float decoding is directly reproducible from frame 2897. Its 60-byte payload begins:

```text
aa 25 2d 01 0c 00 07 8f 0a 04 84 64 0c 00 a3 e3
00 00 00 00 30 8f ad be fc f2 58 40 ...
```

At payload offset 20, `30 8f ad be` decodes as approximately `-0.33898`. At offset 24, `fc f2 58 40` decodes as approximately `3.38983`. These match the supplied extraction for sequence byte `0x2D`.

## Controlled movement checkpoints

The capture's marked action windows (Unix epoch) were:

| Action | Checkpoint window |
|---|---|
| Pan left | `1795096531.538674`–`1795096544.754616` |
| Pan right toward center | `1795096555.420549`–`1795096565.125545` |
| Tilt up | `1795096573.570435`–`1795096584.160410` |
| Tilt down toward center | `1795096598.838249`–`1795096612.714118` |
| Tilt down | `1795096620.732145`–`1795096628.562060` |
| Tilt up toward center | `1795096639.963057`–`1795096648.385964` |
| Pan right | `1795096655.990899`–`1795096665.456963` |
| Pan left toward center | `1795096673.196788`–`1795096684.837733` |
| Zero button | `1795096697.977657`–`1795096706.528582` |

Examples from the extracted sequence show substantial, smooth variation rather than a fixed configuration block:

| Sequence byte | `f20` | `f24` |
|---:|---:|---:|
| `0x2D` | -0.33898 | 3.38983 |
| `0x49` | -0.28491 | 16.52492 |
| `0x7B` | -0.90701 | 17.53550 |
| `0x9D` | -18.56433 | 0.62930 |
| `0xBE` | 12.41889 | 0.85648 |
| `0xFC` | 24.07023 | 1.66002 |
| `0x11` | -16.01667 | -0.27147 |
| `0x32` | -0.28491 | 16.52492 |
| `0x60` | -0.61017 | -10.98305 |
| `0x83` | 1.17255 | -17.29510 |
| `0x9F` | -25.42599 | 0.00000 |
| `0xF5` | 18.06865 | 1.86917 |

The sequence byte wraps, so sequence order must be interpreted with timestamps and capture order, not by sorting the byte numerically.

## Full capture inventory and inbound traffic

The capture contains **4,686 USB records**. The observed endpoint-address counts are:

| Endpoint address | Record count |
|---|---:|
| `0x81` | 2,916 |
| `0x80` | 1,102 |
| `0x00` | 668 |
| `0x86` | 0 |

The device descriptor identifies endpoint `0x86` as a VideoControl interrupt-IN endpoint (16-byte maximum packet, interval 8). It carried no records in this capture. Therefore this capture provides no interrupt-endpoint status/telemetry evidence. Do not confuse the absent `0x86` traffic with the separate video/control endpoint records.

The relevant Extension Unit request counts were:

| XU selector | Request | Count | Result |
|---|---|---:|---|
| `0x02` | SET_CUR | 207 | 60-byte host-to-camera writes beginning `AA 25` |
| `0x02` | GET_LEN | 207 | Every observed successful length response was `3C 00` (60 bytes) |
| `0x02` | GET_CUR | 0 observed | No inbound selector `0x02` position response was polled in this capture |
| `0x06` | GET_LEN | 69 | Successful length response `3C 00` (60 bytes) |
| `0x06` | GET_CUR | 66 | 66 identical 60-byte responses |

All 66 selector `0x06` GET_CUR response blocks were byte-for-byte identical across the capture, including the pan/tilt movement and zero-button windows. This is consistent with a static configuration/status block in this particular test, not live gimbal pose feedback. It does not rule out other status fields changing in a different operating condition.

A key behavioural observation from the decoded SET_CUR stream is that the two float32 fields at offsets 20 and 24 return to zero when the movement activity stops. Together with the controlled pan-only and tilt-only action windows, this supports interpreting the pair as **movement-related command data**, rather than independent physical-position telemetry. It still does not tell us whether the values are velocity, a relative displacement, a target, or another command representation.

### Consequence for the position-feedback hypothesis

This capture does **not** contain selector `0x02` GET_CUR requests. It therefore cannot test whether the earlier candidate GET_CUR float pair changes during the current controlled movement sequence. The 66 selector `0x06` responses are static and must not be substituted for selector `0x02` data.

The earlier selector `0x02` GET_CUR position hypothesis remains a separate, unresolved lead documented in [the PTZ position read-back investigation](07-2026-11-17-ptz-position-readback.md). The present capture adds evidence for a movement-associated selector `0x02` SET_CUR command stream, not proof of live position read-back.

## What this establishes, and what it does not

**Observed:**

- 207 selector `0x02` SET_CUR requests with a 60-byte `AA 25` payload, plus 207 GET_LEN requests returning 60-byte control length.
- Two valid float32 values at payload offsets 20 and 24.
- Both values change across the capture and return to zero when movement activity stops.
- 66 selector `0x06` GET_CUR responses are identical; no selector `0x02` GET_CUR was observed.
- Endpoint `0x86` carried no traffic in this capture.
- The frame-2897 raw bytes independently reproduce the supplied float extraction.

**Not established:**

- The exact movement-command semantics: velocity, relative displacement, target, or another representation.
- Which field is pan and which is tilt.
- Units, coordinate origin, sign convention, valid range, update rate, or latency.
- Whether the zero-button action resets either field to a defined coordinate.
- Whether these SET_CUR writes are necessary for movement, are a side effect of another OBSBOT Center feature, or are a periodic synchronization stream.

**Critical distinction:** SET_CUR is host-to-device. Even if the values correlate with visible movement, they are not independent camera read-back. The separate GET_CUR experiment remains necessary to establish live position telemetry.

## Extraction commands

Fish shell; set the capture path explicitly:

```fish
set PCAP "$HOME/.local/share/tiny4lite-control/captures/obsbot-capture-1795096511-064087666.pcapng"

# Show selector 0x02 control requests.
tshark -r "$PCAP" \
  -Y 'usb.setup.wValue == 0x0200 && usb.setup.wIndex == 512' \
  -T fields -E header=y -E separator='|' \
  -e frame.number -e frame.time_relative \
  -e usb.urb_type -e usb.setup.bRequest -e usb.data_len

# Inspect one SET_CUR packet; the raw dump exposes the Data Fragment.
tshark -r "$PCAP" -Y 'frame.number == 2897' -x
```

Do not rely on `usb.capdata` or `data.data` being populated for these usbmon submission packets. Preserve the original PCAPNG unchanged.

## Next investigation

1. Keep GET_CUR and SET_CUR evidence separate in all analysis.
2. Obtain a capture that includes host-initiated selector `0x02` GET_CUR polling while the gimbal is idle, moving manually, and moving under AI tracking. The present capture contains no such requests.
3. Compare the candidate GET_CUR float pair with the movement-associated SET_CUR pair at matching timestamps.
4. Only call the GET_CUR values live position telemetry after read-only responses demonstrably follow physical movement and settle when the camera stops.
5. Do not add a new SET_CUR movement implementation or decode the SET_CUR pair as confirmed velocity/position from this capture alone.
