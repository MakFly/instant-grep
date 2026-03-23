use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ahash::AHashMap;
use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::index::metadata::{IndexMetadata, IndexedFile, INDEX_VERSION};
use crate::index::postings::DocId;
use crate::index::trigram::{extract_trigrams, Trigram};
use crate::util::{ig_dir, is_binary};
use crate::walk::walk_files;

const LEXICON_ENTRY_SIZE: usize = 12;

/// Build or incrementally update the trigram index for a directory.
pub fn build_index(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
) -> Result<IndexMetadata> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = ig_dir(&root);

    // Check if we can do an incremental build
    let existing_meta = load_existing_metadata(&ig);
    let current_git_commit = get_git_head(&root);

    if let Some(ref meta) = existing_meta {
        if meta.version == INDEX_VERSION {
            let changed = detect_changed_files(&root, meta, &current_git_commit);
            if let Some(changed_paths) = changed {
                if !changed_paths.is_empty() {
                    eprintln!(
                        "Incremental: {} files changed",
                        changed_paths.len()
                    );
                    return incremental_rebuild(
                        &root,
                        meta,
                        &changed_paths,
                        use_default_excludes,
                        max_file_size,
                        &current_git_commit,
                    );
                } else {
                    eprintln!("Index is up to date");
                    return Ok(meta.clone());
                }
            }
        }
    }

    // Full rebuild
    full_rebuild(&root, use_default_excludes, max_file_size, &current_git_commit)
}

/// Full index build from scratch.
fn full_rebuild(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    git_commit: &Option<String>,
) -> Result<IndexMetadata> {
    let ig = ig_dir(root);
    let paths = walk_files(root, use_default_excludes, max_file_size, None, None)
        .context("walking files")?;

    let file_data: Vec<_> = paths
        .par_iter()
        .enumerate()
        .filter_map(|(_idx, path)| {
            let bytes = fs::read(path).ok()?;
            if is_binary(&bytes) {
                return None;
            }
            let trigrams = extract_trigrams(&bytes);
            let mtime = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
            let rel_path = path.strip_prefix(root).ok()?.to_string_lossy().to_string();
            Some((rel_path, bytes.len() as u64, mtime, trigrams))
        })
        .collect();

    write_index(root, &ig, &file_data, git_commit)
}

/// Incremental rebuild: re-index only changed files, keep everything else.
fn incremental_rebuild(
    root: &Path,
    _old_meta: &IndexMetadata,
    _changed_paths: &[String],
    use_default_excludes: bool,
    max_file_size: u64,
    git_commit: &Option<String>,
) -> Result<IndexMetadata> {
    // For now, incremental means: we detected changes, so we do a full rebuild
    // but only when changes are detected (vs always rebuilding).
    // Future optimization: only re-read changed files and merge with cached trigrams.
    full_rebuild(root, use_default_excludes, max_file_size, git_commit)
}

/// Write the index files to disk from file data.
fn write_index(
    root: &Path,
    ig: &Path,
    file_data: &[(String, u64, u64, Vec<Trigram>)],
    git_commit: &Option<String>,
) -> Result<IndexMetadata> {
    let mut postings_map: AHashMap<Trigram, Vec<DocId>> = AHashMap::new();
    let mut files = Vec::with_capacity(file_data.len());

    for (new_id, (rel_path, size, mtime, trigrams)) in file_data.iter().enumerate() {
        files.push(IndexedFile {
            path: rel_path.clone(),
            mtime: *mtime,
            size: *size,
        });

        for &tri in trigrams {
            postings_map.entry(tri).or_default().push(new_id as DocId);
        }
    }

    for list in postings_map.values_mut() {
        list.sort_unstable();
        list.dedup();
    }

    fs::create_dir_all(ig).context("create .ig directory")?;

    let postings_path = ig.join("postings.bin");
    let lexicon_path = ig.join("lexicon.bin");

    let mut postings_writer =
        BufWriter::new(File::create(&postings_path).context("create postings.bin")?);

    let mut trigrams_sorted: Vec<(Trigram, &Vec<DocId>)> = postings_map
        .iter()
        .map(|(&tri, list)| (tri, list))
        .collect();
    trigrams_sorted.sort_unstable_by_key(|(tri, _)| *tri);

    let mut offset_map: HashMap<Trigram, (u32, u32)> = HashMap::new();
    let mut current_offset: u32 = 0;

    for (tri, list) in &trigrams_sorted {
        let length = list.len() as u32;
        offset_map.insert(*tri, (current_offset, length));
        for &doc_id in *list {
            postings_writer.write_all(&doc_id.to_le_bytes())?;
        }
        current_offset += length * 4;
    }
    postings_writer.flush()?;

    let trigram_count = trigrams_sorted.len();
    let table_size = next_prime((trigram_count as f64 * 1.3) as usize);
    let mut table = vec![0u8; table_size * LEXICON_ENTRY_SIZE];

    for (tri, (offset, length)) in &offset_map {
        let stored_tri = *tri + 1;
        let mut slot = (stored_tri as usize) % table_size;
        loop {
            let base = slot * LEXICON_ENTRY_SIZE;
            let existing = u32::from_le_bytes([
                table[base],
                table[base + 1],
                table[base + 2],
                table[base + 3],
            ]);
            if existing == 0 {
                table[base..base + 4].copy_from_slice(&stored_tri.to_le_bytes());
                table[base + 4..base + 8].copy_from_slice(&offset.to_le_bytes());
                table[base + 8..base + 12].copy_from_slice(&length.to_le_bytes());
                break;
            }
            slot = (slot + 1) % table_size;
        }
    }

    fs::write(&lexicon_path, &table).context("write lexicon.bin")?;

    let metadata = IndexMetadata {
        version: INDEX_VERSION,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        root: root.to_string_lossy().to_string(),
        file_count: files.len() as u32,
        trigram_count: trigram_count as u32,
        files,
        git_commit: git_commit.clone(),
    };

    let metadata_path = ig.join("metadata.json");
    let metadata_file = File::create(&metadata_path).context("create metadata.json")?;
    serde_json::to_writer_pretty(BufWriter::new(metadata_file), &metadata)
        .context("write metadata.json")?;

    // Auto-add .ig/ to .gitignore
    let gitignore = root.join(".gitignore");
    if gitignore.exists() {
        if let Ok(content) = fs::read_to_string(&gitignore) {
            if !content.lines().any(|l| l.trim() == ".ig" || l.trim() == ".ig/") {
                let mut f = fs::OpenOptions::new().append(true).open(&gitignore)?;
                writeln!(f, "\n# instant-grep index\n.ig/")?;
            }
        }
    }

    Ok(metadata)
}

