use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use memmap2::Mmap;

use crate::index::metadata::IndexMetadata;
use crate::index::postings::{self, DocId};
use crate::index::trigram::Trigram;
use crate::query::plan::TrigramQuery;

const LEXICON_ENTRY_SIZE: usize = 12;

pub struct IndexReader {
    pub metadata: IndexMetadata,
    lexicon: Mmap,
    postings: Mmap,
    table_size: usize,
}

impl IndexReader {
    /// Open an existing index from the .ig directory.
    pub fn open(ig_dir: &Path) -> Result<Self> {
        let metadata_path = ig_dir.join("metadata.json");
        let metadata: IndexMetadata = serde_json::from_reader(
            File::open(&metadata_path).context("open metadata.json")?,
        )
        .context("parse metadata.json")?;

        let lexicon_file =
            File::open(ig_dir.join("lexicon.bin")).context("open lexicon.bin")?;
        let lexicon =
            unsafe { Mmap::map(&lexicon_file).context("mmap lexicon.bin")? };

        let postings_file =
            File::open(ig_dir.join("postings.bin")).context("open postings.bin")?;
        let postings =
            unsafe { Mmap::map(&postings_file).context("mmap postings.bin")? };

        let table_size = lexicon.len() / LEXICON_ENTRY_SIZE;

        Ok(Self {
            metadata,
            lexicon,
            postings,
            table_size,
        })
    }

    /// Look up a single trigram in the hash table, return matching doc IDs.
    pub fn lookup_trigram(&self, tri: Trigram) -> Vec<DocId> {
        if self.table_size == 0 {
            return Vec::new();
        }

        let stored_tri = tri + 1; // +1 encoding (0 = empty)
        let mut slot = (stored_tri as usize) % self.table_size;

        loop {
            let base = slot * LEXICON_ENTRY_SIZE;
            if base + LEXICON_ENTRY_SIZE > self.lexicon.len() {
                return Vec::new();
            }

            let entry_tri = u32::from_le_bytes([
                self.lexicon[base],
                self.lexicon[base + 1],
                self.lexicon[base + 2],
                self.lexicon[base + 3],
            ]);

            if entry_tri == 0 {
                return Vec::new();
            }

            if entry_tri == stored_tri {
                let offset = u32::from_le_bytes([
                    self.lexicon[base + 4],
                    self.lexicon[base + 5],
                    self.lexicon[base + 6],
                    self.lexicon[base + 7],
                ]) as usize;
                let length = u32::from_le_bytes([
                    self.lexicon[base + 8],
                    self.lexicon[base + 9],
                    self.lexicon[base + 10],
                    self.lexicon[base + 11],
                ]) as usize;

                // Read posting list directly from mmap'd postings
                let byte_len = length * 4;
                let end = offset + byte_len;
                if end > self.postings.len() {
                    return Vec::new();
                }

                let slice = &self.postings[offset..end];
                let doc_ids: Vec<DocId> = slice
                    .chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();

                return doc_ids;
            }

            slot = (slot + 1) % self.table_size;
        }
    }

    /// Resolve a full TrigramQuery into candidate doc IDs.
    pub fn resolve(&self, query: &TrigramQuery) -> Vec<DocId> {
        match query {
            TrigramQuery::Trigram(tri) => self.lookup_trigram(*tri),
            TrigramQuery::And(children) => {
                if children.is_empty() {
                    return Vec::new();
                }
                let mut lists: Vec<Vec<DocId>> = children
                    .iter()
                    .map(|child| self.resolve(child))
                    .collect();
                lists.sort_unstable_by_key(|l| l.len());
                let mut result = lists.remove(0);
                for list in &lists {
                    result = postings::intersect(&result, list);
                    if result.is_empty() {
                        break;
                    }
                }
                result
            }
            TrigramQuery::Or(children) => {
                if children.is_empty() {
                    return Vec::new();
                }
                let mut result = Vec::new();
                for child in children {
                    let list = self.resolve(child);
                    result = postings::union(&result, &list);
                }
                result
            }
            TrigramQuery::All => {
                (0..self.metadata.file_count).collect()
            }
        }
    }

    /// Get the relative path for a doc ID.
    pub fn file_path(&self, doc_id: DocId) -> &str {
        &self.metadata.files[doc_id as usize].path
    }
}
