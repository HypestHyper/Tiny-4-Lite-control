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
