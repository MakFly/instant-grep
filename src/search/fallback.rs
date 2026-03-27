use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::bytes::RegexBuilder;

use crate::search::matcher::{self, FileMatches, SearchConfig};
use crate::walk::{DEFAULT_MAX_FILE_SIZE, walk_files};

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
pub fn search_brute_force(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    config: &SearchConfig,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
    path_filter: Option<&str>,
) -> Result<Vec<FileMatches>> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .unicode(false)
        .build()
        .context("invalid regex")?;

    // Single-file mode: skip walk, search only the target file
    if let Some(pf) = path_filter {
        let full_path = root.join(pf);
        if full_path.exists()
            && let Ok(Some(file_matches)) = matcher::match_file(root, pf, &regex, config)
        {
            return Ok(vec![file_matches]);
        }
        return Ok(Vec::new());
    }

    let paths = walk_files(root, true, DEFAULT_MAX_FILE_SIZE, type_filter, glob_filter)
        .context("walking files")?;

    let mut results: Vec<FileMatches> = paths
        .par_iter()
        .filter_map(|path| {
            let rel_path = path.strip_prefix(root).ok()?.to_string_lossy().to_string();

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
