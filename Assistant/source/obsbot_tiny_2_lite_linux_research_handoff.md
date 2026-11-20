# OBSBOT Tiny 2 Lite Linux Control --- Research & Test-App Handoff

**Purpose:** Give a fresh ChatGPT conversation enough verified research,
repository links, protocol notes, feature inventory, implementation
guidance, and safety constraints to build a small Linux test application
for the **OBSBOT Tiny 2 Lite**.

**Target environment:** CachyOS / Arch Linux, Wayland + Hyprland, GTK4
desktop.\
**Target device:** OBSBOT Tiny 2 Lite.\
**Primary goal:** A native Linux application that exposes as many camera
controls as can be safely implemented and verified, without relying on
OBSBOT's proprietary desktop application or SDK.

------------------------------------------------------------------------

## 1. Executive summary

There is enough public reverse-engineering work to build a useful Linux
control app.

The camera exposes two distinct control paths:

1.  **Standard UVC/V4L2 controls** for conventional camera settings and
    movement.
2.  **Vendor-specific UVC Extension Unit (XU) controls** for
    OBSBOT-specific features such as sleep/wake, AI modes, tracking
    speed, gestures, and presets.

This is conceptually similar to the user's existing K609
keyboard-control project, but the transport/protocol is different:

-   K609 project: direct USB HID feature reports (`hidraw`,
    `HIDIOCSFEATURE` / `HIDIOCGFEATURE`).
-   OBSBOT: V4L2 controls plus UVC Extension Unit requests through
    `UVCIOC_CTRL_QUERY`.

Reuse the K609 project's architectural approach (UI separated from
hardware protocol, explicit device layer, testable command encoding),
**not its HID report protocol**.

**Best starting point:** `Domatix/obsbot-control`, especially its
`docs/PROTOCOL.md`. It specifically documents Tiny 2 Lite behavior and
includes recent hardware validation.\
**Best broad command reference:** `lxman/obsbot-mcp`.\
**Useful alternate implementation:** `OpenFoxes/Tiny4Linux`.

Do not assume that features documented for Tiny 2, Meet 2, Meet 4K, or
the original OBSBOT Tiny are supported by Tiny 2 Lite. Treat each
feature as one of: - **Lite-confirmed:** explicitly tested on Tiny 2
Lite. - **Partially verified:** implementation or status exists, but
Lite behavior is incomplete or uncertain. - **Other-model reference:**
useful protocol history, not proof of Lite support.

------------------------------------------------------------------------

## 2. Repository index

### 2.1 Primary sources

  -------------------------------------------------------------------------------------------------------------------------
  Repository               URL                                         Language / UI     Why it matters    Tiny 2 Lite
                                                                                                           confidence
  ------------------------ ------------------------------------------- ----------------- ----------------- ----------------
  Domatix/obsbot-control   https://github.com/Domatix/obsbot-control   Rust,             Most useful       High; primary
                                                                       GTK4/libadwaita   end-to-end Linux  reference
                                                                                         desktop           
                                                                                         reference;        
                                                                                         detailed protocol 
                                                                                         document; Lite is 
                                                                                         a first-class     
                                                                                         development       
                                                                                         target            

  lxman/obsbot-mcp         https://github.com/lxman/obsbot-mcp         TypeScript +      Broad command     Mixed; primarily
                                                                       Linux native      inventory,        Tiny 2, Lite
                                                                       helper            encoder logic,    support partial
                                                                                         V4L2/XU           
                                                                                         implementation,   
                                                                                         diagnostic        
                                                                                         operations        

  OpenFoxes/Tiny4Linux     https://github.com/OpenFoxes/Tiny4Linux     Rust GUI + CLI    Simple direct XU  Some behavior
                                                                                         implementation,   independently
                                                                                         especially        validated on
                                                                                         sleep/wake,       Lite
                                                                                         tracking, speed,  
                                                                                         presets           

  cgevans/tiny2            https://github.com/cgevans/tiny2            Rust, Iced GUI +  Alternate V4L2    Primarily Tiny
                                                                       CLI/OSC           and               2; use as
                                                                                         tagged-command    corroboration,
                                                                                         implementation;   not Lite proof
                                                                                         useful status and 
                                                                                         command reference 
  -------------------------------------------------------------------------------------------------------------------------

