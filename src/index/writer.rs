use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ahash::{AHashMap, AHashSet};
use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::index::filedata::{self, FileData, FileDataIndex};
use crate::index::merge;
use crate::index::metadata::{INDEX_VERSION, IndexMetadata, IndexedFile};
use crate::index::ngram::{self, BigramDfTable, NgramKey, extract_sparse_ngrams_with_masks};
use crate::index::overlay::{self, ChangedFileEntry, OverlayReader};
use crate::index::spimi;
use crate::index::vbyte::PostingEntry;
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

    // Build the PMI co-occurrence table for semantic expansion when enabled.
    // The daemon disables this by default to keep its background RSS bounded;
    // explicit CLI indexing keeps the historical default unless configured.
    if crate::config::semantic_index_enabled() {
        let _ =
            crate::semantic::cooccur::build_for_root(&root, use_default_excludes, max_file_size);
    }

    // Record provenance in the cache entry (no-op if `ig` is a local .ig/).
    let _ = crate::cache::write_meta(&ig, &root);

    Ok(result)
}

/// Update the overlay index from an explicit set of changed paths.
///
/// This is the daemon/watch hot path: callers already know which files changed,
/// so we avoid the full project scan in `detect_changed_files`. Existing overlay
/// entries are folded into the new changed set before rebuilding the overlay so
/// successive file events do not lose earlier un-compacted changes.
pub fn update_index_for_paths(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    changed_paths: &[std::path::PathBuf],
) -> Result<IndexMetadata> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = ig_dir(&root);

    let Some(base_meta) = load_existing_metadata(&ig) else {
        return build_index(&root, use_default_excludes, max_file_size);
    };
    if base_meta.version != INDEX_VERSION {
        return build_index(&root, use_default_excludes, max_file_size);
    }

    let current_git_commit = get_git_head(&root);
    let mut changed: AHashSet<String> = AHashSet::new();

    if let Ok(Some(overlay_reader)) = OverlayReader::open(&ig) {
        if overlay_reader.metadata.overlay_file_count as usize > OVERLAY_THRESHOLD {
            overlay::clear_overlay(&ig);
            return full_rebuild(
                &root,
                use_default_excludes,
                max_file_size,
                &current_git_commit,
            );
        }

        for file in &overlay_reader.metadata.added_files {
            changed.insert(file.path.clone());
        }
        for doc_id in &overlay_reader.metadata.tombstone_doc_ids {
            if let Some(file) = base_meta.files.get(*doc_id as usize) {
                changed.insert(file.path.clone());
            }
        }
    }

    for path in changed_paths {
        if let Some(rel) = normalize_changed_path(&root, path) {
            changed.insert(rel);
        }
    }

    if changed.is_empty() {
        crate::cache::touch(&ig);
        return Ok(base_meta);
    }

    let mut changed_paths: Vec<String> = changed.into_iter().collect();
    changed_paths.sort();

    if changed_paths.len() > OVERLAY_THRESHOLD {
        eprintln!(
            "Detected {} changed files (>{} threshold), full rebuild...",
            changed_paths.len(),
            OVERLAY_THRESHOLD
        );
        overlay::clear_overlay(&ig);
        return full_rebuild(
            &root,
            use_default_excludes,
            max_file_size,
            &current_git_commit,
        );
    }

    eprintln!(
        "Detected {} changed files from watcher, building overlay...",
        changed_paths.len()
    );
    let meta = incremental_overlay(
        &root,
        use_default_excludes,
        max_file_size,
        &current_git_commit,
        &base_meta,
        &changed_paths,
    )?;
    let _ = crate::cache::write_meta(&ig, &root);
    Ok(meta)
}

fn normalize_changed_path(root: &Path, path: &Path) -> Option<String> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    // canonicalize to make symlink-resolved paths comparable. macOS resolves
    // `/var/folders/...` to `/private/var/folders/...`; without this, watcher
    // events fired with the un-canonicalized path fail to strip the canonical
    // root and silently drop every change.
    let abs = abs.canonicalize().unwrap_or(abs);
    let rel = abs.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    if rel.components().any(|c| c.as_os_str() == ".ig") {
        return None;
    }
    Some(rel.to_string_lossy().to_string())
}

fn merge_ngram_masks(ngrams: &mut Vec<(NgramKey, u8, u8, u32)>) {
    ngrams.sort_unstable_by_key(|(key, _, _, _)| *key);
    let mut merged: Vec<(NgramKey, u8, u8, u32)> = Vec::with_capacity(ngrams.len());
    for &(key, next_mask, loc_mask, zone_mask) in ngrams.iter() {
        if let Some(last) = merged.last_mut()
            && last.0 == key
        {
            last.1 |= next_mask;
            last.2 |= loc_mask;
            last.3 |= zone_mask;
            continue;
        }
        merged.push((key, next_mask, loc_mask, zone_mask));
    }
    *ngrams = merged;
}

