# Standard UVC Backlight Compensation: Hardware-Verified Protocol

**Device:** OBSBOT Tiny 2 Lite (`3564:fef9`)  
**Validation host:** CachyOS / Arch Linux  
**Status:** Direct USB SET_CUR and GET_CUR hardware-verified  
**Last verified:** 2026-11-20

## Summary

The Tiny 2 Lite exposes standard UVC Processing Unit Backlight Compensation (BLC) as entity 3, interface 0, selector 1. The control accepts a two-byte unsigned little-endian value. Direct USB control transfers successfully set and read back values 0, 15, 16, and 18 on the tested camera.

This is a standard UVC Processing Unit control. It is **not** an OBSBOT Extension Unit command and must not be sent through the vendor XU command encoder.

## Descriptor and V4L2 evidence

The camera's Video Control descriptors identify:

- Processing Unit ID: `3`
- Processing Unit source: Camera Input Terminal ID `1`
- Processing Unit interface: `0`
- Processing Unit control bitmap advertises Backlight Compensation.
- Standard UVC Processing Unit selector: `0x01` (Backlight Compensation).

Linux exposes the control as:

```text
backlight_compensation
V4L2_CID_BACKLIGHT_COMPENSATION = 0x0098091c
minimum = 0
maximum = 18
step = 1
default = 9
```

The V4L2 setter returned success in testing, but repeated captures did not show the expected BLC SET_CUR request for several value changes. Therefore, a successful `VIDIOC_S_CTRL` return or V4L2 cached readback alone is not sufficient evidence that the camera received the update. Use direct USB GET_CUR when validating the hardware state.

## Exact USB requests

The UVC control request uses endpoint zero.

### SET_CUR

| Field | Value | Meaning |
|---|---:|---|
| `bmRequestType` | `0x21` | Host-to-device, Class, Interface |
| `bRequest` | `0x01` | SET_CUR |
| `wValue` | `0x0100` | Selector 1 in the high byte |
| `wIndex` | `0x0300` | Entity 3 in high byte, interface 0 in low byte |
| `wLength` | `2` | Two-byte payload |
| Data | `value.to_le_bytes()` | Unsigned 16-bit little-endian value |

For example, value 15 is sent as `0f 00`.

### GET_CUR

| Field | Value | Meaning |
|---|---:|---|
| `bmRequestType` | `0xA1` | Device-to-host, Class, Interface |
| `bRequest` | `0x81` | GET_CUR |
| `wValue` | `0x0100` | Selector 1 |
| `wIndex` | `0x0300` | Entity 3, interface 0 |
| `wLength` | `2` | Two-byte response |

Decode the returned bytes as an unsigned 16-bit little-endian integer.

**Important addressing detail:** `wIndex=0x0300` means entity ID 3 and interface number 0. It does not mean interface 3.

## Direct hardware validation

PyUSB was used to issue the requests directly to VID/PID `3564:fef9`.

The first attempt while `uvcvideo` owned Video Control interface `3-3.3:1.0` failed with `USBError: [Errno 16] Resource busy`. After temporarily unbinding only that interface, both transfers succeeded. The interface was rebound to `uvcvideo` immediately after each test.

Observed results:

| Value written | SET result | GET_CUR response | Decoded readback |
|---:|---:|---|---:|
| 15 | 2 bytes transferred | `0f 00` | 15 |
| 18 | successful | `12 00` | 18 |
| 0 | successful | `00 00` | 0 |
| 16 | successful | `10 00` | 16 |

The final test restored the previous value of 16. The camera's Video Control interface was rebound to `uvcvideo`.

These tests establish a working direct SET_CUR/GET_CUR path at the lower endpoint, an interior value, and the upper endpoint of the advertised 0–18 range. They do not constitute exhaustive testing of every intermediate value or every firmware revision.

## Linux/PyUSB reference test

This is a manual diagnostic example, not production application code. It deliberately does not detach or reattach kernel drivers automatically. On the tested host, interface 0 had to be unbound first because PyUSB returned `Resource busy` while `uvcvideo` owned it.

