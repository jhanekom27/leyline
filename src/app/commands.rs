use super::*;

impl AppState {
    /// Moves `active` forward (`step = 1`) or backward (`step = -1`)
    /// through the channels in the order the navigation panel draws them
    /// (`nav_order`), wrapping, and clears the new active channel's unread
    /// state. Stepping in drawn order rather than join order keeps the
    /// panel's highlight moving top to bottom instead of hopping between
    /// its sections when group channels and conversations were joined
    /// interleaved; with only group channels the two orders are the same.
    /// Also clears any in-progress reply pick or armed reply -- both are
    /// only meaningful against the channel they were made in.
    pub(super) fn switch_channel(&mut self, step: isize) {
        let order = self.nav_order();
        if order.len() <= 1 {
            return;
        }
        let position = order
            .iter()
            .position(|&index| index == self.active)
            .expect("the active channel is always in the navigation order");
        let next = (position as isize + step).rem_euclid(order.len() as isize) as usize;
        self.active = order[next];
        self.finish_activating_channel();
    }

    /// `Enter`: runs a `/command`, or sends what was typed -- as a plain
    /// message or, with attachments staged on the active channel
    /// (`Channel::staged`), as the caption of the first of them. A
    /// `/command` runs as one even then (the stage is left alone), so a
    /// command can't be swallowed as a caption; and with nothing typed,
    /// staged attachments still go out on their own.
    pub(super) fn submit_input(&mut self) -> Option<InputAction> {
        let channel = self.active_mut();
        if channel.input.trim().is_empty() && channel.staged.is_empty() {
            return None;
        }
        let text = std::mem::take(&mut channel.input);
        channel.cursor = 0;
        channel.scroll = 0;

        if let Some(rest) = text.strip_prefix('/') {
            return self.run_command(rest.trim());
        }

        if !self.active().staged.is_empty() {
            return Some(self.send_staged(text));
        }

        let reply_to = self
            .replying_to
            .take()
            .or_else(|| self.active_thread_reply_target());
        Some(self.send_text(&text, reply_to))
    }

    /// The reply target a plain, non-explicitly-armed message should
    /// automatically attach to: the newest message in the active
    /// channel's open `/thread` view (`origin` itself, until the thread
    /// gains descendants of its own), or `None` if no thread view is
    /// open. Without this, a message sent while reading a thread would
    /// land with no `reply_to` at all -- part of the active channel, but
    /// not part of the conversation being viewed. Only consulted by
    /// `submit_input` after `replying_to` (armed via `Ctrl+R`/`/reply`),
    /// so an explicit pick always takes precedence over this default.
    pub(super) fn active_thread_reply_target(&self) -> Option<u64> {
        self.active()
            .thread
            .as_ref()?
            .messages
            .last()
            .map(|message| message.id)
    }

    /// Composes `text` as a message from us -- replying to `reply_to` if
    /// given -- records it in the active channel's transcript, and
    /// returns the `InputAction` for the caller to broadcast/persist. The
    /// shared tail end of sending a plain typed message (`submit_input`,
    /// which passes the armed `replying_to` if any) and `/reply <text>`'s
    /// one-shot send (`run_reply`).
    pub(super) fn send_text(&mut self, text: &str, reply_to: Option<u64>) -> InputAction {
        let channel = self.active().name.clone();
        let message = compose_message(self.self_id, text, reply_to);
        self.active_mut().remember_seen(message.id);
        self.active_mut()
            .push(TranscriptLine::Chat(message.clone()));
        InputAction::Send(channel, message)
    }

    /// Sends everything staged on the active channel: takes the stage (so
    /// it is empty from here on), leaves a breadcrumb per file the way
    /// `run_send` does -- importing and announcing happen in the
    /// background and may take a moment -- and returns the `InputAction`
    /// for the caller to act on. `text` is whatever was typed; unless it is
    /// only whitespace it becomes the caption of the first attachment. An
    /// armed reply or open thread is left as it was: a shared file is
    /// never a reply, same as `/send` today.
    pub(super) fn send_staged(&mut self, text: String) -> InputAction {
        let channel = self.active().name.clone();
        let items = std::mem::take(&mut self.active_mut().staged);
        for item in &items {
            self.push_system(format!("sending {}...", item.filename));
        }
        let caption = if text.trim().is_empty() {
            String::new()
        } else {
            text
        };
        InputAction::SendStaged {
            channel,
            items,
            caption,
        }
    }

