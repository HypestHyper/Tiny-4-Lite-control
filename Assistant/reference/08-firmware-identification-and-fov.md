# 2026-11-17 firmware identity and FOV/zoom findings

## Firmware identification

**Target:** OBSBOT Tiny 2 Lite, USB VID:PID `3564:fef9`.

The USB device descriptor exposes `bcdDevice=5.10` on the development unit. This is a USB device-release value, not a verified user-visible firmware version. Do not display it as “Firmware 5.10”.

The USB product identity and XU selector `0x08` model/stream-name response can identify the device as `OBSBOT Tiny 2 Lite StreamCamera`, but neither provides a confirmed firmware revision.

The official OBSBOT Tiny 2 Lite download page lists firmware `v6.2.8.1` and says the update was published 2025-11-26. This establishes an official published version, not the version installed on the user's camera. OBSBOT Center remains the supported source for reading/updating the installed firmware version.

**App behaviour:** show USB VID:PID, manufacturer, product, bus/address, and `bcdDevice` under device identity. Label `bcdDevice` explicitly as the USB device-release field. Show the XU `0x08` model string as a separate identity read. Do not infer or fabricate the installed firmware version.

## FOV versus zoom

The 2026-11-17 large USB capture documents five standard UVC Camera Terminal Zoom Absolute writes (selector `0x0B`) and nearby XU selector `0x02` float records at payload offset 16. These are zoom-associated observations. The capture findings do not document a controlled FOV change, so they cannot prove that FOV is represented by those zoom values.

The Lite-specific status mapping identifies two separate status fields:

- Status byte `0x04`: crop-level code in the related protocol research (`0x05` wide, `0x0F` normal). Treat this as a crop/zoom-related raw field until the mapping is independently verified on the user's current firmware.
- Status byte `0x11`: FOV (`1` Wide, `2` Normal). Lite testing found Narrow to be a no-op, with read-back remaining `2`.

Therefore the app must not merge FOV with `zoom_absolute` or the XU zoom-associated float. The Diagnostics page displays both raw fields separately. The existing generic V4L2 `zoom_absolute` control remains the user-facing zoom control. Expose a working FOV setter only after a controlled capture and read-back test on the target firmware.

## Capture analysis improvements

The Capture report now includes:

- URB IDs and URB status alongside request/response fields, to make submission/completion pairing auditable.
- A compact summary grouped by transfer type and `bRequest/wValue/wIndex`.
- An explicit warning that URB IDs can be reused and must be paired in capture order.
- The raw decoded transfer rows remain included after the summary.

The original capture is preserved. The summary does not attempt to assign UI actions to packet timestamps when checkpoint metadata is missing, nor does it label unknown function records.

## Evidence boundaries

**Observed:** USB descriptor `bcdDevice=5.10`; model/stream name via selector `0x08`; zoom writes and zoom-associated XU float records in the large 2026-11-17 capture; FOV status byte mapping in the Lite-specific protocol research.

**Not established by the large capture:** an FOV change sequence, a direct firmware-version query, or a reliable relationship that makes the XU zoom-associated float a replacement for the standard V4L2 zoom control.

## Sources

- OBSBOT Tiny 2 Lite official download page: https://www.obsbot.com/download/obsbot-tiny-2-lite/
- Lite-specific protocol and hardware findings: https://github.com/Domatix/obsbot-control/blob/main/docs/PROTOCOL.md
- Project capture analysis: [06-2026-11-17-capture-findings.md](06-2026-11-17-capture-findings.md)
