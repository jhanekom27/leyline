---
name: leyline-release-process
description: Use whenever preparing, cutting, versioning, tagging, publishing, or automating a Leyline release. This skill determines and explains the next release version, prepares the reviewed release pull request on its own branch, and guides the safe post-merge tag trigger. Use it even when the user only asks what version should ship next.
---
# Leyline release process

Prepare releases in two separate phases:

1. A reviewed release-preparation pull request chooses and records the version.
2. A matching tag from the merged `main` commit triggers cargo-dist publication.

Keeping these phases separate makes the shipped source reviewable and prevents a
feature branch from publishing a release.

## Release preparation

Start from a clean checkout. Fetch `origin`, inspect the latest reachable
`vX.Y.Z` tag, the current package version in `Cargo.toml`, completed Backlog
work, and conventional commits since that tag.

Recommend a plain `X.Y.Z` version with the evidence behind it:

- a confirmed breaking change is major, or minor while the major version is
  zero;
- one or more user-visible `feat` changes are minor;
- only `fix` or `perf` changes are patch;
- documentation, Backlog, test, refactor, style, build, CI, and chore changes
  do not independently justify a bump.

Treat the current package version as the candidate when it is already higher
than the latest release tag. Report the discrepancy; never decrease it or
replace it silently. The current package version and release tag may reflect
work prepared before this skill existed.

Before changing files, state the proposed version, supporting evidence, release
branch name (`chore/release-vX.Y.Z`), and whether an existing changelog will be
updated. Get explicit user confirmation before proceeding.

After confirmation:

1. Start `chore/release-vX.Y.Z` from current `main`.
2. Update `Cargo.toml` and refresh the root package entry in `Cargo.lock`.
3. Update release notes only when an established `CHANGELOG` or `RELEASES`
   file exists. Do not introduce a changelog format incidentally.
4. Run `cargo fmt --check`, `cargo clippy --all-targets --all-features`,
   `cargo test --locked`, `cargo build --release --locked`,
   `dist generate --check`, and `dist plan --tag vX.Y.Z`.
5. Commit with `chore(release): prepare vX.Y.Z` and append
   `Co-Authored-By: Warp <agent@warp.dev>`.
6. Push the branch and, when `gh auth status` succeeds, create a PR to `main`.
   Summarize the version decision and validation in the PR body, followed by
   the same Warp co-author attribution.

If validation fails, stop before committing or opening the PR and report the
failure with the command output and affected files.

## Publication after merge

Do not create or push a release tag while preparing the release PR. Once the
PR is merged, update local `main` with `git pull --ff-only`, confirm it is
clean, and run:

```sh
task git:tag
```

That task reruns local release validation, confirms `main` equals
`origin/main`, rejects existing local or remote tags, creates an annotated tag
matching `Cargo.toml`, and pushes only that tag. The push triggers cargo-dist's
GitHub Release workflow.

Never force-push a release tag, tag a feature branch, or manually upload
release assets.
