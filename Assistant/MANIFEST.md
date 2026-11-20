# Assistant bundle manifest

## Research bundle

- Original OBSBOT Tiny 2 Lite research: `source/obsbot_tiny_2_lite_linux_research_handoff.md`
- Fifteen topic notes under `reference/`, including the 2026-11-17 PTZ position read-back and firmware/FOV investigations.
- Original research source retained and updated where later controlled hardware tests supersede its earlier candidate inventories; dated evidence notes preserve test scope and confidence.

## Tiny 2 Lite V4L2 hardware reference

- [2026-11-20 V4L2 inventory and dependency matrix](reference/13-2026-11-20-tiny-2-lite-v4l2-inventory-and-dependencies.md): device identity, nodes, USB/media topology, video/audio descriptors, complete reported V4L2 controls and values, invalid driver values, and experimentally confirmed mode dependencies.

- [2026-11-20 AI mode and gesture read-back](reference/15-2026-11-20-ai-mode-and-gesture-readback.md): verified mode tuples, selector inventory, Target Selection state byte, gesture read-back limits, and test evidence.

## Current app handoff

- [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md): current framework implementation, protocol details, gaps, and next investigation.
- App source of truth: `../app/src/` on branch `claude/upbeat-rubin-gnae3f` (`backend/device.rs` for protocol code, `backend/registry.rs` for the settings, `ui/` for the windows, `cli.rs` for the command line). On `test-app-v0.1` the same protocol code is in `app/src/main.rs`.
- App runtime/build notes: `../app/README.md`; control and protocol notes from the test bench: `../app/docs/controls-and-protocol.md`.
- Current app is a Rust/GTK4 hardware-control test bench. Functional AI mode and gesture state handling is updated; visual redesign is specified in the root [CLAUDE_HANDOFF.md](../CLAUDE_HANDOFF.md).

## USB tracing workflow

The generic USB tracing guide and its partitioned references:

- [Tracing index](tracing/INDEX.md)
- [Tracing source](tracing/source/reverse-engineering-usb-devices.md)
- [Capture template](tracing/templates/capture-session.md)

Also on this branch: [standard UVC Backlight Compensation](reference/06-standard-uvc-backlight-compensation.md), the hardware-verified request format behind Exposure compensation.

**Applicability warning:** the generic guide is HID-oriented. Tiny 2 Lite control is V4L2/UVC XU. Reuse the evidence workflow, not HID-specific report assumptions.

## Current evidence state

Windows OBSBOT Center USB captures were collected and parsed on 2026-11-17. Focus/Zoom UVC controls, selector 0x02 command records, and a candidate PTZ position read-back response are documented in `reference/06-2026-11-17-capture-findings.md` , `reference/07-2026-11-17-ptz-position-readback.md`, and `reference/08-firmware-identification-and-fov.md`. Live position polling during AI tracking remains an uncompleted test. Do not describe that read-back capability as verified until a host-initiated GET_CUR test confirms changing values during physical tracking movement.
