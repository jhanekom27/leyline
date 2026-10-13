---
name: leyline-testing
description: Use for selecting or adding leyline tests, validating a refactor, interpreting Rust test failures, or deciding which build, format, and lint commands a change requires.
---
# Testing leyline
Tests live inline with their module under `#[cfg(test)]`; move them with code when extracting a responsibility. Preserve behavior during refactors: do not rewrite assertions merely to accommodate structural changes.

Validation levels:
- Focused refactor: `cargo test`
- Any Rust edit: `cargo fmt` and `cargo test`
- Cross-module or runtime/network change: `cargo fmt`, `cargo test`, `cargo clippy`, and `cargo build --release`

Investigate a test failure at the ownership boundary first: app state, renderer, runtime side effect, or network event.