### 2.2 Secondary / historical sources

  ---------------------------------------------------------------------------------------------------------------------------------------
  Repository                        URL                                                  Language / UI  Why it matters   Limitation
  --------------------------------- ---------------------------------------------------- -------------- ---------------- ----------------
  aaronsb/obsbot-camera-control     https://github.com/aaronsb/obsbot-camera-control     Qt6            Desktop UX       Uses proprietary
                                                                                                        ideas: PTZ,      OBSBOT SDK
                                                                                                        tracking, image  (`libdev.so`);
                                                                                                        controls,        Tiny 2 Lite
                                                                                                        preview, tray,   support is
                                                                                                        reconnect,       partial
                                                                                                        virtual camera   

  samliddicott/meet4k               https://github.com/samliddicott/meet4k               Rust CLI       Historical       Meet 4K, not
                                                                                                        tagged XU        evidence of Tiny
                                                                                                        commands for     2 Lite support
                                                                                                        Meet 4K; useful  
                                                                                                        protocol         
                                                                                                        archaeology      

  taxfromdk/obsbot_tiny_reversing   https://github.com/taxfromdk/obsbot_tiny_reversing   Python         Early USB        Original Tiny,
                                                                                                        capture/replay   different
                                                                                                        work for         VID/PID and
                                                                                                        original OBSBOT  likely different
                                                                                                        Tiny             protocol

  ananthb/libobsbot                 https://github.com/ananthb/libobsbot                 Rust library   Cross-platform   Primary focus
                                                                                         with C ABI     SDK architecture Meet 2 /
                                                                                                        and additional   pre-release; do
                                                                                                        controls         not assume Lite
                                                                                                                         support; check
                                                                                                                         current license
                                                                                                                         before reuse
  ---------------------------------------------------------------------------------------------------------------------------------------

### 2.3 Most important files to inspect in the repositories

**Domatix/obsbot-control** - `docs/PROTOCOL.md` --- highest-priority
source; Tiny 2 Lite protocol and hardware validation. - README ---
application overview, build requirements, preview and snapshot
architecture. - Search source for V4L2 control setup, XU ioctl wrapper,
command encoder/decoder, preset handling, status parsing, and device
discovery.

**lxman/obsbot-mcp** - `src/codec/commands.ts` --- command encoding,
including sleep/wake. - `src/mcp/tools.ts` --- operation handlers such
as wake and sleep. - Linux native helper --- V4L2 and
`UVCIOC_CTRL_QUERY` implementation. - README --- tool inventory and
Linux-specific limitations.

**OpenFoxes/Tiny4Linux** - Rust source implementing XU control
transfers. - Sleep/wake frame definitions. - Tracking mode/speed and
preset operations.

**cgevans/tiny2** - `src/lib.rs` --- useful V4L2/XU protocol
reference. - CLI and GUI code for command usage and status parsing.

> Repository paths can change. If a path is not present, inspect the
> repository tree rather than assuming the code was deleted.

------------------------------------------------------------------------

## 3. Device identity and Linux interfaces

### 3.1 USB identity

The Tiny 2 Lite USB VID:PID documented by Domatix is:

-   Vendor ID: `3564`
-   Product ID: `fef9`
-   Combined: `3564:fef9`

The regular Tiny 2 is listed as `3564:fef8`. Do not accidentally match
the regular Tiny 2 when detecting the Lite.

### 3.2 Linux device interfaces

The camera should be discovered through its V4L2 nodes and USB parent
device. A single physical camera may expose multiple `/dev/video*`
nodes, for example separate video capture and metadata interfaces.

Useful inspection commands:

``` bash
lsusb -d 3564:
lsusb -t
v4l2-ctl --list-devices
v4l2-ctl --all -d /dev/videoX
v4l2-ctl --list-ctrls-menus -d /dev/videoX
udevadm info --query=all --name=/dev/videoX
```

Install/use the usual tools on Arch-based systems:

``` bash
sudo pacman -S v4l-utils usbutils
```

Do not hard-code `/dev/video0`. Resolve the correct node using
sysfs/udev metadata and the USB parent identity. Prefer stable device
identity and capability inspection.

### 3.3 Standard V4L2 control transport

Use the Linux V4L2 API for standard controls. The `v4l2-ctl` command is
useful for initial manual verification, but the app should use a proper
library or ioctl wrapper.

Relevant V4L2 controls documented by the research include:

**User-class controls** - Brightness - Contrast - Saturation - Hue -
White-balance automatic mode - Red balance - Blue balance - Gain -
Power-line frequency - White-balance temperature - Sharpness - Backlight
compensation

