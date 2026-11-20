# Repository instructions

Before doing project work, read [Assistant/AGENTS.md](Assistant/AGENTS.md) and [Assistant/INDEX.md](Assistant/INDEX.md). The research handoff is authoritative project context. Load only the relevant topic file, using the preserved original when needed.

This repository contains the research and agent guidance, and the Rust/GTK 4 application in `app/` (see `app/README.md`). Never run hardware tests or send commands to a real camera unless the user explicitly asks; use the synthetic camera (`TINY4LITE_UI_TEST=1`) for UI work. Follow all evidence, licensing, and hardware-safety constraints in `Assistant/AGENTS.md`.
