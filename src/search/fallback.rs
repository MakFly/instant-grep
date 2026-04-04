use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::bytes::RegexBuilder;

use crate::search::matcher::{self, FileMatches, SearchConfig};
use crate::walk::walk_files;

/// Quick binary check: read only the first 8KB to look for null bytes.
fn is_binary_file(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    let Ok(n) = (&file).read(&mut buf) else {
        return false;
    };
    buf[..n].contains(&0)
}

/// Brute-force search: scan all files with regex (no index).
/// Uses rayon for parallel file verification.
#[allow(clippy::too_many_arguments)]
pub fn search_brute_force(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    config: &SearchConfig,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
    path_filters: &[String],
    max_file_size: u64,
) -> Result<Vec<FileMatches>> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .unicode(false)
        .build()
        .context("invalid regex")?;

    // Single-file mode: if exactly one filter pointing to a file, skip walk
    if path_filters.len() == 1 && !path_filters[0].ends_with('/') {
        let pf = &path_filters[0];
        let full_path = root.join(pf);
        if full_path.is_file() {
            if let Ok(Some(file_matches)) = matcher::match_file(root, pf, &regex, config) {
                return Ok(vec![file_matches]);
            }
            return Ok(Vec::new());
        }
    }

    let paths =
        walk_files(root, true, max_file_size, type_filter, glob_filter).context("walking files")?;

    let mut results: Vec<FileMatches> = paths
        .par_iter()
        .filter_map(|path| {
            let rel_path = path.strip_prefix(root).ok()?.to_string_lossy().to_string();

            // Apply path filters (files or directory prefixes)
            if !path_filters.is_empty()
                && !path_filters.iter().any(|pf| {
                    if pf.ends_with('/') {
                        rel_path.starts_with(pf.as_str())
                    } else {
                        rel_path == *pf
                    }
                })
            {
                return None;
            }

            // Quick binary check — reads only 8KB, not the whole file
            if is_binary_file(path) {
                return None;
            }

            matcher::match_file(root, &rel_path, &regex, config)
                .ok()
                .flatten()
        })
        .collect();

    results.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(results)
}
