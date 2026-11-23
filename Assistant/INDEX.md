# OBSBOT Tiny 2 Lite Control: project handoff

**Target:** OBSBOT Tiny 2 Lite (`3564:fef9`).  
**Current stage:** Rust/GTK4 application with a full window, a slim window, a pop-out preview and a command line, built on a catalogue of settings (`app/src/backend/registry.rs`). Not yet validated on real hardware in this form.

## Start here

1. Read [AGENTS.md](AGENTS.md) for project, evidence, and hardware-safety rules.
2. Read [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md) for the current app state, implemented control paths, protocol details, known gaps, and next work.
3. Use the topic map below for deeper research. The original handoff remains preserved at [source/obsbot_tiny_2_lite_linux_research_handoff.md](source/obsbot_tiny_2_lite_linux_research_handoff.md).

## Topic map

| Need | Read |
|---|---|
| 2026-11-20 device-specific V4L2 control inventory, full observed ranges/current values, USB/media identity, video formats, and experimentally confirmed exposure/focus/white-balance dependencies | [reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md](reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md) |
| Current implementation and protocol status | [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md) |
| Exposure compensation: verified OBSBOT Center label mapping, standard UVC BLC requests, implementation and read-back behavior | [reference/14-2026-11-20-exposure-compensation-implementation.md](reference/14-2026-11-20-exposure-compensation-implementation.md) |
| 2026-11-20 AI mode tuple verification, selector inventory, Target Selection status read-back, and gesture read-back limits | [reference/15-2026-11-20-ai-mode-and-gesture-readback.md](reference/15-2026-11-20-ai-mode-and-gesture-readback.md) |
| 2026-11-24 SDK probe: AI auto zoom, pan reverse, limited-zone tracking (limits, reset, enable), preset list and start-up position reads, captured from OBSBOT's SDK on a Tiny 2 Lite | [reference/16-2026-11-24-sdk-probe-captures.md](reference/16-2026-11-24-sdk-probe-captures.md) |
| Project purpose, confidence model, repository shortlist, USB identity, V4L2 and UVC XU transport | [reference/01-device-and-repositories.md](reference/01-device-and-repositories.md) |
| Feature findings: sleep/wake, PTZ, zoom, AI modes, tracking speed, gestures, presets, FOV, image controls, status block | [reference/02-feature-inventory.md](reference/02-feature-inventory.md) |
| 2026-11-17 Windows OBSBOT Center capture, Focus/Zoom UVC writes, XU selector 0x02 records | [reference/06-2026-11-17-capture-findings.md](reference/06-2026-11-17-capture-findings.md) |
| Current Tiny 2 Lite gesture, FOV, LED, power, and protocol feature evidence | [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md) and [reference/02-feature-inventory.md](reference/02-feature-inventory.md) |
| Pop filter protocol investigation, all three capture payload sets, action-window workflow, TShark commands, and next test | [reference/09-pop-filter-protocol-investigation.md](reference/09-pop-filter-protocol-investigation.md) |
| Mirror image protocol capture, exact ON/OFF payloads, frame numbers, action windows, and validation status | [reference/10-mirror-image-protocol.md](reference/10-mirror-image-protocol.md) |
| PTZ position read-back hypothesis, selector 0x02 GET_CUR evidence, Windows KS topology attempts, and next AI-tracking polling test | [reference/07-2026-11-17-ptz-position-readback.md](reference/07-2026-11-17-ptz-position-readback.md) |
| Confirmed Windows DirectShow physical pan/tilt feedback, standard Camera Terminal selector 0x0D descriptor mapping, and the newer live gimbal-controller query/startup-state implementation | [reference/12-2026-11-19-directshow-pan-tilt-position-readback.md](reference/12-2026-11-19-directshow-pan-tilt-position-readback.md) |
| 2026-11-19 selector 0x02 SET_CUR stream, AA 25 payload layout, float32 fields at offsets 20/24, movement correlation, and evidence limits | [reference/11-2026-11-19-selector-02-motion-payloads.md](reference/11-2026-11-19-selector-02-motion-payloads.md) |
| Firmware identity and distinction between FOV, crop level, and zoom | [reference/08-firmware-identification-and-fov.md](reference/08-firmware-identification-and-fov.md) |
| Preview, snapshots, recording, audio, Linux/firmware limitations | [reference/03-media-and-limitations.md](reference/03-media-and-limitations.md) |
| Module boundaries, architecture, milestones, hardware test plan | [reference/04-architecture-and-hardware-testing.md](reference/04-architecture-and-hardware-testing.md) |
| Licensing, source URLs, open questions, success criteria | [reference/05-licensing-handoff-and-open-questions.md](reference/05-licensing-handoff-and-open-questions.md) |
| USB tracing workflow and capture template | [tracing/INDEX.md](tracing/INDEX.md) |
| Standard UVC Backlight Compensation SET_CUR/GET_CUR requests and OBSBOT Center +2.0 / +2.3 label cross-checks (the same control as Exposure compensation) | [reference/06-standard-uvc-backlight-compensation.md](reference/06-standard-uvc-backlight-compensation.md) |

