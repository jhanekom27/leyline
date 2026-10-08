# Feature ideas

A running scratchpad of features under consideration, beyond what's already
shipped -- see the [README](./README.md) roadmap for that. Ideas that have
graduated into committed work are tracked as tasks in Backlog.md instead
(`backlog board` or `backlog task list --plain`); this file stays a place to
jot brand-new ideas before they're promoted into a task, plus a historical
record of what's already shipped.

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
- [x] **Persist the broadcast nickname** -- the last value set via `/nick`
  is now saved in `settings.rs` (mirroring `bell_enabled`) and reseeded
  into `Net::nickname` on the next launch, so peers see it again as soon
  as a channel gains a neighbor -- the same way `Net::announce_nickname`
  already re-announces on `NeighborUp` -- without it needing to be
  retyped every session.
- [x] Resolution order in `display_name`: local petname if set, then the
  last-seen broadcast nickname, then the `hex_prefix` fallback it already
  has today.
- [x] **Update notification** -- every instance broadcasts its own build
  version (`Cargo.toml`'s semver) and git commit (stamped in at compile
  time by a new `build.rs`, via `crate::version`) as a
  `GossipPayload::Version`, re-sent on `NeighborUp` the same way
  `Identity`/`Announce` are. The first time a peer's recorded version
  becomes newer than ours, a one-time system notice fires and the TUI
  header shows a durable "update available" indicator for the rest of the
  session; `/who` also shows a peer's version, and a new `/version`
  command (plus a `--version` CLI flag) reports our own. Fully
  peer-to-peer, no release server involved -- consistent with leyline's
  "no servers, no central relay" pitch, at the cost of only ever being
  able to say "someone I've talked to is ahead of you," never "you're
  definitely current."

## Messaging

- [x] **Replies** -- optional `reply_to: Option<u64>` on `ChatMessage`,
  rendered as a quoted snippet. Message ids are already random/unique
  (concept.md's wire format section), so this is a small,
  backward-compatible addition behind a `v` bump. Since the transcript has
  no existing per-message selection UI, picking a target is a dedicated
  `Ctrl+R` mode: `Up`/`Down` highlight a candidate message (by id, not row,
  so it survives rewrapping), `Enter` arms it, `Esc` cancels. A `/reply
  [text]` command covers the common "reply to the most recent message"
  case without picking.
- [x] **Local search** -- `/search <term>` (or its `/s` shorthand) filters
  the active channel down to matches (with the matched text highlighted),
  instantly for whatever's loaded; a `tokio::task::spawn_blocking` scan of
  `storage.rs`'s full on-disk log then extends the result with history
  older than what's loaded, without ever blocking input/render. The pane
  title reminds you a search is active and that `/search`/`/s` with no
  argument clears it.
- [x] **Message bell** -- an incoming message rings the terminal bell
  (ASCII BEL, `main.rs`'s `ring_bell`), regardless of which channel or
  terminal tab is focused, so you notice leyline without watching the
  TUI. `/bell` toggles it, persisted via a new `settings.rs` (mirrors
  `contacts.rs`'s pattern) so the preference survives restarts.
- [x] **Multi-line input** -- `Alt+Enter`/`Ctrl+J` inserts a literal
  newline instead of sending, so a longer, spaced-out message can be
  composed before pressing `Enter`; the input box grows to fit (and
  scrolls, past `MAX_INPUT_VISIBLE_LINES`). `Home`/`End`/`Ctrl+U`/`Ctrl+K`
  act on the current line rather than the whole message, matching
  standard multi-line editors (`Channel::current_line_bounds`).
- [x] **Per-channel input drafts** -- `input`/`cursor` live on `Channel`
  itself, not `AppState`, so half-typed text in `#general` stays put and
  keeps its cursor position when you `Tab` to `#random` and back, instead
  of following you to whichever channel is active when you press `Enter`.
  The editing keys/commands (`insert_char`, `delete_backward`, etc.) moved
  to `Channel` along with the fields, since they only ever touch those
  two; `replying_to` stays scoped on `AppState` exactly as before.
- [x] **Markdown rendering** -- headings (`#`/`##`/`###`), fenced code
  blocks, inline code, bold (`**`), italic (`*`/`_`), unordered lists
  (`-`/`*`/`+`), ordered lists (`N.`), and blockquotes (`>`, nested via
  repeated `>`) typed into a message render as styled output in the
  transcript (a small hand-rolled renderer, `markdown.rs`, not a
  CommonMark parser), rather than showing as literal source; a list
  item's or blockquote's marker gets a hanging indent so wrapped
  continuation lines still line up under the text. Search-term
  highlighting still applies on top of that styling
  (`ui::highlight_spans`). Deliberately minimal for now -- no nested
  lists, tables, links/images, or code syntax highlighting.

## Channels & presence

- [x] **`/leave [channel]`** -- leaves the active channel, or a named one
  if given. Drops the channel's gossip subscription (`net.rs`), forgets it
  in `channel_registry.rs`, and also clears its local message log
  (`storage.rs`) and backfill manifest (`backfill.rs`), so a later rejoin
  under the same display name never inherits an unrelated room's history.
  Refuses to leave your only remaining channel.
- [x] **`/who`** -- lists the active channel's peers with their full
  endpoint id, broadcast nickname, and local alias, each shown
  explicitly (unlike `display_name`'s blended, one-string precedence
  order used elsewhere); previously the header only showed a count and
  the sidebar only a 4-byte hex prefix.

## Files & rich content

- [x] **File sharing** -- `iroh-blobs` is already wired up for history
  backfill (`backfill.rs`); the same blob transport generalizes to
  `/send <path>`, broadcasting a content hash + filename that peers fetch
  on demand (`/save <hash-prefix>`), the same way a history manifest is
  fetched today. Receiving never downloads anything automatically; a saved
  file always lands in the OS Downloads folder (`files.rs`).
- [x] **Clipboard paste-to-share** -- `/paste` (or the `Ctrl+V` keybinding)
  extends `arboard` (previously write-only, for `/invite`'s copy) to read
  the clipboard too: a real file (e.g. a Finder/Explorer copy, sharing its
  exact original bytes via the same path as `/send` -- the only way an
  animated GIF survives intact) if there is one, else rendered image
  pixels re-encoded as PNG (a screenshot, or a browser's "Copy Image" --
  always a single static frame, since no OS clipboard image format
  carries multi-frame/animation data), else plain text shared as a chat
  message. Also enables crossterm's bracketed paste mode so an ordinary
  terminal text-paste is inserted as one atomic edit instead of a burst of
  key events, fixing a premature-submit bug on multi-line pastes
  (`AppState::paste_text`).

## Networking & sharing

- [x] **Direct 1:1 DMs** -- `/msg <alias-or-hex-prefix> [text]` (option
  (a) from this idea's original framing): auto-provisions a private
  2-person channel through the existing ticket flow, rather than
  requiring a bare `/join <new-name>` + remembering which tab is "the
  DM with Alice". A small new `dm_registry.rs` (mirrors
  `channel_registry.rs`'s pattern) persists peer id -> DM channel name,
  consulted by `/msg` to switch to an already-joined DM instead of
  creating a second one. `ChannelTicket` gained a `dm` marker (versioned
  decode, like `ChatMessage`) so the *recipient* of a `/msg`-created
  invite also records the association on their side, keeping both
  parties converged on one channel. First contact with a brand-new peer
  still needs one manual out-of-band ticket share, same as any new
  channel -- inherent to concept.md's "Room privacy" guarantee that a
  topic must never be derivable from public info alone.
