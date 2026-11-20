# Claude handoff: Tiny 2 Lite GTK UI

**Repository:** `HypestHyper/Tiny-4-Lite-control`  
**Working branch:** `test-app-v0.1`  
**App source:** `app/src/main.rs`  
**Project status:** Read `Assistant/APP-IMPLEMENTATION-STATUS.md` first. The newest AI/gesture evidence is in `Assistant/reference/15-2026-11-20-ai-mode-and-gesture-readback.md`.

## Task

Continue the GTK UI redesign and implementation on the existing Rust/GTK4 app. The protocol and state-handling updates described below are already applied to the branch. Do not re-implement or guess those protocol mappings. Focus on layout, interaction clarity, responsive behavior, and visual consistency.

## Verified application behavior now in source

- Eight AI modes are supported and exposed: No Tracking, Normal Tracking, Upper Body, Close-up, Headless, Lower Body, Hand, and Group.
- Mode tuples are read from selector `0x06` status offsets `0x18` and `0x1C`.
- Target Selection gesture state is read from selector `0x06` byte `0x01`: `0` = disabled, `1` = enabled.
- Zoom/L-pose, Dynamic Zoom, and Dynamic Zoom Direction do not have confirmed device-side read-back. Their state is saved/local or Unknown.
- Do not interpret selector `0x06` byte `0x07` as Zoom gesture state.
- Gesture command payloads, Dynamic Zoom Direction inversion, rolling frame sequence, and CRC/frame builder are existing protocol code. Preserve them.
- The meanings of selector `0x03` byte 0 and selector `0x09` byte 0 are unknown. Do not expose them as user-facing status.
- Desk and Whiteboard are out of the product scope and must not be reintroduced in UI, enums, docs, examples, or tests.

## UI design objectives

1. Treat the current UI as a functional starting point, not a final visual design.
2. Build a coherent camera-control experience with clear hierarchy between live preview/connection state, gimbal movement and position, image/exposure/focus, AI tracking mode and speed, gesture controls, camera presets, and capture settings.
3. Keep AI tracking mode selection visually separate from gesture toggles. Target Selection is a gesture, not a tracking mode.
4. Clearly distinguish camera-confirmed state from saved/local state and Unknown. Do not style a local value as a live device status.
5. Keep disabled controls visible when useful and explain their dependency/blocker.
6. Preserve keyboard navigation, accessibility labels, sensible focus behavior, and responsive layout at narrow widths.
7. Follow the repository's existing GTK4/Rust architecture and design tokens. Avoid adding a new UI framework or large dependency unless necessary.
8. Do not regress V4L2/XU paths, persistence, diagnostics, capture workflow, or headless UI tests.

## Required review before visual changes

- Read `AGENTS.md` and `Assistant/AGENTS.md`.
- Read `Assistant/APP-IMPLEMENTATION-STATUS.md`.
- Read `Assistant/reference/15-2026-11-20-ai-mode-and-gesture-readback.md`.
- Inspect `app/src/ui/`, `app/src/main.rs`, `app/src/config.rs`, and `app/README.md`.
- Preserve device-specific evidence confidence labels. A successful ioctl is not proof of physical behavior.

## Acceptance criteria

- The UI contains exactly the eight verified AI modes listed above.
- Target Selection initializes from camera status byte `0x01`.
- The other three gesture controls never claim camera read-back; saved/local and Unknown states are visibly distinct.
- No removed feature labels or controls are present.
- Existing camera controls and diagnostics remain available.
- Headless UI tests and Rust unit tests pass; add/update screenshots for materially redesigned pages.
- Report build/test results accurately. Physical camera validation is separate from headless UI validation.

## Non-goals

- Do not reverse-engineer additional protocol selectors during the UI redesign.
- Do not add guessed modes, gesture states, or status labels.
- Do not modify firmware, USB descriptors, or proprietary OBSBOT software.
- Do not change existing XU command payloads or movement protocol without new evidence and a separate focused task.
