# leyline guide

**Private, peer-to-peer chat for the terminal.** leyline has no accounts,
central message server, or shared public channel directory. A channel invite is
the capability that lets another person join.

## Start here

1. Install a recent stable Rust toolchain, then run `cargo run`.
2. In one instance, run `/invite` to show a channel ticket.
3. In another instance, paste that ticket with `/join <ticket>`.
4. Chat normally, or continue to [Folder sync](folder-sync.md) to share a
   group channel's working folder safely.

> **Tip:** Two local test instances need separate homes. Run one with
> `HOME=/tmp/leyline-a cargo run` and the other with
> `HOME=/tmp/leyline-b cargo run`. Set `CARGO_HOME` to your normal Cargo home
> if you do not want each temporary home to rebuild dependencies.

## What the badges mean

The channel list separates ordinary group channels from one-to-one
conversations:

- `#` is an invited group channel. It can use folder sync.
- `@` is a private message channel.
- `⚡` is a direct point-to-point conversation.

Folder sync is intentionally available only on group channels today. Direct
message and point-to-point conversation sync are planned separately; do not
use them as a substitute for a shared working folder.

## Find your way around

- `/help` lists commands and keybindings in the terminal.
- `/folder` shows where a channel's local files live. `/folder <path>` changes
  the shared files root for every channel on this device.
- `/files` shows files shared in the active conversation and lets you save
  them manually.
- `/sync` reports the active group channel's sync policy, status, and any
  current conflicts.

This guide grows with leyline. Use the GitHub link in the header to report a
problem or suggest an improvement.
