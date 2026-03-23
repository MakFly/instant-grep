use serde::{Deserialize, Serialize};

pub const INDEX_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub version: u32,
    pub created_at: u64,
    pub root: String,
    pub file_count: u32,
    pub trigram_count: u32,
    pub files: Vec<IndexedFile>,
    /// Git commit SHA the index was built from (None if not a git repo).
    #[serde(default)]
    pub git_commit: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexedFile {
    pub path: String,
    pub mtime: u64,
    pub size: u64,
}
