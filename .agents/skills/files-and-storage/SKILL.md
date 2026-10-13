---
name: leyline-files-and-storage
description: Use for shared attachments, clipboard or drag-and-drop files, safe destinations, file-size limits, saved-file tracking, chat persistence, or history storage in leyline.
---
# Files and storage
Start with `src/files.rs` for attachment staging, paths, limits, and destination safety. `src/files/drops.rs` parses terminal drag-and-drop payloads and must continue rejecting ambiguous, relative, missing, and directory paths.

Use `src/storage.rs`, `src/saved_files.rs`, and `src/backfill.rs` for persistence and retrieval. Keep filesystem operations outside `app`; `AppState` should request them through `InputAction`.

Run `cargo test` after changes touching parsing, path safety, or persistence.
