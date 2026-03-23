use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use memmap2::Mmap;

use crate::index::metadata::IndexMetadata;
use crate::index::ngram::NgramKey;
use crate::index::postings::{self, DocId};
use crate::query::plan::NgramQuery;

const LEXICON_ENTRY_SIZE: usize = 16; // u64 key + u32 offset + u32 length

pub struct IndexReader {
    pub metadata: IndexMetadata,
    lexicon: Mmap,
    postings: Mmap,
    table_size: usize,
}

impl IndexReader {
    pub fn open(ig_dir: &Path) -> Result<Self> {
        let metadata = IndexMetadata::load_from(ig_dir).context("load metadata")?;

        let lexicon_file = File::open(ig_dir.join("lexicon.bin")).context("open lexicon.bin")?;
        let lexicon = unsafe { Mmap::map(&lexicon_file).context("mmap lexicon.bin")? };

        let postings_file = File::open(ig_dir.join("postings.bin")).context("open postings.bin")?;
        let postings = unsafe { Mmap::map(&postings_file).context("mmap postings.bin")? };

        let table_size = lexicon.len() / LEXICON_ENTRY_SIZE;

        Ok(Self {
            metadata,
            lexicon,
            postings,
            table_size,
        })
    }

    /// Look up a single n-gram key in the hash table.
    pub fn lookup_ngram(&self, key: NgramKey) -> Vec<DocId> {
        if self.table_size == 0 {
            return Vec::new();
        }

        let stored_key = key + 1; // sentinel: 0 = empty
        let mut slot = (stored_key as usize) % self.table_size;

        loop {
            let base = slot * LEXICON_ENTRY_SIZE;
            if base + LEXICON_ENTRY_SIZE > self.lexicon.len() {
                return Vec::new();
            }

            let entry_key = u64::from_le_bytes([
                self.lexicon[base],
                self.lexicon[base + 1],
                self.lexicon[base + 2],
                self.lexicon[base + 3],
                self.lexicon[base + 4],
                self.lexicon[base + 5],
                self.lexicon[base + 6],
                self.lexicon[base + 7],
            ]);

            if entry_key == 0 {
                return Vec::new();
            }

            if entry_key == stored_key {
                let offset = u32::from_le_bytes([
                    self.lexicon[base + 8],
                    self.lexicon[base + 9],
                    self.lexicon[base + 10],
                    self.lexicon[base + 11],
                ]) as usize;
                let length = u32::from_le_bytes([
                    self.lexicon[base + 12],
                    self.lexicon[base + 13],
                    self.lexicon[base + 14],
                    self.lexicon[base + 15],
                ]) as usize;

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

    /// Resolve a full NgramQuery into candidate doc IDs.
    pub fn resolve(&self, query: &NgramQuery) -> Vec<DocId> {
        match query {
            NgramQuery::Ngram(key) => self.lookup_ngram(*key),
            NgramQuery::And(children) => {
                if children.is_empty() {
                    return Vec::new();
                }
                let mut lists: Vec<Vec<DocId>> =
                    children.iter().map(|child| self.resolve(child)).collect();
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
            NgramQuery::Or(children) => {
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
            NgramQuery::All => (0..self.metadata.file_count).collect(),
        }
    }

    pub fn file_path(&self, doc_id: DocId) -> &str {
        &self.metadata.files[doc_id as usize].path
    }
}
