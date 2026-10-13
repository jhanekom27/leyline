---
id: LEY-25.8
title: Allow overriding the file size limit when sending deliberately
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-13 02:53'
updated_date: '2026-10-13 05:55'
labels:
  - files
  - ux
dependencies:
  - LEY-25.1
parent_task_id: LEY-25
type: enhancement
ordinal: 51000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`files::MAX_FILE_SIZE` (500 MB, `src/files.rs:48`) hard-refuses any file or pasted image over the cap in `stage_file`/`stage_image`, with no way around it short of editing the constant and rebuilding. It is a good guardrail against an accidental huge share, but it blocks a deliberate large send (a video, a backup archive) just as hard as it blocks a mistake.

Keep the default guardrail intact -- normal-sized sends should see no change -- but add a way past it for a deliberate send: a persisted setting that raises or disables the cap, a one-off confirmation for that specific file, or both. Which mechanism(s) to build is an implementation-time decision.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 With no override, staging or sending a file over the limit is refused exactly as today -- the default guardrail is unchanged
- [x] #2 A user who deliberately wants to share an over-limit file has a way to do so without rebuilding the binary
- [x] #3 The refusal message for an oversized file says how to override it
- [x] #4 Any persisted override survives restarts and is discoverable, the way /folder shows the files root today
- [x] #5 Tests cover the default refusal and the override path
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Findings (no code needed). The limit is `files::MAX_FILE_SIZE` (500 MB), enforced in exactly two functions: `stage_file` (every route for a file: `/send <path>` through `Net::send_file`, the bare-`/send` picker, a dropped path, a clipboard file list) and `stage_image` (a pasted image). Nothing re-checks at send (`import_staged` trusts staging), and the refusal text is built in one place (`too_large`). So one parameter on those two functions reaches every route.
2. Mechanism: a persisted setting with a command to show and change it, not a per-file confirmation. A confirmation needs its own modal state in four entry points (command, picker, drop, clipboard) and a way to say "this file"; the setting is one value every route reads, it is what AC 4 describes, and `/folder` and `/bell` already set the pattern of a command that shows and changes a preference. The one-off confirmation is not built (recorded for review).
3. `/maxsize [<size>|off|default]`. `files::SizeLimit { Default, Bytes(u64), Off }` mirrors exactly what is persisted (nothing = Default). Bare shows the current limit and how to change it, as bare `/folder` shows the root; `<size>` is a number and a unit B/KB/MB/GB/TB (the units `human_size` prints, 1024-based, any case, an optional blank); `off` lifts the limit; `default` restores 500 MB. Validated once at the boundary (`SizeLimit::parse`): no unit, an unknown unit, a malformed number, 0 (it would refuse every file) and a size beyond u64 are refused with a message that says what to type, and the limit stays as it was.
4. Persistence: its own one-line file `size_limit` in the data directory, like `files_root` and `browse_dir` (not a field of the postcard `settings` file, which would fail to decode an existing one): `files::size_limit(data_dir)` at startup, `files::set_size_limit`. Absent is Default; content that can't be read as a limit is logged and ignored, so it falls back to the guarded default, never to an unguarded one; a genuine I/O error fails startup, as for `files_root`. Pairing leaves it alone: a preference of this device.
5. Threading: `stage_file(path, limit)`, `stage_image(name, bytes, limit)`, `too_large(size, limit)`; `Net::send_file(channel, path, limit)`; main.rs passes `app.size_limit()` to `send_file`, `stage_files` and `stage_clipboard_image`. `AppState` holds the limit (seeded by `load_size_limit`, like the files root); `/maxsize` applies it at once and returns `InputAction::SetSizeLimit` for main.rs to persist, as `Bell` does. A failed write is reported and the setting still holds for the session.
6. The refusal says how to get past it: `can't send X: too large (501.0 MB, max 500.0 MB) -- raise the limit with /maxsize <size> (say /maxsize 2GB) or lift it with /maxsize off`. Nothing else about the default changes: a file over 500 MB is refused as before.
7. Discoverability and housekeeping: `/maxsize [size|off]` in `COMMAND_HINTS` and `/help`; a README command row; a features.md entry; concept.md (File sharing, the data directory, the files.rs layout line). Cargo.toml 0.5.0 -> 0.6.0 (an enhancement: minor); Cargo.lock follows the build.
8. Tests. files.rs: the parse table (units, case, blanks, decimals, off and default; every rejection), the persisted file (absent, off, bytes, garbage ignored, round trip, default removes it, an I/O error is an error), `stage_file` and `stage_image` under each limit (the default refuses just over 500 MB and accepts exactly 500 MB; a custom limit; off), and the message naming both sizes and the override. app.rs: bare `/maxsize` in each state, setting, off, default, bad arguments leave it unchanged with no action, seeding, `/help`. main.rs: staging files, a drop and a clipboard image obey the limit and its override; pairing leaves `size_limit` alone. net.rs: tests follow the new signatures. ui.rs: the hint is listed.
9. Verify with cargo test, clippy --all-targets and fmt, then the real binary through PTYs, two instances: the default refusal (a sparse 501 MB file, by `/send` and by a drop) and its hint; bad arguments; `/maxsize 1KB` refusing a 2 KB file and `/maxsize 1MB` letting it through to the other instance; a drop of the sparse file staged under `off` (and cleared with Esc, nothing imported); the setting still there after a restart; `default`.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Built as planned. The mechanism is a persisted setting with a command to show and change it, `/maxsize [<size>|off|default]`; the one-off per-file confirmation is not built (see interpretation 1).

