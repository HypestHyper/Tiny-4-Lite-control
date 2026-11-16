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

