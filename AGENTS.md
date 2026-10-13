# AGENTS.md
leyline is a peer-to-peer terminal chat app (Rust, iroh + iroh-gossip + ratatui). See README.md for features/usage and concept.md for architecture notes.
## Commands
- Build: `cargo build --release`
- Run: `cargo run`
- Test: `cargo test`
- Format: `cargo fmt`
- Lint: `cargo clippy`
Tests live inline per-module (`#[cfg(test)]`), not in a separate `tests/` directory. No custom rustfmt/clippy config; defaults apply.
## Versioning
Every completed Backlog task bumps `version` in `Cargo.toml`, in the same commit as the work, following semver (`MAJOR.MINOR.PATCH`) and mirroring the conventional commit type:
- `feat` (a new feature or enhancement): minor
- `fix`, `perf`: patch
- breaking (`!` or `BREAKING CHANGE:` -- anything older peers or existing on-disk data can no longer handle): major. While we are on 0.x, bump the minor instead, and ask before going to 1.0.0
- `docs`, `test`, `refactor`, `style`, `build`, `ci`, `chore`: no bump, nothing user-visible changed
Bumping a component resets the ones below it (0.3.1 -> 0.4.0). Building updates `Cargo.lock` to match, so commit it with `Cargo.toml`.
Peers broadcast this version and compare it as a plain `X.Y.Z` triple (`src/version.rs`) to tell each other an update is available, so never use a pre-release suffix such as `-beta`: it reads as "not newer" and the update goes unnoticed.
<!-- BACKLOG.MD GUIDELINES START -->
<!-- backlog.md-instructions-version: 1.53.0 -->
<CRITICAL_INSTRUCTION>

## Backlog.md Workflow

This project uses Backlog.md for task and project management.

**At the beginning of each conversation in this project, run `backlog instructions overview` before answering or taking action. Re-read it only if you have not read it yet in the current conversation.**

Use the overview to decide whether to search, read, create, or update Backlog tasks.

Before task lifecycle actions, read the matching detailed guide:
- `backlog instructions task-creation` before creating or splitting tasks
- `backlog instructions task-execution` before planning, changing status or assignee, adding a plan or implementation notes, or implementing task work
- `backlog instructions task-finalization` before checking acceptance criteria, writing final summaries, or moving tasks to terminal statuses

Use `backlog <command> --help` before running unfamiliar commands. Help shows options, fields, and examples.

Do not edit Backlog task, draft, document, decision, or milestone markdown files directly. Use the `backlog` CLI so metadata, relationships, and history stay consistent.

</CRITICAL_INSTRUCTION>
<!-- BACKLOG.MD GUIDELINES END -->
