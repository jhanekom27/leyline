use super::*;

impl AppState {
    /// Handles per-channel `/sync` policy and path-rule configuration.
    pub(super) fn run_sync(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        let mut config = self.sync_config(&channel);
        let changed = match arg {
            "" => {
                self.push_system(self.sync_notice(&channel));
                return None;
            }
            "on" | "publish-and-receive" => {
                config.policy = SyncPolicy::PublishAndReceive;
                config.paused = false;
                true
            }
            "receive-only" => {
                config.policy = SyncPolicy::ReceiveOnly;
                config.paused = false;
                true
            }
            "off" => {
                config.policy = SyncPolicy::Off;
                config.paused = false;
                true
            }
            "pause" if config.policy != SyncPolicy::Off => {
                config.paused = true;
                true
            }
            "resume" if config.policy != SyncPolicy::Off => {
                config.paused = false;
                true
            }
            "pause" | "resume" => {
                self.push_system("sync is off -- select receive-only or publish-and-receive first");
                return None;
            }
            "clear-rules" => config.clear_rules(),
            _ => {
                let (operation, pattern) = if let Some(pattern) = arg.strip_prefix("include ") {
                    ("include", pattern)
                } else if let Some(pattern) = arg.strip_prefix("exclude ") {
                    ("exclude", pattern)
                } else if let Some(rule) = arg.strip_prefix("remove include ") {
                    ("remove include", rule)
                } else if let Some(rule) = arg.strip_prefix("remove exclude ") {
                    ("remove exclude", rule)
                } else {
                    self.push_system(
                        "usage: /sync [receive-only|publish-and-receive|off|pause|resume|include <glob>|exclude <glob>|remove <include|exclude> <glob>|clear-rules]",
                    );
                    return None;
                };
                let rule = match operation {
                    "include" | "remove include" => SyncRule::include(pattern),
                    "exclude" | "remove exclude" => SyncRule::exclude(pattern),
                    _ => unreachable!(),
                };
                match rule {
                    Ok(rule) if operation.starts_with("remove") => config.remove_rule(&rule),
                    Ok(rule) => config.add_rule(rule),
                    Err(error) => {
                        self.push_system(error);
                        return None;
                    }
                }
            }
        };
        if !changed || self.sync_config(&channel) == config {
            self.push_system(self.sync_notice(&channel));
            return None;
        }
        self.sync.insert(channel.clone(), config.clone());
        self.push_system(self.sync_notice(&channel));
        Some(InputAction::SetSync { channel, config })
    }

    /// Handles `/trash [restore <path>|purge]` for the active channel.
    pub(super) fn run_trash(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        match arg {
            "" => Some(InputAction::ListSyncTrash(channel)),
            "purge" => Some(InputAction::PurgeSyncTrash(channel)),
            _ => match arg.strip_prefix("restore ") {
                Some(path) if !path.trim().is_empty() => Some(InputAction::RestoreSyncTrash {
                    channel,
                    path: path.trim().to_string(),
                }),
                _ => {
                    self.push_system("usage: /trash [restore <path>|purge]");
                    None
                }
            },
        }
    }

    pub(super) fn run_conflicts(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        let conflicts = self.sync_conflicts(&channel);
        if arg.is_empty() {
            if conflicts.is_empty() {
                self.push_system(format!(
                    "no unresolved sync conflicts in {}",
                    self.channel_heading(&channel)
                ));
            } else {
                self.push_system(format!(
                    "unresolved sync conflicts in {} ({}):",
                    self.channel_heading(&channel),
                    conflicts.len()
                ));
                for conflict in &conflicts {
                    self.push_system(self.conflict_notice(conflict));
                }
                self.push_system(
                    "resolve with /conflicts local|remote|both <local path> -- displaced copies go to sync trash",
                );
            }
            return None;
        }
        let Some((choice, path)) = arg.split_once(' ') else {
            self.push_system("usage: /conflicts [local|remote|both <local path>]");
            return None;
        };
        let resolution = match choice {
            "local" => ConflictResolution::Local,
            "remote" => ConflictResolution::Remote,
            "both" => ConflictResolution::Both,
            _ => {
                self.push_system("usage: /conflicts [local|remote|both <local path>]");
                return None;
            }
        };
        let path = path.trim();
        let Some(conflict) = conflicts
            .into_iter()
            .find(|conflict| conflict.local_path == path)
        else {
            self.push_system(format!("no unresolved sync conflict for {path:?}"));
            return None;
        };
        Some(InputAction::ResolveSyncConflict {
            channel,
            conflict,
            resolution,
        })
    }
    /// What a bare `/sync` reports for `channel`: its persisted policy,
    /// folder, rules and runtime status, once sync has actually been
    /// configured here -- or, the first time anyone asks, a short setup
    /// notice instead (`sync_onboarding_notice`), so exploring the command
    /// on a brand-new channel doesn't read like sync is already on and
    /// idle.
    pub(super) fn sync_notice(&self, channel: &str) -> String {
        if !self.sync.contains_key(channel) {
            return self.sync_onboarding_notice(channel);
        }
        let heading = self.channel_heading(channel);
        let folder = self.folder_dir(channel);
        let status = self.sync_status(channel);
        let pending = status.map_or(0, SyncStatus::pending_count);
        let errors = status.map_or(0, SyncStatus::error_count);
        let conflicts = status.map_or(0, SyncStatus::conflict_count);
        let last_success = status
            .and_then(|status| status.last_success_unix_ms)
            .map(|timestamp| format!("last success: {timestamp}"))
            .unwrap_or_else(|| "last success: never".to_string());
        let mut report = format!(
            "sync is {} for {heading} -- folder: {}; rules: {}; {pending} pending; {conflicts} conflict(s); {errors} error(s); {last_success}",
            self.sync_config(channel).label(),
            folder.display(),
            self.sync_rule_notice(channel),
        );
        if let Some(status) = status {
            for conflict in &status.conflicts {
                report.push_str(&format!("; conflict: {}", self.conflict_notice(conflict)));
            }
            for (path, error) in &status.errors {
                report.push_str(&format!("; error: {path}: {error}"));
            }
        }
        if conflicts > 0 {
            report.push_str(
                " -- resolve with /conflicts local|remote|both <local path> -- displaced copies go to sync trash",
            );
        }
        if pending > 0 {
            report.push_str(
                " -- pending files need their peer online; check who's reachable with /who",
            );
        }
        report
    }

