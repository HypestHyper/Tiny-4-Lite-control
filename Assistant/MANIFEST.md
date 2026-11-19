# Assistant bundle manifest

## Research bundle

- Original OBSBOT Tiny 2 Lite research: `source/obsbot_tiny_2_lite_linux_research_handoff.md`
- Eight topic notes under `reference/`, including the 2026-11-17 PTZ position read-back and firmware/FOV investigations.
- Complete source preserved unchanged; topic files retain original section content/order.

## Tiny 2 Lite V4L2 hardware reference

- [2026-11-20 V4L2 inventory and dependency matrix](reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md): device identity, nodes, USB/media topology, video/audio descriptors, complete reported V4L2 controls and values, invalid driver values, and experimentally confirmed mode dependencies.

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

Windows OBSBOT Center USB captures were collected and parsed on 2026-11-17. Focus/Zoom UVC controls, selector 0x02 command records, and a candidate PTZ position read-back response are documented in `reference/06-2026-11-17-capture-findings.md` , `reference/07-2026-11-17-ptz-position-readback.md`, and `reference/08-firmware-identification-and-fov.md`. Live position polling during AI tracking remains an uncompleted test. Do not describe that read-back capability as verified until a host-initiated GET_CUR test confirms changing values during physical tracking movement.
