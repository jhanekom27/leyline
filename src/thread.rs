//! Reply-chain ("thread") building for `/thread` -- walks a message's
//! `reply_to` links both up (ancestors) and down (descendants) to isolate
//! just that conversation from the rest of a channel's transcript.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::message::ChatMessage;

/// A channel's active `/thread` view -- see `app::Channel::thread`.
///
/// Grows live as new messages that directly extend it are sent or arrive
/// (`try_append`, called from `app::Channel::push`), so a reply made
/// while reading a thread shows up immediately instead of needing
/// `/thread` re-run. It can still fall behind in one narrow case: an
/// incoming message whose own parent hasn't been resolved into the view
/// yet -- unlike a locally-composed reply, which always targets
/// something already shown (see `app::AppState::active_thread_reply_target`),
/// a peer's message can arrive before the message it replies to. Re-run
/// `/thread`, or pick a new message with `Ctrl+T`, to rebuild it from
/// scratch if that happens.
pub struct ThreadView {
    /// The message the thread was opened from -- shown in the pane title
    /// and the opening system notice (`app::AppState::open_thread`) so
    /// it's clear what's being viewed.
    pub origin: ChatMessage,
    /// Every message in `origin`'s chain: its ancestors (walking
    /// `reply_to` up), `origin` itself, and its descendants (anything
    /// that transitively replies to it) -- oldest first. Excludes sibling
    /// branches off an ancestor: only the direct line through `origin` is
    /// included.
    pub messages: Vec<ChatMessage>,
}

impl ThreadView {
    /// Appends `message` to this view if it belongs: it replies (directly)
    /// to a message already in the chain. Returns whether it was added,
    /// so a caller that just pushed `message` into a channel's broader
    /// transcript (`app::Channel::push`) can also keep an already-open
    /// thread view current, instead of only ever reflecting whatever was
    /// in scope when `/thread`/`Ctrl+T` last (re)built it. Re-sorts by
    /// `(ts_unix_ms, id)` after inserting, matching `build_chain`'s own
    /// ordering, though in practice a newly-arrived message is almost
    /// always the newest. Ignores a message already present (defensive --
    /// `Channel::push`'s callers already dedupe by id before ever
    /// reaching this).
    pub fn try_append(&mut self, message: &ChatMessage) -> bool {
        let belongs = message
            .reply_to
            .is_some_and(|parent| self.messages.iter().any(|existing| existing.id == parent));
        if !belongs
            || self
                .messages
                .iter()
                .any(|existing| existing.id == message.id)
        {
            return false;
        }
        self.messages.push(message.clone());
        self.messages
            .sort_by_key(|existing| (existing.ts_unix_ms, existing.id));
        true
    }
}