**Camera-class controls** - Auto exposure - Exposure time - Pan
absolute - Tilt absolute - Focus absolute - Continuous autofocus - Zoom
absolute - Continuous zoom - Pan speed - Tilt speed

Important details: - Auto exposure menu includes Auto, Manual, and
Aperture Priority. - Exposure time is inactive while Auto or Aperture
Priority is selected. - Pan range: `-468000` to `+468000`. - Tilt range:
`-324000` to `+324000`. - Pan/tilt step: `3600`. - These correspond to
approximately ±130° pan and ±90° tilt, in 1° increments, with V4L2
values scaled by 3600.

Always query the live device for control ranges, defaults, flags, and
menu values. Treat the values above as research notes, not a replacement
for runtime discovery.

### 3.4 Vendor UVC Extension Unit

Domatix documents the following Tiny 2 Lite XU identity:

-   Unit ID: `2`
-   GUID: `9A1E7291-6843-4683-6D92-39BC7906EE49`

Key selectors: - `0x02`: framed command messages. - `0x06`: tagged
commands and status; `GET_CUR` returns a 60-byte status block. - `0x07`:
legacy preset pose records. - `0x08`: camera/stream name. - `0x09`:
four-state enum. - `0x0A`: three-state enum. - `0x0B`: two-state enum. -
`0x0C`: preset-list selector. - `0x0D`: preset-entry cursor. - `0x0E`:
constant read-only block. - `0x03`--`0x05`: additional framed channels,
not fully used. - `0x0F` onward: not available through the tested kernel
interface.

UVC requests: - `UVC_SET_CUR`: request `0x01` - `UVC_GET_LEN`: request
`0x85` - `UVC_GET_CUR`: request `0x81`

The Linux transport is `UVCIOC_CTRL_QUERY`, using the XU unit, selector,
request, and data buffer.

The selector `0x02` command format is framed: - 12-byte header. -
Sequence information. - Header CRC16/USB. - Function group and
command. - Optional payload block with its own CRC16. - Zero padding to
the transfer length.

Do not improvise this framing. Reuse or carefully port a known encoder,
then test its output against the source implementation and hardware.
Confirm exact CRC variant, byte order, sequence behavior, and buffer
lengths from the protocol source.

------------------------------------------------------------------------

## 4. Tiny 2 Lite feature inventory

### 4.1 Power and sleep/wake

**Status:** Lite-confirmed.

Domatix documents `CAM_SET_DEV_STATUS`, opcode `0xA0C2`: - Wake/run
payload: `00 00 00 00` - Sleep payload: `01 00 00 00`

The MCP encoder uses the same payload convention (`encodeSetRunStatus`).

Expected physical behavior: - Sleep stows the gimbal face-down,
approximately 84° pitch. - Wake returns the gimbal toward level.

Tiny4Linux implements a different fixed-frame encoding on unit 2,
selector 2, using 36-byte frames padded to a 60-byte transfer buffer:

Sleep frame (36 bytes):

``` text
AA 25 42 00 0C 00 EA 63 0A 02 C2 A0 04 00 BF FB
01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00
```

Wake frame (36 bytes):

``` text
AA 25 A5 00 0C 00 5F EF 0A 02 C2 A0 04 00 BE 07
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00
```

These are different implementations/encodings of the operation. Do not
mix the fixed frames with the other framed-command encoder.

Tiny4Linux does not verify a device acknowledgement; successful ioctl
return does not prove that the camera changed state. Prefer status
read-back if reliable.

**Safety:** Sleep/wake physically moves the gimbal. Warn the user before
the first test and keep the area around the camera clear.

### 4.2 Gimbal movement / PTZ

**Status:** Standard V4L2 controls; Tiny 2 Lite control ranges
documented.

Use: - `pan_absolute` - `tilt_absolute` - `pan_speed` - `tilt_speed`

Research ranges: - Pan: `-468000` to `+468000`, step `3600`. - Tilt:
`-324000` to `+324000`, step `3600`.

A first test app should expose: - Recenter/home (only if a reliable
method is verified). - Small relative pan/tilt buttons. - Absolute
pan/tilt sliders. - Stop/cancel behavior if supported by the API.

Do not assume a position query gives live physical feedback. The MCP
Linux implementation documents gimbal position as cached/last-commanded
rather than live. Display that distinction clearly.

### 4.3 Zoom

