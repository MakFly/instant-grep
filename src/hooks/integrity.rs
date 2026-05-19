//! Hook integrity — SHA-256 of every hook script that `ig setup` installs.
//!
//! Each canonical hook source is `include_bytes!`-embedded at build time so
//! we can recompute the expected digest without touching the filesystem.
//! `verify_installed()` compares it against the digest of the file that is
//! actually present in the user's hooks directory.
//
// Same dead-code-when-included-via-`#[path]` story as `permissions.rs`.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Pair of expected (compile-time, canonical source) vs actual (on-disk)
/// SHA-256 for a single installed hook script.
#[derive(Debug, Clone)]
pub struct HookSignature {
    pub installed_path: PathBuf,
    pub expected_sha256: String,
    /// `None` when the file does not exist (drift = missing).
    pub actual_sha256: Option<String>,
}

impl HookSignature {
    pub fn is_drift(&self) -> bool {
        match &self.actual_sha256 {
            Some(actual) => actual != &self.expected_sha256,
            None => true,
        }
    }

    pub fn is_missing(&self) -> bool {
        self.actual_sha256.is_none()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// The canonical (expected) hook sources. Each tuple is
/// `(installed_path_relative_to_home, canonical_bytes)`.
fn canonical_hooks() -> Vec<(PathBuf, &'static [u8])> {
    let mut out: Vec<(PathBuf, &'static [u8])> = Vec::new();
    let Some(home) = home() else { return out };

    out.push((
        home.join(".claude/hooks/ig-guard.sh"),
        include_bytes!("../../hooks/ig-guard.sh").as_slice(),
    ));
    out.push((
        home.join(".claude/hooks/format.sh"),
        include_bytes!("../../hooks/format.sh").as_slice(),
    ));
    out.push((
        home.join(".claude/hooks/subagent-context.sh"),
        include_bytes!("../../hooks/subagent-context.sh").as_slice(),
    ));
    out
}

/// Snapshot of expected SHA-256 for every hook ig knows how to install.
/// `actual_sha256` is `None` in the snapshot — call `verify_installed()` to
/// fill it in by reading the on-disk file.
pub fn current_expected_signatures() -> Vec<HookSignature> {
    canonical_hooks()
        .into_iter()
        .map(|(path, bytes)| HookSignature {
            installed_path: path,
            expected_sha256: sha256_hex(bytes),
            actual_sha256: None,
        })
        .collect()
}

/// Read every installed hook file and compute its SHA-256, returning a paired
/// list of expected vs actual digests. Missing files have
/// `actual_sha256 = None`.
pub fn verify_installed() -> Vec<HookSignature> {
    let mut out = current_expected_signatures();
    for sig in out.iter_mut() {
        sig.actual_sha256 = std::fs::read(&sig.installed_path)
            .ok()
            .map(|b| sha256_hex(&b));
    }
    out
}

/// Compute the SHA-256 of an arbitrary path. Public for tests.
#[allow(dead_code)]
pub fn sha256_of_path(path: &std::path::Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| sha256_hex(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_signatures_nonempty() {
        let sigs = current_expected_signatures();
        // At least one (ig-guard.sh) on platforms where HOME is set.
        if dirs::home_dir().is_some() {
            assert!(!sigs.is_empty());
            for s in &sigs {
                assert_eq!(s.expected_sha256.len(), 64);
                assert!(s.actual_sha256.is_none());
            }
        }
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = sha256_hex(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
