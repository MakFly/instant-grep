//! `.ig/seal` — atomic publish marker.
//!
//! The writer bumps this 16-byte file as the FINAL act of every rebuild
//! (full or incremental). The daemon uses its `generation` field as the
//! single authoritative cache-invalidation key.
//!
//! Invariance the writer must preserve:
//! 1. Publish all artifacts atomically (postings.bin, lexicon.bin,
//!    metadata.bin, overlay_*.bin) BEFORE bumping the seal.
//! 2. Bump the seal via tmp + rename, last act.
//!
//! Daemon contract: when it observes generation N, the on-disk artifacts
//! of generation N are guaranteed already visible (because seal is renamed
//! last). No torn-state observation possible.
//!
//! Old indexes (built before v1.18.0) have no seal file — `read_seal`
//! returns `None` and callers treat that as generation `0`. The first
//! rebuild after upgrade creates the seal and the daemon switches over
//! transparently.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const SEAL_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seal {
    pub generation: u64,
    pub finalized_at_nanos: u64,
}

pub fn seal_path(ig_dir: &Path) -> PathBuf {
    ig_dir.join("seal")
}

pub fn tmp_path(ig_dir: &Path) -> PathBuf {
    ig_dir.join("seal.tmp")
}

/// Read the current seal. Returns `None` if missing or malformed (legacy
/// indexes, partial writes outside the tmp+rename guarantee — neither is
/// reachable from a writer that uses `bump_seal`, but be defensive).
pub fn read_seal(ig_dir: &Path) -> Option<Seal> {
    let bytes = std::fs::read(seal_path(ig_dir)).ok()?;
    if bytes.len() != SEAL_BYTES {
        return None;
    }
    let generation = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let finalized_at_nanos = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    Some(Seal {
        generation,
        finalized_at_nanos,
    })
}

/// Atomically bump the seal generation by 1. Returns the new generation.
///
/// MUST be called as the final act of any successful rebuild path. Calling
/// this without first publishing artifacts atomically breaks the daemon's
/// "no torn state" contract.
pub fn bump_seal(ig_dir: &Path) -> Result<u64> {
    let prev_gen = read_seal(ig_dir).map(|s| s.generation).unwrap_or(0);
    let next = Seal {
        generation: prev_gen + 1,
        finalized_at_nanos: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    };
    let mut bytes = [0u8; SEAL_BYTES];
    bytes[0..8].copy_from_slice(&next.generation.to_le_bytes());
    bytes[8..16].copy_from_slice(&next.finalized_at_nanos.to_le_bytes());
    let tmp = tmp_path(ig_dir);
    std::fs::write(&tmp, bytes).context("write seal.tmp")?;
    std::fs::rename(&tmp, seal_path(ig_dir)).context("publish seal")?;
    Ok(next.generation)
}

/// Read the generation, mapping missing/corrupt seal to 0. Public helper
/// used by tests and by external tooling that may want to inspect seal
/// state without parsing the 16-byte file directly.
#[inline]
#[allow(dead_code)]
pub fn current_generation(ig_dir: &Path) -> u64 {
    read_seal(ig_dir).map(|s| s.generation).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_seal_reads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_seal(tmp.path()).is_none());
        assert_eq!(current_generation(tmp.path()), 0);
    }

    #[test]
    fn bump_creates_and_increments() {
        let tmp = tempfile::tempdir().unwrap();
        let g1 = bump_seal(tmp.path()).unwrap();
        assert_eq!(g1, 1);
        let g2 = bump_seal(tmp.path()).unwrap();
        assert_eq!(g2, 2);
        let s = read_seal(tmp.path()).unwrap();
        assert_eq!(s.generation, 2);
        assert!(s.finalized_at_nanos > 0);
    }

    #[test]
    fn bump_is_atomic_no_tmp_left() {
        let tmp = tempfile::tempdir().unwrap();
        bump_seal(tmp.path()).unwrap();
        assert!(seal_path(tmp.path()).exists());
        assert!(!tmp_path(tmp.path()).exists());
    }

    #[test]
    fn corrupt_seal_reads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(seal_path(tmp.path()), b"too short").unwrap();
        assert!(read_seal(tmp.path()).is_none());
        // bump must still succeed — it overwrites atomically
        let g = bump_seal(tmp.path()).unwrap();
        assert_eq!(g, 1);
    }
}
