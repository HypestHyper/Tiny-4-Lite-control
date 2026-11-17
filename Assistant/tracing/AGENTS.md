# Tracing workflow: agent instructions

This folder describes how to collect evidence about device communication and turn it into a verified protocol record. It is a research workflow, not permission to write to the device or build the application.

## Mandatory reading

1. Read this file.
2. Read [INDEX.md](INDEX.md).
3. Read only the relevant topic in [reference/](reference/).
4. Consult the complete uploaded guide in [source/reverse-engineering-usb-devices.md](source/reverse-engineering-usb-devices.md) when exact commands or context are needed.
5. For OBSBOT Tiny 2 Lite-specific facts, cross-check [the camera research index](../INDEX.md) and its UVC/V4L2 references.

## Critical protocol distinction

The supplied guide is primarily a **USB HID feature-report** workflow. The OBSBOT Tiny 2 Lite research identifies a different control surface: standard V4L2 controls and vendor UVC Extension Unit requests. These are not interchangeable.

- Do not assume HID raw interfaces, HID report descriptors, HID feature report IDs, or the example HID udev interface number describe the camera's control interface.
- Do not treat the guide's HID-specific `tshark` filter or report decoding as a complete UVC XU decoder.
- Reuse the general evidence method: identify the device/interface, capture official-software activity, make one controlled change, compare transfers, record timing, validate reads, and preserve exact known-good transactions.
- For camera tracing, first establish the actual USB interfaces, UVC entity/unit identifiers, selectors, request types, lengths, and payloads from descriptors and captures. Never fill unknown fields by guessing.
- Keep video streaming traffic separate from control traffic unless a specific investigation requires it. Avoid capturing unrelated input interfaces or private media.

## Evidence rules

- One setting/action per capture whenever possible. Record exact before/after values and the time of the action.
- Preserve original captures unchanged. Work on filtered copies and record the filter used.
- Record VID:PID, bus/device address, interface, endpoint or control-transfer context, OS/software versions, and device connection state.
- Device numbers can change after reconnect. Re-identify before every capture.
- Label every finding as confirmed, partially verified, inferred, or experimental. A packet observed once is not automatically a decoded command.
- Never discard unknown bytes. Preserve them exactly and mark their meaning unknown.
- Do not infer a field's meaning from one sample. Use controlled value sweeps and repeated captures.
- Keep raw evidence, decoded notes, and implementation-ready protocol definitions separate.

## Safety and privacy

- Default to read-only inspection. Do not send guessed or reconstructed writes.
- Do not touch/log keystrokes or unrelated user input. Capture only the relevant device and traffic.
- Never change a write buffer except for fields whose meaning and bounds have been proven.
- Before a new write is added to an allowlist, show the evidence and obtain explicit user approval.
- Avoid root-running the application. Prefer narrowly scoped udev access for the exact proven interface.
- For physical camera actions such as pan, tilt, sleep/wake, or presets, follow the camera-specific safety rules in [../AGENTS.md](../AGENTS.md). Clear movement space and get approval before state-changing tests.
- Stop on disconnects, USB errors, unexpected movement, or unexplained device state changes.

## Scope boundary

This workflow organizes evidence collection and protocol understanding. Do not build the app, add live device writes, or perform hardware tests unless the user explicitly requests that work.
