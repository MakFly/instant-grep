//! Trust system for project-local TOML filter files.
//! Verifies SHA-256 hashes to prevent untrusted filters from running.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize, Default)]
struct TrustStore {
    trusted: HashMap<String, TrustEntry>,
}

#[derive(Serialize, Deserialize)]
struct TrustEntry {
    sha256: String,
    trusted_at: u64,
}

pub enum TrustStatus {
    Trusted,
    Untrusted,
    ContentChanged,
}

fn trust_store_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("ig")
        .join("trusted.json")
}

fn compute_sha256(path: &Path) -> Result<String> {
    let content = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let hash = Sha256::digest(&content);
    Ok(format!("{:x}", hash))
}

fn load_store() -> TrustStore {
    let path = trust_store_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        TrustStore::default()
    }
}

fn save_store(store: &TrustStore) -> Result<()> {
    let path = trust_store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(store)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Check if a filter file is trusted.
pub fn check_trust(path: &Path) -> TrustStatus {
    let canonical = match path.canonicalize() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => return TrustStatus::Untrusted,
    };

    let store = load_store();
    let entry = match store.trusted.get(&canonical) {
        Some(e) => e,
        None => return TrustStatus::Untrusted,
    };

    match compute_sha256(path) {
        Ok(hash) if hash == entry.sha256 => TrustStatus::Trusted,
        Ok(_) => TrustStatus::ContentChanged,
        Err(_) => TrustStatus::Untrusted,
    }
}

/// Trust a filter file (store its SHA-256).
pub fn trust_path(path: &Path) -> Result<()> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?
        .to_string_lossy()
        .to_string();
    let hash = compute_sha256(path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut store = load_store();
    store.trusted.insert(
        canonical,
        TrustEntry {
            sha256: hash,
            trusted_at: now,
        },
    );
    save_store(&store)
}

/// Revoke trust for a filter file.
pub fn untrust_path(path: &Path) -> Result<()> {
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string();

    let mut store = load_store();
    store.trusted.remove(&canonical);
    save_store(&store)
}

/// List all trusted filter files.
pub fn list_trusted() -> Vec<(String, String)> {
    let store = load_store();
    store
        .trusted
        .into_iter()
        .map(|(path, entry)| (path, entry.sha256))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_compute_sha256() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "hello world").unwrap();
        let hash = compute_sha256(f.path()).unwrap();
        assert_eq!(hash.len(), 64); // SHA-256 = 64 hex chars
    }

    #[test]
    fn test_check_untrusted() {
        let f = NamedTempFile::new().unwrap();
        assert!(matches!(check_trust(f.path()), TrustStatus::Untrusted));
    }
}
