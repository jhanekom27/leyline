//! Stamps the current git commit's short hash into `LEYLINE_GIT_HASH` at
//! compile time (see `crate::version`), so a build can report exactly what
//! commit it was built from without needing that hash to already exist
//! inside the commit itself -- the hash is captured when you `cargo build`,
//! which always happens after the commit does. Falls back to `"unknown"` if
//! git isn't available (e.g. building from a source tarball with no
//! `.git`), rather than failing the build over a cosmetic detail.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|hash| !hash.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=LEYLINE_GIT_HASH={hash}");
    // Re-run when HEAD moves (a new commit or checkout) so the stamped hash
    // doesn't go stale -- best-effort, doesn't cover every git ref-update
    // edge case (e.g. packed-refs), but covers the common one.
    println!("cargo:rerun-if-changed=.git/HEAD");
}
