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
- [x] **Staged attachments** -- `Ctrl+V`/`/paste` no longer broadcasts a
  clipboard file or image the instant it is read: it lands in a
  per-channel staging area (`Channel::staged`), drawn as a strip above
  the input box with each file's name and size. `Enter` sends everything
  staged, the typed text becoming the caption of the first -- a
  `ChatMessage` already carried both `text` and `attachment`, so no wire
  change, and the transcript now shows a caption above the attachment
  line. `Backspace` on an empty input takes the newest back out and `Esc`
  clears the lot (after an armed reply, before leaving a thread or
  quitting). Clipboard text still sends at once, `/send <path>` stays an
  explicit send-now, and a `/command` typed while files are staged runs
  as a command and leaves them staged. One validation boundary
  (`files::stage_file`/`stage_image`) serves both `/send` and staging, so
  what each refuses -- and how it says so -- can't drift apart. It is the
  one place the other ways of choosing a file (drag-and-drop, a picker)
  are meant to land.
- [x] **Drag-and-drop** -- a terminal has no drop protocol: dropping a file
  onto its window just pastes the file's path, which leyline used to type
  into the message box as text. `Event::Paste` now goes through main.rs's
  `handle_paste`, which asks `files::dropped_files` whether the paste is
  nothing but existing files and, if so, stages them on the active channel
  exactly like a `Ctrl+V` of a copied file. Terminals spell the path
  differently -- backslash-escaped (Terminal.app, iTerm2, Ghostty), quoted
  (iTerm2's option, GNOME Terminal, WezTerm's Posix mode), as `file://`
  URIs, newline-joined, or raw (kitty) -- so the paste is read as shell
  words first and as one raw path per line second. The decision is all or
  nothing with the filesystem as arbiter: every path must be absolute and
  an existing regular file, or the whole paste is ordinary text, so a
  sentence that mentions a path, a relative name like README.md, a
  directory, or a missing path is never swallowed. Words split on ASCII
  whitespace only, so a macOS screenshot name with a narrow no-break space
  stays one path. A drop is not staged while the switcher, a pick or a
  pairing confirmation owns input (`AppState::is_composing`).
  Not every terminal pastes the path, though: Warp writes the escaped paths
  to the pty as if they were typed -- no bracketed-paste markers, unlike its
  `Cmd+V` -- so all leyline sees is a key press per character. The event
  loop reads the terminal through `term_events::TermEvents`, a wrapper over
  crossterm's `EventStream` that gathers the key presses that arrived
  together and, when they spell nothing but existing files, hands them out
  as the one `Event::Paste` a bracketing terminal would have sent. Typing a
  path by hand never qualifies: one key arrives at a time, and the files
  must exist besides.
- [x] **File picker** -- a bare `/send` opens a keyboard-driven picker in
  place of the exact path `/send <path>` wants, drawn over everything like
  the `Ctrl+P` switcher. It lives inside the TUI rather than being a GUI
  dialog, so it works over SSH and on Linux. `picker::FilePicker` is pure
  state, like `ThreadView`/`SearchResults`, owned by
  `AppState::picking_file`; `picker::read_folder` is the one piece of I/O
  and runs on tokio's blocking pool, answering through its own channel the
  way `/search`'s scan does, so a huge or slow folder never holds up a key:
  the picker says "reading...", still answers `Esc`, and ignores an answer
  that is not the one it is waiting for (a slower earlier read, or one for
  a picker since closed) by request id. It lists folders first, by name,
  then files newest first, leaving out dot-files. `Up`/`Down` move; `Enter`
  opens a folder or, on a file, stages the marked files (the highlighted
  one if none are marked); `Space` marks, so the filter -- typing narrows
  the folder by name, ignoring case -- can't hold a space; `Backspace`
  edits the filter and, once it is empty, goes up a level; `Esc` cancels.
  What is chosen is staged like anything else (`files::stage_file`), on the
  channel the picker was opened on. It opens where files were last chosen
  from, kept across restarts in a one-line `browse_dir` file in the data
  directory -- not a new field of the postcard `settings` file, which
  would fail to decode an existing one and silently reset the saved
  nickname and bell setting -- falling back to Downloads, then home. An
  unreadable folder shows the reason in the bottom border and leaves the
  picker where it was.
- [x] **Bare `/save`** -- `/save` with no hash prefix fetches the newest
  file another peer shared in the active channel, the way a bare `/reply`
  answers the most recent message, so the common case -- grabbing the file
  that was just shared -- no longer means copying a hash prefix out of the
  transcript. "Another peer" is anyone but this device, the test
  `ui::attachment_line` already uses to give a file its `/save <hash> to
  download` hint, so a bare `/save` takes exactly the newest file that
  shows one; that includes a file from one of your own paired devices,
  which is not on this one. It reads the channel's transcript rather than
  what is in view (an open thread or search narrows what `/reply` can
  target, but the file just shared belongs to the channel), and says so
  when there is no such file. It then goes through the same
  `InputAction::SaveFile` as `/save <hash-prefix>`, so the download, the
  destination notice and the failures are the ones that path always had.
  An older file stays with the hash prefix rather than a second picker
  mode.
- [x] **Fetch from any channel peer** -- `/save` used to dial only the peer
  who shared a file, so it failed whenever that peer was offline, even
  though anyone who had already saved the file could have served it. Peers
  serve files out of their own `iroh-blobs` store, never out of the
  Downloads folder: `/send` imports a copy into the store
  (`ImportMode::Copy`), and `/save` downloads into it first and only then
  exports a copy (`ExportMode::Copy`), so a file stays available from
  whoever has it even after they move or delete what they saved. The store
  is never garbage-collected (`FsStore::load` configures none), so a saved
  file stays servable. Nothing on the wire changed.
  `/save` now hands the downloader every peer it knows
  (`AppState::providers_for`), best bet first: the sender, then the peers
  online in the channels that carry the file, then everyone who has posted
  there -- gossip only keeps a handful of direct neighbors, so someone who
  has the file may be online without being one. Never this device, except
  as the sender, whose file is already in our own store
  (`BackfillStore::fetch_file` looks there before it asks anyone). The
  downloader asks one at a time and moves on from one that can't be
  reached (at most its 1s connect timeout) or doesn't have the file, so
  only a save nobody can serve fails, and it says `no reachable peer has
  it right now -- its sender may be offline` instead of a raw network
  error. A provider id comes from a message, so one that isn't a valid key
  is skipped rather than failing the save for everyone else.

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
