# OBSBOT Tiny 2 Lite Control: AI Research Handoff

**Purpose:** Token-efficient navigation of the complete research handoff for a future Linux control/test application.  
**Target:** OBSBOT Tiny 2 Lite (`3564:fef9`) on CachyOS/Arch, Wayland + Hyprland; proposed native UI is GTK4/libadwaita.  
**Scope:** Organize and preserve research. This bundle does not implement the application.

## Start here

Read [`AGENTS.md`](AGENTS.md) for evidence, safety, and scope rules. Then use the topic map below. The complete original handoff is preserved verbatim at [`source/obsbot_tiny_2_lite_linux_research_handoff.md`](source/obsbot_tiny_2_lite_linux_research_handoff.md).

## Topic map

| Need | Read |
|---|---|
| Project purpose, confidence model, repository shortlist, USB identity, V4L2 and UVC XU transport | [`reference/01-device-and-repositories.md`](reference/01-device-and-repositories.md) |
| All feature findings: sleep/wake, PTZ, zoom, AI modes, tracking speed, gestures, presets, FOV, image controls, status block, experimental leads | [`reference/02-feature-inventory.md`](reference/02-feature-inventory.md) |
| Preview, snapshots, recording, audio, Linux/firmware failure modes | [`reference/03-media-and-limitations.md`](reference/03-media-and-limitations.md) |
| Proposed module boundaries, architecture rules, milestone sequence, phased hardware test plan | [`reference/04-architecture-and-hardware-testing.md`](reference/04-architecture-and-hardware-testing.md) |
| License cautions, reusable build prompt, all source URLs, confidence questions, success definition | [`reference/05-licensing-handoff-and-open-questions.md`](reference/05-licensing-handoff-and-open-questions.md) |
| Hardware-verified standard UVC Backlight Compensation SET_CUR/GET_CUR protocol and OBSBOT Center +2.0/+2.3 slider cross-checks | [`reference/06-standard-uvc-backlight-compensation.md`](reference/06-standard-uvc-backlight-compensation.md) |

## Critical facts

- Tiny 2 Lite USB ID: `3564:fef9`; regular Tiny 2: `3564:fef8`.
- Two separate control paths: standard V4L2 and vendor UVC Extension Unit requests via `UVCIOC_CTRL_QUERY`.
- Domatix `obsbot-control` and its `docs/PROTOCOL.md` are the primary Lite-specific reference in the supplied research.
- Never assume other OBSBOT models' features work on the Lite.
- The original document's claims, protocol notes, and repository links are preserved. They have not been silently updated or independently revalidated during this reorganization.

## Retrieval rule for agents

Use the narrowest relevant topic file first. Consult the full source when a detail is absent or context is needed. Do not read every file for every task. Never treat an omitted detail in a summary/index as evidence that the original research did not contain it.


## Device tracing and protocol investigation

A separate evidence-first workflow for observing official-software USB traffic, comparing one controlled change at a time, decoding transfers, validating read paths, and recording findings:
- [tracing/INDEX.md](tracing/INDEX.md): start here; topic map and camera-specific HID/UVC distinction.
- [tracing/AGENTS.md](tracing/AGENTS.md): capture, evidence, safety, privacy, and scope rules.
- [tracing/templates/capture-session.md](tracing/templates/capture-session.md): repeatable record for every trace.
- [tracing/source/reverse-engineering-usb-devices.md](tracing/source/reverse-engineering-usb-devices.md): complete supplied guide.
