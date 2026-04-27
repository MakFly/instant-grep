//! Human-readable JSON store at `.ig/poc-embeddings.json`.
//! Pedagogical: `cat .ig/poc-embeddings.json | jq '.chunks[0]'` shows everything.
//! Phase 4+ would switch to bincode + HNSW for size & speed.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const STORE_PATH: &str = ".ig/poc-embeddings.json";
const STORE_VERSION: &str = "poc-1";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredChunk {
    pub id: u32,
    pub file: String, // relative path
    pub start_line: usize,
    pub end_line: usize,
    pub tokens: u32,
    pub embedding: Vec<f32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Store {
    pub version: String,
    pub model: String,
    pub provider: String,
    pub dim: usize,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub chunks: Vec<StoredChunk>,
}

impl Store {
    pub fn new(provider: &str, model: &str, dim: usize) -> Self {
        Self {
            version: STORE_VERSION.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            dim,
            total_tokens: 0,
            total_cost_usd: 0.0,
            chunks: Vec::new(),
        }
    }

    pub fn save(&self, root: &Path) -> Result<PathBuf> {
        let path = root.join(STORE_PATH);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).with_context(|| "serialize store")?;
        fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join(STORE_PATH);
        if !path.exists() {
            return Ok(None);
        }
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let store: Self = serde_json::from_str(&content).with_context(|| "parse store JSON")?;
        Ok(Some(store))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::new("openai", "text-embedding-3-small", 4);
        store.chunks.push(StoredChunk {
            id: 0,
            file: "src/foo.rs".to_string(),
            start_line: 1,
            end_line: 40,
            tokens: 100,
            embedding: vec![0.1, 0.2, 0.3, 0.4],
        });
        store.total_tokens = 100;
        store.total_cost_usd = 0.000002;

        let p = store.save(dir.path()).unwrap();
        assert!(p.exists());

        let loaded = Store::load(dir.path()).unwrap().expect("must exist");
        assert_eq!(loaded.dim, 4);
        assert_eq!(loaded.chunks.len(), 1);
        assert_eq!(loaded.chunks[0].embedding, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let r = Store::load(dir.path()).unwrap();
        assert!(r.is_none());
    }
}
