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

## Documentation demo

The README animation is generated from committed sources:

```sh
brew install vhs
vhs docs/demo.tape
```

`docs/demo.sh` deliberately renders a deterministic visual walkthrough. This
keeps the documentation recording reproducible without a network, persisted
identity, or two interactive instances.

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
