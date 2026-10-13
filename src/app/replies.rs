//! Reply selection and thread-view state transitions.

use super::*;

impl AppState {
    /// error message they report when the channel has no messages.
    pub(super) fn newest_visible_message(&self) -> Option<u64> {
        self.active().visible_chat_ids().last().copied()
    }

    /// Finds the newest message currently visible in the active channel
    /// to use as a reply target -- shared by `Ctrl+R` (`start_reply_pick`)
    /// and `/reply` (`run_reply`). Reports an error and returns `None` if
    /// the channel has no messages to reply to, mirroring `/who`'s
    /// empty-channel notice.
    pub(super) fn newest_reply_candidate(&mut self) -> Option<u64> {
        match self.newest_visible_message() {
            Some(id) => Some(id),
            None => {
                self.push_system("no messages to reply to");
                None
            }
        }
    }

    /// Finds the newest message currently visible in the active channel
    /// to use as a thread's origin -- shared by `Ctrl+T`
    /// (`start_thread_pick`) and bare `/thread` (`run_thread`). Mirrors
    /// `newest_reply_candidate`, but for thread-viewing instead of
    /// replying.
    pub(super) fn newest_thread_candidate(&mut self) -> Option<u64> {
        match self.newest_visible_message() {
            Some(id) => Some(id),
            None => {
                self.push_system("no messages to view a thread for");
                None
            }
        }
    }

    /// Handles `Ctrl+R`: begins picking a reply target by highlighting the
    /// newest message currently visible in the active channel -- confirm
    /// with `Enter`, move with `Up`/`Down`, cancel with `Esc` (see
    /// `handle_picking_key`).
    pub(super) fn start_reply_pick(&mut self) {
        if let Some(id) = self.newest_reply_candidate() {
            self.picking_reply = Some(id);
        }
    }

    /// Handles `Ctrl+T`: begins picking a message to view its thread, the
    /// same way `start_reply_pick` begins picking a reply target --
    /// confirm with `Enter` (opens the picked message's thread, see
    /// `handle_thread_picking_key`/`open_thread`), move with `Up`/`Down`,
    /// cancel with `Esc`.
    pub(super) fn start_thread_pick(&mut self) {
        if let Some(id) = self.newest_thread_candidate() {
            self.picking_thread = Some(id);
        }
    }

    /// Moves a pick cursor currently at `current` by `delta` through the
    /// active channel's currently visible chat messages
    /// (`Channel::visible_chat_ids`), oldest to newest. Clamps at both
    /// ends rather than wrapping -- "past the newest" and "before the
    /// oldest" aren't meaningful positions to cycle from. Returns
    /// `current` unchanged if it's no longer among the visible ids
    /// (shouldn't normally happen, since both `start_reply_pick` and
    /// `start_thread_pick` always seed their cursor from that same list).
    /// Shared by `move_reply_pick`/`move_thread_pick`, which differ only
    /// in which field they update.
    pub(super) fn move_pick(&self, current: u64, delta: isize) -> u64 {
        let ids = self.active().visible_chat_ids();
        let Some(pos) = ids.iter().position(|&id| id == current) else {
            return current;
        };
        let new_pos = (pos as isize + delta).clamp(0, ids.len() as isize - 1) as usize;
        ids[new_pos]
    }

    /// Moves an in-progress reply pick (`picking_reply`) by `delta` -- see
    /// `move_pick`. A no-op if nothing is being picked.
    pub(super) fn move_reply_pick(&mut self, delta: isize) {
        if let Some(current) = self.picking_reply {
            self.picking_reply = Some(self.move_pick(current, delta));
        }
    }

    /// Moves an in-progress thread pick (`picking_thread`) by `delta` --
    /// see `move_pick`. A no-op if nothing is being picked.
    pub(super) fn move_thread_pick(&mut self, delta: isize) {
        if let Some(current) = self.picking_thread {
            self.picking_thread = Some(self.move_pick(current, delta));
        }
    }

    /// Handles `/reply [text]`: `arg` is everything after `/reply `
    /// (already trimmed). Arms a reply to the newest message in the
    /// active channel -- equivalent to `Ctrl+R` then `Enter` without
    /// moving the selection -- scoped to the most recent message overall,
    /// not per-sender; `Ctrl+R` picking is the way to target an older or
    /// specific-sender message. With no `arg`, leaves the user to type the
    /// reply normally afterward; with one, immediately composes and sends
    /// it as that reply, the same way a normal `Enter` send would once
    /// armed (see `send_text`).
    pub(super) fn run_reply(&mut self, arg: &str) -> Option<InputAction> {
        let id = self.newest_reply_candidate()?;
        let sender = self
            .active()
            .find_message(id)
            .expect("id just came from this channel's own visible_chat_ids")
            .sender;
        self.push_system(format!("replying to {}", self.display_name(&sender)));
        self.replying_to = Some(id);
        if arg.is_empty() {
            return None;
        }
        let reply_to = self.replying_to.take();
        Some(self.send_text(arg, reply_to))
    }

    /// Handles `/thread`: with an active thread view, closes it (mirroring
    /// `/s` clearing an active search) -- otherwise opens a thread view
    /// around the newest message currently visible in the active channel
    /// (`newest_thread_candidate`), mirroring `/reply`'s own "most recent
    /// message" default. Pick an older message instead with `Ctrl+T`
    /// (`start_thread_pick`).
    pub(super) fn run_thread(&mut self) -> Option<InputAction> {
        if self.close_thread() {
            return None;
        }
        if let Some(id) = self.newest_thread_candidate() {
            self.open_thread(id);
        }
        None
    }

    /// Opens a thread view around `origin_id`: resolves it in the active
    /// channel (`Channel::find_message`, so it works whether `origin_id`
    /// came from the loaded transcript or an active search), builds its
    /// reply chain (`thread::build_chain`) against the channel's loaded
    /// chat messages, and activates it as `channel.thread` -- clearing any
    /// active `search` first, since a channel shows at most one alternate
    /// view at a time. A no-op if `origin_id` doesn't currently resolve to
    /// a message (shouldn't normally happen -- both `run_thread` and
    /// `handle_thread_picking_key` always source it from
    /// `newest_thread_candidate`/`visible_chat_ids`, which only ever offer
    /// up ids `find_message` can resolve).
    pub(super) fn open_thread(&mut self, origin_id: u64) {
        let Some(origin) = self.active().find_message(origin_id).cloned() else {
            return;
        };
        let pool: Vec<ChatMessage> = self
            .active()
            .messages
            .iter()
            .filter_map(|line| match line {
                TranscriptLine::Chat(message) => Some(message.clone()),
                TranscriptLine::System(_) => None,
            })
            .collect();
        let messages = thread::build_chain(&pool, &origin);
        let count = messages.len();
        let sender = self.display_name(&origin.sender);
        self.active_mut().search = None;
        self.active_mut().thread = Some(ThreadView { origin, messages });
        self.push_system(format!(
            "thread view: {count} message(s) around {sender}'s message -- /thread or Esc to leave"
        ));
    }

    /// Closes the active channel's thread view, if any, reporting it via a
    /// system notice mirroring `/search`'s "search cleared". Returns
    /// whether a thread view was actually active -- `run_thread`'s
    /// toggle-off path and `handle_key`'s `Esc` chain both use this.
    pub(super) fn close_thread(&mut self) -> bool {
        if self.active_mut().thread.take().is_some() {
            self.push_system("thread view closed");
            true
        } else {
            false
        }
    }
}
