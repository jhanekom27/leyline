//! Local message log persistence.
//!
//! Persists received `ChatMessage`s so scrollback survives restarts, since
//! gossip is live-broadcast only and offers no backfill -- see concept.md's
//! "Persistence & offline history" section. Landing target: build-order step
//! 5.
