# OBSBOT Tiny 2 Lite Control

Research handoff and a small native Linux test application for the OBSBOT Tiny 2 Lite.

## Application

The first test-bench implementation is in [app/](app/README.md). It detects the Lite by USB ID `3564:fef9`, reads the camera's live V4L2 control metadata, and exposes supported numeric, boolean, and menu controls. Numeric sliders apply live, show the camera-reported default, and include a reset button. Binary 0/1 controls are instant toggles. Pan/tilt nudges move one degree immediately.

The app does not yet send vendor UVC Extension Unit commands, sleep/wake, or preset mutations. Those need further protocol/source verification and explicit hardware testing.

**AppImage build:** the [Build AppImage workflow](.github/workflows/appimage.yml) builds an x86_64 AppImage on pushes to the `test-app-v0.1` branch. Download the `Tiny4LiteControl-x86_64` artifact from the workflow run's Artifacts section.

## Research

**Start here:** [Assistant/INDEX.md](Assistant/INDEX.md)  
**Agent rules:** [AGENTS.md](AGENTS.md) and [Assistant/AGENTS.md](Assistant/AGENTS.md)  
**Complete original research:** [Assistant/source/obsbot_tiny_2_lite_linux_research_handoff.md](Assistant/source/obsbot_tiny_2_lite_linux_research_handoff.md)

The `Assistant/reference/` directory contains focused topic partitions. The full original source is preserved unchanged.
