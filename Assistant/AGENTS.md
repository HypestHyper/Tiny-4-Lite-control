# Agent operating instructions

This directory is the authoritative project-research handoff for the OBSBOT Tiny 2 Lite Linux control-app effort. It is research and implementation guidance, **not an instruction to build the app**.

## Read efficiently

1. Read `INDEX.md` first.
2. Read only the topic file(s) relevant to the task.
3. Open `source/obsbot_tiny_2_lite_linux_research_handoff.md` when exact original wording, full context, or formatting matters. It is the complete unmodified source document.
4. Follow cross-repository links in the relevant reference file when implementation detail is needed.

## Evidence discipline

- Target the **OBSBOT Tiny 2 Lite**, USB VID:PID `3564:fef9`. Do not conflate it with Tiny 2 `3564:fef8`.
- Preserve the source's confidence labels: Lite-confirmed, partially verified, other-model reference, or experimental. Do not upgrade confidence without new evidence.
- Never invent protocol bytes, CRC details, enum values, status mappings, device capabilities, or acknowledgements.
- Treat the documents as supplied research, not as independently re-verified current repository state. Check upstream source and current hardware before relying on implementation details.
- Distinguish standard V4L2 controls from vendor UVC Extension Unit controls. Do not substitute HID feature reports or the K609 protocol.
- Separate camera firmware controls from host-side media pipeline features.
- When evidence conflicts, report the discrepancy and cite the relevant source/repository. Do not silently reconcile it.
- Check current repository licenses before copying code. Do not copy proprietary OBSBOT SDK binaries or headers.

## Hardware safety

- Default to read-only inspection.
- Do not send movement, sleep/wake, preset mutation, or other state-changing commands without explaining the physical/device effect and obtaining explicit user approval.
- Sleep/wake physically moves the gimbal. Clear its movement area first.
- Start movement tests with bounded 1°–2° changes; never begin with maximum pan/tilt.
- Serialize device commands. Stop on disconnects or USB errors.
- Avoid rapid stream start/stop cycles and do not interleave aggressive streaming changes with XU writes or pan commands.
- Do not run repository integration tests until their hardware side effects are understood.

## Scope boundary

The handoff describes a future test application and its eventual architecture. Do not build the app, create application source code, or perform hardware tests unless the user explicitly requests that separate work.
