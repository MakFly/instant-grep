use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const INDEX_VERSION: u32 = 13;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub version: u32,
    pub created_at: u64,
    pub root: String,
    pub file_count: u32,
    pub ngram_count: u32,
    pub files: Vec<IndexedFile>,
    #[serde(default)]
    pub git_commit: Option<String>,
    #[serde(default)]
    pub bigram_df_path: Option<String>,
    /// Whether this index was built using IDF-weighted ngram boundaries.
    /// Query must use the same weighting to produce matching ngrams.
    #[serde(default)]
    pub built_with_idf: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexedFile {
    pub path: String,
    pub mtime: u64,
    pub size: u64,
}

impl IndexMetadata {
    pub fn write_to(&self, ig_dir: &Path) -> Result<()> {
        let bin_path = ig_dir.join("metadata.bin");
        let bin_tmp = ig_dir.join("metadata.bin.tmp");
        let encoded = bincode::serialize(self).context("serialize metadata")?;
        // Atomic publish: write tmp + rename. Prevents the daemon from observing
        // a torn metadata.bin during a rebuild — and from holding a stale mmap-
        // backed view via in-place truncate (see fix(v1.17.2) H1 hardening).
        std::fs::write(&bin_tmp, &encoded).context("write metadata.bin.tmp")?;
        std::fs::rename(&bin_tmp, &bin_path).context("publish metadata.bin")?;

        if std::env::var("IG_DEBUG").is_ok() {
            let json_path = ig_dir.join("metadata.json");
            let json_tmp = ig_dir.join("metadata.json.tmp");
            let file = File::create(&json_tmp).context("create metadata.json.tmp")?;
            serde_json::to_writer_pretty(BufWriter::new(file), self)
                .context("write metadata.json.tmp")?;
            std::fs::rename(&json_tmp, &json_path).context("publish metadata.json")?;
        }

        Ok(())
    }

    pub fn load_from(ig_dir: &Path) -> Result<Self> {
        let bin_path = ig_dir.join("metadata.bin");
        if bin_path.exists() {
            let data = std::fs::read(&bin_path).context("read metadata.bin")?;
            let meta: Self = bincode::deserialize(&data).context("deserialize metadata.bin")?;
            if meta.version == INDEX_VERSION {
                return Ok(meta);
            }
        }

        let json_path = ig_dir.join("metadata.json");
        let file = File::open(&json_path).context("open metadata.json")?;
        let meta: Self = serde_json::from_reader(file).context("parse metadata.json")?;
        Ok(meta)
    }

    pub fn exists(ig_dir: &Path) -> bool {
        ig_dir.join("metadata.bin").exists() || ig_dir.join("metadata.json").exists()
    }
}
