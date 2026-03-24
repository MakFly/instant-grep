use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::bytes::RegexBuilder;

use crate::index::reader::IndexReader;
use crate::query::extract::regex_to_query;
use crate::search::fallback;
use crate::search::matcher::{self, FileMatches, SearchConfig};
use crate::util::ig_dir;

pub struct SearchStats {
    pub total_files: usize,
    pub candidate_files: usize,
    pub search_duration: std::time::Duration,
    pub used_index: bool,
}

/// Search using the trigram index.
/// Falls back to brute-force if the pattern cannot be optimized.
pub fn search_indexed(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    config: &SearchConfig,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
) -> Result<(Vec<FileMatches>, SearchStats)> {
    let ig = ig_dir(root);
    let start = Instant::now();

    let reader = IndexReader::open(&ig).context("open index")?;
    let total_files = reader.total_file_count() as usize;

    let query = regex_to_query(pattern, case_insensitive)?;

    if query.is_all() {
        let results = fallback::search_brute_force(
            root,
            pattern,
            case_insensitive,
            config,
            type_filter,
            glob_filter,
        )?;
        let stats = SearchStats {
            total_files,
            candidate_files: total_files,
            search_duration: start.elapsed(),
            used_index: false,
        };
        return Ok((results, stats));
    }

    let candidates = reader.resolve(&query);

    // Escape hatch: if index can't filter enough (>60% of files are candidates),
    // fall back to brute-force — avoids index overhead (mmap, VByte decode, hash probe)
    // that exceeds rg's direct-scan cost at high candidate ratios.
    if total_files > 0 && candidates.len() * 100 / total_files > 60 {
        let results = fallback::search_brute_force(
            root,
            pattern,
            case_insensitive,
            config,
            type_filter,
            glob_filter,
        )?;
        let stats = SearchStats {
            total_files,
            candidate_files: candidates.len(),
            search_duration: start.elapsed(),
            used_index: false,
        };
        return Ok((results, stats));
    }

    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .unicode(false)
        .build()
        .context("invalid regex")?;

    // Collect candidate paths, applying type/glob filters
    let candidate_paths: Vec<(u32, String)> = candidates
        .iter()
        .filter_map(|doc_id| {
            let rel_path = reader.file_path(*doc_id).to_string();

            // Apply type filter
            if let Some(ft) = type_filter
                && !matches_type(&rel_path, ft)
            {
                return None;
            }

            // Apply glob filter
            if let Some(glob) = glob_filter
                && !matches_glob(&rel_path, glob)
            {
                return None;
            }

            Some((*doc_id, rel_path))
        })
        .collect();

    let filtered_count = candidate_paths.len();

    // Parallel candidate verification with rayon
    let mut results: Vec<FileMatches> = candidate_paths
        .par_iter()
        .filter_map(|(_doc_id, rel_path)| {
            matcher::match_file(root, rel_path, &regex, config)
                .ok()
                .flatten()
        })
        .collect();

    // Sort by path for deterministic output
    results.sort_by(|a, b| a.path.cmp(&b.path));

    let stats = SearchStats {
        total_files,
        candidate_files: filtered_count,
        search_duration: start.elapsed(),
        used_index: true,
    };

    Ok((results, stats))
}

/// Simple type matching based on file extension.
fn matches_type(path: &str, file_type: &str) -> bool {
    let ext = match path.rsplit('.').next() {
        Some(e) => e,
        None => return false,
    };
    match file_type {
        "rs" | "rust" => ext == "rs",
        "ts" | "typescript" => ext == "ts" || ext == "tsx",
        "js" | "javascript" => ext == "js" || ext == "jsx" || ext == "mjs" || ext == "cjs",
        "py" | "python" => ext == "py" || ext == "pyi",
        "go" => ext == "go",
        "java" => ext == "java",
        "php" => ext == "php",
        "rb" | "ruby" => ext == "rb",
        "c" => ext == "c" || ext == "h",
        "cpp" | "cxx" => {
            ext == "cpp" || ext == "cxx" || ext == "cc" || ext == "hpp" || ext == "hxx"
        }
        "css" => ext == "css",
        "html" => ext == "html" || ext == "htm",
        "json" => ext == "json",
        "yaml" | "yml" => ext == "yaml" || ext == "yml",
        "toml" => ext == "toml",
        "md" | "markdown" => ext == "md" || ext == "markdown",
        "sh" | "bash" => ext == "sh" || ext == "bash",
        "sql" => ext == "sql",
        "vue" => ext == "vue",
        "svelte" => ext == "svelte",
        "swift" => ext == "swift",
        "kt" | "kotlin" => ext == "kt" || ext == "kts",
        "dart" => ext == "dart",
        "zig" => ext == "zig",
        _ => ext == file_type,
    }
}

/// Simple glob matching (supports *.ext patterns).
fn matches_glob(path: &str, glob: &str) -> bool {
    if let Some(ext_pattern) = glob.strip_prefix("*.") {
        path.ends_with(&format!(".{}", ext_pattern))
    } else {
        path.contains(glob)
    }
}
