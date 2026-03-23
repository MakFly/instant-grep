use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result};
use memmap2::Mmap;
use regex::bytes::Regex;

/// Configuration for search behavior.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    pub before_context: usize,
    pub after_context: usize,
    pub count_only: bool,
    pub files_only: bool,
}

/// A single match within a file.
#[derive(Debug)]
pub struct LineMatch {
    pub line_number: usize,
    pub line: Vec<u8>,
    pub match_ranges: Vec<Range<usize>>, // byte ranges within the line
    pub is_context: bool,
}

/// All matches in a single file.
#[derive(Debug)]
pub struct FileMatches {
    pub path: String,
    pub matches: Vec<LineMatch>,
    pub match_count: usize,
}

/// Match a regex against a file's contents and extract matching lines with context.
pub fn match_file(
    root: &Path,
    rel_path: &str,
    regex: &Regex,
    config: &SearchConfig,
) -> Result<Option<FileMatches>> {
    let full_path = root.join(rel_path);
    let file = std::fs::File::open(&full_path)
        .with_context(|| format!("open {}", full_path.display()))?;

    let mmap = unsafe {
        Mmap::map(&file).with_context(|| format!("mmap {}", full_path.display()))?
    };
    let content = &*mmap;

    if content.is_empty() {
        return Ok(None);
    }

    // Find all match positions
    let mut match_lines: Vec<(usize, Range<usize>)> = Vec::new(); // (line_number, range_in_line)
    let mut line_starts: Vec<usize> = vec![0];

    // Build line index
    for (i, &byte) in content.iter().enumerate() {
        if byte == b'\n' {
            line_starts.push(i + 1);
        }
    }

    let mut match_count = 0;

    for m in regex.find_iter(content) {
        match_count += 1;

        if config.count_only || config.files_only {
            continue;
        }

        // Find which line this match is on
        let line_idx = match line_starts.binary_search(&m.start()) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };

        let line_start = line_starts[line_idx];
        let range_in_line = (m.start() - line_start)..(m.end() - line_start);
        match_lines.push((line_idx, range_in_line));
    }

    if match_count == 0 {
        return Ok(None);
    }

    if config.count_only || config.files_only {
        return Ok(Some(FileMatches {
            path: rel_path.to_string(),
            matches: Vec::new(),
            match_count,
        }));
    }

    // Collect unique lines to display (including context)
    let mut display_lines: std::collections::BTreeMap<usize, LineMatch> =
        std::collections::BTreeMap::new();

    for (line_idx, match_range) in &match_lines {
        // Context before
        let ctx_start = line_idx.saturating_sub(config.before_context);
        for ctx_line in ctx_start..*line_idx {
            display_lines.entry(ctx_line).or_insert_with(|| {
                let line = get_line(content, &line_starts, ctx_line);
                LineMatch {
                    line_number: ctx_line + 1,
                    line,
                    match_ranges: Vec::new(),
                    is_context: true,
                }
            });
        }

        // The match line itself
        let entry = display_lines.entry(*line_idx).or_insert_with(|| {
            let line = get_line(content, &line_starts, *line_idx);
            LineMatch {
                line_number: line_idx + 1,
                line,
                match_ranges: Vec::new(),
                is_context: false,
            }
        });
        entry.is_context = false;
        entry.match_ranges.push(match_range.clone());

        // Context after
        let ctx_end = (line_idx + 1 + config.after_context).min(line_starts.len());
        for ctx_line in (line_idx + 1)..ctx_end {
            display_lines.entry(ctx_line).or_insert_with(|| {
                let line = get_line(content, &line_starts, ctx_line);
                LineMatch {
                    line_number: ctx_line + 1,
                    line,
                    match_ranges: Vec::new(),
                    is_context: true,
                }
            });
        }
    }

    let matches: Vec<LineMatch> = display_lines.into_values().collect();

    Ok(Some(FileMatches {
        path: rel_path.to_string(),
        matches,
        match_count,
    }))
}

fn get_line(content: &[u8], line_starts: &[usize], line_idx: usize) -> Vec<u8> {
    let start = line_starts[line_idx];
    let end = line_starts
        .get(line_idx + 1)
        .map(|&s| s.saturating_sub(1)) // strip \n
        .unwrap_or(content.len());
    content[start..end].to_vec()
}