    /// First-use `/sync` guidance for a channel whose sync has never been
    /// configured: where its folder will be, and what the other side needs
    /// to do too -- the detail a bare status report has no room for. A
    /// group channel needs an invite first; a DM or point-to-point chat
    /// already has its one peer, so it only needs turning on both sides.
    pub(super) fn sync_onboarding_notice(&self, channel: &str) -> String {
        let heading = self.channel_heading(channel);
        let folder = self.folder_dir(channel);
        let folder = folder.display();
        match self.channel_kind(channel) {
            ChannelKind::Group => format!(
                "sync is off for {heading} -- its folder will be {folder} once enabled; invite someone into this group channel with /invite, have them /join, then run /sync publish-and-receive here and on their side (or /sync receive-only to only receive)"
            ),
            ChannelKind::GossipDm { .. } | ChannelKind::Direct { .. } => format!(
                "sync is off for {heading} -- its folder will be {folder} once enabled; run /sync publish-and-receive here and ask them to do the same on their side (or /sync receive-only to only receive)"
            ),
        }
    }

    pub(super) fn conflict_notice(&self, conflict: &SyncConflict) -> String {
        let local_time = conflict
            .local_modified_unix_ms
            .map_or_else(|| "unknown".to_string(), |time| time.to_string());
        let remote_time = conflict
            .remote_modified_unix_ms
            .map_or_else(|| "unknown".to_string(), |time| time.to_string());
        let author = conflict
            .author
            .map(|id| self.display_name(&id))
            .unwrap_or_else(|| "unknown".to_string());
        let remote_copy = conflict
            .remote_copy_path
            .as_deref()
            .unwrap_or("remote delete");
        format!(
            "local {} (modified {local_time}) vs remote {} ({remote_copy}, modified {remote_time}, author {author})",
            conflict.local_path, conflict.remote_path
        )
    }

    pub(super) fn sync_rule_notice(&self, channel: &str) -> String {
        let rules = &self.sync_config(channel).rules;
        if rules.is_empty() {
            "all paths".to_string()
        } else {
            rules
                .iter()
                .map(|rule| format!("{} {}", rule.label(), rule.pattern()))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// What `/outbox` says about the active channel's local one-way folder.
    pub(super) fn outbox_notice(&self, channel: &str) -> String {
        let heading = self.channel_heading(channel);
        let folder = self.folder_dir(channel).join("send");
        if self.outbox.contains(channel) {
            format!(
                "outbox is on for {heading} -- new stable files in {} are shared once; /outbox off turns it off",
                folder.display()
            )
        } else {
            format!(
                "outbox is off for {heading} -- /outbox on watches {} for new files to share",
                folder.display()
            )
        }
    }

    /// What `/autosave` says about `channel`: whether auto-save is on for it,
    /// whose files it takes, up to how big, and where they go -- the same
    /// words for asking and for having just switched it.
    pub(super) fn autosave_notice(&self, channel: &str) -> String {
        let heading = self.channel_heading(channel);
        let limit = human_size(self.autosave_limit_for(channel).max_bytes());
        let folder = channel_dir(&self.files_root, channel);
        let folder = folder.display();
        if self.autosave.contains(channel) {
            format!(
                "auto-save is on for {heading} -- files from your own devices and the peers you have aliased, up to {limit}, are saved into {folder} as they arrive; /autosave off turns it off"
            )
        } else {
            format!(
                "auto-save is off for {heading} -- /autosave on saves files from your own devices and the peers you have aliased, up to {limit}, into {folder} as they arrive"
            )
        }
    }

    pub(super) fn autosave_limit_notice(&self) -> String {
        let channel = &self.active().name;
        match self.autosave_limits.get(channel) {
            Some(limit) => format!(
                "automatic downloads in this channel are limited to {} -- /autosave limit default restores the device-wide fallback",
                human_size(limit.max_bytes())
            ),
            None => format!(
                "automatic downloads in this channel use the device-wide {} limit -- /autosave limit <size> sets an override",
                human_size(self.autosave_limit.max_bytes())
            ),
        }
    }

    /// What `/maxsize` says about the limit as it stands, and how to change
    /// it -- the same words for asking and for having just changed it.
    pub(super) fn size_limit_notice(&self) -> String {
        let default = human_size(DEFAULT_MAX_FILE_SIZE);
        match self.size_limit {
            SizeLimit::Default => format!(
                "files over {default} are refused (the default) -- /maxsize <size> raises or lowers it, /maxsize off lifts it"
            ),
            SizeLimit::Bytes(bytes) => format!(
                "files over {} are refused (set with /maxsize) -- /maxsize <size> changes it, /maxsize off lifts it, /maxsize default restores the {default} default",
                human_size(bytes)
            ),
            SizeLimit::Off => format!(
                "there is no limit on the size of a file you share (set with /maxsize) -- /maxsize <size> sets one, /maxsize default restores the {default} default"
            ),
        }
    }
}
