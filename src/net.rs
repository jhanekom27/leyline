//! iroh + iroh-gossip networking actor(s).
//!
//! Not wired up yet. This module will own an `iroh::Endpoint` and one
//! gossip-receive task per joined topic, forwarding parsed `NetEvent`s into
//! the app's event loop over an `mpsc` channel -- see concept.md's
//! "Architecture at a glance" section. Landing target: build-order step 2.
