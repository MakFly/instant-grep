use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ahash::AHashMap;
use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::index::merge;
use crate::index::metadata::{INDEX_VERSION, IndexMetadata, IndexedFile};
use crate::index::ngram::{NgramKey, extract_sparse_ngrams};
use crate::index::overlay::{self, OverlayReader};
use crate::index::postings::DocId;
use crate::index::spimi;
use crate::util::{ig_dir, is_binary};
use crate::walk::walk_files;

/// Max changed files for overlay path (above this, full rebuild).
const OVERLAY_THRESHOLD: usize = 100;

/// Batch size for streaming file processing.
/// Only this many files' ngrams are in memory at once.
const BATCH_SIZE: usize = 1000;

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

    let result = if let Some(ref meta) = existing_meta
        && meta.version == INDEX_VERSION
    {
        // Check if existing overlay is too large and needs compaction
        if let Ok(Some(overlay_reader)) = OverlayReader::open(&ig) {
            if overlay_reader.needs_compaction(meta.file_count) {
                eprintln!("Overlay too large, compacting...");
                overlay::clear_overlay(&ig);
                full_rebuild(
                    &root,
                    use_default_excludes,
                    max_file_size,
                    &current_git_commit,
                )?
            } else {
                // Fall through to changed-files check below
                check_and_rebuild(
                    &root,
                    &ig,
                    use_default_excludes,
                    max_file_size,
                    &current_git_commit,
                    meta,
                )?
            }
        } else {
            check_and_rebuild(
                &root,
                &ig,
                use_default_excludes,
                max_file_size,
                &current_git_commit,
                meta,
            )?
        }
    } else {
        // Clear any existing overlay before full rebuild
        overlay::clear_overlay(&ig);
        full_rebuild(
            &root,
            use_default_excludes,
            max_file_size,
            &current_git_commit,
        )?
    };

    // Generate tree.txt and context.md alongside index artifacts
    generate_tree(&root, &ig);
    crate::pack::generate_context_quiet(&root, &ig);

    Ok(result)
}

fn check_and_rebuild(
    root: &Path,
    ig: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    current_git_commit: &Option<String>,
    meta: &IndexMetadata,
) -> Result<IndexMetadata> {
    let changed = detect_changed_files(root, meta, current_git_commit);
    if let Some(changed_paths) = changed {
        if changed_paths.is_empty() {
            eprintln!("Index is up to date");
            return Ok(meta.clone());
        }

        if changed_paths.len() <= OVERLAY_THRESHOLD {
            eprintln!(
                "Detected {} changed files, building overlay...",
                changed_paths.len()
            );
            return incremental_overlay(
                root,
                use_default_excludes,
                max_file_size,
                current_git_commit,
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

    overlay::clear_overlay(ig);
    full_rebuild(
        root,
        use_default_excludes,
        max_file_size,
        current_git_commit,
    )
}

/// Generate `.ig/tree.txt` — a depth-3 directory tree for AI agent onboarding.
/// Uses the same exclude list as the index walker. Runs in <1ms on typical projects.
fn generate_tree(root: &Path, ig: &Path) {
    use std::collections::BTreeSet;

    let tree_path = ig.join("tree.txt");

    // Collect directories up to depth 3 from walked files
    let paths = match walk_files(root, true, 0, None, None) {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut dirs: BTreeSet<String> = BTreeSet::new();
    for path in &paths {
        if let Ok(rel) = path.strip_prefix(root) {
            let components: Vec<_> = rel.components().collect();
            // Add parent directories up to depth 3
            for depth in 1..=3.min(components.len()) {
                let dir: String = components[..depth]
                    .iter()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                if depth < components.len() {
                    // It's a directory (has children)
                    dirs.insert(format!("{}/ ", dir));
                } else {
                    // It's a file at this depth
                    dirs.insert(dir);
                }
            }
        }
    }

    // Write simple flat listing (fast to generate, easy for LLMs to parse)
    let mut output = String::with_capacity(dirs.len() * 40);
    for entry in &dirs {
        output.push_str(entry.trim_end());
        output.push('\n');
    }

    let _ = fs::write(&tree_path, output.as_bytes());
}

/// Streaming full rebuild: processes files in batches of BATCH_SIZE.
/// Only one batch of ngrams is in memory at a time — truly bounded RAM.
fn full_rebuild(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    git_commit: &Option<String>,
) -> Result<IndexMetadata> {
    let ig = ig_dir(root);
    fs::create_dir_all(&ig).context("create .ig directory")?;

    let paths = walk_files(root, use_default_excludes, max_file_size, None, None)
        .context("walking files")?;

    let segment_dir = ig.join("segments");
    fs::create_dir_all(&segment_dir).context("create segment directory")?;

    let mut budget = spimi::MemoryBudget::new(spimi::DEFAULT_MEMORY_BUDGET);
    let mut postings_map: AHashMap<NgramKey, Vec<DocId>> = AHashMap::new();
    let mut files: Vec<IndexedFile> = Vec::with_capacity(paths.len());
    let mut segments: Vec<spimi::SegmentInfo> = Vec::new();
    let mut segment_id: u32 = 0;
    let mut doc_id: u32 = 0;

    // Process files in batches — only BATCH_SIZE files' ngrams in memory at once
    for batch in paths.chunks(BATCH_SIZE) {
        let batch_data: Vec<_> = batch
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

        // Feed batch into SPIMI accumulator, then drop batch (ngrams freed)
        for (rel_path, size, mtime, ngrams) in batch_data {
            files.push(IndexedFile {
                path: rel_path,
                mtime,
                size,
            });

            for &key in &ngrams {
                let is_new = !postings_map.contains_key(&key);
                postings_map.entry(key).or_default().push(doc_id);
                budget.track_posting(is_new);
            }
            doc_id += 1;
            // ngrams Vec dropped here — not stored

            if budget.should_flush() && !postings_map.is_empty() {
                let info = spimi::flush_segment(&mut postings_map, &segment_dir, segment_id)?;
                segments.push(info);
                segment_id += 1;
                budget.reset();
            }
        }
        // entire batch_data dropped here — RAM freed
    }

    // Flush remaining postings
    if !postings_map.is_empty() {
        let info = spimi::flush_segment(&mut postings_map, &segment_dir, segment_id)?;
        segments.push(info);
    }
    drop(postings_map); // free AHashMap

    let file_count = files.len() as u32;

    eprintln!(
        "Built {} segment(s) from {} files",
        segments.len(),
        file_count
    );

    // K-way merge segments into postings.bin (streaming: entries go to temp file, not RAM)
    let postings_path = ig.join("postings.bin");
    let merge_result =
        merge::merge_segments_streaming(&segments, &postings_path).context("merge segments")?;

    let ngram_count = merge_result.entry_count as u32;

    // Build lexicon via mmap from the entries temp file (no heap allocation)
    let lexicon_path = ig.join("lexicon.bin");
    merge::build_lexicon_mmap_from_file(
        &merge_result.entries_path,
        merge_result.entry_count,
        &lexicon_path,
    )
    .context("write lexicon.bin")?;

    drop(merge_result); // entries temp file cleaned up

    // Cleanup segments
    merge::cleanup_segments(&segments);

    // Write metadata
    let metadata = IndexMetadata {
        version: INDEX_VERSION,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        root: root.to_string_lossy().to_string(),
        file_count,
        ngram_count,
        files,
        git_commit: git_commit.clone(),
    };

    metadata.write_to(&ig).context("write metadata")?;

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
                    changed_file_data.push((rel_path.clone(), bytes.len() as u64, mtime, ngrams));
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