## Critical facts

- Tiny 2 Lite USB ID: `3564:fef9`; regular Tiny 2: `3564:fef8`.
- The 2026-11-20 device-specific V4L2 inventory and controlled dependency matrix are recorded in [reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md](reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md). This is a hardware/driver baseline; the app must still query live V4L2 metadata.
- Camera controls use standard V4L2 and vendor UVC Extension Unit requests. Do not conflate these with USB HID feature reports.
- Domatix `obsbot-control` and its `docs/PROTOCOL.md` are the primary Lite-specific reference in the supplied research.
- Windows OBSBOT Center USB traces were captured and parsed on 2026-11-17. A separate 2026-11-18 Tiny 2 Lite capture established the Dynamic Zoom Direction ON/OFF payload inversion and showed a rolling frame sequence across Direction Flip and a separate Dynamic Zoom disable command. See [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md) and [reference/02-feature-inventory.md](reference/02-feature-inventory.md) for the latest implementation and evidence.
- The Diagnostics tab exposes read-only 60-byte XU reads for selectors 0x02 and 0x06, plus the model string from selector 0x08 and USB descriptor identity. The 2026-11-19 capture recorded 207 selector 0x02 SET_CUR writes whose float32 fields at offsets 20/24 vary with movement and return to zero when stopped; it contained no selector 0x02 GET_CUR requests. Selector 0x06 GET_CUR returned 66 identical blocks, and endpoint 0x86 had no traffic. These SET_CUR values are movement-associated command data, not independent position read-back. Their exact command semantics remain unknown.
- A Windows DirectShow test confirmed live physical Pan/Tilt values. The active Linux app path now queries gimbal-controller command `0x0043` (receiver 3), decoding pitch/yaw as signed hundredths of a degree; zoom is queried with `0x6804`. The Controls page polls every 250 ms. The user's saved capture also contains the camera-native startup write sequence `0x3EC4` + `0x3E44`; the app now exposes Set current position as initial state and verifies via `0x3884`. Linux hardware validation remains pending. See [reference/12-2026-11-19-directshow-pan-tilt-position-readback.md](reference/12-2026-11-19-directshow-pan-tilt-position-readback.md).
- Selector 0x02 GET_CUR responses contain a strong candidate pan/tilt float32 pair at payload offset 20. Live read-back during AI tracking remains unverified; see [reference/07-2026-11-17-ptz-position-readback.md](reference/07-2026-11-17-ptz-position-readback.md).
- Eight AI modes are confirmed by selector 0x06 mode/submode read-back; Target Selection state is confirmed at selector 0x06 byte 0x01. The three other gesture toggles have no confirmed status read-back. See [reference/15-2026-11-20-ai-mode-and-gesture-readback.md](reference/15-2026-11-20-ai-mode-and-gesture-readback.md).
- Do not assume features from other OBSBOT models work on the Lite.

## Retrieval rule

Read the implementation status first for app work, then only the relevant topic file. Consult the full source when detail is missing. Do not load every file by default. The app source of truth is [app/src/main.rs](../app/src/main.rs) on branch `test-app-v0.1`.