    /// Drops everything staged on the active channel -- `Esc`'s second
    /// level, see `handle_key`. Returns whether there was anything to drop,
    /// so the caller knows whether the key was used up.
    pub(super) fn clear_staged(&mut self) -> bool {
        let staged = &mut self.active_mut().staged;
        let had_any = !staged.is_empty();
        staged.clear();
        had_any
    }

    /// Clears the active channel's search filter and records the result.
    /// Returns whether a filter was active, so callers such as `Esc` know
    /// whether they consumed the key.
    pub(super) fn clear_active_search(&mut self) -> bool {
        let had_search = self.active_mut().search.take().is_some();
        if had_search {
            self.push_system("search cleared");
        }
        had_search
    }

    /// Parses a `/command [args]` line (the text after the leading `/`,
    /// already trimmed). Recognized commands may still return `None` after
    /// pushing a system notice (e.g. usage errors) -- only `/join`,
    /// `/invite`, and a successful `/alias` or `/nick` need the caller to
    /// actually do anything.
    pub(super) fn run_command(&mut self, command: &str) -> Option<InputAction> {
        let (name, arg) = command.split_once(' ').unwrap_or((command, ""));
        match name {
            "join" => {
                let arg = arg.trim();
                if arg.is_empty() {
                    self.push_system("usage: /join <channel-name-or-ticket>");
                    None
                } else if arg == DEVICE_SYNC_CHANNEL {
                    self.push_system(format!(
                        "#{DEVICE_SYNC_CHANNEL} is reserved for syncing your own paired devices"
                    ));
                    None
                } else {
                    Some(InputAction::Join(arg.to_string()))
                }
            }
            "invite" => self.run_invite(),
            "pair" => self.run_pair(arg.trim()),
            "forget-device" => self.run_forget_device(arg.trim()),
            "msg" => self.run_msg(arg.trim()),
            "dm" => self.run_dm(arg.trim()),
            "alias" => self.run_alias(arg.trim()),
            "nick" => self.run_nick(arg.trim()),
            "leave" => self.run_leave(arg.trim()),
            "rename" => self.run_rename(arg.trim()),
            "who" => self.run_who(),
            "send" => self.run_send(arg.trim()),
            "save" => self.run_save(arg.trim()),
            "folder" => self.run_folder(arg.trim()),
            "maxsize" => self.run_maxsize(arg.trim()),
            "autosave" => self.run_autosave(arg.trim()),
            "outbox" => self.run_outbox(arg.trim()),
            "sync" => self.run_sync(arg.trim()),
            "activity" => Some(InputAction::ListSyncActivity(self.active().name.clone())),
            "conflicts" => self.run_conflicts(arg.trim()),
            "trash" => self.run_trash(arg.trim()),
            "files" => self.run_files(),
            "paste" => self.run_paste(),
            "search" | "s" => self.run_search(arg.trim()),
            "reply" => self.run_reply(arg.trim()),
            "thread" => self.run_thread(),
            "hints" => self.run_hints(),
            "mouse" => self.run_mouse(),
            "bell" => self.run_bell(),
            "version" => self.run_version(),
            "help" => self.run_help(),
            _ => {
                self.push_system(format!("unknown command: /{name}"));
                None
            }
        }
    }

    /// Handles `/invite`: builds an invite ticket for the active channel,
    /// unless it's a point-to-point one (see `direct_channels`) -- those
    /// have no ticket or room secret at all (`net::Net::ticket_for` would
    /// panic if asked for one), so this refuses with a clear message
    /// instead of ever reaching the caller's network layer.
    pub(super) fn run_invite(&mut self) -> Option<InputAction> {
        let channel = self.active().name.clone();
        if self.direct_peer_for(&channel).is_some() {
            self.push_system(
                "point-to-point conversations have no invite ticket -- share your endpoint id directly instead",
            );
            return None;
        }
        Some(InputAction::Invite(channel))
    }

