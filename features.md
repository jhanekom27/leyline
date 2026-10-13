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
  fetched today. Receiving never downloads anything unasked (bar a channel's opt-in
  `/autosave`, below); a saved
  file lands in the OS Downloads folder (`files.rs`) -- in a folder of its
  channel's, see "Per-channel save folders" below.
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
- [x] **Per-channel save folders** -- every channel used to save into one
  flat `~/Downloads/leyline/`, so files from different channels and people
  mixed together and collided into `name (1).ext`. `/save` now writes into
  `<files root>/<channel folder>/`, made when the first file lands in it;
  `files::resolve_destination`, filename sanitization and the
  `name (1).ext` handling are untouched, they just get a different
  directory. `run_save` works that directory out
  (`InputAction::SaveFile::dir`), so the path stays pure and testable and
  app.rs still does no I/O. The folder is named from the channel's
  *stored* name (`app::Channel::name`, its identity), never its label, so
  `/rename` and `/alias` can't move it (`files::channel_dir`). A stored
  name is not safe path material -- `/join` takes any text, and a ticket
  brings a name chosen by whoever shared it -- and two channels must never
  share a folder, even where `General` and `general` are one name (macOS,
  Windows). So a name that is already a safe folder name everywhere
  (lowercase ASCII letters, digits, `-` and `_`, not too long, not a
  Windows device name such as `con` or `lpt1`) is the folder itself --
  `general`, `project-x` -- and anything else becomes a readable form of
  it, a `~` and 16 hex digits of the blake3 hash of the exact name:
  `Team-Alpha~83f57e5b1e8db868`. A `~` never occurs in a plain name, so
  the two kinds can't meet, and the hash keeps apart whatever differs only
  by case, Unicode form, punctuation or what lies past the cut; the
  readable part is only for the eye. The function is pure and frozen --
  nothing is stored, and changing it would strand folders. A file shared
  in several channels goes to the active channel's folder when that
  carries it, else to the first joined channel that does, and leaving a
  channel never touches its folder.
  The folders live under one files root, `~/Downloads/leyline` unless
  `/folder <path>` has moved it; a bare `/folder` says where the active
  channel's folder is. The root is remembered in a one-line `files_root`
  file in the data directory -- not a new field of the postcard `settings`
  file, for the reason given for the picker's remembered folder -- and
  pairing leaves it alone, being a preference of this device. It is the
  groundwork for the `/files` overlay, auto-save, the outbox and synced
  folders (LEY-25.7, LEY-26), which each need one stable place per
  channel.
