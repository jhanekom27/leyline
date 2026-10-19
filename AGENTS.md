# leyline agent guide
leyline is a peer-to-peer terminal chat app built with Rust, iroh, iroh-gossip, and ratatui. Read `README.md` for user-facing behavior and `concept.md` before changing architectural boundaries.
## Branches and pull requests
- Never implement changes directly on `main`. Create or continue a focused feature branch from up-to-date `main`, then open a pull request for review and merge.
- Keep each branch scoped to one coherent task. Use a conventional branch name such as `feat/ley-42-command-picker`, `fix/file-transfer-timeout`, or `chore/ley-36-ci-cd`.
## Architecture invariants
- The `main` event loop is the only owner of `AppState`; communicate with it through actions and events rather than sharing it behind a mutex.
- `app` contains pure UI-state transitions. It must not perform network or filesystem I/O.
- `ui` only reads `AppState` and renders. It must not mutate state or perform I/O.
- Network and slow filesystem work stay asynchronous so input and rendering remain responsive.
## Topic routing
Read only the skill relevant to the requested change:
- `.agents/skills/architecture/SKILL.md` — boundaries, dependency direction, and cross-cutting changes.
- `.agents/skills/app-state/SKILL.md` — `AppState`, commands, input, overlays, replies, and threads.
- `.agents/skills/command-documentation/SKILL.md` — adding, removing, renaming, or changing a user-facing slash command.
- `.agents/skills/ui-rendering/SKILL.md` — ratatui rendering, layout, and overlays.
- `.agents/skills/networking/SKILL.md` — iroh, gossip, direct messaging, tickets, and backfill.
- `.agents/skills/files-and-storage/SKILL.md` — attachments, destinations, persistence, and file safety.
- `.agents/skills/runtime-event-loop/SKILL.md` — startup, `tokio::select!`, and side-effect routing.
- `.agents/skills/testing/SKILL.md` — inline tests and validation selection.
- `.agents/skills/release-process/SKILL.md` — release version selection, preparation pull requests, and post-merge tagging.
## Behavioral scope discovery
Before adding a refusal, capability guard, or user-facing claim that narrows supported behavior:
- Read the relevant Backlog task, its parent, and completed sibling or child tasks; search Backlog for the affected command, channel kind, or capability.
- Inspect the focused code path and existing inline tests for supported variants.
- Treat documentation wording as potentially stale, not proof that a feature is unsupported. When documentation and tested behavior conflict, preserve the behavior and correct the stale documentation or task wording.
## Commands
- Format: `cargo fmt`
- Test: `cargo test`
- Lint: `cargo clippy`
- Release build: `cargo build --release`
Tests live inline beside the module they cover. Do not add feature behavior while performing a structural refactor.
## Versioning
Versioning happens only in a dedicated release-preparation pull request. Ordinary feature, fix, documentation, task, and Backlog work must not change `Cargo.toml` solely to bump the version. The release owner selects the next semver value from the accumulated user-visible changes, merges the reviewed release PR to `main`, then creates and pushes the matching `vX.Y.Z` tag from that commit. GitHub Actions validates the tag and publishes the artifacts. Never add a pre-release suffix because peers compare versions as plain `X.Y.Z` triples in `src/version.rs`.
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