**Status:** Standard V4L2 zoom controls available; gesture zoom
separately confirmed.

Use V4L2: - `zoom_absolute` - `zoom_continuous`

Gesture-based zoom is applied internally by the camera and does not
update V4L2 `zoom_absolute`.

### 4.4 AI tracking / framing modes

**Current validated scope (2026-11-20):** Eight AI tracking modes
were individually set and read back on the Tiny 2 Lite. The older
candidate inventory below is superseded by the controlled Linux test.

| Mode | Selector 0x06 tuple |
|---|---|
| No Tracking | `00 00` |
| Normal Tracking | `02 00` |
| Upper Body | `02 01` |
| Close-up | `02 02` |
| Headless | `02 03` |
| Lower Body | `02 04` |
| Hand | `03 00` |
| Group | `01 00` |

The tuple is read from selector 0x06 status offsets 0x18 and 0x1C.
See [reference/15-2026-11-20-ai-mode-and-gesture-readback.md](../reference/15-2026-11-20-ai-mode-and-gesture-readback.md)
for the controlled test results and evidence limits.

### 4.5 Tracking speed

**Status:** Lite-confirmed, firmware-sensitive.

-   Standard
-   Sport

On firmware 6.2.5.x, Sport value is `0x02`. Older firmware may use
`0x21`.

Read device firmware/version where possible and verify the accepted
value before sending. Do not assume a single enum mapping works across
all firmware.

### 4.6 Gestures

**Status:** Gesture controls discovered and verified on Tiny 2 Lite.

Available controls: - Target selection - Zoom - Dynamic zoom - Dynamic
zoom direction

The selector `0x02` framed messages are used. Function groups
documented:

  Gesture                  Function group
  ------------------------ ---------------------
  Target selection         `0a 04 c4 30 01 00`
  Zoom                     `0a 04 44 31 01 00`
  Dynamic zoom             `0a 04 44 33 01 00`
  Dynamic zoom direction   `0a 04 c4 33 01 00`

Gesture on/off payload bytes: - Off: `e6 3f 00 00 00 00` - On:
`27 ff 01 00 00 00`

**Naming trap:** The L-pose gesture is called **Zoom**, not Dynamic
Zoom. Disabling Zoom is the action that stops the L-shaped gesture.
Disabling Dynamic Zoom appears to do nothing for that gesture.

Do not present the gesture controls as a single generic "gesture mode";
expose each function separately and document the naming.

### 4.7 Presets

**Status:** Tiny 2 Lite preset operations verified in Domatix protocol
notes dated 2026-10-20.

Operations/opcodes: - Create: `0x3944` - Delete: `0x3984` - Recall:
`0x39C4` - Read pose: `0x3A44` - Rename: `0x3A84` - Read name:
`0x3B04` - List: `0x3B44` - Update: `0x3E04`

Preset command family is derived as `(index << 6) | 0x04`.

Important behavior: - Delete may ACK even if a slot does not exist.
Verify the preset list after deletion. - Preset names use padded
Base64. - Preset list selector is `0x0C`. - Preset entry cursor is
`0x0D`. - The cursor is consuming; reset it by echo-writing the selector
`0x0C` block before a new enumeration. - Recall payload requires four
`1.0f32` floats. - Pose uses raw V4L2 pan/tilt/zoom integer values
converted to `f32` (scale 1.0).

**Documentation discrepancy:** Older README text may say only preset
recall is supported. The newer protocol document reports
create/update/delete/rename/read-back verified on Tiny 2 Lite. Prefer
the newer protocol notes and source implementation, and validate each
operation on hardware.

### 4.8 Field of view (FOV)

**Status:** Wide and Normal confirmed; Narrow is a no-op on Tiny 2 Lite
firmware 5.10.

-   Wide: 86°
-   Normal: 78°
-   Narrow: 65° (does not change state on the tested Lite firmware;
    read-back remains Normal)

Do not expose Narrow as working on all Lite firmware without retesting.

### 4.9 Exposure, focus, white balance and image settings

**Status:** Standard V4L2 controls available; individual behavior should
be queried and verified.

Potential UI: - Auto/manual exposure. - Exposure time. - Autofocus
toggle. - Manual focus. - Auto/manual white balance. - White-balance
temperature. - Brightness, contrast, saturation, sharpness, gain, hue,
red/blue balance, backlight compensation, power-line frequency.

