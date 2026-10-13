---
name: leyline-command-documentation
description: Use whenever adding, removing, renaming, aliasing, or changing the behavior or arguments of a user-facing leyline slash command. Keep the command parser, sidebar hints, `/help`, README, and focused tests aligned so people can discover and use the command consistently.
---
# Leyline command documentation
User-facing slash commands have four complementary discovery surfaces. Update all that apply whenever a command changes so the UI never advertises stale syntax and a working command is not hidden from users.

## Required update points
- `src/app.rs`
  - Add or change parser routing in `AppState::run_command`.
  - Update `AppState::run_help` with the accepted syntax and material behavior.
  - Add or revise inline command tests, including valid input and invalid usage where relevant.
- `src/ui.rs`
  - Add or change the compact usage entry in `COMMAND_HINTS`.
  - Add or revise the focused `COMMAND_HINTS` test.
- `README.md`
  - Update the Commands table with user-facing behavior, scope, persistence, and safety implications.

## Keep the surfaces consistent
- Use the same command spelling, aliases, and argument syntax everywhere.
- The sidebar entry should be short usage only; the `/help` notice and README may explain details.
- If a command is intentionally unavailable in a conversation type, platform, or state, make that boundary clear in `/help` and README.
- If a command changes persisted state, document its default, scope, lifecycle (restart, leave, pairing), and failure behavior.

## Validation
Run `cargo fmt` and `cargo test` after changing command behavior or hints. Inspect the final diff to confirm the parser, hints, `/help`, README, and relevant tests agree.
