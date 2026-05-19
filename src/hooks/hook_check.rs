//! Rate-limited drift warning. Runs `integrity::verify_installed()` at most
//! once per 24 hours and returns a formatted multi-line string when any
//! installed hook diverges from the canonical source baked into the binary.
//!
//! Wired into `main.rs` after `cache::ensure_layout()` and before command
//! dispatch (skipped for `hook-audit`, `setup`, `uninstall`, `update`,
//! `version`).
//
// Same dead-code-when-included-via-`#[path]` story as `permissions.rs`.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::integrity::{HookSignature, verify_installed};

const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

fn marker_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|c| c.join("ig").join("hook_check_last.timestamp"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_marker_secs(path: &std::path::Path) -> Option<u64> {
    let s = std::fs::read_to_string(path).ok()?;
    s.trim().parse::<u64>().ok()
}

fn touch_marker(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, now_secs().to_string());
}

/// Run a drift check if more than 24h have passed since the last check.
/// Returns `Some(warning_text)` when drift was found, `None` otherwise.
/// The marker file is always touched on every check.
pub fn maybe_warn_drift() -> Option<String> {
    let path = marker_path()?;
    let now = now_secs();
    let due = match read_marker_secs(&path) {
        None => true, // never ran
        Some(last) => now.saturating_sub(last) >= CHECK_INTERVAL.as_secs(),
    };
    if !due {
        return None;
    }
    touch_marker(&path);

    let sigs = verify_installed();
    format_drift_warning(&sigs)
}

/// Same as `maybe_warn_drift` but ignores the marker file. Used by
/// `ig hook-audit` so the manual command always runs a fresh check.
#[allow(dead_code)]
pub fn force_check() -> Option<String> {
    if let Some(p) = marker_path() {
        touch_marker(&p);
    }
    let sigs = verify_installed();
    format_drift_warning(&sigs)
}

/// Build a human-readable warning string from the verify result.
/// Returns `None` when every installed file matches its expected digest.
pub fn format_drift_warning(sigs: &[HookSignature]) -> Option<String> {
    let drifted: Vec<&HookSignature> = sigs
        .iter()
        .filter(|s| s.actual_sha256.is_some() && s.is_drift())
        .collect();
    let missing: Vec<&HookSignature> = sigs.iter().filter(|s| s.is_missing()).collect();

    if drifted.is_empty() && missing.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str(
        "[ig] hook drift detected — installed hooks differ from this binary's canonical sources.\n",
    );
    out.push_str("     Run `ig setup` to re-install, or `ig hook-audit` for details.\n");
    for s in &drifted {
        out.push_str(&format!(
            "     DRIFT  {}\n            expected {}…  actual {}…\n",
            s.installed_path.display(),
            &s.expected_sha256[..16],
            s.actual_sha256.as_deref().map(|h| &h[..16]).unwrap_or("??"),
        ));
    }
    for s in &missing {
        out.push_str(&format!("     MISSING {}\n", s.installed_path.display()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::HookSignature;
    use super::*;
    use std::path::PathBuf;

    fn sig(path: &str, exp: &str, act: Option<&str>) -> HookSignature {
        HookSignature {
            installed_path: PathBuf::from(path),
            expected_sha256: exp.into(),
            actual_sha256: act.map(|s| s.into()),
        }
    }

    #[test]
    fn no_drift_returns_none() {
        let sigs = vec![sig("/x", "abc", Some("abc"))];
        assert!(format_drift_warning(&sigs).is_none());
    }

    #[test]
    fn drift_is_reported() {
        let sigs = vec![sig(
            "/x",
            "abcdef0123456789abcdef0123456789",
            Some("ffffffff11111111ffffffff11111111"),
        )];
        let s = format_drift_warning(&sigs).expect("drift");
        assert!(s.contains("DRIFT"));
        assert!(s.contains("/x"));
    }

    #[test]
    fn missing_is_reported() {
        let sigs = vec![sig("/x", "abc", None)];
        let s = format_drift_warning(&sigs).expect("missing");
        assert!(s.contains("MISSING"));
    }
}