    /// Handles `/help`: pushes the full command/keybinding reference as a
    /// system notice. The input box's border only shows a short hint (see
    /// `ui::render_input`) to stay uncluttered, so this is where that
    /// detail actually lives. One entry per line, with a blank line between
    /// entries -- `ui::build_rows` renders each `\n` as its own row.
    pub(super) fn run_help(&mut self) -> Option<InputAction> {
        let autosave_limit = human_size(self.autosave_limit.max_bytes());
        let commands = [
            "/join <name|ticket> -- join a channel from a ticket, or create a new private one by name".to_string(),
            "/invite -- show an invite ticket for this channel".to_string(),
            "/pair [ticket] -- with no argument, shows a ticket for a new device to redeem; /pair <ticket> redeems one (after an Enter/Esc confirmation -- this replaces this device's identity and data)".to_string(),
            "/forget-device <hex-prefix> -- forget a paired device".to_string(),
            "/msg <alias|hex-prefix> [text] -- start a private channel message with a peer".to_string(),
            "/dm <alias|hex-prefix|endpoint-id> [text] -- connect directly to a peer's endpoint id with no ticket or gossip channel, unlike /msg".to_string(),
            "/leave [channel] -- leave a channel".to_string(),
            "/rename [name] -- set this channel's local display name".to_string(),
            "/who -- open this device plus online and remembered peer details for the active channel".to_string(),
            "/send [path] -- share a file; a bare /send opens a file picker".to_string(),
            "/save [hash-prefix] -- download a shared file; each channel's files are kept in a folder of their own".to_string(),
            "/folder [path] -- show where saved files go and, with a path, move them all".to_string(),
            "/maxsize [size|off|default] -- show the biggest file you can share (500 MB to begin with) and, with a size like 2GB, change it (off lifts it, default restores it)".to_string(),
            format!("/autosave [on|off|limit <size|default>] -- /autosave on saves trusted peers' files in this channel as they arrive, up to its limit (the device-wide {autosave_limit} default unless this channel overrides it); /autosave limit <size> sets this channel's finite override and default restores its device-wide fallback"),
            "/outbox [on|off] -- /outbox on watches this channel's local send folder and shares new stable files once (off by default; existing, hidden, and temporary files are ignored)".to_string(),
            "/files -- list shared attachments and local synced files in this channel (type to filter, \u{2191}/\u{2193} pick, Enter save a shared file, Ctrl+O open, Ctrl+R reveal, Ctrl+E edit a local copy; never default-open a program or script, Esc close)".to_string(),
            "/paste -- share the clipboard".to_string(),
            "/sync [receive-only|publish-and-receive|off|pause|resume|include <glob>|exclude <glob>|remove <include|exclude> <glob>|clear-rules] -- /sync receive-only mirrors safe remote index entries into this channel's folder, while /sync publish-and-receive also publishes stable local files (off by default, per channel); /sync include and /sync exclude add relative glob rules, with excludes taking precedence; it never propagates deletes or renames".to_string(),
            "/conflicts [local|remote|both <path>] -- list unresolved sync collisions in this channel; /conflicts local|remote|both <path> resolves one by keeping the chosen copy (or both) and moving any displaced copy to recoverable sync trash".to_string(),
            "/trash [restore <path>|purge] -- list this channel's recoverable remote deletes; /trash restore <path> brings one back without overwriting a local file, and /trash purge permanently removes them all (trash is never automatically purged)".to_string(),
            "/alias <hex-prefix> <name> -- set a local name for a peer".to_string(),
            "/nick <name> -- set your broadcast display name".to_string(),
            "/search <term> (or /s) -- search channel messages".to_string(),
            "/reply [text] -- reply to the latest message".to_string(),
            "/thread -- view the latest reply thread".to_string(),
            "/hints -- toggle the command hints panel".to_string(),
            "/mouse -- toggle mouse reporting (on by default, remembered across restarts); hover a command hint or a peer to see a popup; while on, hold Shift (or Option in iTerm2) to select text".to_string(),
            "/bell -- toggle the message bell".to_string(),
            "/version -- show this build's version".to_string(),
            "/help -- show this help".to_string(),
        ];
        let keys = [
            "Tab/Shift+Tab -- switch channels",
            "Ctrl+P -- jump to a channel (type to filter, \u{2191}/\u{2193} pick, Enter confirm)",
            "Ctrl+K -- browse slash commands (type to filter; ↑/↓ pick; each row says whether Enter runs it, keeping your draft, or inserts it, replacing your draft)",
            "/send file picker -- type to filter, \u{2191}/\u{2193} pick, Space mark, Enter open a folder or stage the marked files, Backspace up a level, Esc cancel",
            "Alt+Enter/Ctrl+J -- newline",
            "Ctrl+V -- share the clipboard (a file or image is staged first: Enter sends it, Backspace on an empty input unstages, Esc clears)",
            "Ctrl+R -- reply (\u{2191}/\u{2193} pick, Enter confirm)",
            "Ctrl+T -- thread (\u{2191}/\u{2193} pick, Enter confirm)",
            "Up/Down -- scroll",
            "Esc -- close /who peer details, the Ctrl+P jump-to list, or the Ctrl+K command picker; cancel an in-progress reply or thread pick, or an armed reply; otherwise clear an active search, clear staged attachments, leave an open thread, or quit",
            "Ctrl+C -- quit",
        ];
        self.push_system(format!(
            "commands:\n\n{}\n\nkeys:\n\n{}",
            commands.join("\n\n"),
            keys.join("\n\n"),
        ));
        None
    }

