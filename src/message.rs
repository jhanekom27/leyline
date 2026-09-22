//! Wire format for chat messages exchanged over gossip.

use serde::{Deserialize, Serialize};

/// A single chat message, as it travels over the wire and is stored locally.
///
/// Kept small and versioned from day one -- see concept.md's "Message wire
/// format". Dedupe/ordering helpers land here once gossip is wired up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Wire format version.
    pub v: u8,
    /// Random message id, used for local dedupe.
    pub id: u64,
    /// Sender's NodeId. Embedded so messages can be verified/displayed
    /// without an extra lookup, even though gossip also gives you this at
    /// delivery.
    pub sender: [u8; 32],
    /// Unix timestamp in milliseconds.
    pub ts_unix_ms: u64,
    /// Message body.
    pub text: String,
}
