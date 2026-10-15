# Contributing to leyline

Thank you for helping improve leyline. The user-facing guide is
[README.md](./README.md); this document collects project workflow and
validation information for contributors.

## Development

```sh
cargo fmt
cargo test
cargo clippy
cargo build --release
```

Read `AGENTS.md` before making changes. It defines the event-loop, UI, and
networking boundaries, plus the task-management workflow. Start each task
with `backlog instructions overview`, use the matching detailed Backlog guide
before planning or changing task state, and update tasks through the CLI rather
than editing their Markdown directly.

## Project history

leyline evolved from a local `ratatui` prototype into private peer-to-peer
chat with invite tickets, persisted identity and history, multi-channel
navigation, and offline history backfill. User-facing behavior belongs in
`README.md` and the published guide; architecture decisions belong in
`concept.md` and Backlog decision records.

## Task management

Feature and bug work uses [Backlog.md](https://github.com/MrLesk/Backlog.md).
Install it with `brew install backlog-md`; `backlog board` opens the terminal
board and `backlog task list --plain` lists current `LEY-` tasks. Capture
uncommitted ideas in [`features.md`](./features.md) before promoting them to a
task, then follow the workflow in `AGENTS.md`.

## Documentation demo

Regenerate the README's two-peer terminal recording with:

```sh
task docs:demo
```

The task builds the release binary, uses tmux to run isolated Alice and Bob
instances, and renders `docs/assets/leyline-demo.gif` through VHS. It requires
`vhs`, `tmux`, `ttyd`, and `ffmpeg` on your `PATH`.

## Manual folder-sync verification

Verify sync between a macOS machine (normally case-insensitive) and a Linux
machine: join both to one channel, turn `/sync publish-and-receive` on both,
create and edit a small nested file on either side, and confirm it arrives only
after it stops changing. Confirm files present before enabling sync and written
while the app was stopped are picked up after two scans. Repeat with hidden,
temporary/editor, `send/`, oversized, and in-progress files; none should
publish. Confirm a received file is not echoed back as a fresh index write,
`/sync off` stops new applications and publications, a remote delete moves
the local copy into recoverable sync trash instead of erasing it, and a
rename arrives as its new path without re-transferring unchanged content.