    /// Mouse hover needs mouse reporting (`/mouse`) and is suppressed
    /// while another view owns input.
    pub fn can_hover(&self) -> bool {
        self.mouse_enabled
            && self.command_picker.is_none()
            && self.viewing_peers.is_none()
            && self.switching.is_none()
            && self.picking_file.is_none()
            && self.viewing_files.is_none()
            && self.pending_redeem.is_none()
            && self.picking_reply.is_none()
            && self.picking_thread.is_none()
    }

    /// Like `can_hover`, but also needs the command hints panel shown
    /// (`/hints`).
    pub fn can_hover_hints(&self) -> bool {
        self.show_hints && self.can_hover()
    }

    /// Records the command-hints row measured under the mouse by the
    /// read-only UI layout (`ui::command_hint_at`); returns whether the
    /// frame changed.
    pub fn set_hint_hover(&mut self, row: Option<usize>) -> bool {
        let row = row.filter(|_| self.can_hover_hints());
        if self.hint_hover == row {
            return false;
        }
        self.hint_hover = row;
        true
    }

    /// The hovered command-hints row whose popup should show -- never
    /// while another view owns input, even before the next mouse event
    /// clears the hover.
    pub fn hovered_hint(&self) -> Option<usize> {
        self.hint_hover.filter(|_| self.can_hover_hints())
    }

    /// Records the peers-panel peer measured under the mouse by the
    /// read-only UI layout (`ui::peer_at`); returns whether the frame
    /// changed. Unlike `set_hint_hover`, works whether or not the hints
    /// panel is shown.
    pub fn set_peer_hover(&mut self, peer: Option<[u8; 32]>) -> bool {
        let peer = peer.filter(|_| self.can_hover());
        if self.peer_hover == peer {
            return false;
        }
        self.peer_hover = peer;
        true
    }

    /// The hovered peer's popup details -- never while another view owns
    /// input, nor once that peer has left the active channel's list, even
    /// before the next mouse event clears the hover.
    pub fn hovered_peer(&self) -> Option<PeerDetails> {
        let id = self.peer_hover.filter(|_| self.can_hover())?;
        let peer = self
            .peer_presence()
            .into_iter()
            .find(|peer| peer.id == id)?;
        Some(self.peer_details(&peer))
    }

    /// Everything the peers-panel popup shows about `peer`. Alias, nickname,
    /// and version are recorded per person (`canonical_id`), like
    /// `display_name`.
    pub fn peer_details(&self, peer: &PeerPresence) -> PeerDetails {
        let id = self.canonical_id(&peer.id);
        PeerDetails {
            id: peer.id,
            alias: self.petnames.get(&id).cloned(),
            nickname: self.nicknames.get(&id).cloned(),
            last_seen_unix_ms: peer.last_seen_unix_ms,
            version: self
                .peer_versions
                .get(&id)
                .map(|(version, _)| version.clone()),
        }
    }

