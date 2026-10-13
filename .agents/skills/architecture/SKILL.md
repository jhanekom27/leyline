---
name: leyline-architecture
description: Use for cross-cutting leyline changes, module refactors, dependency-boundary questions, or any work that spans app state, rendering, networking, and runtime orchestration.
---
# Leyline architecture
Read `concept.md` before changing boundaries. Preserve its core direction:

`main` owns the async event loop and I/O → `app` owns pure UI state → `ui` renders read-only state.

Keep slow filesystem and network work off the render path. Prefer extending an existing focused module over adding a cross-layer shortcut. Relevant module roots are `src/app.rs`, `src/ui.rs`, `src/files.rs`, `src/net.rs`, and `src/runtime.rs`.

Run `cargo test` after moving code across module boundaries, then run the full validation set for integrated changes.
