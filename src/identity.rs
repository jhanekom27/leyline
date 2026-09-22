//! Local identity: a persisted `iroh::SecretKey` that *is* the user's
//! identity -- no accounts, no server. Will load the key from disk (via the
//! `directories` crate) on startup, generating and persisting a new one on
//! first run -- see concept.md's "Identity & channels" section. Landing
//! target: build-order step 3.
