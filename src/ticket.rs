//! Invite ticket encode/decode.
//!
//! A ticket bundles a `NodeId` (+ known relay/addr hints) and a channel's
//! `TopicId` into a single string a friend can paste in to join -- see
//! concept.md's "Identity & channels" section. Landing target: build-order
//! step 3.
