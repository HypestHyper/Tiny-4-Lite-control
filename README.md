# OBSBOT Tiny 2 Lite Control

Research handoff and a Rust/GTK 4 Linux control application for the OBSBOT Tiny 2 Lite: a full window, a slim quick-controls window, a pop-out preview, and a command line for every setting.

**Start here:** [Assistant/INDEX.md](Assistant/INDEX.md)  
**Agent rules:** [AGENTS.md](AGENTS.md) and [Assistant/AGENTS.md](Assistant/AGENTS.md)  
**Complete original research:** [Assistant/source/obsbot_tiny_2_lite_linux_research_handoff.md](Assistant/source/obsbot_tiny_2_lite_linux_research_handoff.md)

The `Assistant/reference/` directory contains focused topic partitions for efficient retrieval. The full original source is preserved unchanged. The application is in [`app/`](app/README.md). Start with its README for the windows, the command line, how settings are defined, and what has not yet been verified on a real camera. [`app/docs/controls-and-protocol.md`](app/docs/controls-and-protocol.md) keeps the control and protocol notes from the earlier test bench.

The 2026-11-20 Linux tests verify all eight AI tracking-mode tuples and Target Selection state read-back. The updated protocol evidence and UI state contract are in [Assistant/reference/15-2026-11-20-ai-mode-and-gesture-readback.md](Assistant/reference/15-2026-11-20-ai-mode-and-gesture-readback.md); [CLAUDE_HANDOFF.md](CLAUDE_HANDOFF.md) lists what the UI must respect.

**Credits:** the OBSBOT protocol work builds on [Domatix/obsbot-control](https://github.com/Domatix/obsbot-control), [lxman/obsbot-mcp](https://github.com/lxman/obsbot-mcp) and [OpenFoxes/Tiny4Linux](https://github.com/OpenFoxes/Tiny4Linux). Focus peaking uses the Canny edge detector from [imageproc](https://github.com/image-rs/imageproc). The full list of sources, including fonts, icons and the Studio camera tools, is in [app/README.md](app/README.md#credits-and-sources).
