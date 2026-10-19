# 60-second quickstart

Run two leyline peers, share a private channel invite, and send a message.
For two real people, each person follows the same steps on their own computer.
For a local test, use two terminals and the separate `HOME` directories below.

## 1. Install leyline

Follow the [installation guide](installation.md) to download a published
release or install a published release tag with Cargo. Confirm that
`leyline --version` works before continuing.

## 2. Start Alice

In the first terminal, run `leyline`. For a local two-terminal test, give
Alice a separate home:

```sh
HOME=/tmp/leyline-alice leyline
```

At the prompt, create a channel just for this demo:

```text
/join demo
```

Then create an invite ticket:

```text
/invite
```

Copy the ticket leyline prints. It is the private capability to join this
channel, so send it only to the person you mean to invite.

## 3. Start Bob

In a second terminal, run `leyline`. For the local test, use a different home:

```sh
HOME=/tmp/leyline-bob leyline
```

Paste Alice's ticket into Bob's message box:

```text
/join <Alice's ticket>
```

Both peers now have `#demo` open. Type a message in either pane and press
`Enter`; it appears for the other peer shortly afterward.

## Next steps

- Use `/invite` whenever you want to add another person to a group channel.
- Use `/msg` for an invite-backed one-to-one conversation, or `/dm` to message
  a known endpoint directly.
- Read [Folder sync](folder-sync.md) to safely share a channel folder.
- Find releases, report problems, or suggest improvements at
  [leyline on GitHub](https://github.com/jhanekom27/leyline).

`#general` is private too: each fresh installation starts with its own secret,
so create a named channel such as `#demo` before sharing an invite with a
second fresh instance.