**Important quirk:** Tiny 2 XU exposure-mode labels are swapped relative
to V4L2. Domatix removed the XU exposure-mode GUI control and uses
standard V4L2 `auto_exposure` as the sole UI control.

Face-AE GUI control was retired in Domatix because it meters correctly
only in the XU auto-exposure path and silently no-ops when the V4L2
exposure route is used. Treat Face AE as experimental, not a reliable
basic feature.

### 4.10 Status block and read-back

Selector `0x06` `GET_CUR` returns a 60-byte status block.

Documented offsets (confidence varies):

  -----------------------------------------------------------------------
  Offset                  Meaning                 Confidence / notes
  ----------------------- ----------------------- -----------------------
  `0x02`                  Sleep state             Tiny4Linux: 0 awake, 1
                                                  sleep

  `0x06`                  HDR                     Listed in protocol
                                                  notes

  `0x07`                  Face AE /               Uncertain
                          gesture-related state   

  `0x09`                  Device status           Field map partly based
                                                  on regular Tiny 2

  `0x0A`                  Auto-sleep seconds      Needs further
                                                  validation

  `0x11`                  FOV                     Lite-confirmed: Wide=1,
                                                  Normal=2; Narrow
                                                  remains 2

  `0x13`                  Mirror                  Partial

  `0x18` / `0x1C`         AI mode / framing       Mapping requires source
                                                  verification

  `0x1F`                  FPS                     Needs validation

  `0x21`                  LED brightness          Needs validation

  `0x24`                  Tracking speed          Lite-confirmed
  -----------------------------------------------------------------------

Do not label unknown bytes as facts. Build a diagnostic page that
displays raw status bytes in hex and decoded fields with a confidence
indicator.

------------------------------------------------------------------------

## 5. Features found in other projects but not yet confirmed for Tiny 2 Lite

These are useful research leads, not promises:

-   LED brightness.
-   Mirror mode.
-   Auto-sleep timeout.
-   HDR control details.
-   Face AE.
-   Additional AI framing/status states.
-   Video FPS settings.
-   Audio capture/recording and microphone controls.
-   Background replacement, blur, noise reduction, and visual effects
    (historical Meet 4K work).
-   Voice-command toggles and microphone pickup distance (proprietary
    SDK app).
-   Virtual camera via `v4l2loopback`.
-   GPU filters and advanced preview processing.

Keep these behind an **Experimental** or **Diagnostics** section until
tested on the Lite.

------------------------------------------------------------------------

## 6. Preview, snapshots, recording and audio

The Domatix app uses GStreamer for live preview and snapshot capture,
with GTK4 paintable integration. The Qt app demonstrates a broader
desktop experience including preview and virtual-camera concepts.

The MCP project lists operations for: - Preview. - Snapshot. - Video
recording. - Audio capture/recording.

However, these capabilities are not all necessarily camera firmware
controls. Some are host-side media pipeline features. Keep the
architecture separate: - Camera control service. - Video capture/preview
pipeline. - Snapshot/recording service. - Optional audio capture
service.

Do not repeatedly open and close streaming sessions during early
hardware testing.

------------------------------------------------------------------------

## 7. Known Linux and firmware limitations

1.  **USB streaming fragility:** Tiny 2 Lite firmware 5.10 may
    disconnect with `-71 EPROTO` if clients rapidly start/stop streaming
    while XU writes or pan commands are interleaved.
2.  Use one long-lived stream during preview tests.
3.  Avoid repeated stream negotiation cycles.
4.  After unplug/replug, wait before starting the stream.
5.  A single preset save/pan/recall/capture round trip has been reported
    as okay.
6.  MCP Linux implementation reports cached/last-commanded gimbal
    position rather than live feedback.
7.  MCP Linux gimbal speed operation is unavailable.
8.  Face AE is not reliable through the standard V4L2 exposure route.
9.  Narrow FOV was a no-op on tested Tiny 2 Lite firmware 5.10.
10. Tracking speed enum may vary with firmware.
11. Some status fields are inherited from regular Tiny 2 mapping and
    remain unverified on Lite.

**Do not run aggressive integration tests.** Some scripts physically
move the gimbal. Review their behavior before running.

------------------------------------------------------------------------

## 8. Suggested test application architecture

A native Rust + GTK4/libadwaita app is a strong fit, especially because
the user wants a polished GTK desktop application.

Suggested modules:

