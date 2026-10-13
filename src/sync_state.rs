//! Pure sync-control types shared by app state and persistence.

use serde::{Deserialize, Serialize};

/// The persisted control for one channel's folder sync.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    #[default]
    Off,
    Active,
    Paused,
}

impl SyncMode {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Active => "active",
            Self::Paused => "paused",
        }
    }
}
