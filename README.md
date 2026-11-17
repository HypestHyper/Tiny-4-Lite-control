# OBSBOT Tiny 2 Lite Control

Research handoff and a Rust/GTK4 Linux hardware-control test bench for the OBSBOT Tiny 2 Lite.

## Current project state

- App source: [app/src/main.rs](app/src/main.rs)
- App build/runtime notes: [app/README.md](app/README.md)
- Current framework status and protocol summary: [Assistant/APP-IMPLEMENTATION-STATUS.md](Assistant/APP-IMPLEMENTATION-STATUS.md)
- Research and agent navigation: [Assistant/INDEX.md](Assistant/INDEX.md)

The app identifies the Lite by USB ID `3564:fef9`, reads V4L2 controls dynamically, and contains a separate vendor UVC Extension Unit path for selected AI tracking and gesture operations. This is a functional test bench, not a finished UI. Visual redesign is deferred.

## Research

**Start here:** [Assistant/INDEX.md](Assistant/INDEX.md)  
**Agent rules:** [AGENTS.md](AGENTS.md) and [Assistant/AGENTS.md](Assistant/AGENTS.md)  
**Original feature research:** [Assistant/source/obsbot_tiny_2_lite_linux_research_handoff.md](Assistant/source/obsbot_tiny_2_lite_linux_research_handoff.md)  
**USB tracing workflow:** [Assistant/tracing on main](https://github.com/HypestHyper/Tiny-4-Lite-control/tree/main/Assistant/tracing)

No new Windows OBSBOT Center USB traces have been collected yet. The next phase is controlled capture, changing one setting/action at a time and validating the resulting transfers before adding more protocol support.