``` text
src/
  main.rs
  app.rs
  ui/
    window.rs
    camera_page.rs
    ptz_page.rs
    tracking_page.rs
    presets_page.rs
    image_page.rs
    diagnostics_page.rs
  device/
    discovery.rs
    identity.rs
    session.rs
  v4l2/
    device.rs
    controls.rs
    metadata.rs
  obsbot/
    xu.rs
    framing.rs
    crc.rs
    commands.rs
    status.rs
    presets.rs
    gestures.rs
  media/
    preview.rs
    snapshot.rs
    recording.rs
  settings/
    config.rs
```

Architecture rules: - UI must not encode protocol bytes. - V4L2
operations must be separate from vendor XU operations. - Keep command
encoders pure and unit-testable. - Add protocol tests using known byte
vectors from the repositories. - Keep raw XU access behind a narrow,
auditable API. - Discover the camera dynamically; never hard-code
`/dev/video0`. - Read control metadata and firmware/status before
enabling model-specific features. - Keep experimental features visibly
separate from verified controls. - Log raw requests/responses in a
diagnostics panel, with secrets absent (none expected). - Use a single
serialized device command queue to avoid overlapping XU operations. - Do
not block the GTK main thread; use async tasks or worker threads and
marshal results back to the UI. - Persist user settings locally, but do
not persist unsafe or stale device state as if it were current. -
Include a "Stop / cancel movement" mechanism where the underlying API
supports it.

### Suggested first milestone

Build a **test bench**, not a full-featured polished app:

1.  Detect Tiny 2 Lite by USB identity and associated V4L2 nodes.
2.  Show device model, firmware if discoverable, node paths and
    available controls.
3.  Read V4L2 control values/ranges without changing anything.
4.  Read the 60-byte XU status block and show raw hex.
5.  Add read-only status decoding only for confirmed fields.
6.  Add one carefully bounded pan/tilt test.
7.  Add zoom and basic image controls.
8.  Add sleep/wake only after explicit UI confirmation because it moves
    the gimbal.
9.  Add AI modes, tracking speed, gestures and presets after validating
    command encoders.

Do not begin with preview, recording, or a full UI. Prove device
discovery and safe control first.

------------------------------------------------------------------------

## 9. Hardware test plan and safety

### Phase A: Read-only inspection

Run:

``` bash
lsusb -d 3564:
v4l2-ctl --list-devices
v4l2-ctl --all -d /dev/videoX
v4l2-ctl --list-ctrls-menus -d /dev/videoX
```

Record: - Exact USB VID:PID. - All associated video nodes. - Driver and
card name. - Firmware version if available. - Full V4L2 control list and
ranges. - Whether selector `0x06` status read works.

### Phase B: Low-risk controls

Test one at a time: - Brightness/contrast/saturation. - White balance. -
Exposure mode. - Zoom.

Record original value, requested value, ioctl result, read-back value,
and visible result. Restore original values after each test.

### Phase C: Movement

-   Warn the user first.
-   Clear the camera's movement area.
-   Use small 1° or 2° changes initially.
-   Never send max-range pan/tilt as a first test.
-   Do not run multiple movement commands concurrently.
-   Record requested and read-back/cached values accurately.

### Phase D: Vendor commands

Test in this order: 1. Status read. 2. Tracking speed. 3. AI mode. 4.
One gesture toggle. 5. Sleep/wake (explicit confirmation; physical
movement). 6. Preset create/list/recall/update/delete, verifying after
every mutation.

Do not run a whole test suite blindly. Stop if the camera disconnects or
starts returning USB errors.

### Phase E: Preview

-   Start one stream.
-   Keep it open.
-   Avoid cycling the stream while issuing XU commands.
-   Test snapshot capture while the stream is active if supported.
-   Monitor kernel logs for USB errors.

Useful monitoring:

``` bash
journalctl -kf
```

------------------------------------------------------------------------

## 10. Implementation and licensing cautions

-   Read each repository's current LICENSE before copying code.
-   Domatix is GPL-3.0-or-later according to the research; copying its
    implementation may impose GPL obligations on a distributed
    derivative.
-   Tiny4Linux and other repositories need their current license checked
    at the point of reuse.
-   `obsbot-mcp` is MIT according to its repository metadata; verify the
    exact files and bundled native helper license.
-   `cgevans/tiny2` is EUPL-1.2 according to the research.
-   `libobsbot` was reported as GPL-3.0-only in repository search;
    verify current metadata.
