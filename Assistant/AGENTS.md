# Agent operating instructions

This directory is the authoritative project handoff for the OBSBOT Tiny 2 Lite Linux control-app effort. The project currently has a functional hardware-control framework/test bench; UI design and visual polish are deferred.

## Read efficiently

1. Read [INDEX.md](INDEX.md).
2. For app work, read [APP-IMPLEMENTATION-STATUS.md](APP-IMPLEMENTATION-STATUS.md).
3. Read only the relevant topic file(s) under `reference/`.
4. Consult `source/obsbot_tiny_2_lite_linux_research_handoff.md` for original context.
5. The USB tracing workflow is linked from the index and currently lives on the repository's `main` branch under `Assistant/tracing/`.

## Evidence discipline

- Target OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`; do not conflate it with regular Tiny 2 (`3564:fef8`).
- Preserve confidence labels. Do not upgrade confidence without new evidence.
- Never invent protocol bytes, CRC details, enum values, status mappings, capabilities, or acknowledgements.
- Distinguish standard V4L2 controls from vendor UVC Extension Unit controls. The generic USB tracing guide is HID-focused; do not apply HID assumptions to camera XU traffic.
- Keep observed bytes, decoded meaning, implementation assumptions, and verified behaviour separate.
- A successful ioctl is not proof that the camera applied a command. Validate with independent read-back or observable device behaviour.
- Do not claim build, CI, AppImage, or hardware-test success unless actually verified.
- Check licenses before copying code. Do not copy proprietary OBSBOT SDK binaries or headers.

## Hardware safety

- Default to read-only inspection.
- Explain physical/device effects and obtain explicit user approval before state-changing tests not already explicitly requested.
- Clear the gimbal movement area before pan/tilt or sleep/wake tests.
- Start with bounded 1°–2° movement; never begin at maximum pan/tilt.
- Serialize commands. Stop on disconnects, USB errors, unexpected movement, or inconsistent state.
- Avoid aggressive stream start/stop cycles and do not interleave them with XU writes or movement commands.
- Preserve original captures unchanged. Work on copies and record filters/decoding steps.
- Change one setting/action per capture whenever possible. Keep unknown bytes unknown.

## Scope

The current task is to develop and validate the hardware-control framework. Do not spend effort redesigning or polishing the UI unless explicitly requested. Do not add undocumented writes, firmware fallbacks, sleep/wake, or preset mutation without evidence and approval.
