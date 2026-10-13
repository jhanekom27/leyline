# leyline

A peer-to-peer terminal chat app. No servers, no accounts, no central relay
to run -- your identity is a local keypair, "channels" are private gossip
topics (each with its own randomly-generated key, not a guessable hash of
its name), and messages travel directly between peers using
[iroh](https://github.com/n0-computer/iroh) + iroh-gossip, all inside a
[ratatui](https://ratatui.rs) TUI.

## Status: private channels, with history backfill for offline peers

`leyline` channels are private by default: creating or first joining one
(by name or ticket) generates a random secret that -- not the name you
typed -- actually determines its gossip topic, so a channel is only
reachable by someone you've handed a ticket to, and two people who happen
to pick the same name never end up in the same swarm. `leyline` also
backfills channel history for peers that were offline (or join a channel
fresh): whenever a channel gains a gossip neighbor, and again periodically
regardless of neighbor churn, every peer floods its current history's
root hash -- an `iroh-blobs` content manifest -- to the whole channel,
and whoever's behind automatically fetches the messages they're missing
directly from whoever announced them, no manual command needed -- not
just from whichever peer you happened to connect through. Combined with
local persistence, multi-channel support (one gossip task per joined
topic, a left-hand panel to switch between them, and a presence sidebar
scoped to whichever channel is active), a couple of things are still
deliberately minimal:

- **Identity persists across runs** -- your keypair is generated once and
  saved to disk, so your endpoint id stays the same every time you start
  `leyline` (see [Build & run](#build--run) for how this affects local
  multi-instance testing).
- **A channel's name is just a local label, not a credential** -- what
  actually gates entry is its random secret, carried inside its invite
  ticket. Guessing or reusing a name -- even `#general`, which every fresh
  install starts with its own private copy of -- never joins you to
  someone else's channel.
- **Channel membership and known peers persist too** -- every channel you
  join (by name or ticket) and the addresses of peers you've talked to
  are remembered on disk, so restarting `leyline` rejoins all of them and
  reconnects automatically, without re-pasting an invite ticket. An
  invite ticket itself is one-directional -- it bundles *your* current
  address so someone else can bootstrap onto you -- so pasting your own
  ticket back in after a restart can't reconnect you to anyone; it's
  detected and ignored in favor of previously-known peers for that
  channel.
- **Backfill only covers channels you've joined** -- it rides each
  channel's existing gossip topic, so you still need a name or ticket to
  join a channel before its history can sync to you.

See [Roadmap](#roadmap) below for how it got here.

## Requirements

- A recent stable Rust toolchain with edition 2024 support (Rust 1.91 or newer -- iroh-gossip's current MSRV)
- [Backlog.md](https://github.com/MrLesk/Backlog.md) for task management -- `brew install backlog-md` (see [Task management](#task-management))

## Build & run

```sh
cargo build --release
cargo run
```

Run `cargo run -- --version` (or `-V`) to print the build's version and
git commit and exit immediately, without touching disk or the network.

On startup, `leyline` prints an invite ticket for each channel it joined
(before the TUI takes over -- just "#general" by default, and private to
this instance alone until you share it) and shows your endpoint id in the
TUI header (`you: <hex id>`). To have a second instance join the same
channel and talk to the first, run it the same way in another terminal:

```sh
cargo run
```

then paste that printed ticket into its message box:

```
/join <ticket printed by the first instance>
```

The ticket bundles the channel's name, its private room secret, and the
sharer's address (including relay/direct-address hints), so pasting it is
enough to connect -- no separate directory service needed, and no one who
only knows (or guesses) the channel's name can join, since the name never
determines the swarm. Connecting still relies on iroh's default relay
servers for NAT traversal, so all instances need outbound internet access.

Once running, join or create additional channels from the message box with
`/join <channel-name-or-ticket>`: a bare name (e.g. `/join project-x`)
creates a brand-new, private channel under that name locally -- share its
ticket with others via `/invite` -- while pasting someone else's ticket
joins their swarm directly.

Every joined channel is listed in the panel down the left, in two
sections: **CHANNELS** for group channels (`#`) and **PEOPLE** for your 1:1
conversations (`@` for a `/msg` DM, `⚡` for a `/dm` point-to-point chat),
each in the order you joined them. The active one is highlighted, and a `*`
marks another with unread messages; the panel scrolls once the list is
taller than the terminal. Switch between channels with `Tab` / `Shift+Tab`,
which step down / up the panel and wrap around, or press `Ctrl+P` to jump
straight to one by typing part of its name.

Your identity now persists in your OS config directory, keyed by your `$HOME`.
Chat history persists the same way in your OS data directory -- one log
file per joined channel, so scrollback survives a restart.
Running two instances under the *same* `$HOME` on one machine loads the
*same* identity for both -- and iroh rejects a peer connecting to itself --
so for local multi-instance testing, give each instance its own fake home,
e.g.:

```sh
HOME=/tmp/leyline-a cargo run
HOME=/tmp/leyline-b cargo run
```

...then `/join` the first instance's printed `#general` ticket from the
second, as above.

Each fake `$HOME` also redirects Cargo's own cache, so the first build under
a new one re-fetches and rebuilds every dependency. To avoid that, capture
your real cargo home first and pass it through explicitly:

```sh
REAL_CARGO_HOME="$HOME/.cargo"
HOME=/tmp/leyline-a CARGO_HOME="$REAL_CARGO_HOME" cargo run
HOME=/tmp/leyline-b CARGO_HOME="$REAL_CARGO_HOME" cargo run
```

## Keybindings

| Key | Action |
| --- | --- |
| Type | Insert a character at the cursor |
| `Left` / `Right` | Move the cursor one character (across a line break too, in a multi-line message) |
| `Home` / `Ctrl+A` | Jump to the start of the current line |
| `End` / `Ctrl+E` | Jump to the end of the current line |
| `Backspace` / `Delete` | Delete the character before / at the cursor (on an empty input, `Backspace` instead takes the most recently staged attachment back out -- see `Ctrl+V`) |
| `Ctrl+U` | Delete from the start of the current line to the cursor |
| `Ctrl+K` | Delete from the cursor to the end of the current line |
| `Ctrl+W` | Delete the word before the cursor |
| `Alt+Enter` / `Ctrl+J` | Insert a newline in the message box without sending, for a longer, spaced-out message -- the box grows to fit (and scrolls, past a handful of lines) |
| `Enter` | Send the composed message, or run a `/` command. With attachments staged (see `Ctrl+V`) it sends them all instead, the typed text becoming the caption of the first -- but a `/` command still runs as a command and leaves them staged |
| (paste, e.g. `Cmd+V`/`Ctrl+V`) | Paste text from the terminal as one atomic edit, so a multi-line paste can't prematurely send a partial line or trigger a stray `/command` -- unless the paste is really a dropped file (see the next rows) |
| `Ctrl+V` | Share whatever's on the OS clipboard (a file, an image, or text) with the active channel -- same as `/paste`. A file or image is *staged* in a strip above the input box (name and size) rather than sent at once: `Enter` sends it, `Backspace` on an empty input removes the newest, `Esc` clears them all, and the staged files stay with their channel when you switch tabs. Plain text is sent at once. `Cmd+V` also works if your terminal forwards it, but most (including this one) reserve it for their own text-paste above |
| (drag a file onto the window) | A terminal has no drop protocol: it pastes the dropped files' paths, and leyline recognizes a paste made up of nothing but existing files -- absolute paths, backslash-escaped, quoted, or `file://` URIs, separated by spaces or newlines -- and *stages* them on the active channel exactly like `Ctrl+V` of a copied file, instead of typing the path into the box. It is all or nothing: a sentence that mentions a path, a relative name, a directory, or a path that doesn't exist is pasted as text as usual, and so is anything pasted while the `Ctrl+P` jump-to list, a pick or a pairing confirmation has input. A file that exists but can't be staged (unreadable, over the size limit) is reported and skipped. Works whether the terminal pastes the path (through bracketed paste, which leyline turns on) or, like Warp, types it out as ordinary keys: keys that arrive together and spell nothing but existing files count as a drop, while typing a path by hand never does |
| `Ctrl+R` | Pick a message to reply to -- `Up`/`Down` move the highlight, `Enter` confirms, `Esc` cancels |
| `Ctrl+T` | Pick a message to view its reply chain in isolation -- `Up`/`Down` move the highlight, `Enter` confirms, `Esc` cancels |
| `Up` / `Down` | Scroll the message history (or move the highlight while picking a reply or thread target, see `Ctrl+R`/`Ctrl+T`) |
| `Tab` / `Shift+Tab` | Switch to the next / previous channel in the left-hand panel (group channels first, then conversations), wrapping around |
| `Ctrl+P` | Jump to a channel or conversation by name -- type to narrow the list to names containing what you typed (case-insensitive), `Up`/`Down` move the highlight, `Enter` switches to it, `Esc` cancels and leaves you where you were |
| `Esc` / `Ctrl+C` | Close the `Ctrl+P` jump-to list, or cancel an in-progress reply or thread pick, or an armed reply, first, if any; otherwise clear any staged attachments; otherwise leave an open thread view if one is active; otherwise quit |

## Commands

Typed into the message box and submitted with `Enter`:

| Command | Action |
| --- | --- |
| `/join <channel-name>` | Join or create a channel by name, then switch to it |
| `/join <ticket>` | Join the channel named in a pasted invite ticket |
| `/invite` | Show the active channel's invite ticket in the transcript (and copy it to your clipboard, if one is available) to share with others |
| `/msg <alias-or-hex-prefix> [text]` | Message a specific peer 1:1: switches to (or auto-creates) a private channel just for the two of you, listed under PEOPLE as `@<name>` where `<name>` is the peer's current alias, nickname, or hex prefix (it updates as soon as you `/alias` them); share its invite ticket the same way as any new channel so they can join it too. With `[text]`, sends it immediately once the channel is ready |
| `/dm <alias-or-hex-prefix-or-endpoint-id> [text]` | Message a peer directly over a point-to-point QUIC connection -- no gossip channel, room secret, or invite ticket, unlike `/msg`. Switches to (or auto-creates) a conversation listed under PEOPLE as `⚡<name>`, named the same way as a `/msg` DM so it follows `/alias` too; accepts a full endpoint id for a peer you've never shared a channel with, as long as you already know their id. Reconnects automatically on disconnect, and the conversation, its history, and its connection attempt all come back after a restart. With `[text]`, sends it immediately once connected |
| `/leave` | Leave the active channel |
| `/leave <channel-name>` | Leave a specific joined channel without switching to it, by the name shown for it in the left-hand panel (a DM or point-to-point chat shows the peer's current name). Add its badge -- `#general`, `@Alice`, `⚡Alice` -- to pick between a `/msg` DM and a `/dm` chat with the same person, which share a name |
| `/rename <name>` | Give the active group channel a local display name -- to shorten a long one, or tell two similar ones apart -- shown everywhere the channel is named and kept across restarts. Local only: peers still see the original name, and so does any invite ticket. With no name, restores the original. Refused on a DM or point-to-point chat (use `/alias` for those) and for a name another channel already goes by. Leaving the channel forgets its name |
| `/who` | List the active channel's peers, showing each one's full endpoint id, broadcast nickname (if any), and local alias (if any) |
| `/send` | Open a keyboard-driven file picker -- no path to type, and it works over SSH since it lives inside the terminal UI. It opens in the folder you last chose files from (remembered across restarts; your Downloads folder to begin with) and lists folders first, then files newest first, with hidden files left out. `Up`/`Down` move; `Enter` opens a folder or, on a file, *stages* the marked files (just that one if none are marked) exactly like `Ctrl+V` of a copied file; `Space` marks and unmarks the highlighted file, and marks are kept as you move between folders; type to narrow the folder by name, ignoring case (so the filter has no spaces); `Backspace` deletes a typed character and, once none are left, goes up a level; `Esc` closes it without staging anything. A folder that can't be opened shows the reason and leaves the picker where it was, and a huge or slow folder is read in the background, so keys keep working while it loads |
| `/send <path>` | Share a local file with the active channel right away (unlike `/paste` or the picker, nothing is staged first); peers see its name and size and can fetch it with `/save`, never automatically |
| `/save` | Download the newest file another peer shared in the active channel (one you shared yourself doesn't count) to your Downloads folder, in a `leyline` subfolder -- no hash to copy out of the transcript, much as `/reply` needs no message picked. Says so if there is no such file; use `/save <hash-prefix>` for an older one. Fetched the same way as `/save <hash-prefix>`, so it doesn't need the sender online either |
| `/save <hash-prefix>` | Download a file previously shared in any joined channel to your Downloads folder (in a `leyline` subfolder). The sender doesn't have to be online: it is asked first, then every other peer known in that channel, and anyone who already saved the file serves it -- from leyline's own store, not from their Downloads folder, so moving or deleting what they saved makes no difference. If no reachable peer has it, it says so |
| `/paste` | Share whatever's on the OS clipboard with the active channel: a real file (e.g. copied in Finder/Explorer, including an animated GIF, byte-for-byte) if there is one, else a screenshot or other copied image (always shared as a single static PNG frame -- no OS clipboard format carries multi-frame/animation data), else plain copied text as a chat message. A file or image is staged above the input box for review rather than sent at once -- `Enter` sends it (any typed text becomes its caption), `Backspace` on an empty input removes the newest, `Esc` clears them -- while text is sent immediately. Same as the `Ctrl+V` keybinding |
| `/alias <hex-prefix> <name>` | Assign a local pet name to the peer whose endpoint id starts with `<hex-prefix>`, shown in place of their hex id from then on (local only, never sent to peers) |
| `/nick <name>` | Broadcast a chosen display name to every joined channel (spoofable -- shown as `name (hex-prefix)` until you `/alias` that peer) |
| `/search <term>` (alias `/s`) | Filter the active channel's messages down to ones containing `<term>` (instant for what's loaded, extended in the background with a scan of the full on-disk history); the pane title reminds you it's active and that `/search`/`/s` with no argument clears it |
| `/reply` | Arm a reply to the most recent message in the active channel -- type normally afterward to send it as a reply (or press `Ctrl+R` instead to pick an older or specific-sender message) |
| `/reply <text>` | Arm and immediately send `<text>` as a reply to the most recent message, in one step |
| `/thread` | Show just the reply chain (ancestors and descendants) around the most recent message, isolated from the rest of the transcript -- press `Ctrl+T` instead to pick an older message. A plain sent message automatically replies within the open thread (an explicit `Ctrl+R`/`/reply` target still wins); run `/thread` again (or press `Esc`) to return to the normal transcript |
| `/hints` | Toggle the sidebar's command hints panel on or off |
| `/bell` | Toggle the terminal bell that rings when a message arrives, on or off (on by default; the setting persists across restarts) |
| `/version` | Show this build's version and git commit, plus the newest peer version seen that's newer than yours, if any -- see the header's "update available" indicator |
| `/help` | Show the full list of commands and keybindings |

## Roadmap

Rough build order this project followed, from a `ratatui`-only prototype to
the full peer-to-peer app -- all shipped:

- [x] 1. `ratatui` chat UI against fake/local messages -- validate layout and keybindings
- [x] 2. wire up `iroh` + `iroh-gossip` so two local instances can talk over one gossip topic
- [x] 3. Invite ticket generation/parsing, identity persistence
- [x] 4. Multi-channel support (one gossip task per joined topic), presence sidebar
- [x] 5. Local message persistence + reload on start
- [x] 6. `iroh-blobs`-based history backfill for offline peers
- [x] 7. Room privacy: per-channel random secrets instead of name-derived topics

## Task management

Feature and bug work is tracked with [Backlog.md](https://github.com/MrLesk/Backlog.md),
a markdown-native task manager -- `backlog board` paints a terminal Kanban
board, and `backlog task list --plain` lists tasks (tasks use a `LEY-`
prefix). Install it with `brew install backlog-md`. New, not-yet-committed
ideas still start in [`features.md`](./features.md) before being promoted
into a task. See [`AGENTS.md`](./AGENTS.md) for the full agent workflow.

## Architecture

See [`concept.md`](./concept.md) for the architecture notes: wire format,
the room-privacy model, event loop design, module layout, and the
reasoning behind the network/UI split.
