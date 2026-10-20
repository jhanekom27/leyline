# Share a file

Use a channel for files you want another person to retrieve deliberately.
Both people must be connected to the same channel first: create one with
`/join <name>`, or join the other person's invite with `/join <ticket>`.

![Alice sends demo-note.txt to the shared channel. Before Bob saves it, his received-files pane says the file is absent; after Bob runs /save, that pane lists the saved copy.](assets/leyline-file-demo.gif)

## Send and save a file

On the sending peer, share a local file directly:

```text
/send ~/share/demo-note.txt
```

The channel transcript shows the file's name and size. On the receiving peer,
retrieve the newest file sent by another peer with:

```text
/save
```

leyline saves it in that channel's own folder under the files root. Run
`/folder` to show the exact destination, or `/folder <path>` to choose a
different root for every channel on this device. A bare `/save` always chooses
the newest peer-shared file in the active channel; use `/save <hash-prefix>`
for an older attachment.

Use `/files` to browse, filter, and save attachments in the active channel.
Files are never downloaded automatically unless you deliberately enable
`/autosave` for that channel and trust the sending peer.
