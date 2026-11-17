# OBSBOT Tiny 2 Lite Control: project handoff

**Target:** OBSBOT Tiny 2 Lite (`3564:fef9`).  
**Current stage:** Rust/GTK4 hardware-control framework and test bench. UI design comes later.

## Start here

1. Read [AGENTS.md](AGENTS.md) for project, evidence, and hardware-safety rules.
2. Read [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md) for the current app state, implemented control paths, protocol details, known gaps, and next work.
3. Use the topic map below for deeper research. The original handoff remains preserved at [source/obsbot_tiny_2_lite_linux_research_handoff.md](source/obsbot_tiny_2_lite_linux_research_handoff.md).

## Topic map

| Need | Read |
|---|---|
| Current implementation and protocol status | [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md) |
| Project purpose, confidence model, repository shortlist, USB identity, V4L2 and UVC XU transport | [reference/01-device-and-repositories.md](reference/01-device-and-repositories.md) |
| Feature findings: sleep/wake, PTZ, zoom, AI modes, tracking speed, gestures, presets, FOV, image controls, status block | [reference/02-feature-inventory.md](reference/02-feature-inventory.md) |
| Preview, snapshots, recording, audio, Linux/firmware limitations | [reference/03-media-and-limitations.md](reference/03-media-and-limitations.md) |
| Module boundaries, architecture, milestones, hardware test plan | [reference/04-architecture-and-hardware-testing.md](reference/04-architecture-and-hardware-testing.md) |
| Licensing, source URLs, open questions, success criteria | [reference/05-licensing-handoff-and-open-questions.md](reference/05-licensing-handoff-and-open-questions.md) |
| USB tracing workflow and capture template | [Tracing documentation on main](https://github.com/HypestHyper/Tiny-4-Lite-control/tree/main/Assistant/tracing) |

## Critical facts

- Tiny 2 Lite USB ID: `3564:fef9`; regular Tiny 2: `3564:fef8`.
- Camera controls use standard V4L2 and vendor UVC Extension Unit requests. Do not conflate these with USB HID feature reports.
- Domatix `obsbot-control` and its `docs/PROTOCOL.md` are the primary Lite-specific reference in the supplied research.
- No new Windows OBSBOT Center USB traces have been collected yet. The next research phase is controlled capture, one setting/action at a time.
- Do not assume features from other OBSBOT models work on the Lite.

## Retrieval rule

Read the implementation status first for app work, then only the relevant topic file. Consult the full source when detail is missing. Do not load every file by default. The app source of truth is [app/src/main.rs](../app/src/main.rs) on branch `test-app-v0.1`.