    pub(super) fn self_peer_details(&self) -> PeerDetails {
        PeerDetails {
            id: self.self_id,
            alias: None,
            nickname: self.local_nickname.clone(),
            last_seen_unix_ms: None,
            version: None,
        }
    }

    /// Handles `/hints`: toggles the sidebar's command hints panel (see
    /// `ui::render_sidebar`) and reports the new state as a system
    /// notice. Purely local UI state -- no `InputAction` needed, unlike
    /// commands that require the caller to do network I/O.
    pub(super) fn run_hints(&mut self) -> Option<InputAction> {
        self.show_hints = !self.show_hints;
        self.set_hint_hover(None);
        // The peers panel grows or shrinks with the hints panel, so the
        // row under the mouse may now be someone else.
        self.set_peer_hover(None);
        let state = if self.show_hints { "shown" } else { "hidden" };
        self.push_system(format!("command hints {state}"));
        None
    }

    /// Handles `/mouse`: toggles mouse reporting, which hover popups need,
    /// and reports the new state as a system notice. Persisted like
    /// `/bell`, so an `InputAction` is returned for the caller to save it.
    pub(super) fn run_mouse(&mut self) -> Option<InputAction> {
        self.mouse_enabled = !self.mouse_enabled;
        self.set_hint_hover(None);
        self.set_peer_hover(None);
        let state = if self.mouse_enabled { "on" } else { "off" };
        self.push_system(format!("mouse reporting {state}"));
        Some(InputAction::Mouse(self.mouse_enabled))
    }

    /// Handles `/bell`: toggles whether an incoming message rings the
    /// terminal bell (see main.rs's `ring_bell`) and reports the new state
    /// as a system notice, mirroring `run_hints`. Unlike `show_hints`,
    /// this preference is persisted (see settings.rs), so -- unlike
    /// `run_hints` -- an `InputAction` is returned for the caller to save
    /// it to disk.
    pub(super) fn run_bell(&mut self) -> Option<InputAction> {
        self.bell_enabled = !self.bell_enabled;
        let state = if self.bell_enabled { "on" } else { "off" };
        self.push_system(format!("message bell {state}"));
        Some(InputAction::Bell(self.bell_enabled))
    }

    /// Handles `/alias <hex-prefix> <name>`: `arg` is everything after
    /// `/alias ` (already trimmed). Splits it into the hex-prefix and the
    /// (possibly multi-word) name to assign, resolves the prefix via
    /// `resolve_id`, and on success updates `petnames` immediately -- for
    /// instant UI feedback -- while returning an `InputAction::Alias` for
    /// the caller to persist (see contacts.rs). Usage and resolution
    /// errors are reported as a system notice instead.
    pub(super) fn run_alias(&mut self, arg: &str) -> Option<InputAction> {
        let Some((prefix, name)) = arg.split_once(' ') else {
            self.push_system("usage: /alias <hex-prefix> <name>");
            return None;
        };
        let name = name.trim();
        if name.is_empty() {
            self.push_system("usage: /alias <hex-prefix> <name>");
            return None;
        }
        match self.resolve_id(prefix) {
            Ok(id) => {
                // Store under the canonical id (the user id, if a
                // certificate for `id` is already known) so the alias
                // applies to every one of that person's known devices, not
                // just the specific one whose hex prefix was typed --
                // while still confirming against the device id actually
                // typed, which is what the user recognizes.
                let canonical = self.canonical_id(&id);
                self.petnames.insert(canonical, name.to_string());
                self.push_system(format!("aliased {} as {name}", hex_id(&id)));
                Some(InputAction::Alias(canonical, name.to_string()))
            }
            Err(err) => {
                self.push_system(err);
                None
            }
        }
    }