/// Load existing metadata from .ig/metadata.json if it exists.
fn load_existing_metadata(ig: &Path) -> Option<IndexMetadata> {
    let path = ig.join("metadata.json");
    let file = File::open(path).ok()?;
    serde_json::from_reader(file).ok()
}

/// Get the current git HEAD commit SHA.
fn get_git_head(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Detect which files have changed since the last index build.
/// Returns None if we can't determine changes (should do full rebuild).
/// Returns Some(vec) with list of changed relative paths.
fn detect_changed_files(
    root: &Path,
    meta: &IndexMetadata,
    current_commit: &Option<String>,
) -> Option<Vec<String>> {
    let mut changed = Vec::new();

    // Strategy 1: if git commits differ, use git diff
    if let (Some(old_commit), Some(new_commit)) = (&meta.git_commit, current_commit) {
        if old_commit != new_commit {
            return detect_git_diff_files(root, old_commit, new_commit);
        }
    }

    // Strategy 2: compare mtime of each indexed file
    for file in &meta.files {
        let full_path = root.join(&file.path);
        match fs::metadata(&full_path) {
            Ok(m) => {
                let current_mtime = m
                    .modified()
                    .ok()?
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                if current_mtime != file.mtime || m.len() != file.size {
                    changed.push(file.path.clone());
                }
            }
            Err(_) => {
                // File was deleted
                changed.push(file.path.clone());
            }
        }
    }

    // Also check for new files (not in old index)
    // This is expensive without git, so only do mtime-based for now
    Some(changed)
}

/// Use git diff to find changed files between two commits.
fn detect_git_diff_files(root: &Path, old: &str, new: &str) -> Option<Vec<String>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", &format!("{}..{}", old, new)])
        .current_dir(root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect();

    // Also add any uncommitted changes
    let status_output = Command::new("git")
        .args(["status", "--porcelain", "--no-renames"])
        .current_dir(root)
        .output()
        .ok()?;

    if status_output.status.success() {
        let mut all_files: std::collections::HashSet<String> =
            files.into_iter().collect();
        for line in String::from_utf8_lossy(&status_output.stdout).lines() {
            if line.len() > 3 {
                all_files.insert(line[3..].to_string());
            }
        }
        Some(all_files.into_iter().collect())
    } else {
        Some(files)
    }
}

fn next_prime(n: usize) -> usize {
    if n <= 2 {
        return 2;
    }
    let mut candidate = if n % 2 == 0 { n + 1 } else { n };
    loop {
        if is_prime(candidate) {
            return candidate;
        }
        candidate += 2;
    }
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    if n == 2 || n == 3 {
        return true;
    }
    if n % 2 == 0 || n % 3 == 0 {
        return false;
    }
    let mut i = 5;
    while i * i <= n {
        if n % i == 0 || n % (i + 2) == 0 {
            return false;
        }
        i += 6;
    }
    true
}
