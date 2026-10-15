//! Pure per-channel sync configuration shared by app state and persistence.

use std::path::{Component, Path};

use globset::GlobBuilder;
use serde::{Deserialize, Serialize};

/// The active direction for one channel's folder sync.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncPolicy {
    #[default]
    Off,
    ReceiveOnly,
    PublishAndReceive,
}

impl SyncPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReceiveOnly => "receive-only",
            Self::PublishAndReceive => "publish-and-receive",
        }
    }
}

/// One include or exclude glob. Paths are always normalized relative paths
/// using `/`, so rule parsing can validate them once at the user boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncRule {
    Include(String),
    Exclude(String),
}

impl SyncRule {
    pub fn include(pattern: &str) -> Result<Self, String> {
        validate_pattern(pattern)?;
        Ok(Self::Include(pattern.to_string()))
    }

    pub fn exclude(pattern: &str) -> Result<Self, String> {
        validate_pattern(pattern)?;
        Ok(Self::Exclude(pattern.to_string()))
    }

    pub fn pattern(&self) -> &str {
        match self {
            Self::Include(pattern) | Self::Exclude(pattern) => pattern,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Include(_) => "include",
            Self::Exclude(_) => "exclude",
        }
    }

    fn matches(&self, path: &str) -> bool {
        GlobBuilder::new(self.pattern())
            .literal_separator(true)
            .backslash_escape(false)
            .build()
            .expect("sync rules are validated before storage")
            .compile_matcher()
            .is_match(path)
    }
}

/// Persisted configuration for one channel. Pausing is independent from the
/// direction so a resume restores the policy the user selected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfig {
    pub policy: SyncPolicy,
    pub paused: bool,
    pub rules: Vec<SyncRule>,
}

impl SyncConfig {
    pub fn receives(&self) -> bool {
        !self.paused && self.policy != SyncPolicy::Off
    }

    pub fn publishes(&self) -> bool {
        !self.paused && self.policy == SyncPolicy::PublishAndReceive
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn label(&self) -> String {
        if self.paused {
            format!("paused ({})", self.policy.label())
        } else {
            self.policy.label().to_string()
        }
    }

    pub fn allows_path(&self, path: &str) -> bool {
        let has_includes = self
            .rules
            .iter()
            .any(|rule| matches!(rule, SyncRule::Include(_)));
        let included = !has_includes
            || self
                .rules
                .iter()
                .any(|rule| matches!(rule, SyncRule::Include(_)) && rule.matches(path));
        included
            && !self
                .rules
                .iter()
                .any(|rule| matches!(rule, SyncRule::Exclude(_)) && rule.matches(path))
    }
    pub fn rules_are_valid(&self) -> bool {
        self.rules
            .iter()
            .all(|rule| validate_pattern(rule.pattern()).is_ok())
    }

    pub fn add_rule(&mut self, rule: SyncRule) -> bool {
        if self.rules.contains(&rule) {
            false
        } else {
            self.rules.push(rule);
            true
        }
    }

    pub fn remove_rule(&mut self, rule: &SyncRule) -> bool {
        let before = self.rules.len();
        self.rules.retain(|known| known != rule);
        self.rules.len() != before
    }

    pub fn clear_rules(&mut self) -> bool {
        if self.rules.is_empty() {
            false
        } else {
            self.rules.clear();
            true
        }
    }
}

fn validate_pattern(pattern: &str) -> Result<(), String> {
    if pattern.is_empty()
        || pattern.contains('\\')
        || Path::new(pattern).is_absolute()
        || Path::new(pattern)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(
            "sync rules must be non-empty relative glob patterns using `/` separators".to_string(),
        );
    }
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .map(|_| ())
        .map_err(|err| format!("invalid sync glob {pattern:?}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_limits_paths_and_exclude_always_wins() {
        let mut config = SyncConfig::default();
        config.add_rule(SyncRule::include("notes/**/*.md").unwrap());
        config.add_rule(SyncRule::exclude("notes/private/**").unwrap());

        assert!(config.allows_path("notes/project/plan.md"));
        assert!(!config.allows_path("notes/private/plan.md"));
        assert!(!config.allows_path("images/logo.png"));
    }

    #[test]
    fn rules_reject_unsafe_or_invalid_patterns() {
        for pattern in ["", "/absolute", "../outside", "folder\\*.txt", "["] {
            assert!(SyncRule::include(pattern).is_err(), "{pattern:?}");
        }
    }

    #[test]
    fn pausing_preserves_the_selected_policy() {
        let config = SyncConfig {
            policy: SyncPolicy::ReceiveOnly,
            paused: true,
            rules: Vec::new(),
        };

        assert!(!config.receives());
        assert!(!config.publishes());
        assert_eq!(config.label(), "paused (receive-only)");
    }
}
