# Feature ideas

A running scratchpad of features under consideration, beyond what's already
shipped -- see the [README](./README.md) roadmap for that. Nothing here is
committed to; it's a place to capture ideas before deciding what's worth
building next.

## Identity & aliases

The most requested gap: peers only ever show up as a truncated hex id
(`app::hex_prefix`, via `AppState::display_name`) -- fine for debugging, bad
for knowing which of your friends just said something. Two complementary
mechanisms, not one, since they trade off differently:

- [x] **Local petnames** -- `/alias <hex-prefix> <name>` assigns a name to a
  specific endpoint id yourself. Stored locally only (a new `contacts.rs`,
  mirroring `channel_registry.rs`'s persisted-postcard-file pattern), never
  sent over the wire. Not spoofable, since only you control it -- fits
  leyline's existing capability-based trust model (concept.md's "Room
  privacy") instead of fighting it. Does nothing for a peer you haven't met
  yet.
- [x] **Broadcast nicknames** -- `/nick <name>` announces a chosen display
  name to every joined channel, as a new `GossipPayload::Identity { sender,
  nickname }` variant sent alongside `Chat`/`Announce`, re-sent on
  `NeighborUp` the same way `HistoryAnnounce` already is. Convenient for
  peers you haven't petnamed, but spoofable -- nothing stops two peers both
  claiming "alice" -- so it should supplement the id, never fully replace
  it (e.g. `alice (a1b2)` until pinned locally).
- [x] Resolution order in `display_name`: local petname if set, then the
  last-seen broadcast nickname, then the `hex_prefix` fallback it already
  has today.

## Messaging

- [ ] **Replies** -- optional `reply_to: Option<u64>` on `ChatMessage`,
  rendered as a quoted snippet. Message ids are already random/unique
  (concept.md's wire format section), so this is a small,
  backward-compatible addition behind a `v` bump.
- [ ] **Edit / delete own messages** -- a new envelope variant referencing
  the original `id`, accepted only if `sender` matches. Tombstone as
  "(deleted)" rather than actually removing, so dedupe/backfill never have
  to reason about holes in history.
- [ ] **Reactions** -- lightweight emoji-react-to-message-id broadcasts,
  rendered inline under a message. Same envelope-extension shape as
  replies.
- [ ] **@mentions** -- highlight your own name when it appears in a
  message, plus a bell/notification when that happens on a tab that isn't
  active.
- [ ] **Local search** -- `/search <term>` to filter/jump within a loaded
  channel's scrollback; a stretch version re-scans `storage.rs`'s on-disk
  log for history older than what's currently loaded.

## Channels & presence

- [x] **`/leave [channel]`** -- leaves the active channel, or a named one
  if given. Drops the channel's gossip subscription (`net.rs`), forgets it
  in `channel_registry.rs`, and also clears its local message log
  (`storage.rs`) and backfill manifest (`backfill.rs`), so a later rejoin
  under the same display name never inherits an unrelated room's history.
  Refuses to leave your only remaining channel.
- [ ] **`/who`** -- list full endpoint ids (plus any alias) for the active
  channel's peers; today the header only shows a count and the sidebar
  only a 4-byte hex prefix.
- [ ] **Last-seen timestamps** -- presence is purely ephemeral today
  (`NeighborUp`/`NeighborDown`, concept.md's "Presence" section).
  Persisting "last seen at T" per peer per channel would let offline
  friends stay listed (greyed out) instead of just disappearing.
- [ ] **Channel topic/description** -- a short, gossiped metadata string
  per channel, shown under the tab bar.

## Files & rich content

- [ ] **File sharing** -- `iroh-blobs` is already wired up for history
  backfill (`backfill.rs`); the same blob transport generalizes to
  `/send <path>`, broadcasting a content hash + filename that peers fetch
  on demand, the same way a history manifest is fetched today.
- [ ] **Clipboard paste-to-share** -- extend the existing `arboard`
  integration (currently only used to copy outgoing invite tickets) to
  accept incoming paste too, e.g. pasting an image shares it as a blob.

## Trust & safety

- [ ] **Out-of-band fingerprint verification** -- a "safety number"-style
  comparison (a la Signal) so two people can confirm an `EndpointId`
  actually belongs to who they think it does, since anyone who intercepts
  a ticket exchange could otherwise impersonate the sharer.
- [ ] **Mute/block a peer** -- client-side filter on a specific endpoint
  id's messages, stored alongside contacts; no network changes needed.
- [ ] **Document channel-secret rotation as the "kick" story** -- a pure
  capability/gossip model has no real ban mechanism (anyone holding the
  `RoomSecret` can always rejoin), so the actual answer to "remove a
  compromised member" is: generate a fresh `RoomSecret` and re-invite
  everyone else. Worth writing down explicitly rather than later trying to
  bolt on a ban-list gossip can't enforce.

## Networking & sharing

- [ ] **Direct 1:1 DMs** -- either (a) auto-provision a private 2-person
  channel through the existing ticket flow behind a friendlier
  `/msg <alias>` command, or (b) a true direct QUIC stream to a known
  `EndpointId` that bypasses gossip entirely. (b) is a bigger departure
  from today's "one gossip task per channel" model, worth prototyping
  separately.
- [ ] **QR-code invite tickets** -- render a ticket as an in-terminal ASCII
  QR code for scanning from a phone, instead of copy/pasting a long base32
  string.
- [ ] **`leyline://` deep links** -- a URI scheme so a ticket shared over
  email/Slack/etc. can be clicked to auto-join, instead of manual
  copy-paste into `/join`.

## UX polish

- [ ] **Command history & tab-completion** -- up-arrow through previously
  submitted commands/messages; tab-complete `/join`, `/invite`, and (once
  they exist) aliases.
- [ ] **Mouse support** -- `crossterm` already supports mouse events; click
  to switch tabs, scroll wheel for scrollback instead of only `Up`/`Down`.
- [ ] **Configurable theme** -- pull the hardcoded colors in `ui.rs` out
  into a small config file.
