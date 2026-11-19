# Mirror image protocol capture

**Device:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Capture:** Linux, usbmon3, device address 17  
**Status:** Three controlled OBSBOT Center transitions support the ON/OFF payload mapping. The app implementation is experimental; physical validation from the app is pending.

## 1. Observed USB request

All three writes are host-to-device class/interface control transfers with the same setup fields:

| Field | Observed value |
|---|---:|
| `bmRequestType` | `0x21` |
| `bRequest` | `0x01` (SET_CUR) |
| `wValue` | `0x0600` (selector `0x06`) |
| `wIndex` | `0x0200` (Extension Unit 2, VideoControl interface 0) |
| `wLength` | `60` |

The selector is **0x06**, not 0x02. The USB setup bytes in the supplied packet are `21 01 00 06 00 02 3c 00`; the little-endian `wValue` is therefore `0x0600`.

## 2. Controlled action windows

Capture started at Unix time `1795045128`.

| Action | State change | Action window (Unix epoch) | SET_CUR frame |
|---|---|---|---:|
| 1 | Mirror ON | `1795045136.649245`–`1795045142.183912` | 37555 |
| 2 | Mirror OFF | `1795045146.196878`–`1795045149.683854` | 73445 |
| 3 | Mirror ON | `1795045156.254866`–`1795045160.364781` | 124577 |

Raw capture on the user's machine:

```text
~/.local/share/tiny4lite-control/captures/obsbot-capture-1795045128-532261069.pcapng
```

## 3. Payload comparison

The USB Control section in each packet contains the seven-byte setup header followed by the 60-byte data payload.

| Frame | UI state | Payload bytes 0–2 | Payload bytes 3–59 |
|---:|---|---|---|
| 37555 | ON | `14 01 01` | All zero |
| 73445 | OFF | `14 01 00` | All zero |
| 124577 | ON | `14 01 01` | All zero |

The two ON writes are byte-for-byte identical. The only observed payload difference between ON and OFF is byte 2:

- ON: `14 01 01`, followed by 57 zero bytes.
- OFF: `14 01 00`, followed by 57 zero bytes.

This supports the observed state mapping. It does not establish the semantic meaning of the first two payload bytes beyond their repeated appearance in these commands.

## 4. App implementation and validation boundary

The experimental app switch uses the exact 60-byte payload shape above and sends it through Extension Unit 2 / selector `0x06`. The last successfully submitted state is persisted locally.

No reliable camera-side GET_CUR read-back has been established for this setting. A successful ioctl only confirms host-driver request completion. The camera's visible image must be checked after toggling ON and OFF from the app before the feature is considered hardware-validated.

## 5. Capture extraction

Fish command used to inspect the three supplied frames:

```fish
set PCAP "$HOME/.local/share/tiny4lite-control/captures/obsbot-capture-1795045128-532261069.pcapng"
tshark -r "$PCAP" -Y 'frame.number == 37555 || frame.number == 73445 || frame.number == 124577' -x
```

Keep the original PCAPNG unchanged. The three action windows, rather than nearby traffic alone, establish the correlation between each write and the OBSBOT Center UI action.
