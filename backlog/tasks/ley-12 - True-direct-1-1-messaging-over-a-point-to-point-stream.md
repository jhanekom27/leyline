---
id: LEY-12
title: 'True direct 1:1 messaging over a point-to-point stream'
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-08 02:12'
updated_date: '2026-10-12 02:52'
labels:
  - networking
dependencies: []
type: feature
ordinal: 12000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The existing `/msg` command (Direct 1:1 DMs) still provisions a private gossip channel and requires a one-time ticket exchange. A true point-to-point option, using a direct QUIC stream to a known `EndpointId` instead of gossip, `RoomSecret`, or tickets, would remove that remaining manual step for a peer already met, and would avoid gossip-mesh overhead for a two-node conversation. This is a bigger departure from the one-gossip-task-per-channel model and needs its own connection lifecycle, message framing, presence, and history sync, so it is worth prototyping separately from the rest of the messaging work.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Two peers who already know each other's EndpointId can exchange messages over a direct point-to-point connection, without a ticket exchange
- [x] #2 The connection recovers from a disconnect via retry/lifecycle handling
- [x] #3 Message history for the conversation is available after a restart
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add src/direct.rs: a Direct manager (mirrors net::Net), registered on the shared Router under ALPN leyline/direct/1. Owns one long-running actor task per peer: dials endpoint.connect(peer_id, ALPN) immediately, retries with capped exponential backoff on failure/disconnect (AC2), and also accepts inbound connections handed off by its ProtocolHandler::accept (last-connection-wins, no tie-break). Sends: one open_uni() stream per ChatMessage (postcard). Receives: accept_uni() loop, decoded via ChatMessage::decode, forwarded as NetEvent::Received. Emits PeerJoined/PeerLeft on connect/disconnect (reused as-is for presence). Exposes open(peer)->channel name, send(peer,message), close(peer). channel_name(peer) = p2p-<hex_prefix> is the single source of truth for naming, used by both outbound and inbound paths so they converge on one tab/history file.
2. net.rs: construct+register Arc<Direct> in Net::start (alongside pairing); add Net::open_direct/send_direct/close_direct wrappers; add one new NetEvent::DirectJoined{channel,peer} variant (fired only on first-ever contact from the inbound side, to create a background tab without stealing focus).
3. app.rs: reuse the existing generic Channel/storage/ui machinery (no changes needed there). Add direct_channels: HashMap<[u8;32], String>, direct_peer_for(), activate_direct_channel()/background_direct_channel() (create-or-switch + bookkeeping, return is-new for history loading), forget_direct_channel(). Add InputAction::OpenDirect{peer,text}. Add /dm <alias|hex-prefix|full-64-hex-id> [text] command + run_dm (mirrors run_msg: resolve via resolve_msg_target, fall back to parsing a raw 64-hex-char EndpointId for true cold-start contact, refuse self-target, reuse existing tab if present, else return OpenDirect so main.rs centralizes tab creation+history load). Guard /invite to refuse clearly on a point-to-point channel (net.ticket_for would otherwise panic). Update /help. Make hex_prefix pub(crate) for direct.rs to reuse.
4. main.rs: mod direct; handle InputAction::OpenDirect (net.open_direct, activate tab, load history+send queued text once). Handle NetEvent::DirectJoined (background_direct_channel + load history) as its own match arm -- NOT folded into the Joined/DmJoined/ChannelSyncJoined registry-persisting block, which assumes a gossip RoomSecret and would panic. InputAction::Leave branches on direct_peer_for to call net.close_direct instead of net.leave, plus forget_direct_channel. Add a small route_send helper used at all net.send(...) call sites (InputAction::Send, NetEvent::FileReady, NetEvent::ClipboardText) to route to net.send_direct when the channel is a direct one; same call sites (plus the generic Received fallback) skip backfill.record_message(s) for direct channels (that store only matters for gossip's Announce/sync_history flow). /send, /paste, /save keep working over a direct tab for free via this routing change -- no dedicated file-sharing work.
5. Deliberate scope boundaries: no new persisted registry/startup auto-reconnect for direct peers (a restart needs a fresh /dm, or the other side's still-retrying actor reconnecting to us first, to bring the tab+history back -- the history itself is already safely on disk, satisfying AC3); no inbound access control/allowlist (anyone with your EndpointId can open a background tab, like an unsolicited message).
6. Tests: direct.rs unit test for channel_name determinism, loopback tokio::test(s) (two real Endpoints+Router, mirroring pairing.rs's test style) for connect/send/receive and for reconnect-after-disconnect. app.rs unit tests mirroring the existing /msg suite: new DM via alias/hex-prefix/full-id, reuse of an existing tab, self-target refusal, /invite refusal on a direct channel. Run cargo test, cargo clippy, cargo fmt before wrapping up.
7. Docs: README.md command table gains /dm; concept.md gains a short section on the point-to-point connection lifecycle alongside the gossip model.

8. Revised: persist known point-to-point peers after all (no technical blocker to doing so -- was an unnecessary scope cut). Add src/direct_registry.rs mirroring dm_registry.rs but storing just the set of peer ids (channel name is always deterministically p2p-<hex_prefix>, so nothing else needs storing). At startup, for each persisted peer: net.open_direct(peer) to start reconnecting immediately, app.background_direct_channel(channel,peer) to restore its tab without stealing focus, and store.load+app.load_history to restore history -- mirroring the existing gossip-channel startup loop. Record into the registry wherever a direct tab is newly created (OpenDirect and DirectJoined handling), forget on /leave. Add direct_peers to main.rs's paths_to_wipe_for_pairing list. Still out of scope: syncing known direct peers to a user's other paired devices.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation complete. New: src/direct.rs (Direct connection manager: DIRECT_ALPN, channel_name(), per-peer actor with immediate-connect + capped exponential backoff retry, last-connection-wins handling of inbound hand-offs, one QUIC uni-stream per ChatMessage, AbortOnDrop guard for the receive-loop subtask to avoid a task leak on close/redial) and src/direct_registry.rs (persisted Vec<peer-id> registry mirroring dm_registry.rs). Wired into src/net.rs (NetEvent::DirectJoined, Net::open_direct/send_direct/close_direct, registered on the shared Router). Wired into src/app.rs (direct_channels map, direct_peer_for/activate_direct_channel/background_direct_channel/forget_direct_channel, InputAction::OpenDirect, /dm command with cold-start full-endpoint-id resolution via decode_hex_id, /invite guarded against point-to-point channels, /help updated). Wired into src/main.rs (mod declarations, startup restore loop for persisted direct peers, OpenDirect/DirectJoined handling, Leave branch closing the direct connection + forgetting the registry entry, route_send helper centralizing gossip-vs-direct send routing reused at all net.send(...) call sites, backfill.record_message(s) skipped for direct channels, direct_peers added to paths_to_wipe_for_pairing). Docs: README.md command table and concept.md both updated with a point-to-point messaging section. Verification: cargo build, cargo test (471 passed, including new direct.rs loopback tests for send/receive, reconnect-after-disconnect, and close-then-reconnect), cargo clippy --all-targets (clean), cargo fmt --check (clean except one pre-existing, unrelated line in dm_registry.rs left untouched to keep the diff scoped).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added true point-to-point 1:1 messaging (/dm) alongside the existing gossip-based /msg: a direct QUIC connection to a peer's EndpointId with no gossip topic, RoomSecret, or invite ticket. New src/direct.rs owns the connection lifecycle (per-peer actor, immediate connect with capped exponential backoff retry on disconnect, inbound-connection hand-off, one QUIC uni-stream per ChatMessage) registered on the shared Router under its own ALPN; new src/direct_registry.rs persists known point-to-point peers so their tab, history, and connection attempt all come back automatically on restart. Reuses the existing app::Channel/storage::MessageStore/ui.rs machinery unmodified, so history, search, threads, and presence all work for free. Wired through net.rs (NetEvent::DirectJoined, Net::open_direct/send_direct/close_direct), app.rs (/dm command with cold-start full-endpoint-id resolution, /invite guarded on point-to-point channels), and main.rs (startup restore loop, OpenDirect/DirectJoined handling, /leave teardown, gossip-vs-direct send routing). Verified with cargo test (472 passed), including three new direct.rs loopback network tests using real iroh endpoints (fresh-connection send/receive proving AC1's no-ticket exchange, disconnect-then-reconnect proving AC2's retry/lifecycle handling) and a direct_registry.rs test that persists a peer + message in one simulated session and reloads both in a fresh one exactly as main.rs's startup path does, proving AC3. cargo clippy --all-targets is clean; cargo fmt --check is clean except one pre-existing, unrelated line in dm_registry.rs left untouched. README.md and concept.md document the new command and architecture.
<!-- SECTION:FINAL_SUMMARY:END -->
