# Device tracing and protocol investigation

**Purpose:** A repeatable, evidence-first workflow for discovering what the official software sends to a USB device, documenting the protocol, and validating observations without guessing.

## Start here

Read [AGENTS.md](AGENTS.md) before collecting or interpreting traffic. The complete supplied guide is preserved at [source/reverse-engineering-usb-devices.md](source/reverse-engineering-usb-devices.md).

## Topic map

| Task | Read |
|---|---|
| Applicability, limits, seven safety ground rules | [reference/01-applicability-and-safety.md](reference/01-applicability-and-safety.md) |
| VID:PID, bus/device identity, HID descriptors, narrow udev access | [reference/02-device-identification-and-access.md](reference/02-device-identification-and-access.md) |
| Wine/Bottles, usbmon, dumpcap, one-change captures, filtering | [reference/03-official-software-capture.md](reference/03-official-software-capture.md) |
| tshark fields, report decoding, byte diffs, timing, read-back checks | [reference/04-decoding-and-validation.md](reference/04-decoding-and-validation.md) |
| Allowlisted writes, read-modify-write, backups, fake-device tests, UI and packaging | [reference/05-protocol-ui-and-packaging.md](reference/05-protocol-ui-and-packaging.md) |
| Known pitfalls and AI collaboration protocol | [reference/06-pitfalls-and-ai-collaboration.md](reference/06-pitfalls-and-ai-collaboration.md) |
| Repeatable per-capture notes | [templates/capture-session.md](templates/capture-session.md) |

## Applying it to OBSBOT Tiny 2 Lite

The guide's overall method applies, but its HID-specific mechanics do **not** directly describe the camera's protocol. The camera research in the parent [Assistant index](../INDEX.md) identifies V4L2 and vendor UVC Extension Unit controls. Use this workflow's evidence discipline while deriving camera-specific capture filters and decoding from the camera's actual descriptors and captured traffic.

Do not assume:
- camera controls are HID feature reports;
- HID interface 01 is the camera control interface;
- the guide's example udev rule grants the correct camera permissions;
- the example HID report type/ID interpretation fully describes UVC XU requests.

## Collection loop

1. Identify the currently connected device and relevant interface.
2. Record a baseline/start-up trace.
3. Change exactly one setting/action in official software.
4. Capture and preserve the raw trace.
5. Filter a working copy to the target device/control traffic.
6. Compare bytes, request metadata, ordering, repetition, and timing.
7. Repeat across controlled values to establish field meaning and bounds.
8. Validate read-only behaviour against a real device state change.
9. Document confidence, unknown bytes, capture provenance, and any remaining questions.
10. Propose writes separately; never enable them without explicit review and approval.

Use [the capture template](templates/capture-session.md) for every trace.
