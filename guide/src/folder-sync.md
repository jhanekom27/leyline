# Folder sync

Folder sync keeps the regular files in a **channel's** local
folder aligned between its members. It is designed for a small shared working
set, not as an unattended backup service.

> **Supported today:** group channels (`#`), private-message channels (`@`),
> and point-to-point conversations (`⚡`). Every participant enables sync on
> their own local copy of the conversation.

## Set up two instances

Use two devices, or two local instances with distinct `HOME` directories.
Each instance needs leyline running.

1. Choose the channel to share.
   - For a group channel, run `/invite` on the first instance, send the ticket
     through a trusted channel, then paste it on the second instance with
     `/join <ticket>`.
   - For a `/msg` or `/dm` conversation, start or open that conversation on
     both instances first. No separate folder-sync invite is needed.
2. On each instance, select the same channel and run:

   ```text
   /sync publish-and-receive
   ```

   This publishes eligible local changes and receives eligible changes from
   peers. `/sync on` is an alias for the same policy.
3. Run `/folder` on either instance to see that channel's local folder. The
   default root is `Downloads/leyline`; each channel gets its own safe folder.
   Change the root for this device with `/folder <path>` if needed.
4. Add or edit a small file in one channel folder. leyline waits until a file
   is unchanged for two scans before publishing it, then the other instance
   downloads it into its matching channel folder.

If one participant should only download shared files, use `/sync receive-only`
there instead. `/sync off` disables both directions for the active channel.

## Read the status

The active channel's `/sync` output reports its policy, folder, rules,
pending transfers, conflicts, errors, and last successful sync. Its sidebar
badge gives a compact signal:

| Badge | Meaning | What to do |
| --- | --- | --- |
| `S` | Sync is idle and healthy. | Continue working. |
| `~` | One or more changes are pending. | Wait for the file to stop changing and for a peer to be reachable. |
| `C` | A conflict needs your decision. | Run `/conflicts`. |
| `!` | A sync operation failed. | Run `/sync` and `/activity` for the affected path. |
| `P` | Sync is paused. | Run `/sync resume` when ready. |

Pause without losing your selected direction:

```text
/sync pause
/sync resume
```

Resuming restores the previous `receive-only` or `publish-and-receive`
policy; it does not silently switch a download-only channel into publishing.

## Safety model

leyline favors preserving a copy over silently overwriting or deleting work.

### Conflicts never overwrite a local file

If a remote update targets a path that already has a local file, leyline saves
the remote bytes under a name such as `report (remote conflict 1).txt` and
marks the channel `C`. Inspect the choices with:

```text
/conflicts
```

Resolve an individual path deliberately:

```text
/conflicts local <path>
/conflicts remote <path>
/conflicts both <path>
```

Keeping local or remote moves the displaced copy into recoverable sync trash.
Keeping both leaves both copies in place. No conflict command silently
overwrites a file.

### Remote deletes are recoverable

When a remote delete applies to an unchanged file that sync previously wrote,
leyline moves it into that channel's hidden `.leyline-trash` folder rather
than deleting it permanently. A local edit turns a remote delete into a
conflict instead.

Use these commands to inspect and control recovery:

```text
/trash
/trash restore <path>
/trash purge
```

Trash is retained until you explicitly purge it. Restoring a path never
overwrites an existing local file; leyline gives the restored copy a distinct
name instead.

### Scope and size are bounded

Sync recursively considers regular files in the channel folder, but does not
publish:

- hidden paths, including `.leyline-trash`;
- the local-only `send/` outbox;
- symbolic links;
- common editor and download temporary files such as `*.part`, `*.tmp`,
  `*.swp`, `*.crdownload`, and names starting with `~$`.

Incoming sync files over **500 MB** are skipped. This is a fixed sync safety
limit, independent of `/maxsize`, which controls the files you share manually.

You can further scope a channel with relative glob rules:

```text
/sync include docs/**
/sync exclude docs/private/**
/sync remove include docs/**
/sync clear-rules
```

When include rules exist, only paths matching an include rule are eligible.
Excludes always win. Rules use `/` separators and cannot point outside the
channel folder.

## Troubleshoot deliberately

1. Confirm both people are in the same conversation: the same invited
   **group** channel, or the same `/msg` or `/dm` pair.
2. Run `/sync` on both instances and check for `P`, `C`, or `!`.
3. Run `/activity` to see the latest local outcome for each affected path.
4. Ensure the file is not hidden, temporary, oversized, or beneath `send/`.
5. If files disagree, inspect `/conflicts` before modifying either copy.
