//! Build identifiers: our own semver (from `Cargo.toml`, via Cargo's
//! built-in `CARGO_PKG_VERSION`) and the git commit we were built from
//! (`LEYLINE_GIT_HASH`, stamped in by `build.rs`). Broadcast to peers via
//! `message::VersionAnnounce` (see `net::Net::announce_version`) so anyone
//! running an older build can be told about it -- see features.md's
//! version-notification idea.

/// Our own version, from `Cargo.toml`'s `version` field -- bumped by hand
/// per release. This is what `is_newer` compares against, and what's
/// broadcast in every `message::VersionAnnounce`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The short hash of the git commit this binary was built from, stamped in
/// by `build.rs` at compile time -- `"unknown"` if git wasn't available at
/// build time. Purely informational (shown in `/version` and `/who`, never
/// compared), since commit hashes have no inherent ordering the way a
/// semver triple does.
pub const GIT_HASH: &str = env!("LEYLINE_GIT_HASH");

/// Whether `candidate` (a peer's announced `VERSION`) is newer than ours,
/// compared as `(major, minor, patch)` tuples -- plain manual parsing
/// rather than pulling in the `semver` crate, since every version this
/// project has ever used is a plain `X.Y.Z`. An unparseable `candidate`
/// (e.g. a pre-release suffix, or a malformed/hostile peer) is treated as
/// "not newer" rather than an error -- there's nothing actionable to do
/// with it either way, so it's simplest to just ignore it.
pub fn is_newer(candidate: &str) -> bool {
    match (parse(candidate), parse(VERSION)) {
        (Some(candidate), Some(ours)) => candidate > ours,
        _ => false,
    }
}

/// Parses a plain `major.minor.patch` version string into a comparable
/// tuple. `None` for anything else (wrong number of parts, non-numeric
/// parts, pre-release/build-metadata suffixes, ...). `pub(crate)` so
/// `app::AppState::newest_known_update` can rank multiple peers' versions
/// without duplicating this parsing logic.
pub(crate) fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_is_false_for_an_equal_version() {
        assert!(!is_newer(VERSION));
    }

    #[test]
    fn is_newer_is_true_for_a_greater_major() {
        let (major, minor, patch) = parse(VERSION).unwrap();
        assert!(is_newer(&format!("{}.{minor}.{patch}", major + 1)));
    }

    #[test]
    fn is_newer_is_true_for_a_greater_minor_even_with_a_lower_patch() {
        let (major, minor, _) = parse(VERSION).unwrap();
        assert!(is_newer(&format!("{major}.{}.0", minor + 1)));
    }

    #[test]
    fn is_newer_is_false_for_a_lesser_version() {
        // 0.0.0 sorts below any real release this project would ever tag.
        assert!(!is_newer("0.0.0"));
    }

    #[test]
    fn is_newer_is_false_for_malformed_input() {
        assert!(!is_newer(""));
        assert!(!is_newer("not-a-version"));
        assert!(!is_newer("1.2"));
        assert!(!is_newer("1.2.3.4"));
        assert!(!is_newer("1.2.3-beta"));
    }

    #[test]
    fn parse_rejects_wrong_component_counts() {
        assert_eq!(parse("1.2"), None);
        assert_eq!(parse("1.2.3.4"), None);
    }

    #[test]
    fn parse_rejects_non_numeric_components() {
        assert_eq!(parse("1.2.x"), None);
    }

    #[test]
    fn parse_accepts_a_plain_triple() {
        assert_eq!(parse("1.2.3"), Some((1, 2, 3)));
    }
}