/// Builds `origin`'s reply chain from `pool` (a channel's loaded chat
/// messages -- see `app::AppState::open_thread`): `origin`'s ancestors
/// (walking `reply_to` upward), `origin` itself, and its descendants
/// (anything that transitively replies to it), sorted by `(ts_unix_ms,
/// id)` like every other message list in the app.
///
/// `origin` need not be present in `pool` itself -- it's always included
/// regardless, which matters since a thread can be opened on a message
/// resolved only via an active search's on-disk-scan results (see
/// `app::Channel::find_message`). Ancestor-walking stops at a missing
/// parent or a repeated id, since `reply_to` is never verified to resolve
/// or be acyclic -- see `ChatMessage::reply_to`'s doc comment.
pub fn build_chain(pool: &[ChatMessage], origin: &ChatMessage) -> Vec<ChatMessage> {
    let by_id: HashMap<u64, &ChatMessage> =
        pool.iter().map(|message| (message.id, message)).collect();
    let mut seen = HashSet::new();
    seen.insert(origin.id);
    let mut chain = vec![origin.clone()];

    // Ancestors: walk `reply_to` pointers up.
    let mut cursor = origin.reply_to;
    while let Some(parent_id) = cursor {
        if !seen.insert(parent_id) {
            break; // cycle guard
        }
        let Some(parent) = by_id.get(&parent_id) else {
            break; // dangling parent -- stop rather than guessing
        };
        chain.push((*parent).clone());
        cursor = parent.reply_to;
    }

    // Descendants: breadth-first over anything that transitively replies
    // to `origin`.
    let mut frontier = VecDeque::new();
    frontier.push_back(origin.id);
    while let Some(id) = frontier.pop_front() {
        for message in pool {
            if message.reply_to == Some(id) && seen.insert(message.id) {
                chain.push(message.clone());
                frontier.push_back(message.id);
            }
        }
    }

    chain.sort_by_key(|message| (message.ts_unix_ms, message.id));
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: u64, ts: u64, reply_to: Option<u64>) -> ChatMessage {
        ChatMessage {
            v: 3,
            id,
            sender: [1; 32],
            ts_unix_ms: ts,
            text: format!("message {id}"),
            attachment: None,
            reply_to,
        }
    }

    #[test]
    fn build_chain_includes_just_the_origin_when_it_has_no_relatives() {
        let origin = message(1, 100, None);
        let chain = build_chain(std::slice::from_ref(&origin), &origin);
        assert_eq!(chain.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn build_chain_walks_ancestors_up_to_the_root() {
        let grandparent = message(1, 100, None);
        let parent = message(2, 200, Some(1));
        let origin = message(3, 300, Some(2));
        let pool = vec![grandparent, parent, origin.clone()];

        let chain = build_chain(&pool, &origin);

        assert_eq!(
            chain.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn build_chain_walks_descendants_transitively() {
        let origin = message(1, 100, None);
        let reply = message(2, 200, Some(1));
        let reply_to_reply = message(3, 300, Some(2));
        let pool = vec![origin.clone(), reply, reply_to_reply];

        let chain = build_chain(&pool, &origin);

        assert_eq!(
            chain.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn build_chain_includes_both_ancestors_and_descendants() {
        let grandparent = message(1, 100, None);
        let origin = message(2, 200, Some(1));
        let reply = message(3, 300, Some(2));
        let pool = vec![grandparent, origin.clone(), reply];

        let chain = build_chain(&pool, &origin);

        assert_eq!(
            chain.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn build_chain_excludes_an_unrelated_sibling_reply() {
        let parent = message(1, 100, None);
        let origin = message(2, 200, Some(1));
        // Also replies to `parent`, not `origin`.
        let sibling = message(3, 300, Some(1));
        let pool = vec![parent, origin.clone(), sibling];

        let chain = build_chain(&pool, &origin);

        assert_eq!(
            chain.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2],
            "a sibling reply to the same parent must not be pulled into origin's thread"
        );
    }

    #[test]
    fn build_chain_stops_the_ancestor_walk_at_a_missing_parent() {
        let origin = message(2, 200, Some(1)); // id 1 is never actually in `pool`
        let pool = vec![origin.clone()];

        let chain = build_chain(&pool, &origin);

        assert_eq!(chain.iter().map(|m| m.id).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn build_chain_terminates_on_a_cycle() {
        // A replies to B, B replies to A -- reply_to is never verified to
        // be acyclic (see ChatMessage::reply_to's doc comment), so the
        // ancestor walk must still terminate instead of looping forever.
        let a = message(1, 100, Some(2));
        let b = message(2, 200, Some(1));
        let pool = vec![a.clone(), b];

        let mut ids: Vec<u64> = build_chain(&pool, &a).iter().map(|m| m.id).collect();
        ids.sort();

        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn build_chain_includes_the_origin_even_if_absent_from_the_pool() {
        // Mirrors `Channel::find_message`'s own fallback: a thread can be
        // opened on a message resolved only via an active search's
        // on-disk-scan results, which may not be in the loaded pool.
        let origin = message(1, 100, None);

        let chain = build_chain(&[], &origin);

        assert_eq!(chain.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn build_chain_sorts_by_timestamp_not_discovery_order() {
        // id 1 (an ancestor, discovered after `origin`) has an earlier
        // timestamp than id 2 (`origin`) -- the final chain must be
        // ordered by (ts_unix_ms, id), not by discovery order.
        let parent = message(1, 50, None);
        let origin = message(2, 200, Some(1));
        let pool = vec![parent, origin.clone()];

        let chain = build_chain(&pool, &origin);

        assert_eq!(chain.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn build_chain_breaks_timestamp_ties_by_id() {
        let origin = message(5, 100, None);
        let reply_a = message(2, 100, Some(5));
        let reply_b = message(1, 100, Some(5));
        let pool = vec![origin.clone(), reply_a, reply_b];

        let chain = build_chain(&pool, &origin);

        assert_eq!(
            chain.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2, 5]
        );
    }

    #[test]
    fn try_append_adds_a_message_replying_to_the_origin() {
        let origin = message(1, 100, None);
        let mut thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin],
        };
        let reply = message(2, 200, Some(1));

        assert!(thread.try_append(&reply));

        assert_eq!(
            thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn try_append_adds_a_message_replying_to_an_existing_descendant() {
        let origin = message(1, 100, None);
        let reply = message(2, 200, Some(1));
        let mut thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin, reply],
        };
        let reply_to_reply = message(3, 300, Some(2));

        assert!(thread.try_append(&reply_to_reply));

        assert_eq!(
            thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn try_append_rejects_a_message_with_no_reply_to() {
        let origin = message(1, 100, None);
        let mut thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin],
        };
        let unrelated = message(2, 200, None);

        assert!(!thread.try_append(&unrelated));
        assert_eq!(thread.messages.len(), 1);
    }

    #[test]
    fn try_append_rejects_a_message_replying_outside_the_view() {
        let origin = message(1, 100, None);
        let mut thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin],
        };
        let unrelated = message(2, 200, Some(99)); // 99 isn't in the view

        assert!(!thread.try_append(&unrelated));
        assert_eq!(thread.messages.len(), 1);
    }

    #[test]
    fn try_append_ignores_a_message_already_present() {
        let origin = message(1, 100, None);
        let reply = message(2, 200, Some(1));
        let mut thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin, reply.clone()],
        };

        assert!(!thread.try_append(&reply));
        assert_eq!(thread.messages.len(), 2);
    }

    #[test]
    fn try_append_sorts_an_out_of_order_arrival() {
        let origin = message(5, 100, None);
        let mut thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin],
        };
        // Older timestamp than `origin`, but still a valid reply to it --
        // e.g. a peer's message arriving late over gossip.
        let earlier_reply = message(1, 50, Some(5));

        assert!(thread.try_append(&earlier_reply));

        assert_eq!(
            thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 5],
            "must stay sorted by (ts_unix_ms, id) regardless of arrival order"
        );
    }
}
