---
id: LEY-21
title: Live in-TUI device pairing with confirm-and-wipe
status: Done
assignee: []
created_date: '2026-10-11 06:33'
updated_date: '2026-10-11 06:59'
labels:
  - identity
  - networking
  - ux
dependencies: []
type: feature
ordinal: 24000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Redeeming a pairing ticket (`--pair <ticket>`) currently only works as a startup CLI flag: pairing bootstraps a brand-new identity before this device has generated its own user key and un-shared "general" channel, and by the time a live command could run that would already have happened (see pairing.rs's module doc comment). That makes pairing an already-running instance clunky -- quit, copy the ticket, restart with a flag.

Add a live `/pair <ticket>` command (the existing no-arg `/pair` still creates an outbound ticket, unchanged) that warns the user this will replace the device's current identity/channels/history with the paired device's, and on confirmation wipes the local state that would otherwise conflict with the incoming one and restarts the process with `--pair <ticket>`, so the actual redeem goes through the existing, already-tested `pairing::bootstrap` path rather than a new, separately-tested live code path. Net/AppState are not torn down and rebuilt in place -- see pairing.rs's doc comment for why that unwinding was deliberately avoided the first time.

What needs wiping before the restart: the shared user key (bootstrap's own precondition), the channel registry (otherwise a same-named existing channel like "general" silently keeps its old secret instead of adopting the paired one -- see channel_registry.rs's record_channel), local message history and blobs for those channels (otherwise old private history would surface inside the newly-adopted, differently-secreted channel of the same name), and the smaller local caches (contacts, dms, settings, forgotten_devices) for a genuinely clean adopted identity. The device's own identity keypair (EndpointId) is never touched -- pairing changes the user key, not the device key.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Running /pair <ticket> with a well-formed ticket shows an explicit warning -- mentioning how many channels are currently joined -- that confirming will erase this device's current identity, channels, and message history, before anything on disk is touched
- [x] #2 Confirming (Enter) wipes the superseding local state, restarts the process with --pair <ticket>, and the instance comes back up already paired: same shared identity and channel membership as the device that shared the ticket, with no separate manual CLI invocation needed
- [x] #3 Canceling (Esc) leaves every local file (identity, user key, channel registry, messages, blobs, contacts, settings, forgotten devices) completely untouched and reports that pairing was cancelled
- [x] #4 A malformed pairing ticket is rejected with an explanatory message immediately -- before the warning is ever shown and before anything is wiped
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Why not redeem live, in place: pairing::bootstrap deliberately only ever runs before the event loop starts, since unwinding an already-running Net/AppState (tearing down every joined gossip topic, swapping Net's user_key_bytes/our_cert, rebuilding AppState's channel tabs/history) is a lot of new, independently-risky surface area for a UX fix. Instead: validate and confirm in the TUI, wipe exactly the local state that would otherwise conflict with the incoming identity, then restart this same binary with --pair <ticket>, so the actual redeem goes through the existing, already-tested startup path unchanged.

Command behavior: /pair (no arg) unchanged. /pair <ticket>: decode synchronously in app.rs (ticket::PairingTicket::decode_string, a pure parse, no I/O), reject a malformed one immediately. If valid, arm a new pending_redeem: Option<String> field and push a warning naming the current channel count. handle_key gets a new early-dispatch branch (mirroring picking_reply/picking_thread) while pending_redeem is armed: Enter moves the ticket into a new pub restart_with_ticket: Option<String> field and sets should_quit=true; Esc clears pending_redeem and reports cancellation; every other key is ignored. No new InputAction variant needed -- reuses the existing should_quit-breaks-the-loop mechanism.

Wipe + restart (main.rs): run()'s return type becomes anyhow::Result<Option<String>>. After the loop ends and net.shutdown()/ratatui::restore() run as today, Ok(Some(ticket)) triggers a new restart_for_pairing(&dirs, &ticket): removes config_dir/user_key and data_dir/{channels, messages, blobs, contacts, dms, settings, forgotten_devices} (abort on first failure, never touch identity or leyline.log), then re-execs this binary with --pair <ticket> (exec on unix, spawn+exit fallback elsewhere). user_key must go for bootstrap's own precondition; channels must go or record_channel silently keeps the old secret for a same-named channel instead of adopting the paired one; messages/blobs must go or old private history would surface inside the newly-adopted same-named channel; the rest go for a genuinely clean adopted identity. On success this never returns; the only Err path is a wipe failure (nothing further touched) or exec failing (rare), either way the ticket is still valid so the error names the exact manual command that still finishes pairing.

Testing: app.rs unit tests for the confirm/cancel/validate state machine (replacing the now-stale pair_with_an_argument_reports_usage_instead_of_the_pair_action test). main.rs gets its first #[cfg(test)] module, covering the wipe step against a tempdir (listed paths removed when present, missing ones are not an error, unrelated files like identity/leyline.log untouched). The actual exec/process-restart path isn't unit tested (would replace the test process); verified manually instead via the two-device tmux harness, triggering pairing from inside an already-running instance's TUI.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as: AppState gains pending_redeem (private) + pub restart_with_ticket fields. /pair <ticket> decodes PairingTicket::decode_string synchronously in app.rs (pure parse, no I/O) -- malformed tickets are rejected immediately with no state change. A valid ticket arms pending_redeem and warns, naming the current channel count. handle_key gains an early-dispatch branch (mirroring picking_reply/picking_thread) while pending_redeem is armed: Enter moves the ticket into restart_with_ticket and sets should_quit (Ctrl+C still force-quits without arming a restart); Esc clears it and reports cancellation; every other key is ignored. No new InputAction variant was needed.

main.rs: run() now returns anyhow::Result<Option<String>>, propagating app.restart_with_ticket once the loop exits (after Net::shutdown runs exactly as for a normal quit). main() calls a new restart_for_pairing(&dirs, &ticket) after ratatui::restore() when a ticket comes back: wipe_superseded_state removes config_dir/user_key and data_dir/{channels, messages, blobs, contacts, dms, settings, forgotten_devices} (aborting on the first failure, identity/leyline.log never touched), then exec_pair re-execs this binary with --pair <ticket> (std::os::unix::process::CommandExt::exec on unix, spawn+exit fallback elsewhere) so the actual redeem goes through the existing, already-tested pairing::bootstrap startup path.

Verification: cargo build clean, cargo test -- 433 passed/0 failed (9 new: 4 net-new in app.rs covering the confirm/cancel/validate/ignore-while-armed state machine, replacing the now-stale pair_with_an_argument_reports_usage test; 5 in main.rs's first #[cfg(test)] module, covering remove_path and wipe_superseded_state against tempdir -- every listed path removed when present, missing ones tolerated, identity/leyline.log stand-ins never touched). cargo clippy --all-targets clean. cargo fmt clean for every file touched (same two pre-existing, untouched-by-this-change fmt diffs in dm_registry.rs/net.rs noted in earlier LEY-20.x tasks remain, left alone).

Live manual verification (two isolated real leyline instances, separate fake HOME dirs, real QUIC/relay networking): started two brand-new, unpaired devices; got an outbound ticket from device-a's /pair; ran /pair <ticket> live inside device-b's already-running TUI (no restart needed to trigger it); confirmed the exact warning text (channel count included); pressed Enter; observed device-b's TUI exit and a fresh instance come back up within ~8s already showing #general with 1 peer -- device-a. Confirmed on disk: device-b's identity file timestamp was unchanged (device identity preserved, as designed -- pairing changes the user key, not the device key) while user_key was freshly rewritten and is byte-identical (sha1-verified) to device-a's. Device-a's own view confirmed the connection is bidirectional.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a live /pair <ticket> command: a confirmation warning (naming the current channel count) gates a destructive redeem, which on Enter wipes exactly the local state that would otherwise conflict with the incoming identity and restarts this binary with --pair <ticket>, reusing the existing, already-tested pairing::bootstrap startup path rather than mutating a running Net/AppState in place. Esc cancels with zero side effects. Verified all 4 ACs: unit tests for the confirm/cancel/validate state machine and the wipe-path logic, plus a live two-device manual test redeeming a ticket from inside an already-running TUI -- the device came back up paired (byte-identical shared user key, bidirectional gossip connection) with its own device identity untouched. cargo build/test(433)/clippy/fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