    /// Handles `/pair` with no argument: builds a one-time pairing ticket
    /// for a brand-new device to redeem, and reports it back as a system
    /// notice, mirroring `/invite`. See `run_pair_redeem` for `/pair
    /// <ticket>`, which redeems one instead.
    pub(super) fn run_pair(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            return Some(InputAction::Pair);
        }
        self.run_pair_redeem(arg)
    }

    /// Handles `/pair <ticket>`: validates `ticket` decodes (a pure parse,
    /// no I/O -- see this module's doc comment) and, if it does, arms
    /// `pending_redeem` and warns that confirming will replace this
    /// device's current identity, channels, and message history with the
    /// paired device's, naming how many channels are currently joined so
    /// the stakes are concrete rather than abstract. Deliberately rejects
    /// a malformed ticket immediately, before ever arming the
    /// confirmation or touching anything -- see main.rs's `restart_for_
    /// pairing` for what actually gets wiped once confirmed
    /// (`handle_redeem_confirmation_key`).
    pub(super) fn run_pair_redeem(&mut self, ticket: &str) -> Option<InputAction> {
        match PairingTicket::decode_string(ticket) {
            Ok(_) => {
                self.pending_redeem = Some(ticket.to_string());
                self.push_system(format!(
                    "redeeming this ticket will ERASE this device's current identity, {} \
                     channel(s), and all local message history, replacing them with the \
                     paired device's -- this cannot be undone. Press Enter to confirm, Esc \
                     to cancel.",
                    self.channels.len()
                ));
            }
            Err(err) => {
                self.push_system(format!("invalid pairing ticket: {err}"));
            }
        }
        None
    }

    /// Handles `/forget-device <hex-prefix>`: resolves the prefix via
    /// `resolve_id` and, on success, immediately stops trusting that
    /// device locally -- for instant UI feedback -- while returning an
    /// `InputAction::ForgetDevice` for the caller to persist (see
    /// `crate::forgotten_devices`). Deliberately operates on the literal
    /// device id resolved, not its canonical user id (unlike `/alias`):
    /// forgetting targets one specific device (e.g. a lost phone), not
    /// every device belonging to that person. This is local and
    /// best-effort, not real revocation -- see
    /// `crate::forgotten_devices`'s doc comment.
    pub(super) fn run_forget_device(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /forget-device <hex-prefix>");
            return None;
        }
        match self.resolve_id(arg) {
            Ok(id) => {
                self.forgotten.insert(id);
                self.push_system(format!("forgot device {}", hex_id(&id)));
                Some(InputAction::ForgetDevice(id))
            }
            Err(err) => {
                self.push_system(err);
                None
            }
        }
    }

    /// Handles `/nick <name>`: `arg` is everything after `/nick ` (already
    /// trimmed). Unlike `/alias`, there's no id to resolve -- a `/nick` is
    /// about our own identity, not a peer's -- so a bare non-empty name is
    /// all that's needed. Reports the outcome as a system notice either
    /// way, and returns an `InputAction::Nick` for the caller to broadcast
    /// (see `net::Net::set_nickname`) on success.
    pub(super) fn run_nick(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /nick <name>");
            return None;
        }
        self.local_nickname = Some(arg.to_string());
        self.push_system(format!(
            "nickname set to {arg} (broadcast to joined channels)"
        ));
        Some(InputAction::Nick(arg.to_string()))
    }

    /// Handles `/leave [channel]`: `arg` is everything after `/leave `
    /// (already trimmed), naming the channel to leave -- as it appears on
    /// screen, see `resolve_channel_arg` -- or empty to leave the
    /// currently active one (mirroring `/invite`). Refuses -- with a
    /// system notice, returning `None` -- to leave a channel that isn't
    /// currently joined, or leyline's only remaining joined channel,
    /// since `AppState` always needs at least one (see `AppState::new`).
    /// On success, returns an `InputAction::Leave` for the caller to tear
    /// down; the tab itself is only removed once that's done, via
    /// `remove_channel`.
    pub(super) fn run_leave(&mut self, arg: &str) -> Option<InputAction> {
        let target = if arg.is_empty() {
            self.active().name.clone()
        } else {
            match self.resolve_channel_arg(arg) {
                Ok(name) => name,
                Err(err) => {
                    self.push_system(err);
                    return None;
                }
            }
        };
        if self.channels.len() <= 1 {
            self.push_system("cannot leave your only channel");
            return None;
        }
        Some(InputAction::Leave(target))
    }

    /// Resolves a channel typed by the user (e.g. after `/leave`) to a
    /// joined channel's own `name`. Tries an exact match on `name` first,
    /// then on what's shown for it: its label (`Alice`) or its heading
    /// (`@Alice`, `#general`). The label differs from `name` for a DM or
    /// point-to-point chat, and the heading is how a channel is named in
    /// titles and notices. Errors on a reference shared by several
    /// channels -- a `/msg` DM and a `/dm` chat with the same person share
    /// a label, as do two peers given the same alias -- rather than
    /// guessing which was meant.
    pub(super) fn resolve_channel_arg(&self, typed: &str) -> Result<String, String> {
        if self.channels.iter().any(|c| c.name == typed) {
            return Ok(typed.to_string());
        }
        let matches: Vec<&str> = self
            .channels
            .iter()
            .map(|c| c.name.as_str())
            .filter(|name| self.channel_label(name) == typed || self.channel_heading(name) == typed)
            .collect();
        match matches.as_slice() {
            [name] => Ok((*name).to_string()),
            [] => Err(format!("not currently in #{typed}")),
            _ => Err(format!(
                "'{typed}' matches more than one channel -- switch to the one you mean and run /leave with no argument"
            )),
        }
    }

    /// Handles `/rename [name]`: `arg` is everything after `/rename `
    /// (already trimmed). Gives the active *group* channel a local display
    /// name -- shown wherever the channel is named (`channel_label`), kept
    /// in `renames`, and never sent to peers -- and returns an
    /// `InputAction::Rename` for the caller to persist (see
    /// `crate::channel_renames`). With no name it restores the stored one
    /// instead, the way a bare `/search` clears a search, or shows usage
    /// when there's nothing to restore.
    ///
    /// Refuses a DM or point-to-point chat, whose label already follows the
    /// peer's `/alias` -- a second, independent override for the same
    /// conversation would only be confusing -- and a name that could be
    /// mistaken for another channel (`clashes_with_another_channel`).
    pub(super) fn run_rename(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        if self.channel_kind(&channel) != ChannelKind::Group {
            self.push_system(
                "a conversation takes its name from the peer -- use /alias <hex-prefix> <name> to rename them",
            );
            return None;
        }
        if arg.is_empty() {
            if self.renames.remove(&channel).is_none() {
                self.push_system(
                    "usage: /rename <name> (a bare /rename restores the original name)",
                );
                return None;
            }
            self.push_system(format!("name restored to #{channel}"));
            return Some(InputAction::Rename {
                channel,
                name: None,
            });
        }
        if self.clashes_with_another_channel(&channel, arg) {
            self.push_system(format!(
                "another channel already goes by #{arg} -- pick a different name"
            ));
            return None;
        }
        self.renames.insert(channel.clone(), arg.to_string());
        self.push_system(format!(
            "renamed #{channel} to #{arg} (local only -- peers still see #{channel})"
        ));
        Some(InputAction::Rename {
            channel,
            name: Some(arg.to_string()),
        })
    }

    /// Whether `name` could be mistaken for a channel other than `channel`:
    /// it's another channel's stored name -- which `/leave` matches before
    /// anything displayed (`resolve_channel_arg`), so a stored name hidden
    /// behind a rename would otherwise shadow the one on screen -- or it's
    /// what another group channel already shows, which would leave two
    /// identical `#name` rows no one could tell apart. A conversation can
    /// share a name with a group channel: its `@` or bolt heading sets the
    /// two apart.
    pub(super) fn clashes_with_another_channel(&self, channel: &str, name: &str) -> bool {
        self.channels
            .iter()
            .filter(|other| other.name != channel)
            .any(|other| {
                other.name == name
                    || (self.channel_kind(&other.name) == ChannelKind::Group
                        && self.channel_label(&other.name) == name)
            })
    }

    /// Handles `/who`: opens a local, dismissible table for the active
    /// channel's online and remembered device endpoints. Unlike the sidebar,
    /// this keeps separate rows for a person's devices; alias and nickname
    /// still follow `peer_details`' canonical identity lookup. The table is
    /// a snapshot, so a focus change or network event cannot replace its data
    /// while it is open.
    pub(super) fn run_who(&mut self) -> Option<InputAction> {
        let heading = self.channel_heading(&self.active().name);
        let presence = self.who_presence();
        let first_offline = presence
            .iter()
            .position(|peer| peer.last_seen_unix_ms.is_some())
            .unwrap_or(presence.len());
        let mut peers: Vec<_> = presence[..first_offline]
            .iter()
            .map(|peer| self.peer_details(peer))
            .collect();
        peers.push(self.self_peer_details());
        peers.extend(
            presence[first_offline..]
                .iter()
                .map(|peer| self.peer_details(peer)),
        );
        self.viewing_peers = Some(PeersOverlay { heading, peers });
        None
    }

    /// Handles `/version`: reports our own build version and git commit,
    /// plus the newest peer version we've heard that's newer than ours, if
    /// any (see `newest_known_update`). Purely local, like `/who`/`/help`:
    /// no `InputAction` needed.
    pub(super) fn run_version(&mut self) -> Option<InputAction> {
        let update = self
            .newest_known_update()
            .map(|(version, hash)| (version.to_string(), hash.to_string()));
        self.push_system(format!(
            "running v{} ({})",
            version::VERSION,
            version::GIT_HASH
        ));
        if let Some((peer_version, peer_hash)) = update {
            self.push_system(format!(
                "a peer is running a newer v{peer_version} ({peer_hash}) -- consider updating"
            ));
        }
        None
    }

    /// Handles `/search <term>` (`/s` is a shorthand alias, see
    /// `run_command`): `arg` is everything after the command name (already
    /// trimmed). With no argument, clears an active search filter -- or
    /// shows a usage hint if none is active -- mirroring `/hints`'s toggle
    /// shape. Otherwise scans the active channel's loaded transcript
    /// synchronously for an instant result (`Channel::search_loaded`),
    /// stores it as the channel's active search, and returns an
    /// `InputAction::Search` so the caller can extend it with a background
    /// scan of history older than what's loaded (see
    /// `search::run_on_disk_scan` and `AppState::apply_search_outcome`).
    /// The pane title (`ui::render_messages`) reminds you a search is
    /// active and how to clear it for as long as it stays active, since
    /// this system notice will otherwise scroll out of view.
    pub(super) fn run_search(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            if !self.clear_active_search() {
                self.push_system("usage: /search <term> (or /s <term>)");
            }
            return None;
        }
        let term_lower = arg.to_lowercase();
        let messages = self.active().search_loaded(&term_lower);
        let count = messages.len();
        self.active_mut().thread = None;
        self.active_mut().search = Some(SearchResults {
            term: arg.to_string(),
            messages,
            pending: true,
        });
        self.push_system(format!(
            "search: {count} match(es) so far for '{arg}' (scanning full history...) -- /s to clear"
        ));
        Some(InputAction::Search {
            channel: self.active().name.clone(),
            term: arg.to_string(),
        })
    }

    /// Finds the newest message currently visible in the active channel --
    /// shared by reply targeting (`newest_reply_candidate`) and thread
    /// targeting (`newest_thread_candidate`), which differ only in which
    /// Handles `/send [path]`: `arg` is everything after `/send ` (already
    /// trimmed). With no path, opens the file picker (`open_picker`) to
    /// choose files in. With one, it names a local file to share with the
    /// active channel right away: strips one layer of surrounding quotes
    /// (see `strip_quotes`) and otherwise only checks that a path was given
    /// -- app.rs never touches the filesystem (see this module's doc
    /// comment), so the path itself (existence, `~`-expansion, etc.) is
    /// validated and resolved by the caller (`net::Net::send_file`) once
    /// this returns. Pushes an immediate breadcrumb since importing and
    /// broadcasting a file happens in the background and may take a moment.
    pub(super) fn run_send(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            return Some(self.open_picker());
        }
        let path = strip_quotes(arg);
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        self.push_system(format!("sending {name}..."));
        Some(InputAction::SendFile {
            channel: self.active().name.clone(),
            path: path.to_string(),
        })
    }
}