-   Do not copy proprietary OBSBOT SDK binaries or headers from the Qt
    app.
-   Prefer clean-room protocol implementation using public documentation
    and independently written code if license compatibility is
    uncertain.
-   Keep attribution and license notices for any reused source.

------------------------------------------------------------------------

## 11. Build prompt for a new ChatGPT conversation

Use the following as the initial prompt in a new chat, together with
this document:

> I want to build a native Linux control application for my OBSBOT Tiny
> 2 Lite on CachyOS/Arch Linux. Use this research document as the
> project handoff.
>
> Start by inspecting the primary repository, especially
> `https://github.com/Domatix/obsbot-control` and its
> `docs/PROTOCOL.md`, then compare the relevant implementation in
> `https://github.com/lxman/obsbot-mcp` and
> `https://github.com/OpenFoxes/Tiny4Linux`.
>
> The app should eventually be a polished GTK4/libadwaita desktop
> application, preferably Rust, but the first milestone is a safe
> hardware test bench. Do not start by building the entire app.
>
> First produce a concise implementation plan and identify the exact
> protocol facts that need to be checked against source. Then implement
> device discovery and read-only V4L2/XU diagnostics. Do not send
> movement, sleep/wake, preset mutation, or other state-changing
> commands without asking me first. Do not run repository integration
> tests until their hardware side effects are understood.
>
> Keep V4L2 and vendor UVC Extension Unit operations separate. Do not
> confuse the OBSBOT XU protocol with HID feature reports from my
> unrelated K609 keyboard project. Use the K609 project only as an
> architectural reference:
> `https://github.com/HypestHyper/k609-keyboard-control`.
>
> Target the Tiny 2 Lite specifically (`3564:fef9`), not just the Tiny 2
> (`3564:fef8`). Clearly label each feature as Lite-confirmed, partially
> verified, or experimental. Never invent command bytes or enum values.
> Build unit tests around known protocol vectors and keep the UI off the
> GTK main-thread blocking path.
>
> Before any physical test, explain what the command will do and ask for
> approval. Sleep/wake physically moves the gimbal. Use cautious,
> bounded movement tests and avoid rapid stream start/stop cycles
> because the Lite may disconnect with `-71 EPROTO`.

------------------------------------------------------------------------

## 12. Source links

### Primary

-   Domatix OBSBOT Control: https://github.com/Domatix/obsbot-control
-   Domatix protocol documentation:
    https://github.com/Domatix/obsbot-control/blob/main/docs/PROTOCOL.md
-   OBSBOT MCP: https://github.com/lxman/obsbot-mcp
-   Tiny4Linux: https://github.com/OpenFoxes/Tiny4Linux
-   cgevans/tiny2: https://github.com/cgevans/tiny2

### Secondary

-   Qt OBSBOT Camera Control:
    https://github.com/aaronsb/obsbot-camera-control
-   meet4k: https://github.com/samliddicott/meet4k
-   Original Tiny reversing:
    https://github.com/taxfromdk/obsbot_tiny_reversing
-   libobsbot: https://github.com/ananthb/libobsbot

### User's related project

-   K609 Keyboard Control:
    https://github.com/HypestHyper/k609-keyboard-control

------------------------------------------------------------------------

## 13. Research confidence and open questions

The strongest Tiny 2 Lite-specific evidence comes from Domatix's
protocol documentation and its hardware validation notes. Other
repositories provide useful implementation patterns, but their model
support differs.

Before treating the app as feature-complete, answer these questions on
the actual device:

1.  What firmware version is installed, and which tracking-speed enum
    mapping does it use?
2.  Can all ten AI modes be set and read back?
3.  Can the 60-byte status block be read reliably over repeated
    sessions?
4.  What is the exact status mapping for sleep, AI mode, mirror, FPS,
    LED and auto-sleep?
5.  Can LED brightness and mirror be controlled on the Lite?
6.  Can Face AE be made reliable without switching to the fragile XU
    exposure path?
7.  Are any additional XU selectors accessible through a newer
    kernel/driver?
8.  Does the camera expose audio interfaces, and are those controls
    separate from the video XU?
9.  Does preview remain stable while XU commands are sent on the user's
    firmware?
10. Which preset operations remain reliable after unplug/replug and
    application restart?

**Definition of success for the first test app:** reliably identify the
Tiny 2 Lite, read its standard controls and status without changing
device state, and provide a trustworthy diagnostics report. Everything
else can be added incrementally.
