# Contributing to leyline

Thank you for helping improve leyline. The user-facing guide is
[README.md](./README.md); this document collects project workflow and
validation information for contributors.

## Pull requests

`main` is protected and is updated only through pull requests. Start every
change from an up-to-date `main` checkout on a focused feature branch, for
example `feat/ley-42-command-picker` or `fix/file-transfer-timeout`. Use
[Conventional Commits](https://www.conventionalcommits.org/) for commit
subjects, open a pull request, and address review before squash-merging it.

Create or find the relevant Backlog task before planning or implementing
non-trivial work. Read `AGENTS.md` before making changes: it defines the
event-loop, UI, and networking boundaries, plus the required task-management
workflow.

## Development

Contributors need Rust 1.91 or newer. Run the same Rust checks as pull-request
CI before opening or updating a pull request:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --locked
cargo build --release --locked
```

Pull-request CI runs these checks on the stable toolchain and validates the
test/build path on Rust 1.91. Documentation changes also build and test the
guide before merge.

## Releases

Versions change only in a release-preparation pull request. Normal feature,
fix, documentation, and Backlog pull requests do not increment `Cargo.toml`.
The release owner chooses the next plain `X.Y.Z` version from the accumulated
user-visible changes, includes any curated release notes, and merges that
reviewed pull request to `main`.

From the resulting clean `main` commit, run `task release` to build and verify
the candidate, then run `task git:tag` to create and push the matching
annotated `vX.Y.Z` tag. The repository's `release-process` skill can prepare
the release branch and pull request, but never publishes a release tag itself.
Cargo-dist's release workflow validates the tag against `Cargo.toml`, builds
the macOS, Linux, and Windows artifacts, verifies checksums, and publishes the
GitHub Release. Do not use local tooling to upload release assets.

Documentation merged to `main` is built, tested, and deployed to GitHub Pages
by the documentation workflow. Use `task docs:build` and `task docs:test` for
local guide feedback; do not force-push a `gh-pages` branch.

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

## Documentation demos

Regenerate the README's connection-and-chat recording with:

```sh
task docs:demo
```

The task builds the release binary, uses tmux to run isolated Alice and Bob
instances, and renders `docs/assets/leyline-demo.gif` through VHS. It requires
`vhs`, `tmux`, `ttyd`, and `ffmpeg` on your `PATH`.
Regenerate the guide's file-sharing recording with:

```sh
task docs:file-demo
```

This second task uses the same isolated-peer setup and renders
`guide/src/assets/leyline-file-demo.gif`. Its observer pane shows that Bob's
channel folder lacks the fixture before `/save`, then lists the saved copy
after the verified transfer.

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