```python
import struct
import usb.core

dev = usb.core.find(idVendor=0x3564, idProduct=0xfef9)
if dev is None:
    raise RuntimeError("OBSBOT Tiny 2 Lite not found")

value = 15

written = dev.ctrl_transfer(
    0x21, 0x01, 0x0100, 0x0300,
    struct.pack("<H", value),
    timeout=2000,
)
print("SET_CUR bytes:", written)

raw = bytes(dev.ctrl_transfer(
    0xA1, 0x81, 0x0100, 0x0300, 2,
    timeout=2000,
))
if len(raw) != 2:
    raise RuntimeError(f"Unexpected GET_CUR length: {len(raw)}")

actual = struct.unpack("<H", raw)[0]
print("GET_CUR:", raw.hex(" "), "value:", actual)
if actual != value:
    raise RuntimeError(f"BLC readback mismatch: requested {value}, got {actual}")
```

On the tested machine the temporary driver operation was:

```bash
echo '3-3.3:1.0' | sudo tee /sys/bus/usb/drivers/uvcvideo/unbind
# Run the direct PyUSB transfer as root.
echo '3-3.3:1.0' | sudo tee /sys/bus/usb/drivers/uvcvideo/bind
```

The physical USB path `3-3.3` and device address are machine/session-specific. Production code must discover the device and its interfaces dynamically. It must also restore the driver binding on every exit path if it ever needs to manage binding itself. Do not unbind the Video Streaming interface (`3-3.3:1.1`) or the audio interfaces for this control.

## OBSBOT Center Exposure Compensation: separate finding

A Windows USB capture of the OBSBOT Center Exposure Compensation slider recorded 18 distinct SET_CUR writes to the same Processing Unit control:

- `bmRequestType=0x21`
- `bRequest=0x01`
- `wValue=0x0100`
- `wIndex=0x0300`
- `wLength=2`
- Payloads `01 00` through `12 00` (decimal 1 through 18).

The 18 captured writes occurred while stepping through the displayed slider labels from -3 to +3. This strongly indicates that OBSBOT Center uses the standard UVC Backlight Compensation control for that slider.

### Direct cross-check: +2.3 = BLC 16

The mapping was subsequently confirmed on the Linux test host using the camera itself:

1. The camera was sent a direct UVC `SET_CUR` request for BLC value `16` (`10 00`).
2. A subsequent UVC `GET_CUR` returned `10 00`, decoded as `16`.
3. OBSBOT Center was opened and its Exposure Compensation slider displayed `+2.3`.

This establishes the specific mapping **OBSBOT Center `+2.3` ↔ UVC BLC integer `16`** on the tested Tiny 2 Lite. The test used a direct USB write and hardware readback, not a V4L2 cached value.

The original Windows capture and this cross-check together support the slider's use of the standard UVC BLC control. The exact mapping of every other fractional label has not yet been individually verified. This also does not establish that the slider is equivalent to a conventional exposure-time control.

Do not infer every remaining label's mapping solely from this one confirmed point or the capture endpoints. The confirmed primitive is BLC integer 0–18 with direct USB readback.

## Implementation guidance

- Keep this in the standard UVC/Processing Unit control layer, separate from the OBSBOT XU transport.
- Represent the hardware value as an integer in the verified range 0–18; query descriptors/runtime capabilities where possible rather than assuming all devices share the same range.
- Use SET_CUR followed by GET_CUR for an explicit readback. Report the device-returned value, not merely the requested value.
- Treat short reads, USB errors, and mismatched readback as failures.
- Do not treat a successful V4L2 ioctl return as proof of hardware state on this device; this investigation observed setter/readback behavior that did not consistently correspond to USB traffic.
- Avoid unbinding the kernel driver in normal application operation unless required by the chosen USB API. Prefer the kernel UVC/V4L2 path when it reliably performs the transfer; the direct PyUSB test required interface unbinding on this host.
- Do not claim the OBSBOT Center fractional slider mapping is implemented or verified until that mapping has its own evidence.

## Confidence

- **High:** UVC entity/interface/selector and exact SET_CUR/GET_CUR request fields.
- **High:** direct hardware write and matching two-byte GET_CUR readback for 0, 15, 16, and 18 on the tested Tiny 2 Lite.
- **High:** Windows OBSBOT Center capture sent BLC SET_CUR payloads 1 through 18 while the slider was stepped.
- **High:** OBSBOT Center `+2.3` corresponds to BLC integer `16`, confirmed by direct SET_CUR, matching GET_CUR readback, and the app's displayed slider value on the tested camera.
- **Not yet fully verified:** exact label-to-integer mapping for every other slider position and whether the same behavior applies across firmware versions.