- [x] **Files overlay** -- `/files` opens a keyboard-first browser over the
  active channel's shared files, so a file that has scrolled out of view is
  found again without scrolling the transcript or remembering its hash. It
  is `Ctrl+P`'s overlay once more (`AppState::viewing_files`: modal, it owns
  every key and a paste, and `is_composing` is false while it is open) and,
  like it, stores no list: `files_matches` re-derives the rows from the
  channel's loaded transcript on every read -- the very window `/save`
  resolves against (`known_attachments`), so it lists exactly what `/save`
  can fetch, history loaded at startup, backfilled and live alike, and
  shares its `MAX_SCROLLBACK` limit -- newest first, one row per share (a
  file sent twice is two rows), narrowed by a case-insensitive contains on
  the file name or on the sender as shown (`display_name`: a pet name
  matches, the raw id of a named peer doesn't). Besides the query and the
  highlight it keeps the channel it was opened on -- a join finishing
  elsewhere can move focus under it, and `Enter` must save into the folder
  of the list the user was looking at -- and `saved`, what the disk said
  when it opened. `Enter` saves the highlighted file through the same
  `save_request` as `/save`, so providers, folder, notices and failures are
  `/save`'s, and closes the overlay, which would otherwise hide that
  feedback behind it. `Ctrl+O` opens an already saved file with the OS's
  default application. `Enter` on a saved file does not save it again, so a
  file is never saved twice by accident, and the bottom border offers
  whichever of the two the highlighted row can do. A key the row doesn't
  offer -- `Enter` on a saved file, `Ctrl+O` on an unsaved one -- would look
  dead, so it leaves a `FilesNotice` that takes the hints' place in the
  bottom border, in the picker's notice style, until the next key or paste
  (`already saved · Ctrl+O opens it`, `not saved yet · Enter saves it`).
  What the keys can do for a row is worked out once -- `AppState::file_offer`,
  a `FileOffer` -- so the bottom border and both keys agree.
  `Ctrl+O` is the one place leyline makes the OS act on a file a peer chose:
  `/save` only writes bytes, and a file leyline saves has none of the OS's
  usual download caution (checked on macOS: no quarantine flag, only
  `com.apple.provenance`). So a file whose extension means it runs, installs
  or applies something when opened -- programs and installers, scripts,
  shortcuts and launchers -- is saved like any other but never handed over:
  `files::runs_when_opened` checks the saved copy's own name, which is what
  the OS will go by, against `RUNS_WHEN_OPENED`. The list is wider than any
  one system needs, since a peer's file name says nothing about where it
  will be opened. The name is read as written and as Windows reads it
  (trailing dots and blanks dropped, a `:` and what follows taken for a
  stream), so `setup.exe.` and `setup.exe::$DATA` are caught like
  `setup.exe`. Such a row is offered neither key; a press says
  `already saved · won't open programs or scripts from here`, and the file is
  opened by hand from the file manager, if at all.
  "Saved" is a fact about the disk, which app.rs never touches, and nothing
  recorded it. `NetEvent::FileSaved` now carries the channel and the content
  hash, and main.rs writes each finished save into `saved_files.rs` -- a
  postcard file in the data directory, like the DM registry, with one
  record per (channel, hash) -> absolute path. It is keyed by channel
  because every channel has a folder of its own: a file saved from
  `#general` is not saved in `#random`. When the overlay opens, `/files`
  asks (`InputAction::CheckSavedFiles`) and main.rs answers before the next
  frame from what is still on disk (`SavedFiles::present_in`, a stat per
  file saved for that one channel), so a deleted copy shows as unsaved
  again -- and as saved again if it comes back: nothing is forgotten for
  being absent, since a drive that is unmounted right now must not erase
  what is known about what it holds. A save that finishes while the overlay
  is open marks its row at once. Pairing leaves the record alone, like the
  files root. `files::open_in_default_app` runs `open` (macOS), `explorer`
  (Windows) or `xdg-open` with the path as its one argument -- never through
  a shell, since a file's name is a peer's to choose -- with the standard
  streams closed so it can't draw over the TUI, and reaps the launcher on a
  thread; main.rs checks the file is still there first.
  Not covered: files shared further back than the channel's loaded history,
  files saved before this existed (they show as unsaved until saved again),
  and any check of what a file is beyond its extension -- the list is a
  safety net, not a sandbox: a document with macros, an archive or a web
  page still opens as the OS always did. See LEY-25.7's notes.
- [x] **File size limit override** -- `files::MAX_FILE_SIZE` (500 MB, now
  `DEFAULT_MAX_FILE_SIZE`) refused any file or pasted image over it with no
  way around short of editing the constant and rebuilding: a good guardrail
  against an accidental huge share, and as firm a wall for a deliberate one
  (a video, a backup archive). The limit is enforced in exactly two
  functions -- `stage_file`, which every route for a file passes through
  (`/send <path>` by way of `Net::send_file`, the picker, a drop, a
  clipboard file list), and `stage_image` for a pasted image -- and nothing
  re-checks it at send, so it became a parameter of those two
  (`files::SizeLimit`) and every route is handed the one `AppState` holds
  (`size_limit`). `/maxsize` shows it and changes it: `/maxsize 2GB` (a
  number and a unit, `B` to `TB`, 1024 apart as `human_size` prints them,
  any case, a blank allowed), `/maxsize off` to lift it, `/maxsize default`
  to restore the 500 MB. A bare `/maxsize` says what the limit is and how to
  change it, the way a bare `/folder` shows the root, and the refusal says
  the same: `too large (501.0 MB, max 500.0 MB) -- raise the limit with
  /maxsize <size> (say /maxsize 2GB) or lift it with /maxsize off`. A
  persisted setting was chosen over a one-off per-file confirmation:
  staging has four entry points (command, picker, drop, clipboard), a
  confirmation would need modal state in each and a way to say "this file",
  and a setting is one value they all read.
  `SizeLimit::parse` is the one place a limit is validated: no unit, an
  unknown unit, a malformed number, a size that rounds down to nothing (it
  would refuse every file) and one too big to count are refused, saying
  what to type, and leave the limit as it was. The limit is
  `Default | Bytes(n) | Off`, exactly what is remembered: a one-line
  `size_limit` file in the data directory, like `files_root` (not a field of
  the postcard `settings` file, which would fail to decode an existing
  one), with nothing there meaning the default -- so a build that changes
  the default moves everyone who never chose a limit. Text that is not a
  limit falls back to the default, the guarded state and never a looser
  one, while a genuine I/O error fails startup, as for `files_root`. A write
  that fails is reported and the limit still holds for the session. Pairing
  leaves it alone, being a preference of this device.
  Not covered, each left out on purpose for now and worth revisiting if it
  starts to matter: only files you share are limited, never ones you
  receive -- `/save` fetches whatever a peer announced, whatever its size,
  and a receive-side guard (refuse, or confirm, above some size) is not
  built. A pasted clipboard image answers to the same limit, checked once it
  is encoded as PNG in memory, and under `off` that check goes too. The
  limit is checked when a file is staged (or `/send`ed), not again when it
  goes out -- `import_staged` trusts staging, so a file that grows between
  the two is not re-checked, as it never was. It is one limit for the whole
  device, not one per channel, and there is no one-off confirmation for a
  single file (a `/send --force`, say, or a confirm on `Enter`): a limit
  raised for one deliberate send stays raised until `/maxsize default`. A
  size needs its unit (a bare `5` is refused) and only `B` to `TB` are read,
  1024 apart -- no `G`, `MiB` or the like. See LEY-25.8's notes.
- [x] **Opt-in auto-save** -- receiving never downloads anything by
  itself, on purpose: any channel member could otherwise fill your disk.
  For a channel you trust, such as a private one between your own devices,
  the manual `/save` is pure friction, so `/autosave on` makes it
  unnecessary -- for that channel only, and off everywhere to begin with.
  Which channels is a small `autosave` file in the data directory
  (`autosave.rs`: a list of stored names, mirroring `channel_renames.rs`);
  a corrupt one means off everywhere, the safe way to fail. A channel's
  setting is forgotten when it is left -- main.rs does that first in the
  leave sequence, so a crash midway leaves it off rather than stale -- and
  by pairing's wipe, because a ticket's name is chosen by whoever shares
  it and a setting that outlived its channel would switch downloads on in
  a different room that reuses the name.
  `AppState::auto_save` decides, and main.rs calls it for exactly one
  thing: a message newly accepted off the network, in the live arm of the
  event loop. History loaded at startup and history backfilled from a peer
  never reach it, so a file shared before auto-save was on, before this
  device joined, or while it was away is never fetched on its own, and
  turning it on is not retroactive. "After it was enabled" is judged by
  arrival, never by the message's timestamp, which its sender writes. A
  message without a file, a file of our own, and any file in a channel
  with auto-save off are none of its business and it says nothing.
  Who may have a file saved without being asked is one predicate,
  `is_trusted_peer`: one of your own devices (certified under your own
  user key, which `display_name` already shows as "you"; a device
  forgotten with `/forget-device` is not) or a peer you have given an alias
  (which covers every device of that person). It is used twice, and the
  second use is the one that matters. Nothing signs a chat message --
  neither `net::forward_events` nor `direct::receive_loop` checks
  `ChatMessage::sender` against anything -- so anyone in a channel can
  claim to be your other device or an aliased friend. The claimed sender
  therefore only decides whether to try at all, and to say why not; the
  file is then asked only of peers that are themselves trusted, who are
  known by their connection (iroh authenticates the endpoint id) rather
  than by what a message says, and its bytes are checked by their hash.
  A forged share leads to asking the real devices, which don't have the
  blob, and fails.
  The size is checked twice. A file announced as over the persisted,
  device-wide `autosave::AutoSaveLimit` (25 MB by default -- not `/maxsize`, which limits
  what you share) is skipped before any network is touched; and because
  the announced size is only a number in a message,
  `BackfillStore::fetch_file` reads the real one too: a blob already in
  the store is measured there, and for any other the providers are asked
  in turn for its verified size (`iroh_blobs::get::request::get_verified_size`,
  which the protocol checks against the hash, so it can't be a lie)
  before the downloader runs. Over the limit, nothing is downloaded, or
  exported.
  Both kinds of save go through one `SaveRequest` and main.rs's
  `start_save` (claim a name with `files::resolve_destination`, hand over
  to `Net::save_file`), and differ only in `files::SaveMode`: a manual
  save is unbounded and answered in the channel you are in, as it always
  was; an automatic one is bounded and answered in the channel its file
  arrived in, wherever you are looking -- which is why `FileSaveFailed`
  now names its channel, and what keeps a background channel's notice off
  the one on screen. Sanitizing, `name (1).ext` and the record behind the
  `/files` check mark are therefore the very same code as `/save`'s.
  What it says goes to the file's channel, and each skip says why and
  points to the `/save` that fetches the file by hand: `not auto-saving X
  from <who>: auto-save only takes files from your own devices and the
  peers you have aliased`, `not auto-saving X (80.0 MB): over the 25.0 MB
  auto-save limit`, `not auto-saving X: this channel already has a saved
  copy of it`; a file that landed says `auto-saved X to <path>`, a failure
  `couldn't auto-save X: <why>`. There is no "saving..." line. A repeat is
  dropped rather than saved again: a file the channel already holds
  (`SavedFiles::is_saved`, asked by main.rs through a closure so app.rs
  still touches no disk) or one still downloading (`auto_saving`, in
  memory, cleared when it lands, fails or the channel is left) -- a
  re-sent or replayed share is a new message, and would otherwise start a
  download, and leave a `name (n).ext`, per copy.
  Not covered, each left out on purpose for now: the limit is fixed, not
  a setting (`/maxsize`'s machinery would do for one); there is no cap on
  the total, so many files each within the limit still add up, held back
  only by who is trusted and the repeat check; the sender of a chat
  message is still unauthenticated, which this works around for downloads
  but not for anything else, and a forged share of a file only a stranger
  has just produces a failure notice; a sibling device's first file can
  arrive before its device certificate does (gossip has no ordering), in
  which case it is skipped, saying why, and `/save` fetches it; and
  nothing is ever opened -- only `Ctrl+O` in the files overlay does that,
  and auto-save never presses it. LEY-9 (verified identities) and LEY-10
  (block a peer) are meant to feed `is_trusted_peer`: a blocked peer
  should never be auto-saved. See LEY-26.1's notes.

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
