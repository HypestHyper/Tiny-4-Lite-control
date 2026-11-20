# Pop filter protocol investigation

**Device:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`  
**Control path:** UVC Extension Unit 2, selector `0x02`  
**Status:** State mapping is strongly supported by three controlled captures and an experimental fixed-payload prototype exists in the app. The payload builder has an unresolved byte-offset discrepancy against the written capture summary; do not treat it as a validated replay or general command encoder.

## 1. Executive summary

Three controlled OBSBOT Center captures each contain an On → Off → On sequence. Every correlated write is a 60-byte host-to-device class/interface control transfer:

- `bmRequestType = 0x21`
- `bRequest = 0x01` (SET_CUR)
- `wValue = 0x0200` (selector 0x02)
- `wIndex = 0x0200` (Extension Unit 2, VideoControl interface 0)
- `wLength = 60`

The 60-byte payload starts with `AA 25`.

Observed state-correlated fields:

| Payload offset (zero-based) | Pop filter On | Pop filter Off | Confidence |
|---:|---|---|---|
| 14–15 | `EF 4F` | `BE 8F` | Repeated in all three captures |
| 17 | `01` | `00` | Repeated in all three captures |

These are observed correlations with the labelled UI actions. The meaning of bytes 14–15 has not been decoded; do not call them a checksum or state marker as a fact.

Byte 2 behaves like a sequence/counter: it increments by one across adjacent writes in each capture. Bytes 6–7 repeat for the same byte-2 value across captures:

| Byte 2 | Bytes 6–7 |
|---:|---|
| `0D` | `E9 6F` |
| `0E` | `19 60` |
| `0F` | `48 A5` |
| `10` | `79 00` |

The algorithm and purpose of bytes 6–7 are unknown. The initial counter value and reset/session behaviour are also unknown.

Bytes 20–59 are not stable across captures. Capture 1 contains nonzero values including `E7 01`, `2B`, and `19 00 02 00 3D`; capture 2 contains `50 C8 D7 B9 0F 22 01` in part of this region; capture 3 has zeros throughout this region. Their meaning is unknown. Do not assume zero-filling is always valid.

GET_CUR reads of selector `0x02` did not provide a reliable Pop filter read-back in the first investigation. No camera-side read-back mapping has been established.

## 2. Capture provenance

All captures were made on Linux using the project's Capture tab and usbmon3. Device identity in the reports: USB ID `3564:fef9`, bus 3. Device address was 16 in capture 1 and 17 in captures 2 and 3. The address is runtime-assigned and must not be treated as a stable identity.

Raw captures are preserved under:

```text
~/.local/share/tiny4lite-control/captures/
```

| Capture | Raw PCAPNG | Action windows | SET_CUR frames | State sequence |
|---|---|---|---|---|
| 1 | `obsbot-capture-1795042200-144801911.pcapng` | 1795042209.607360–1795042214.205275; 1795042218.533286–1795042223.498211; 1795042228.133231–1795042233.068127 | 37997, 84755, 128051 | On, Off, On |
| 2 | `obsbot-capture-1795043160-409054044.pcapng` | 1795043170.903572–1795043181.381521; 1795043186.331439–1795043194.956386; 1795043199.847371–1795043205.547310 | 41791, 126043, 175409 | On, Off, On |
| 3 | `obsbot-capture-1795043383-742124080.pcapng` | 1795043400.187941–1795043406.772821; 1795043411.272832–1795043415.225768; 1795043419.432833–1795043423.482732 | 32271, 79397, 118447 | On, Off, On |

The action windows are the authoritative correlation between the user's UI action and USB packet timestamps. Do not use nearby GET_CUR requests as if they were the SET_CUR write.

## 3. Captured payloads

The following are the 60-byte data payloads, not the seven-byte USB setup header.

### Capture 1

Frame 37997, On, counter `0D`:

```text
aa 25 0d 00 0c 00 e9 6f 0a 02 82 c0 04 00 ef 4f
06 01 00 00 e7 01 00 00 2b 00 00 00 00 00 00 00
2b 00 00 00 00 00 00 00 00 00 00 00 19 00 02 00
3d 00 00 00 00 00 00 00 00 00 00 00
```

Frame 84755, Off, counter `0E`:

```text
aa 25 0e 00 0c 00 19 60 0a 02 82 c0 04 00 be 8f
06 00 00 00 e7 01 00 00 2b 00 00 00 00 00 00 00
2b 00 00 00 00 00 00 00 00 00 00 00 19 00 02 00
3d 00 00 00 00 00 00 00 00 00 00 00
```

Frame 128051, On, counter `0F`:

```text
aa 25 0f 00 0c 00 48 a5 0a 02 82 c0 04 00 ef 4f
06 01 00 00 e7 01 00 00 2b 00 00 00 00 00 00 00
2b 00 00 00 00 00 00 00 00 00 00 00 19 00 02 00
3d 00 00 00 00 00 00 00 00 00 00 00
```

### Capture 2

Frame 41791, On, counter `0D`:

```text
aa 25 0d 00 0c 00 e9 6f 0a 02 82 c0 04 00 ef 4f
06 01 00 00 50 c8 d7 b9 0f 22 01 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00
```

Frame 126043, Off, counter `0E`:

```text
aa 25 0e 00 0c 00 19 60 0a 02 82 c0 04 00 be 8f
06 00 00 00 50 c8 d7 b9 0f 22 01 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00
```

Frame 175409, On, counter `0F`:

```text
aa 25 0f 00 0c 00 48 a5 0a 02 82 c0 04 00 ef 4f
06 01 00 00 50 c8 d7 b9 0f 22 01 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00
```

### Capture 3

Frame 32271, On, counter `0E`:

```text
aa 25 0e 00 0c 00 19 60 0a 02 82 c0 04 00 ef 4f
06 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00
```

Frame 79397, Off, counter `0F`:

```text
aa 25 0f 00 0c 00 48 a5 0a 02 82 c0 04 00 be 8f
06 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00
```

Frame 118447, On, counter `10`:

```text
aa 25 10 00 0c 00 79 00 0a 02 82 c0 04 00 ef 4f
06 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00
```

## 4. What is known and unknown

### Supported by repeated controlled evidence

- The UI action correlates with a 60-byte SET_CUR to XU unit 2 / selector 0x02.
- The command signature is `AA 25`.
- Byte 2 increments by one across adjacent writes in the observed sequences.
- Bytes 6–7 repeat for matching byte-2 values across independent captures: `0D → E9 6F`, `0E → 19 60`, `0F → 48 A5`.
- Bytes 14–15 correlate with On/Off as `EF 4F` / `BE 8F`.
- Byte 17 correlates with On/Off as `01` / `00`.
- Bytes 3–5 and 8–13 are constant in all nine supplied payloads.

### Not established

- The algorithm or semantic meaning of bytes 6–7.
- Whether bytes 14–15 are a checksum, a second state encoding, or another field.
- The purpose and validity rules for bytes 18–59.
- Whether trailing fields are optional, session-derived, or dependent on other application/device state.
- The sequence counter's seed, lifetime, wraparound, and reset behaviour.
- A reliable GET_CUR read-back for Pop filter.
- Whether a command with reconstructed or zero-filled unknown fields is accepted by the camera.

Do not infer that a 60-byte buffer with only the known state bytes changed is safe. The third capture's zero tail does not prove that zero-filling is universally valid.

## 5. Capture method used

The Capture tab in the GTK app records continuous usbmon traffic and produces both a raw PCAPNG and a text report. Use this action-window method because the user cannot operate OBSBOT Center and click a checkpoint in the capture app at the exact same time:

1. Connect the Tiny 2 Lite and start a capture in the app's **Capture** tab.
2. Add a baseline/checkpoint before changing the setting if useful.
3. Enter a clear action label, such as `Pop filter ON`, and click **Begin action**.
4. Switch to OBSBOT Center and make exactly one Pop filter change.
5. Switch back to the capture app and click **End action**.
6. Repeat with a separate action window for each change. Do not try to click a checkpoint while making the setting change.
7. Finish the capture. Preserve both generated files unchanged.

Each action window records monotonic elapsed time and Unix epoch start/end timestamps. Use `frame.time_epoch` to correlate USB packets to the window. The report's **PACKETS INSIDE ACTION WINDOWS** section supplies candidate frame numbers, but inspect the actual OUT/SET_CUR payload before assigning meaning.

The capture app currently identifies/filter reports by USB device address. USB device addresses can change after a camera restart or re-enumeration. Record bus and VID:PID, and do not assume an old address remains valid. The capture implementation should eventually follow the device by bus + VID:PID across re-enumeration rather than a fixed address.

## 6. TShark commands used

Set the PCAP path in fish:

```fish
set PCAP "$HOME/.local/share/tiny4lite-control/captures/obsbot-capture-1795043383-742124080.pcapng"
```

List generic USB control-transfer rows, including frame number, epoch time, device address, endpoint, data length, and captured data:

```fish
tshark -r "$PCAP" -Y 'usb.transfer_type == 0x02' -T fields -e frame.number -e frame.time_epoch -e usb.device_address -e usb.endpoint_address -e usb.data_len -e usb.capdata
```

Extract complete packet hex for the three SET_CUR frames from capture 3:

```fish
tshark -r "$PCAP" -Y 'frame.number == 32271 || frame.number == 79397 || frame.number == 118447' -x
```

For the other captures, substitute the corresponding frame numbers from Section 2:

- Capture 1: `37997`, `84755`, `128051`.
- Capture 2: `41791`, `126043`, `175409`.

Important TShark quirks observed in this environment:

- `usb.setup.bmRequestType` was not a valid field name in the installed TShark build.
- Filtering with `usb.setup.wValue == 0x0200` returned no rows for these traces.
- `usb.capdata` was empty for the relevant OUT writes.
- Use the report's frame numbers and `-x` to inspect raw packets. In each packet, the USB Control section begins with the seven-byte setup header; the 60-byte data payload follows it.
- Do not mistake the URB submit/completion pair for two separate setting changes.

## 7. Captured payloads used by the experimental app toggle

A fresh OBSBOT Center session capture (`pop filter fresh session`, Unix start 1795044907) reproduced byte-2 values `0x0f` through `0x12` and the same bytes 6–7 as the prior session. The written extraction summary records the session-stable sequence `50 C8 D7 B9 0F 22 01` at payload offsets 25–31 and says offsets 32–59 are zero. However, the current app builder places `0x01` at offset 32 after copying its 32-byte prefix. Until the complete raw 60-byte packets are re-extracted and compared by offset, this section must not describe the app's payload as an exact replay. The fresh capture remains the basis for the experimental prototype, but it does not establish validity after a sequence advance, Center restart, or camera reconnect.

The app stores only the last submitted UI state. It does not claim read-back or confirmed device acceptance. Treat the implementation as experimental and validate it on the physical camera.

## 8. Next experiment required

The next useful experiment is not another broad ten-toggle survey. First
resolve the one-byte source/document discrepancy using the existing raw
fresh-session capture:

1. Re-extract the complete 60-byte ON and OFF SET_CUR data payloads from
   the fresh-session PCAPNG.
2. Compare all 60 offsets against the arrays in
   `app/src/main.rs::pop_filter_payload()`.
3. Confirm whether the trailing `0x01` belongs at offset 31 or 32, and
   whether the remaining tail is zero.
4. Correct the implementation and add a byte-for-byte unit test against
   the confirmed 60-byte vectors.
5. Only then run one app-driven ON and OFF test, checking the visible
   effect and preserving the capture.

If either app action fails after the payload is reconciled, capture the
failed interaction and investigate sequence/session state. Do not
reconstruct unknown bytes or run an arbitrary number of extra toggles.
