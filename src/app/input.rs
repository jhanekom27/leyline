use super::*;

impl AppState {
    /// Pure key handling: no I/O, no network. Returns the action to
    /// perform when `Enter` sends a message or runs a `/join`
    /// or `/invite` command, so the caller can talk to the network layer.
    ///
    /// Delegates to `handle_picking_key` first, before any of the normal
    /// typing/editing arms below, whenever a reply pick is in progress
    /// (`picking_reply.is_some()`, see `start_reply_pick`) -- keeps that
    /// transient mode's key handling (`Up`/`Down`/`Enter`/`Esc` all mean
    /// something different while picking) from having to be threaded
    /// through every arm of the match below.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        if self.pending_redeem.is_some() {
            self.handle_redeem_confirmation_key(key);
            return None;
        }
        if self.picking_reply.is_some() {
            self.handle_picking_key(key);
            return None;
        }
        if self.picking_thread.is_some() {
            self.handle_thread_picking_key(key);
            return None;
        }
        if self.switching.is_some() {
            self.handle_switcher_key(key);
            return None;
        }
        if self.command_picker.is_some() {
            return self.handle_command_picker_key(key);
        }
        if self.viewing_peers.is_some() {
            self.handle_peers_overlay_key(key);
            return None;
        }
        if self.picking_file.is_some() {
            return self.handle_picker_key(key);
        }
        if self.viewing_files.is_some() {
            return self.handle_files_key(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // Backs out one level at a time: cancel an armed reply first
            // (mirroring `handle_picking_key`'s own Esc, which cancels a
            // pick-in-progress instead of confirming it), then clear an
            // active search, drop any staged attachments, leave an open
            // thread view, only quitting once none of those is active.
            KeyCode::Esc => {
                if self.replying_to.take().is_some() {
                    // cancelled the armed reply, nothing further to do
                } else if self.clear_active_search() {
                    // left the filtered transcript, nothing further to do
                } else if self.clear_staged() {
                    // dropped the staged attachments, nothing further to do
                } else if !self.close_thread() {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('r') if ctrl => self.start_reply_pick(),
            KeyCode::Char('t') if ctrl => self.start_thread_pick(),
            KeyCode::Char('p') if ctrl => self.start_switcher(),
            KeyCode::Char('a') if ctrl => self.active_mut().move_home(),
            KeyCode::Char('e') if ctrl => self.active_mut().move_end(),
            KeyCode::Char('u') if ctrl => self.active_mut().delete_to_start(),
            KeyCode::Char('k') if ctrl => self.start_command_picker(),
            KeyCode::Char('w') if ctrl => self.active_mut().delete_word_backward(),
            // Inserts a literal newline instead of submitting. Ctrl+J is
            // the plain ASCII linefeed byte, so every terminal reports it
            // the same way regardless of Alt/Meta configuration. Alt+Enter
            // is the more discoverable binding (mirrors Slack/Discord's
            // Shift+Enter convention) and works just as reliably: any
            // terminal using the standard xterm Alt-prefixing convention
            // sends `ESC` then `\r`, which crossterm reports as `Enter`
            // with the `ALT` modifier. Both are checked before the plain
            // `Enter` arm below so a bare `Enter` still always submits.
            KeyCode::Char('j') if ctrl => self.active_mut().insert_newline(),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                self.active_mut().insert_newline()
            }
            // Cmd on macOS -- reported as `KeyModifiers::SUPER` by
            // crossterm, if the terminal forwards it at all. Plain Ctrl+V
            // is the dependable cross-platform trigger, since terminals
            // (Warp included) conventionally reserve their own paste
            // shortcut as Ctrl+Shift+V/Cmd+V and consume it before it ever
            // reaches a foreground app -- see `run_paste`'s doc comment.
            KeyCode::Char('v') if ctrl || key.modifiers.contains(KeyModifiers::SUPER) => {
                return self.run_paste();
            }
            KeyCode::Enter => return self.submit_input(),
            // With an empty input there is no character to delete, so the
            // key takes back the newest staged attachment instead -- like
            // removing the last chip from an address field.
            KeyCode::Backspace => {
                let channel = self.active_mut();
                if channel.input.is_empty() && !channel.staged.is_empty() {
                    channel.staged.pop();
                } else {
                    channel.delete_backward();
                }
            }
            KeyCode::Delete => self.active_mut().delete_forward(),
            KeyCode::Home => self.active_mut().move_home(),
            KeyCode::End => self.active_mut().move_end(),
            KeyCode::Left => self.active_mut().move_left(),
            KeyCode::Right => self.active_mut().move_right(),
            KeyCode::Up => {
                let channel = self.active_mut();
                channel.unread_anchor = None;
                channel.scroll = channel.scroll.saturating_add(1);
            }
            KeyCode::Down => {
                let channel = self.active_mut();
                channel.unread_anchor = None;
                channel.scroll = channel.scroll.saturating_sub(1);
            }
            KeyCode::Tab => self.switch_channel(1),
            KeyCode::BackTab => self.switch_channel(-1),
            KeyCode::Char(c) => self.active_mut().insert_char(c),
            _ => {}
        }
        None
    }

    /// Handles a keypress while confirming a live pairing redeem (`/pair
    /// <ticket>`, see `run_pair`): `Enter` confirms -- taking
    /// `pending_redeem` into `restart_with_ticket` and setting
    /// `should_quit`, so main.rs's normal quit path (loop exit, `Net::
    /// shutdown`, terminal restore) runs first, then wipes local state and
    /// restarts into `--pair <ticket>` -- `Esc` cancels back to normal
    /// typing. `Ctrl+C` still force-quits (a plain, non-pairing quit,
    /// `restart_with_ticket` stays unset), mirroring `handle_picking_key`.
    /// Every other key is ignored: confirming this is destructive enough
    /// to warrant an explicit, narrow yes/no gate rather than letting a
    /// stray keystroke do anything else.
    pub(super) fn handle_redeem_confirmation_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Enter => {
                if let Some(ticket) = self.pending_redeem.take() {
                    self.restart_with_ticket = Some(ticket);
                    self.should_quit = true;
                }
            }
            KeyCode::Esc => {
                self.pending_redeem = None;
                self.push_system("pairing cancelled");
            }
            _ => {}
        }
    }

    /// Handles a keypress while picking a reply target (`Ctrl+R`, see
    /// `start_reply_pick`): `Up`/`Down` move the highlighted candidate
    /// (`move_reply_pick`), `Enter` arms it as `replying_to`, `Esc`
    /// cancels back to normal typing. `Ctrl+C` still force-quits, since
    /// `Esc` no longer does while picking. Every other key is ignored --
    /// picking is a dedicated mode, not an overlay on normal editing, so a
    /// stray keystroke can't leak into the input box.
    pub(super) fn handle_picking_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Up => self.move_reply_pick(-1),
            KeyCode::Down => self.move_reply_pick(1),
            KeyCode::Enter => self.replying_to = self.picking_reply.take(),
            KeyCode::Esc => self.picking_reply = None,
            _ => {}
        }
    }

    /// Handles a keypress while picking a message to view its thread
    /// (`Ctrl+T`, see `start_thread_pick`): `Up`/`Down` move the
    /// highlighted candidate (`move_thread_pick`), `Enter` opens its
    /// thread view (`open_thread`), `Esc` cancels back to normal typing.
    /// Mirrors `handle_picking_key`'s shape, but for thread-picking
    /// instead of reply-picking -- kept as a separate method (rather than
    /// a shared one parameterized by purpose) since the two confirm
    /// actions differ.
    pub(super) fn handle_thread_picking_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Up => self.move_thread_pick(-1),
            KeyCode::Down => self.move_thread_pick(1),
            KeyCode::Enter => {
                if let Some(id) = self.picking_thread.take() {
                    self.open_thread(id);
                }
            }
            KeyCode::Esc => self.picking_thread = None,
            _ => {}
        }
    }

    /// Handles `Ctrl+P`: opens the jump-to-conversation overlay with an
    /// empty query, listing every channel -- type to filter, move with
    /// `Up`/`Down`, confirm with `Enter`, cancel with `Esc` (see
    /// `handle_switcher_key`). `Ctrl+K`, the usual quick-switch key, is
    /// already `delete_to_end`, so this deliberately takes `Ctrl+P`.
    pub(super) fn start_switcher(&mut self) {
        self.switching = Some(Switcher::default());
    }

    /// Handles a keypress while the switcher is open: typing and
    /// `Backspace` edit the query, `Up`/`Down` move the highlight, `Enter`
    /// switches to it (`confirm_switcher`), `Esc` closes without
    /// switching. `Ctrl+C` still force-quits, since `Esc` no longer does
    /// while the overlay is open. Every other key is ignored -- like the
    /// pick modes above this is a dedicated mode, so a stray keystroke
    /// can't leak into the input box.
    pub(super) fn handle_switcher_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Esc => self.switching = None,
            KeyCode::Enter => self.confirm_switcher(),
            KeyCode::Up => self.move_switcher(-1),
            KeyCode::Down => self.move_switcher(1),
            KeyCode::Backspace => self.edit_switcher_query(|query| {
                query.pop();
            }),
            KeyCode::Char(c) if !ctrl => self.edit_switcher_query(|query| query.push(c)),
            _ => {}
        }
    }

    /// Applies `edit` to the open switcher's query and puts the highlight
    /// back on the first match. The one place the query changes, so a typed
    /// character, a `Backspace` and a paste can't disagree about where the
    /// highlight lands. A no-op if the switcher isn't open.
    pub(super) fn edit_switcher_query(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(switcher) = &mut self.switching {
            edit(&mut switcher.query);
            switcher.selected = 0;
        }
    }

    /// The channels the open switcher lists, as indices into `channels`:
    /// those whose label (`channel_label` -- what's on screen, never the
    /// stored name) contains the typed text, ignoring case, in the order
    /// the navigation panel draws them (`nav_order`), so the list reads
    /// like the panel and steps like `Tab`. An empty query lists every
    /// channel; nothing is listed while the switcher is closed.
    pub fn switcher_matches(&self) -> Vec<usize> {
        let Some(switcher) = &self.switching else {
            return Vec::new();
        };
        let query = switcher.query.to_lowercase();
        self.nav_order()
            .into_iter()
            .filter(|&index| {
                self.channel_label(&self.channels[index].name)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }

    /// The channel (an index into `channels`) `Enter` would switch to: the
    /// highlighted match, or `None` while nothing matches or the switcher
    /// is closed. The stored position is clamped here, and only here --
    /// matches can change under an open overlay (a peer gets renamed, a
    /// channel joins), and a position left over from before must never
    /// point past the end.
    pub fn switcher_highlight(&self) -> Option<usize> {
        let switcher = self.switching.as_ref()?;
        let matches = self.switcher_matches();
        let last = matches.len().checked_sub(1)?;
        Some(matches[switcher.selected.min(last)])
    }

    /// Moves the highlight by `delta` among the current matches, clamping
    /// at both ends rather than wrapping, like `move_pick` does for
    /// messages. A no-op while nothing matches.
    pub(super) fn move_switcher(&mut self, delta: isize) {
        let Some(last) = self.switcher_matches().len().checked_sub(1) else {
            return;
        };
        if let Some(switcher) = &mut self.switching {
            let current = switcher.selected.min(last) as isize;
            switcher.selected = (current + delta).clamp(0, last as isize) as usize;
        }
    }

    /// Switches to the highlighted match and closes the switcher. Does
    /// nothing while nothing matches, leaving the overlay open to edit.
    /// Choosing the channel you're already on just closes it: a real switch
    /// clears an armed reply (`finish_activating_channel`), and staying put
    /// mustn't discard one for no reason.
    pub(super) fn confirm_switcher(&mut self) {
        let Some(index) = self.switcher_highlight() else {
            return;
        };
        self.switching = None;
        if index != self.active {
            self.active = index;
            self.finish_activating_channel();
        }
    }

    pub(super) fn start_command_picker(&mut self) {
        self.command_picker = Some(CommandPicker::default());
    }

    pub(super) fn handle_command_picker_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Esc => self.command_picker = None,
            KeyCode::Enter => return self.confirm_command_picker(),
            KeyCode::Up => self.move_command_picker(-1),
            KeyCode::Down => self.move_command_picker(1),
            KeyCode::Backspace => self.edit_command_picker_query(|query| {
                query.pop();
            }),
            KeyCode::Char(c) if !ctrl => self.edit_command_picker_query(|query| query.push(c)),
            _ => {}
        }
        None
    }

    pub(super) fn handle_peers_overlay_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
        } else if key.code == KeyCode::Esc {
            self.viewing_peers = None;
        }
    }

    pub(super) fn edit_command_picker_query(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(picker) = &mut self.command_picker {
            edit(&mut picker.query);
            picker.selected = 0;
        }
    }

    pub fn command_picker_matches(&self) -> Vec<usize> {
        let Some(picker) = &self.command_picker else {
            return Vec::new();
        };
        let query = picker.query.to_lowercase();
        let command_name_matches = command_catalog()
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                command.syntax.to_lowercase().contains(&query)
                    || command
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&query))
            })
            .map(|(index, _)| index);
        let description_matches = command_catalog()
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                !command.syntax.to_lowercase().contains(&query)
                    && !command
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&query))
                    && command.description.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index);
        command_name_matches.chain(description_matches).collect()
    }

    pub fn command_picker_highlight(&self) -> Option<usize> {
        let picker = self.command_picker.as_ref()?;
        let matches = self.command_picker_matches();
        let last = matches.len().checked_sub(1)?;
        Some(matches[picker.selected.min(last)])
    }

    pub(super) fn move_command_picker(&mut self, delta: isize) {
        let Some(last) = self.command_picker_matches().len().checked_sub(1) else {
            return;
        };
        if let Some(picker) = &mut self.command_picker {
            let current = picker.selected.min(last) as isize;
            picker.selected = (current + delta).clamp(0, last as isize) as usize;
        }
    }

    pub(super) fn confirm_command_picker(&mut self) -> Option<InputAction> {
        let index = self.command_picker_highlight()?;
        let command = command_catalog()[index];
        self.command_picker = None;
        match command.picker_action {
            CommandPickerAction::Execute => {
                self.run_command(command.input.trim_start_matches('/').trim())
            }
            CommandPickerAction::Insert => {
                let channel = self.active_mut();
                channel.input = command.input.to_string();
                channel.cursor = channel.input.chars().count();
                None
            }
        }
    }

    /// Opens the file picker -- a bare `/send` -- for the channel that is
    /// active now, in the folder files were last chosen from, and asks the
    /// caller to read it. The picker shows "reading" until the listing
    /// arrives (`apply_folder_listing`), so input is never held up on it.
    pub(super) fn open_picker(&mut self) -> InputAction {
        let dir = self.browse_dir.clone();
        let request = next_request(&mut self.browse_requests);
        self.picking_file = Some(FilePicker::new(
            self.active().name.clone(),
            dir.clone(),
            request,
        ));
        InputAction::ReadFolder { request, dir }
    }

    /// Handles a keypress while the file picker is open: what the keys
    /// mean is `FilePicker::handle_key`'s business, and this turns what it
    /// asks for into state changes and an `InputAction`. `Ctrl+C` still
    /// force-quits, since `Esc` only closes the picker.
    pub(super) fn handle_picker_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return None;
        }
        let picker = self
            .picking_file
            .as_mut()
            .expect("handle_key only calls this while the picker is open");
        match picker.handle_key(key) {
            Step::Stay => None,
            Step::Cancel => {
                self.picking_file = None;
                None
            }
            Step::Open(dir) => {
                let request = next_request(&mut self.browse_requests);
                picker.read(request, dir.clone());
                Some(InputAction::ReadFolder { request, dir })
            }
            Step::Choose(paths) => {
                let picker = self.picking_file.take()?;
                self.browse_dir = picker.dir().to_path_buf();
                Some(InputAction::StageFiles {
                    channel: picker.channel,
                    paths,
                    folder: self.browse_dir.clone(),
                })
            }
        }
    }

    /// Handles `/files`: opens the files overlay on the active channel with
    /// an empty query, listing every file shared there -- type to filter,
    /// move with `Up`/`Down`, `Enter` saves, `Ctrl+O` opens one already
    /// saved, `Esc` closes (see `handle_files_key`). Which of them are saved
    /// is a fact about the disk, so the caller is asked
    /// (`InputAction::CheckSavedFiles`) and answers at once
    /// Applies a network event: updates presence, pushes a deduped incoming
    /// message, records a peer's broadcast nickname, or reports a channel
    /// join's outcome. Returns the channel name and message when `event`
    /// was a `NetEvent::Received` that resulted in a newly-accepted
    /// (non-duplicate) message, so the caller can persist it (see
    /// storage.rs) -- `None` for every other event, including a duplicate
    /// Removes `channel`'s tab and all its in-memory state (transcript,
    /// presence, dedupe buffers, any DM or point-to-point association, any
    /// `/rename` display name, and its auto-save setting -- left behind, the
    /// first would misclassify, the second mislabel and the third switch
    /// downloads on in a later channel that happens to reuse the name, see
    /// `channel_kind`/`channel_label`/`autosave`) -- called once the
    /// caller has actually torn down its gossip subscription and persisted
    /// state (see `InputAction::Leave`). Adjusts `active` to keep pointing
    /// at the same tab if it was after the removed one, or lands on the
    /// previous tab if the active tab itself was the one removed. A
    /// no-op if `channel` isn't currently joined.
    ///
    /// # Panics
    /// Panics if `channel` is leyline's only joined channel -- `AppState`
    /// always needs at least one (see `AppState::new`), and `run_leave`
    /// is the only caller that decides whether to request a removal, so
    /// it must already refuse this case before ever returning
    /// `InputAction::Leave`.
    pub fn remove_channel(&mut self, channel: &str) {
        let Some(index) = self.channels.iter().position(|c| c.name == channel) else {
            return;
        };
        assert!(
            self.channels.len() > 1,
            "must not remove leyline's only channel"
        );
        // Resolved before the association and rename are dropped below:
        // once they're gone the channel would fall back to showing its
        // raw name, and the notice should say what the channel was
        // actually called on screen.
        let heading = self.channel_heading(channel);
        self.dm_channels.retain(|_, name| name != channel);
        self.direct_channels.retain(|_, name| name != channel);
        self.direct_folders.remove(channel);
        self.shared_files.remove(channel);
        self.renames.remove(channel);
        self.autosave.remove(channel);
        self.autosave_limits.remove(channel);
        self.outbox.remove(channel);
        self.auto_saving.retain(|(name, _)| name != channel);
        self.channels.remove(index);
        if self.active > index {
            self.active -= 1;
        } else if self.active == index {
            self.active = self.active.min(self.channels.len().saturating_sub(1));
            // Only this branch actually switches to a different channel
            // (the other one just re-numbers the same channel after an
            // earlier tab's removal) -- a reply id only makes sense
            // against the channel it was picked in.
            self.picking_reply = None;
            self.picking_thread = None;
            self.replying_to = None;
        }
        self.push_system(format!("left {heading}"));
    }
}
