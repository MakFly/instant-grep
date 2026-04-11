use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::index::filedata::FileData;

pub struct BlockResult {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub lines: Vec<(usize, String)>, // (line_number, content)
}

/// Extract the enclosing code block for a given line in a file.
pub fn extract_block(file: &Path, target_line: usize) -> Result<BlockResult> {
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

    let all_lines: Vec<&str> = content.lines().collect();

    if target_line == 0 || target_line > all_lines.len() {
        bail!(
            "line {} is out of range (file has {} lines)",
            target_line,
            all_lines.len()
        );
    }

    let target_idx = target_line - 1; // 0-based

    // Strategy: find the enclosing block using brace matching
    // 1. Walk up from target to find a line at indentation 0 or a definition start
    // 2. Walk down from that point to find the matching closing brace

    let (block_start, block_end) = find_brace_block(&all_lines, target_idx).unwrap_or_else(|| {
        // Fallback: ±20 lines
        let start = target_idx.saturating_sub(20);
        let end = (target_idx + 20).min(all_lines.len().saturating_sub(1));
        (start, end)
    });

    let lines: Vec<(usize, String)> = (block_start..=block_end)
        .map(|i| (i + 1, all_lines[i].to_string()))
        .collect();

    Ok(BlockResult {
        file: file.to_string_lossy().to_string(),
        start: block_start + 1,
        end: block_end + 1,
        lines,
    })
}

/// Extract the enclosing code block using pre-computed symbol boundaries.
pub fn extract_block_cached(
    file: &Path,
    target_line: usize,
    filedata: &FileData,
) -> Result<BlockResult> {
    // Find the enclosing symbol: last symbol where sym.line <= target_line && target_line <= sym.block_end
    let enclosing = filedata
        .symbols
        .iter()
        .rev()
        .find(|s| (s.line as usize) <= target_line && target_line <= (s.block_end as usize));

    let (start, end) = match enclosing {
        Some(sym) => (sym.line as usize, sym.block_end as usize),
        None => {
            // No enclosing symbol -- fall back to uncached
            return extract_block(file, target_line);
        }
    };

    // Read only the relevant lines
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let lines: Vec<&str> = content.lines().collect();
    let end = end.min(lines.len());
    let start = start.max(1);

    let block_lines: Vec<(usize, String)> = (start..=end)
        .filter_map(|i| lines.get(i - 1).map(|l| (i, l.to_string())))
        .collect();

    Ok(BlockResult {
        file: file.to_string_lossy().to_string(),
        start,
        end,
        lines: block_lines,
    })
}

/// Find the enclosing brace-delimited block containing `target_idx`.
fn find_brace_block(lines: &[&str], target_idx: usize) -> Option<(usize, usize)> {
    // Walk up to find the block start (line with no leading whitespace that opens a brace block)
    let mut start_idx = target_idx;
    for i in (0..=target_idx).rev() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check if this is a definition line at column 0 (or nearly)
        let indent = lines[i].len() - lines[i].trim_start().len();
        if indent == 0 && is_definition_line(trimmed) {
            start_idx = i;
            break;
        }

        // For Python-style (no braces): look for def/class at lower indentation
        if indent == 0 && i < target_idx {
            start_idx = i;
            break;
        }
    }

    // Now find the end of this block by counting braces from start_idx
    let mut brace_depth = 0i32;
    let mut found_open = false;
    let mut end_idx = start_idx;

    for (i, line) in lines.iter().enumerate().skip(start_idx) {
        for ch in line.chars() {
            match ch {
                '{' => {
                    brace_depth += 1;
                    found_open = true;
                }
                '}' => {
                    brace_depth -= 1;
                    if found_open && brace_depth == 0 {
                        end_idx = i;
                        // Make sure the target line is within this block
                        if target_idx >= start_idx && target_idx <= end_idx {
                            return Some((start_idx, end_idx));
                        }
                        // Otherwise keep looking
                        found_open = false;
                    }
                }
                _ => {}
            }
        }
        end_idx = i;
    }

    // For brace-less languages (Python), use indentation
    if !found_open {
        return find_indent_block(lines, start_idx, target_idx);
    }

    // If we ran out of lines with braces still open, return what we have
    if target_idx >= start_idx && target_idx <= end_idx {
        Some((start_idx, end_idx))
    } else {
        None
    }
}

