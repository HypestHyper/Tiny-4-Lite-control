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
- Only the eight enumerated AI modes are in product scope. Do not add other mode choices without new hardware evidence and an explicit product decision.

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

---

## Result on `claude/upbeat-rubin-gnae3f`

Checked against the acceptance criteria above. Everything below was verified with the synthetic camera, unit tests and headless screenshots, **not on a physical camera**.

| Criterion | Result |
|---|---|
| Exactly the eight verified AI modes | Done, then changed at the user's request: Hand tracking was removed, so seven are offered (No tracking, Normal, Upper body, Close-up, Headless, Lower body, Group). A unit test pins the labels and the tuples. |
| Target Selection initialises from status byte 0x01 | Done. Camera read-back wins; a saved value is used only if the byte cannot be read. |
| Other three gestures never claim read-back; saved and Unknown are visibly distinct | Done. They show a neutral "Saved" label or a coloured "Unknown" label. Byte 0x07 is not used anywhere (a test covers it). |
| No removed feature labels or controls | The HDR switch is gone from the windows and the command line. |
| Existing controls and diagnostics remain | All settings are in the windows and the command line; diagnostics and USB capture are under Device, Developer tools. |
| Tests pass; screenshots updated | `cargo test` and `tests/headless-screenshots.sh` pass. |

Also applied from the newer evidence: the corrected autofocus-priority mapping with read-back, Auto Exposure priority read-back, and the corrected exposure-compensation mapping.

Two upstream inconsistencies found while integrating (both fixed on this branch):

- `set_face_focus` on the autofocus read-back branch takes an enum, but `presets.rs` still called it with a bool, so that branch did not compile when combined with `test-app-v0.1`.
- `presets.rs` read selector 0x06 byte 0x07 as the zoom-gesture state (disproven) and had only seven AI modes.