Changed: src/files.rs (`MAX_FILE_SIZE` is now `DEFAULT_MAX_FILE_SIZE`; new `SizeLimit { Default, Bytes(u64), Off }` with `max_bytes` and the one validating `parse`; `size_limit` and `set_size_limit` for the one-line `size_limit` file; `stage_file` and `stage_image` take the limit; `too_large` builds the refusal, now with the way past it), src/net.rs (`Net::send_file` takes the limit), src/app.rs (`AppState::size_limit`/`load_size_limit`, `/maxsize`, `InputAction::SetSizeLimit`, `/help`), src/main.rs (loads the limit at startup, persists a change, passes `app.size_limit()` to `send_file`, `stage_files` and `stage_clipboard_image`), src/ui.rs (the hint), README, features.md, concept.md, Cargo.toml 0.5.0 -> 0.6.0 (an enhancement: minor; Cargo.lock follows the build).

Validation: cargo test 887 passed (29 new, was 858), cargo clippy --all-targets clean, cargo fmt --check flags only the old hunk in src/dm_registry.rs (left alone). The real binary, two instances under isolated homes each in a PTY read with pyte (scratch script outside the repo, /tmp/ley-25-8/run.py): 37 of 37 checks pass. By default a normal file is sent and reaches the other instance, and a sparse 501 MB file is refused by `/send` and by a bracketed-paste drop with `too large (501.0 MB, max 500.0 MB)` and the hint, and nothing about it reaches the other instance; under `/maxsize 1KB` a 2 KB file is refused and under `/maxsize 1MB` the same file is sent and arrives, with no rebuild in between; `banana`, `5` and `0MB` are refused with what to type and leave the limit as it was; under `/maxsize off` a drop of the 501 MB file is staged (the strip shows it and its size) and Esc clears it before anything is imported; 2 GB and `off` come back after a restart, `/maxsize default` remembers nothing, a bare `/maxsize` shows each state, and a `size_limit` file holding garbage means the guarded default and the 501 MB file is still refused. Every notice check reads the newest notice in the transcript, so an earlier identical line cannot satisfy it.

Interpretations to confirm:
(1) A persisted setting only; no one-off confirmation. Staging has four entry points (the command, the picker, a drop, the clipboard); a confirmation would need modal state in each and a way to say "this file", while a setting is one value they all read. The cost: raising the limit for one deliberate send leaves it raised until `/maxsize default`. A one-off `/send --force` or a confirm-on-Enter could be added later.
(2) The command is `/maxsize`. A size is a number and a unit -- B, KB, MB, GB or TB, any case, a blank allowed -- 1024-based like the sizes the app prints, so what a refusal says is what can be typed. A bare number, an unknown unit, a malformed number, a size that rounds down to under one byte (it would refuse every file) and one beyond u64 are refused with what to type; `off` lifts the limit and `default` restores 500 MB.
(3) "Refused exactly as today" is read as the behaviour -- the file is refused, by every route, at the same size. The text of the refusal gains the way past it, as criterion 3 asks.
(4) The limit is `Default | Bytes | Off` and is remembered as a one-line `size_limit` file in the data directory (not in the postcard `settings` file, which cannot grow a field without failing to decode an old file). `default` forgets the file, so a build that moves the default moves everyone who never chose a limit. A file that is not a limit is logged and means the default, never a looser limit; a genuine I/O error fails startup, as for `files_root`; a failed write is reported and the limit still holds for the session. Pairing leaves it alone, as a preference of this device.
(5) Only what you share is limited: a file someone sends you is not checked against it when you `/save` it.
(6) The limit also governs a pasted clipboard image (checked after it is encoded as PNG, in memory), as the old cap did; under `off` that check is gone too.

Reviewed and accepted: the persisted-setting approach, `/maxsize` and its persistence. The limits it leaves -- only files you share are limited, a pasted image answers to the same limit and `off` drops that check, the limit is not re-checked at send, there is no one-off confirmation, a unit is required -- are accepted for now and written down in features.md (the "File size limit override" entry) so they can be revisited.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added `/maxsize [<size>|off|default]`: a persisted limit on the size of a file or pasted image you share. It is enforced in `files::stage_file` and `files::stage_image`, so every route (`/send <path>`, the picker, a drop, the clipboard) answers to it; it is remembered in a one-line `size_limit` file in the data directory; and the refusal now says how to get past it. The default 500 MB guardrail is unchanged. Verified with 887 unit tests (29 new), clippy, and 37 of 37 scripted checks of the real binary under PTYs: a sparse 501 MB file refused by `/send` and by a drop, a 2 KB file refused under 1KB and delivered under 1MB, `off` staging the 501 MB drop, settings surviving restarts, and a garbage settings file meaning the default. Version 0.6.0.
<!-- SECTION:FINAL_SUMMARY:END -->
