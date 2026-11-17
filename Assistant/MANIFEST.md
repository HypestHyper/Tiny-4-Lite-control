# Assistant bundle manifest

## Research bundle

- Original OBSBOT Tiny 2 Lite research: `source/obsbot_tiny_2_lite_linux_research_handoff.md`
- Five topic partitions under `reference/`.
- Complete source preserved unchanged; topic files retain original section content/order.

## Current app handoff

- [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md): current framework implementation, protocol details, gaps, and next investigation.
- App source of truth: `../app/src/main.rs` on branch `test-app-v0.1`.
- App runtime/build notes: `../app/README.md`.
- Current app is a Rust/GTK4 hardware-control test bench. UI redesign is deferred.

## USB tracing workflow

The generic USB tracing guide and its partitioned references currently live on the repository `main` branch:

- [Tracing index](https://github.com/HypestHyper/Tiny-4-Lite-control/tree/main/Assistant/tracing)
- [Tracing source](https://github.com/HypestHyper/Tiny-4-Lite-control/blob/main/Assistant/tracing/source/reverse-engineering-usb-devices.md)
- [Capture template](https://github.com/HypestHyper/Tiny-4-Lite-control/blob/main/Assistant/tracing/templates/capture-session.md)

**Applicability warning:** the generic guide is HID-oriented. Tiny 2 Lite control is V4L2/UVC XU. Reuse the evidence workflow, not HID-specific report assumptions.

## Current evidence state

No new Windows OBSBOT Center USB captures have been collected yet. Existing command/frame examples and protocol findings are documented in the original research and summarized in `APP-IMPLEMENTATION-STATUS.md`. Do not describe the next capture phase as completed.
