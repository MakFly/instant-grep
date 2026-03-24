use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::index::merge;
use crate::index::metadata::{INDEX_VERSION, IndexMetadata, IndexedFile};
use crate::index::ngram::extract_sparse_ngrams;
use crate::index::overlay;
use crate::index::spimi;
use crate::util::{ig_dir, is_binary};
use crate::walk::walk_files;

/// Max changed files for overlay path (above this, full rebuild).
const OVERLAY_THRESHOLD: usize = 100;

/// Build or incrementally update the index.
pub fn build_index(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
) -> Result<IndexMetadata> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = ig_dir(&root);

    let existing_meta = load_existing_metadata(&ig);
    let current_git_commit = get_git_head(&root);

    if let Some(ref meta) = existing_meta
        && meta.version == INDEX_VERSION
    {
        let changed = detect_changed_files(&root, meta, &current_git_commit);
        if let Some(changed_paths) = changed {
            if changed_paths.is_empty() {
                eprintln!("Index is up to date");
                return Ok(meta.clone());
            }

            // Use overlay for small changes, full rebuild for large changes
            if changed_paths.len() <= OVERLAY_THRESHOLD {
                eprintln!(
                    "Detected {} changed files, building overlay...",
                    changed_paths.len()
                );
                return incremental_overlay(
                    &root,
                    use_default_excludes,
                    max_file_size,
                    &current_git_commit,
                    meta,
                    &changed_paths,
                );
            } else {
                eprintln!(
                    "Detected {} changed files (>{} threshold), full rebuild...",
                    changed_paths.len(),
                    OVERLAY_THRESHOLD
                );
            }
        }
    }

    // Clear any existing overlay before full rebuild
    overlay::clear_overlay(&ig);

    full_rebuild(
        &root,
        use_default_excludes,
        max_file_size,
        &current_git_commit,
    )
}

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
        .filter_map(|path| {
            let bytes = fs::read(path).ok()?;
            if is_binary(&bytes) {
                return None;
            }
            let ngrams = extract_sparse_ngrams(&bytes);
            let mtime = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
            let rel_path = path.strip_prefix(root).ok()?.to_string_lossy().to_string();
            Some((rel_path, bytes.len() as u64, mtime, ngrams))
        })
        .collect();

    write_index_spimi(root, &ig, &file_data, git_commit)
}

/// Build index using SPIMI pipeline: segment build → k-way merge → lexicon → metadata.
fn write_index_spimi(
    root: &Path,
    ig: &Path,
    file_data: &[(String, u64, u64, Vec<u64>)],
    git_commit: &Option<String>,
) -> Result<IndexMetadata> {
    fs::create_dir_all(ig).context("create .ig directory")?;

    let segment_dir = ig.join("segments");
    let postings_path = ig.join("postings.bin");
    let lexicon_path = ig.join("lexicon.bin");

    // Phase 1: Build SPIMI segments with bounded memory
    let segments = spimi::build_segments(file_data, spimi::DEFAULT_MEMORY_BUDGET, &segment_dir)
        .context("build SPIMI segments")?;

    eprintln!(
        "Built {} segment(s) from {} files",
        segments.len(),
        file_data.len()
    );

    // Phase 2: K-way merge segments into postings.bin
    let merged_entries =
        merge::merge_segments(&segments, &postings_path).context("merge segments")?;

    // Phase 3: Build and write lexicon
    let lexicon_data = merge::build_lexicon(&merged_entries);
    fs::write(&lexicon_path, &lexicon_data).context("write lexicon.bin")?;

    // Phase 4: Cleanup segments
    merge::cleanup_segments(&segments);

    // Phase 5: Build and write metadata (without file_ngrams!)
    let files: Vec<IndexedFile> = file_data
        .iter()
        .map(|(rel_path, size, mtime, _)| IndexedFile {
            path: rel_path.clone(),
            mtime: *mtime,
            size: *size,
        })
        .collect();

    let metadata = IndexMetadata {
        version: INDEX_VERSION,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        root: root.to_string_lossy().to_string(),
        file_count: files.len() as u32,
        ngram_count: merged_entries.len() as u32,
        files,
        git_commit: git_commit.clone(),
    };

    metadata.write_to(ig).context("write metadata")?;

    // Auto-add .ig/ to .gitignore
    let gitignore = root.join(".gitignore");
    if gitignore.exists()
        && let Ok(content) = fs::read_to_string(&gitignore)
        && !content
            .lines()
            .any(|l| l.trim() == ".ig" || l.trim() == ".ig/")
    {
        let mut f = fs::OpenOptions::new().append(true).open(&gitignore)?;
        writeln!(f, "\n# instant-grep index\n.ig/")?;
    }

    Ok(metadata)
}

fn load_existing_metadata(ig: &Path) -> Option<IndexMetadata> {
    IndexMetadata::load_from(ig).ok()
}

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

fn detect_changed_files(
    root: &Path,
    meta: &IndexMetadata,
    current_commit: &Option<String>,
) -> Option<Vec<String>> {
    let mut changed = Vec::new();

    if let (Some(old_commit), Some(new_commit)) = (&meta.git_commit, current_commit)
        && old_commit != new_commit
    {
        return detect_git_diff_files(root, old_commit, new_commit);
    }

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
                changed.push(file.path.clone());
            }
        }
    }

    Some(changed)
}

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

    let status_output = Command::new("git")
        .args(["status", "--porcelain", "--no-renames"])
        .current_dir(root)
        .output()
        .ok()?;

    if status_output.status.success() {
        let mut all_files: std::collections::HashSet<String> = files.into_iter().collect();
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

/// Build an overlay index for a small number of changed files.
fn incremental_overlay(
    root: &Path,
    _use_default_excludes: bool,
    _max_file_size: u64,
    git_commit: &Option<String>,
    base_meta: &IndexMetadata,
    changed_paths: &[String],
) -> Result<IndexMetadata> {
    let ig = ig_dir(root);

    // Separate changed files into modified/new vs deleted
    let mut changed_file_data: Vec<(String, u64, u64, Vec<u64>)> = Vec::new();
    let mut deleted_paths: Vec<String> = Vec::new();

    for rel_path in changed_paths {
        let full_path = root.join(rel_path);
        if full_path.exists() {
            match fs::read(&full_path) {
                Ok(bytes) => {
                    if is_binary(&bytes) {
                        continue;
                    }
                    let ngrams = extract_sparse_ngrams(&bytes);
                    let mtime = fs::metadata(&full_path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    changed_file_data.push((
                        rel_path.clone(),
                        bytes.len() as u64,
                        mtime,
                        ngrams,
                    ));
                }
                Err(_) => {
                    deleted_paths.push(rel_path.clone());
                }
            }
        } else {
            deleted_paths.push(rel_path.clone());
        }
    }

    overlay::build_overlay(
        &ig,
        base_meta.file_count,
        &base_meta.files,
        &changed_file_data,
        &deleted_paths,
        &base_meta.git_commit,
        git_commit,
    )?;

    eprintln!(
        "Overlay: {} modified/new, {} deleted",
        changed_file_data.len(),
        deleted_paths.len()
    );

    // Return base metadata (overlay is transparent at query time)
    Ok(base_meta.clone())
}