/// For indentation-based languages (Python), find the block by indentation level.
fn find_indent_block(
    lines: &[&str],
    start_idx: usize,
    target_idx: usize,
) -> Option<(usize, usize)> {
    let base_indent = lines[start_idx].len() - lines[start_idx].trim_start().len();
    let mut end_idx = start_idx;

    for (i, line) in lines.iter().enumerate().skip(start_idx + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            end_idx = i;
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent <= base_indent && !trimmed.is_empty() {
            break;
        }
        end_idx = i;
    }

    if target_idx >= start_idx && target_idx <= end_idx {
        Some((start_idx, end_idx))
    } else {
        None
    }
}

fn is_definition_line(trimmed: &str) -> bool {
    let keywords = [
        "fn ",
        "pub fn ",
        "pub(crate) fn ",
        "async fn ",
        "pub async fn ",
        "struct ",
        "pub struct ",
        "enum ",
        "pub enum ",
        "impl ",
        "trait ",
        "pub trait ",
        "mod ",
        "pub mod ",
        "function ",
        "class ",
        "interface ",
        "type ",
        "export function ",
        "export class ",
        "export interface ",
        "export type ",
        "export default ",
        "export const ",
        "def ",
        "async def ",
        "func ",
        "const ",
        "let ",
    ];
    keywords.iter().any(|kw| trimmed.starts_with(kw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_context_finds_rust_function() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "use std::io;").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "fn main() {{").unwrap(); // line 3
        writeln!(f, "    let x = 42;").unwrap(); // line 4
        writeln!(f, "    println!(\"hello\");").unwrap(); // line 5
        writeln!(f, "}}").unwrap(); // line 6
        writeln!(f, "").unwrap();
        writeln!(f, "fn other() {{").unwrap();
        writeln!(f, "    // other").unwrap();
        writeln!(f, "}}").unwrap();

        let block = extract_block(f.path(), 4).unwrap();
        assert!(
            block.start <= 3,
            "block should start at or before fn main (line 3), got {}",
            block.start
        );
        assert!(
            block.end >= 6,
            "block should end at or after closing brace (line 6), got {}",
            block.end
        );
        assert!(block.lines.iter().any(|(_, l)| l.contains("fn main")));
    }

    #[test]
    fn test_context_out_of_range() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();

        let result = extract_block(f.path(), 100);
        assert!(result.is_err(), "should error on out-of-range line");
    }

    #[test]
    fn test_context_single_line_file() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "fn solo() {{}}").unwrap();

        let block = extract_block(f.path(), 1).unwrap();
        assert_eq!(block.start, 1);
        assert!(!block.lines.is_empty());
    }

    #[test]
    fn test_context_line_zero_is_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "fn foo() {{}}").unwrap();

        let result = extract_block(f.path(), 0);
        assert!(result.is_err(), "line 0 should be out of range");
    }

    #[test]
    fn test_context_result_contains_target_line() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "fn outer() {{").unwrap(); // line 1
        writeln!(f, "    let a = 1;").unwrap(); // line 2
        writeln!(f, "    let b = 2;").unwrap(); // line 3
        writeln!(f, "}}").unwrap(); // line 4

        let block = extract_block(f.path(), 3).unwrap();
        let line_numbers: Vec<usize> = block.lines.iter().map(|(n, _)| *n).collect();
        assert!(
            line_numbers.contains(&3),
            "result should include target line 3"
        );
    }
}
