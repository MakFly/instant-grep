use std::path::Path;

use anyhow::{Context, Result};
use regex::bytes::RegexBuilder;

use crate::search::matcher::{self, FileMatches, SearchConfig};
use crate::util::is_binary;
use crate::walk::{DEFAULT_MAX_FILE_SIZE, walk_files};

/// Brute-force search: scan all files with regex (no index).
pub fn search_brute_force(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    config: &SearchConfig,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
) -> Result<Vec<FileMatches>> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .unicode(false)
        .build()
        .context("invalid regex")?;

    let paths = walk_files(root, true, DEFAULT_MAX_FILE_SIZE, type_filter, glob_filter)
        .context("walking files")?;
    let mut results = Vec::new();

    for path in &paths {
        let rel_path = match path.strip_prefix(root) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => continue,
        };

        // Quick binary check
        if let Ok(bytes) = std::fs::read(path)
            && is_binary(&bytes)
        {
            continue;
        }

        match matcher::match_file(root, &rel_path, &regex, config) {
            Ok(Some(file_matches)) => results.push(file_matches),
            Ok(None) => {}
            Err(_) => continue,
        }
    }

    Ok(results)
}
