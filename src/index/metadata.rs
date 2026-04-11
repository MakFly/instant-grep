use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const INDEX_VERSION: u32 = 10;

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
        let encoded = bincode::serialize(self).context("serialize metadata")?;
        std::fs::write(&bin_path, &encoded).context("write metadata.bin")?;

        if std::env::var("IG_DEBUG").is_ok() {
            let json_path = ig_dir.join("metadata.json");
            let file = File::create(&json_path).context("create metadata.json")?;
            serde_json::to_writer_pretty(BufWriter::new(file), self)
                .context("write metadata.json")?;
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
