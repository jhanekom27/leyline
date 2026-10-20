# Leyline documentation

**Private, peer-to-peer chat for the terminal.** leyline has no accounts,
central message server, or shared public channel directory. A channel invite is
the capability that lets another person join.

## Start here

1. [Install leyline](installation.md) from a published release.
2. Run `leyline`.
3. In one instance, run `/invite` to show a channel ticket.
4. In another instance, paste that ticket with `/join <ticket>`.
5. Chat normally, or continue to [Folder sync](folder-sync.md) to share a
   channel's working folder safely.

> **Tip:** To test two local instances, run the installed executable with a
> separate home for each peer: `HOME=/tmp/leyline-a leyline` and
> `HOME=/tmp/leyline-b leyline`.

## What the badges mean

The channel list separates ordinary group channels from one-to-one
conversations:

- `#` is an invited group channel.
- `@` is a private message channel.
- `⚡` is a direct point-to-point conversation.

All three channel kinds can use folder sync. A group channel needs an invite
ticket before someone else can join it; a `/msg` or `/dm` conversation already
has its one peer, so each person only needs to enable sync in that conversation.

## Find your way around

- [Command & keyboard reference](reference.md) is the complete reference; `/help` shows it inside leyline.
- `/folder` shows where a channel's local files live. `/folder <path>` changes
  the shared files root for every channel on this device.
- `/files` shows files shared in the active conversation and lets you save
  them manually.
- `/sync` reports the active channel's sync policy, status, and any
  current conflicts.

This guide grows with leyline. Use the GitHub link in the header to report a
problem or suggest an improvement.
