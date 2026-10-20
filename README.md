# leyline

**Private chat that lives in your terminal.** leyline lets small groups and
trusted peers talk, share files, and keep channel folders in sync without
accounts, a central chat server, or a relay for your messages.

Create a channel, share its invite with the people you choose, and keep
working. Your identity, channels, messages, and known peers stay on your
device; peers who were offline automatically catch up when they return.
![Alice creates a private channel, Bob joins with the invite ticket, and both exchange messages in side-by-side leyline terminals.](docs/assets/leyline-demo.gif)

## Why leyline

- **Private by default.** Channel names are local labels, not credentials.
  Every channel has its own random secret, carried only in an invite ticket.
- **No accounts or central chat service.** Your local keypair is your
  identity. Messages travel peer-to-peer over
  [iroh](https://github.com/n0-computer/iroh) and iroh-gossip inside a
  [ratatui](https://ratatui.rs) terminal interface.
- **Built for the work around a conversation.** Chat across group channels,
  private messages, or direct peer connections; share files on demand; and
  opt into safe folder sync with conflict copies and recoverable trash.
- **Pick up where you left off.** leyline persists your identity, joined
  channels, and local history, then backfills missed messages for channels
  you are already allowed to join.

## Documentation

The user guide covers setup and safe folder synchronization:
[jhanekom27.github.io/leyline](https://jhanekom27.github.io/leyline/).
## 60-second quickstart

Start two local peers and exchange your first message with the
[quickstart guide](https://jhanekom27.github.io/leyline/quickstart.html).
## Install leyline

Download a prebuilt binary or install from a pinned Git tag with Cargo. The
[installation guide](https://jhanekom27.github.io/leyline/installation.html)
covers each supported end-user path, including checksum verification.

## Contributor build & run

Contributors need a recent stable Rust toolchain with edition 2024 support
(Rust 1.91 or newer -- iroh-gossip's current MSRV).

```sh
cargo build --release
cargo run
```

Run `cargo run -- --version` (or `-V`) to print the build's version and
git commit and exit immediately, without touching disk or the network.

On startup, `leyline` prints an invite ticket for each channel it joined
(before the TUI takes over -- just `#general` by default, and private to
this instance alone until you share it) and shows your endpoint id in the
TUI header (`you: <hex id>`). For a working two-instance walkthrough, use
the [60-second quickstart](#60-second-quickstart): it creates a new shared
channel, then uses `/invite` and `/join` to connect the peers.

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
conversations (`@` for a `/msg` DM, `⚡` for a `/dm` point-to-point chat), each in the order you joined them. The active one is highlighted, and another
with unread messages shows its count. Sync-enabled channels show `S`, `~`, `C`, `!`, or `P` for idle, pending, conflict, error, or paused sync; the
panel scrolls once the list is
taller than the terminal. Switch between channels with `Tab` / `Shift+Tab`,
which step down / up the panel and wrap around, or press `Ctrl+P` to jump
straight to one by typing part of its name. Press `Ctrl+K` to browse or filter
slash commands by syntax or description; command-name and alias matches appear
before description-only matches, and `Enter` runs the selected command.

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

...then follow the [60-second quickstart](#60-second-quickstart) to create
and share a `#demo` channel between them.

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
| `End` / `Ctrl+E` | Jump to the end of the current line; while `/files` is open, `Ctrl+E` opens the highlighted local file in the platform text editor |
| `Backspace` / `Delete` | Delete the character before / at the cursor (on an empty input, `Backspace` instead takes the most recently staged attachment back out -- see `Ctrl+V`) |
| `Ctrl+U` | Delete from the start of the current line to the cursor |
| `Ctrl+K` | Open the command picker: type to filter slash commands by syntax, alias, or description (syntax and aliases rank first). Each row marks whether `Enter` will run it (`▶ run`, keeping the current draft) or insert it for editing (`✎ edit`, replacing the draft); `Up` / `Down` choose one and `Esc` cancels |
| `Ctrl+W` | Delete the word before the cursor |
| `Alt+Enter` / `Ctrl+J` | Insert a newline in the message box without sending, for a longer, spaced-out message -- the box grows to fit (and scrolls, past a handful of lines) |
| `Enter` | Send the composed message, or run a `/` command. With attachments staged (see `Ctrl+V`) it sends them all instead, the typed text becoming the caption of the first -- but a `/` command still runs as a command and leaves them staged |
| (paste, e.g. `Cmd+V`/`Ctrl+V`) | Paste text from the terminal as one atomic edit, so a multi-line paste can't prematurely send a partial line or trigger a stray `/command` -- unless the paste is really a dropped file (see the next rows) |
| `Ctrl+V` | Share whatever's on the OS clipboard (a file, an image, or text) with the active channel -- same as `/paste`. A file or image is *staged* in a strip above the input box (name and size) rather than sent at once: `Enter` sends it, `Backspace` on an empty input removes the newest, `Esc` clears them all, and the staged files stay with their channel when you switch tabs. Plain text is sent at once. `Cmd+V` also works if your terminal forwards it, but most (including this one) reserve it for their own text-paste above |
| (drag a file onto the window) | A terminal has no drop protocol: it pastes the dropped files' paths, and leyline recognizes a paste made up of nothing but existing files -- absolute paths, backslash-escaped, quoted, or `file://` URIs, separated by spaces or newlines -- and *stages* them on the active channel exactly like `Ctrl+V` of a copied file, instead of typing the path into the box. It is all or nothing: a sentence that mentions a path, a relative name, a directory, or a path that doesn't exist is pasted as text as usual, and so is anything pasted while the `Ctrl+P` jump-to list, a pick or a pairing confirmation has input. A file that exists but can't be staged (unreadable, over the size limit) is reported and skipped. Works whether the terminal pastes the path (through bracketed paste, which leyline turns on) or, like Warp, types it out as ordinary keys: keys that arrive together and spell nothing but existing files count as a drop, while typing a path by hand never does |
| `Ctrl+R` | Pick a message to reply to -- `Up`/`Down` move the highlight, `Enter` confirms, `Esc` cancels; while `/files` is open, reveal the highlighted local file in the file manager |
| `Ctrl+T` | Pick a message to view its reply chain in isolation -- `Up`/`Down` move the highlight, `Enter` confirms, `Esc` cancels |
| `Up` / `Down` | Scroll the message history (or move the highlight while picking a reply or thread target, see `Ctrl+R`/`Ctrl+T`) |
| `Tab` / `Shift+Tab` | Switch to the next / previous channel in the left-hand panel (group channels first, then conversations), wrapping around |
| `Ctrl+P` | Jump to a channel or conversation by name -- type to narrow the list to names containing what you typed (case-insensitive), `Up`/`Down` move the highlight, `Enter` switches to it, `Esc` cancels and leaves you where you were |
| `Esc` | Close the `/who` peer-details table, `Ctrl+P` jump-to list, `Ctrl+K` command picker, or an in-progress reply or thread pick, or cancel an armed reply, first, if any; otherwise clear staged attachments; otherwise leave an open thread view if one is active; otherwise quit |
| `Ctrl+C` | Quit immediately, including from an open overlay |

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
| `/who` | Open a dismissible table of this device, the active channel's online peers, and remembered offline peers, showing status circles, each full endpoint id, local alias, and broadcast nickname; `Esc` returns to the conversation |
| `/send` | Open a keyboard-driven file picker -- no path to type, and it works over SSH since it lives inside the terminal UI. It opens in the folder you last chose files from (remembered across restarts; your Downloads folder to begin with) and lists folders first, then files newest first, with hidden files left out. `Up`/`Down` move; `Enter` opens a folder or, on a file, *stages* the marked files (just that one if none are marked) exactly like `Ctrl+V` of a copied file; `Space` marks and unmarks the highlighted file, and marks are kept as you move between folders; type to narrow the folder by name, ignoring case (so the filter has no spaces); `Backspace` deletes a typed character and, once none are left, goes up a level; `Esc` closes it without staging anything. A folder that can't be opened shows the reason and leaves the picker where it was, and a huge or slow folder is read in the background, so keys keep working while it loads |
| `/send <path>` | Share a local file with the active channel right away (unlike `/paste` or the picker, nothing is staged first); peers see its name and size and can fetch it with `/save` -- never on their own, unless they have turned on `/autosave` for the channel and trust you |
| `/save` | Download the newest file another peer shared in the active channel (one you shared yourself doesn't count) into that channel's folder (see `/folder`) -- no hash to copy out of the transcript, much as `/reply` needs no message picked. Says so if there is no such file; use `/save <hash-prefix>` for an older one. Fetched the same way as `/save <hash-prefix>`, so it doesn't need the sender online either |
| `/save <hash-prefix>` | Download a file previously shared in any joined channel into the folder of the channel it was shared in (the active channel's, if it was shared in several and you are in one of them; see `/folder`). The sender doesn't have to be online: it is asked first, then every other peer known in that channel, and anyone who already saved the file serves it -- from leyline's own store, not from their saved copy, so moving or deleting what they saved makes no difference. If no reachable peer has it, it says so |
| `/folder` | Say where received files go. Every channel saves into a folder of its own under one files root -- your Downloads folder's `leyline` subfolder to begin with -- created when the first file lands in it, with `name (1).ext` still settling clashes inside it. The folder is named after the channel's stored name, not the label shown for it, so `/rename` and `/alias` never move it: a name that is already safe as a folder name everywhere (`general`, `project-x`) is the folder as it stands, and any other (capitals, spaces, punctuation, another script) becomes a readable form of it, a `~` and a hash of the exact name -- `Team-Alpha~83f57e5b1e8db868` -- so two channels never share a folder, even on a filesystem that ignores case. Leaving a channel leaves its folder alone |
| `/folder <path>` | Move the files root to `<path>` for every channel: `~` and relative paths are expanded, a folder that doesn't exist yet is created, and the choice is kept across restarts. Files saved before stay where they are |
| `/maxsize` | Say how big a file you can share. A file over the limit is refused whether you `/send` it, pick it, drop it or paste it -- 500 MB to begin with, as a guard against sharing something huge by accident -- and the refusal says how to get past it. A bare `/maxsize` shows the limit and how to change it, the way a bare `/folder` shows where files go. Only files you share are limited, never ones you receive, and a pasted image counts as a file |
| `/maxsize <size>` | Set the limit to `<size>`: a number and a unit, `B`, `KB`, `MB`, `GB` or `TB` in any case (1024 apart, as sizes are shown), like `750MB`, `2gb` or `1.5 GB`. `/maxsize off` lifts the limit altogether and `/maxsize default` restores the 500 MB one. The choice is kept across restarts. Something that is not a size, or that would refuse every file, is refused with what to type instead and leaves the limit as it was |
| `/autosave` | Say whether auto-save is on for the active channel and report its effective finite limit. Channels inherit the device-wide limit unless they have a local override. |
| `/autosave on` / `/autosave off` | Turn it on or off for the active channel. The channel choice is kept across restarts, forgotten when you leave the channel, and wiped by pairing. |
| `/autosave limit <size>` | Set the largest file automatic saves fetch for the active channel. Sizes use the same `B` through `TB` syntax as `/maxsize`; `/autosave limit default` removes this channel's override and restores its device-wide fallback. Overrides persist across restart and are removed when leaving the channel. `off` is refused: unattended downloads always retain a finite ceiling. |
| `/outbox` | Say whether the active channel watches its local `send/` folder. The folder is `<channel folder>/send/`, alongside received files (see `/folder`), and is off by default. It is one-way only: nothing is synced into or back from it. |
| `/outbox on` / `/outbox off` | Turn the one-way outbox on or off for the active channel. Enabling creates its `send/` folder, treats files already there as already shared, and remembers the setting across restarts. New regular files are shared only after two unchanged scans, so a copy in progress is not sent half-written; hidden files and common temporary names (`.DS_Store`, `*.part`, `*.crdownload`, `~$*`) are ignored. A name is shared at most once, even if edited or after restart. Files that cannot be read or exceed `/maxsize` are reported in the channel transcript. |
| `/sync` | Report the active channel's persisted policy (`off`, `receive-only`, `publish-and-receive`, or paused), configured path rules, folder path, pending count, conflicts, errors (with file names), and last successful sync -- or, before it has ever been configured here, where its folder will be and how to enable it. Group channels explain their `/invite` and `/join` prerequisite; `/msg` and `/dm` conversations tell both people to enable their existing shared conversation. Conflict entries name both the remote path and its local conflict copy; a report with pending files points to `/who` to check who's currently reachable |
| `/sync receive-only` / `/sync publish-and-receive` / `/sync off` | Persist the active channel's policy independently of every other channel. Receive-only mirrors verified remote files without publishing local ones; publish-and-receive does both. `/sync on` remains an alias for publish-and-receive. |
| `/sync pause` / `/sync resume` | Temporarily stop or restore the active channel's selected policy while retaining its current status and rules. A resumed receive-only channel stays receive-only. |
| `/sync include <glob>` / `/sync exclude <glob>` | Add a relative-path glob rule for the active channel. With include rules, only matching paths sync; excludes always win. Rules apply to both startup scans and live local/remote changes. Use `/sync remove include <glob>`, `/sync remove exclude <glob>`, or `/sync clear-rules` to edit them. |
| `/activity` | List recent local sync activity for the active channel: safe relative path, action, outcome, and time. The history is bounded to the newest 100 entries per channel, persists locally across restart, and is never sent to peers. |
| `/conflicts` | List unresolved sync conflicts for the active channel, including local and remote paths, known modification timestamps, and the remote author when the index supplies one. The navigation `C` indicator remains until a resolution succeeds. |
| `/conflicts local <path>` / `/conflicts remote <path>` / `/conflicts both <path>` | Resolve one conflict by local path. Keeping local or remote moves the displaced copy to recoverable sync trash; keeping both leaves both paths in place. No action overwrites a conflicting copy silently. |
| `/trash` | List recoverable files from remote deletes for the active channel. Each channel keeps its own hidden sync-trash folder, which is retained indefinitely and never automatically purged. |
| `/trash restore <path>` | Restore one listed relative path into the active channel folder. An existing local file is never overwritten; the restored file receives a distinct `restored` name instead. In publish-and-receive mode, the normal stable-file scan can subsequently publish the restored copy. |
| `/trash purge` | Permanently delete every recoverable remote-delete file for the active channel. This explicit command is the only purge path. |
| `/files` | Browse shared attachments and local sync-folder files in one fuzzy-searchable box over the transcript. Shared entries retain their sender, date, and green saved/autosave `✓`; sync-local entries show `↔` and their relative path. Type to narrow by attachment name/sender or sync path; `Up`/`Down` move the highlight and `Esc` closes it. `Enter` saves an unavailable shared attachment exactly as `/save` does. For any local copy, `Ctrl+O` opens it with the default application, `Ctrl+R` reveals it in the file manager, and `Ctrl+E` opens it in the platform text editor. A program, script, installer, or shortcut is never default-opened, but can still be revealed or edited. Hidden, temporary, outbox, symlink, and sync-trash files are excluded using the same eligibility rules as `/sync`. |
| `/paste` | Share whatever's on the OS clipboard with the active channel: a real file (e.g. copied in Finder/Explorer, including an animated GIF, byte-for-byte) if there is one, else a screenshot or other copied image (always shared as a single static PNG frame -- no OS clipboard format carries multi-frame/animation data), else plain copied text as a chat message. A file or image is staged above the input box for review rather than sent at once -- `Enter` sends it (any typed text becomes its caption), `Backspace` on an empty input removes the newest, `Esc` clears them -- while text is sent immediately. Same as the `Ctrl+V` keybinding |
| `/alias <hex-prefix> <name>` | Assign a local pet name to the peer whose endpoint id starts with `<hex-prefix>`, shown in place of their hex id from then on (local only, never sent to peers) |
| `/nick <name>` | Broadcast a chosen display name to every joined channel (spoofable -- shown as `name (hex-prefix)` until you `/alias` that peer) |
| `/search <term>` (alias `/s`) | Filter the active channel's messages down to ones containing `<term>` (instant for what's loaded, extended in the background with a scan of the full on-disk history); the pane title reminds you it's active and that `/search`/`/s` with no argument clears it |
| `/reply` | Arm a reply to the most recent message in the active channel -- type normally afterward to send it as a reply (or press `Ctrl+R` instead to pick an older or specific-sender message) |
| `/reply <text>` | Arm and immediately send `<text>` as a reply to the most recent message, in one step |
| `/thread` | Show just the reply chain (ancestors and descendants) around the most recent message, isolated from the rest of the transcript -- press `Ctrl+T` instead to pick an older message. A plain sent message automatically replies within the open thread (an explicit `Ctrl+R`/`/reply` target still wins); run `/thread` again (or press `Esc`) to return to the normal transcript |
| `/hints` | Toggle the sidebar's command hints panel and mouse reporting on or off (shown by default; resets on restart). Hover over any command to see a popup with its description and full usage, or over a peer to see their alias, nickname, and online status (or when they were last seen), plus their build version while they are online and their short id; moving away hides it. While shown, hold Shift (Option in iTerm2) for native text selection; link-click modifiers depend on your terminal. Mouse-wheel scrolling still works |
| `/bell` | Toggle the terminal bell that rings when a message arrives, on or off (on by default; the setting persists across restarts) |
| `/version` | Show this build's version and git commit, plus the newest peer version seen that's newer than yours, if any -- see the header's "update available" indicator |
| `/help` | Show the full list of commands and keybindings |

## Sync-folder manual verification

Verify between a macOS machine (normally case-insensitive) and a Linux
machine: join both to one channel, turn `/sync publish-and-receive` on both, create and edit
a small nested file on either side, and confirm it arrives only after it
stops changing. Confirm files present before enabling sync and written while
the app was stopped are picked up after two scans. Repeat with hidden,
temporary/editor, `send/`, oversized, and in-progress files; none should
publish. Confirm a received file is not echoed back as a fresh index write,
`/sync off` stops new applications and publications, a remote delete moves
the local copy into recoverable sync trash instead of erasing it, and a
rename arrives as its new path without re-transferring unchanged content.
This repository's automated tests cover the local inventory boundary; this
two-machine verification must be performed manually.


## Architecture

See [`concept.md`](./concept.md) for the architecture notes: wire format,
the room-privacy model, event loop design, module layout, and the
reasoning behind the network/UI split.