fn check_and_rebuild(
    root: &Path,
    ig: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    current_git_commit: &Option<String>,
    meta: &IndexMetadata,
) -> Result<IndexMetadata> {
    let changed = detect_changed_files(
        root,
        meta,
        current_git_commit,
        use_default_excludes,
        max_file_size,
    );
    if let Some(changed_paths) = changed {
        if changed_paths.is_empty() {
            // Sanity check: verify total file count on disk matches what the index knows.
            // This catches edge cases where detect_changed_files missed new files (e.g.
            // git-diff path used but untracked files were added outside git's view).
            let overlay_file_count = OverlayReader::open(ig)
                .ok()
                .flatten()
                .map(|r| r.metadata.overlay_file_count as usize)
                .unwrap_or(0);
            let indexed_count = meta.file_count as usize + overlay_file_count;

            let disk_count = walk_files(root, use_default_excludes, max_file_size, None, None)
                .map(|f| f.len())
                .unwrap_or(0);

            if disk_count != indexed_count {
                eprintln!(
                    "File count mismatch (index: {}, disk: {}), rebuilding...",
                    indexed_count, disk_count
                );
                overlay::clear_overlay(ig);
                return full_rebuild(
                    root,
                    use_default_excludes,
                    max_file_size,
                    current_git_commit,
                );
            }

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

/// Streaming full rebuild: processes files in bounded batches.
/// Only one batch of ngrams is in memory at a time.
fn full_rebuild(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    git_commit: &Option<String>,
) -> Result<IndexMetadata> {
    let ig = ig_dir(root);
    fs::create_dir_all(&ig).context("create .ig directory")?;

    // Best-effort cleanup of `.ig-entries-*.tmp` orphans left by killed runs
    // (SIGKILL, OOM, panic). The merge step does its own sweep too — this is
    // the earliest opportunity, before the index dir grows large.
    merge::sweep_orphan_entries(&ig);

    let paths = walk_files(root, use_default_excludes, max_file_size, None, None)
        .context("walking files")?;

    let segment_dir = ig.join("segments");
    fs::create_dir_all(&segment_dir).context("create segment directory")?;

    let mut budget = spimi::MemoryBudget::new(crate::config::index_memory_budget_bytes());
    let mut postings_map: AHashMap<NgramKey, Vec<PostingEntry>> = AHashMap::new();
    let mut files: Vec<IndexedFile> = Vec::with_capacity(paths.len());
    let mut segments: Vec<spimi::SegmentInfo> = Vec::new();
    let mut segment_id: u32 = 0;
    let mut doc_id: u32 = 0;
    let mut bigram_df: AHashMap<u32, u32> = AHashMap::new();
    let mut filedata_entries: Vec<(String, FileData)> = Vec::with_capacity(paths.len());

    // Load previous DF table if available (bootstraps IDF weighting on rebuild)
    let prev_df_table = BigramDfTable::load(&ig);

    // Process files in batches — only one batch's ngrams are in memory at once.
    let batch_size = crate::config::index_batch_size();
    for batch in paths.chunks(batch_size) {
        let batch_data: Vec<_> = batch
            .par_iter()
            .filter_map(|path| {
                let bytes = fs::read(path).ok()?;
                if is_binary(&bytes) {
                    return None;
                }
                let mut ngrams_with_masks =
                    extract_sparse_ngrams_with_masks(&bytes, prev_df_table.as_ref());
                let folded_bytes: Vec<u8> = bytes.iter().map(|b| b.to_ascii_lowercase()).collect();
                if folded_bytes != bytes {
                    ngrams_with_masks.extend(extract_sparse_ngrams_with_masks(
                        &folded_bytes,
                        prev_df_table.as_ref(),
                    ));
                    merge_ngram_masks(&mut ngrams_with_masks);
                }
                // Collect unique bigram hashes for this file (DF collection).
                // Cap initial capacity at 8192: even very large source files
                // exhaust the bigram space well before saturating; using
                // `bytes.len()` over-allocated by 10–100× on every worker.
                let cap = bytes.len().min(8192);
                let mut bigram_hashes: AHashSet<u32> = AHashSet::with_capacity(cap);
                for window in bytes.windows(2) {
                    bigram_hashes.insert(ngram::hash_bigram(window[0], window[1]));
                }
                let mtime = fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()?
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                let rel_path = path.strip_prefix(root).ok()?.to_string_lossy().to_string();

                // Extract pre-computed filedata (line offsets, symbols, summaries)
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let line_offsets = filedata::compute_line_offsets(&bytes);
                let text = std::str::from_utf8(&bytes).ok();
                let symbols = text
                    .map(|t| filedata::extract_symbols_with_boundaries(t, ext))
                    .unwrap_or_default();
                let role = text
                    .map(|t| filedata::extract_simple_role(t, ext))
                    .unwrap_or_default();
                let public_api = text
                    .map(|t| filedata::extract_simple_api(t, ext))
                    .unwrap_or_default();
                let fd = FileData {
                    line_offsets,
                    symbols,
                    role,
                    public_api,
                };

                Some((
                    rel_path,
                    bytes.len() as u64,
                    mtime,
                    ngrams_with_masks,
                    bigram_hashes,
                    fd,
                ))
            })
            .collect();

        // Feed batch into SPIMI accumulator, then drop batch (ngrams freed)
        for (rel_path, size, mtime, ngrams_with_masks, bigram_hashes, fd) in batch_data {
            filedata_entries.push((rel_path.clone(), fd));
            files.push(IndexedFile {
                path: rel_path,
                mtime,
                size,
            });

            for &(key, bloom, loc, zone) in &ngrams_with_masks {
                let is_new = !postings_map.contains_key(&key);
                postings_map.entry(key).or_default().push(PostingEntry {
                    doc_id,
                    next_mask: bloom,
                    loc_mask: loc,
                    zone_mask: zone,
                });
                budget.track_posting(is_new);
            }
            // Accumulate bigram document frequencies
            for h in bigram_hashes {
                *bigram_df.entry(h).or_default() += 1;
            }
            doc_id += 1;
            // ngrams_with_masks Vec dropped here — not stored

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

    // Serialize bigram document frequencies to .ig/bigram_df.bin
    let bigram_df_path_rel = {
        let mut df_pairs: Vec<(u32, u32)> = bigram_df.into_iter().collect();
        df_pairs.sort_unstable_by_key(|&(h, _)| h);
        let df_bytes = bincode::serialize(&df_pairs).context("serialize bigram_df")?;
        let df_path = ig.join("bigram_df.bin");
        fs::write(&df_path, &df_bytes).context("write bigram_df.bin")?;
        eprintln!(
            "Wrote bigram_df.bin ({} entries, {} bytes)",
            df_pairs.len(),
            df_bytes.len()
        );
        Some("bigram_df.bin".to_string())
    };

    // Build and save pre-computed filedata index
    filedata_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let filedata_index = FileDataIndex {
        version: 1,
        entries: filedata_entries,
    };
    let fd_count = filedata_index.entries.len();
    filedata_index.save(&ig).context("write filedata.bin")?;
    eprintln!("Wrote filedata.bin ({} files)", fd_count);

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
        bigram_df_path: bigram_df_path_rel,
        built_with_idf: prev_df_table.is_some(),
    };

    metadata.write_to(&ig).context("write metadata")?;

    // Final act: bump the seal. Daemon's reload_if_changed observes the new
    // generation and is guaranteed all artifacts of this generation are
    // already on disk (each has been atomically renamed above).
    let _gen = super::seal::bump_seal(&ig).context("bump seal after full rebuild")?;

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
    use_default_excludes: bool,
    max_file_size: u64,
) -> Option<Vec<String>> {
    let mut changed = Vec::new();

    if let (Some(old_commit), Some(new_commit)) = (&meta.git_commit, current_commit)
        && old_commit != new_commit
    {
        return detect_git_diff_files(root, old_commit, new_commit);
    }

    // Build a set of indexed paths for O(1) lookup when scanning for new files
    let indexed_paths: std::collections::HashSet<&str> =
        meta.files.iter().map(|f| f.path.as_str()).collect();

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

    // Discover new files on disk that are not yet in the index
    if let Ok(disk_files) = walk_files(root, use_default_excludes, max_file_size, None, None) {
        for path in disk_files {
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy();
                if !indexed_paths.contains(rel_str.as_ref()) {
                    changed.push(rel_str.into_owned());
                }
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
    max_file_size: u64,
    git_commit: &Option<String>,
    base_meta: &IndexMetadata,
    changed_paths: &[String],
) -> Result<IndexMetadata> {
    let ig = ig_dir(root);

    // Separate changed files into modified/new vs deleted
    let mut changed_file_data: Vec<ChangedFileEntry> = Vec::new();
    let mut deleted_paths: Vec<String> = Vec::new();

    let df_table = if base_meta.built_with_idf {
        BigramDfTable::load(&ig)
    } else {
        None
    };

    for rel_path in changed_paths {
        let full_path = root.join(rel_path);
        if full_path.exists() {
            match fs::read(&full_path) {
                Ok(bytes) => {
                    if bytes.len() as u64 > max_file_size || is_binary(&bytes) {
                        continue;
                    }
                    let mut ngrams = extract_sparse_ngrams_with_masks(&bytes, df_table.as_ref());
                    let folded_bytes: Vec<u8> =
                        bytes.iter().map(|b| b.to_ascii_lowercase()).collect();
                    if folded_bytes != bytes {
                        ngrams.extend(extract_sparse_ngrams_with_masks(
                            &folded_bytes,
                            df_table.as_ref(),
                        ));
                        merge_ngram_masks(&mut ngrams);
                    }
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

    // Final act: bump the seal so the daemon notices this incremental
    // update on its next pull check or its FSEvents push handler.
    let _gen = super::seal::bump_seal(&ig).context("bump seal after overlay")?;

    // Return base metadata (overlay is transparent at query time)
    Ok(base_meta.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_bigram_df_bin_generated() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Pre-create <root>/.ig/ to keep the index local — avoids races with
        // the shared XDG cache when tests run in parallel.
        fs::create_dir_all(root.join(".ig")).unwrap();

        // Create a few source files with known content
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        let mut f1 = fs::File::create(src.join("a.rs")).unwrap();
        writeln!(f1, "fn main() {{ println!(\"hello\"); }}").unwrap();

        let mut f2 = fs::File::create(src.join("b.rs")).unwrap();
        writeln!(f2, "fn helper() {{ let x = 42; }}").unwrap();

        let mut f3 = fs::File::create(root.join("readme.txt")).unwrap();
        writeln!(f3, "This is a readme file with some text.").unwrap();

        // Build index
        let meta = build_index(root, false, 10_000_000).unwrap();

        // Verify bigram_df.bin exists (resolve via ig_dir so the test works
        // whether the index lives under <root>/.ig/ or in the XDG cache).
        let ig = ig_dir(root);
        let df_path = ig.join("bigram_df.bin");
        assert!(df_path.exists(), "bigram_df.bin should be created");

        // Verify metadata records the path
        assert_eq!(meta.bigram_df_path.as_deref(), Some("bigram_df.bin"));

        // Deserialize and check contents
        let data = fs::read(&df_path).unwrap();
        let pairs: Vec<(u32, u32)> = bincode::deserialize(&data).unwrap();

        // Should have entries (3 text files with bigrams)
        assert!(!pairs.is_empty(), "bigram_df should have entries");

        // All doc frequencies should be between 1 and file_count
        for &(hash, df) in &pairs {
            assert!(df >= 1, "df must be >= 1 for hash {}", hash);
            assert!(
                df <= meta.file_count,
                "df must be <= file_count for hash {}",
                hash
            );
        }

        // Pairs should be sorted by hash
        for w in pairs.windows(2) {
            assert!(
                w[0].0 < w[1].0,
                "pairs must be sorted by hash: {} >= {}",
                w[0].0,
                w[1].0
            );
        }
    }

    #[test]
    fn test_update_index_for_paths_preserves_existing_overlay_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".ig")).unwrap();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("a.rs"),
            "pub fn a() -> &'static str { \"old_a\" }\n",
        )
        .unwrap();
        fs::write(
            src.join("b.rs"),
            "pub fn b() -> &'static str { \"old_b\" }\n",
        )
        .unwrap();

        build_index(root, false, 10_000_000).unwrap();

        fs::write(
            src.join("a.rs"),
            "pub fn a() -> &'static str { \"new_a\" }\n",
        )
        .unwrap();
        update_index_for_paths(root, false, 10_000_000, &[src.join("a.rs")]).unwrap();

        fs::write(
            src.join("b.rs"),
            "pub fn b() -> &'static str { \"new_b\" }\n",
        )
        .unwrap();
        update_index_for_paths(root, false, 10_000_000, &[src.join("b.rs")]).unwrap();

        let overlay = overlay::OverlayReader::open(&ig_dir(root))
            .unwrap()
            .expect("overlay should exist");
        let mut paths: Vec<_> = overlay
            .metadata
            .added_files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        paths.sort_unstable();
        assert_eq!(paths, vec!["src/a.rs", "src/b.rs"]);
    }
}
