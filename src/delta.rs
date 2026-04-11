//! Delta-aware file reading — show only git-changed lines with context.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Result, bail};

use crate::read::ReadResult;
use crate::symbols::Lang;

/// Read only the git-changed portions of a file with surrounding context.
pub fn read_delta(file: &Path) -> Result<ReadResult> {
    let file_str = file.to_string_lossy();

    // Get git diff for this file (unstaged changes)
    let output = Command::new("git")
        .args(["diff", "--unified=0", "--no-color", "--", &file_str])
        .output()?;

    if !output.status.success() {
        bail!("git diff failed for {}", file_str);
    }

    let diff_output = String::from_utf8_lossy(&output.stdout);
    if diff_output.trim().is_empty() {
        // Also check staged changes
        let staged = Command::new("git")
            .args([
                "diff", "--cached", "--unified=0", "--no-color", "--", &file_str,
            ])
            .output()?;
        let staged_output = String::from_utf8_lossy(&staged.stdout);
        if staged_output.trim().is_empty() {
            bail!("no git changes for {}", file_str);
        }
        return parse_delta(file, &staged_output);
    }

    parse_delta(file, &diff_output)
}

fn parse_delta(file: &Path, diff: &str) -> Result<ReadResult> {
    // Parse @@ -a,b +c,d @@ lines to get changed line ranges
    let mut changed_lines: Vec<usize> = Vec::new();

    for line in diff.lines() {
        if line.starts_with("@@") {
            // Parse +c,d from @@ -a,b +c,d @@
            if let Some(plus_part) = line.split('+').nth(1) {
                let nums: &str = plus_part.split_whitespace().next().unwrap_or("");
                let parts: Vec<&str> = nums.split(',').collect();
                let start: usize = parts[0].parse().unwrap_or(1).max(1);
                let count: usize = if parts.len() > 1 {
                    parts[1].parse().unwrap_or(1)
                } else {
                    1
                };
                for l in start..start + count {
                    changed_lines.push(l);
                }
            }
        }
    }

    if changed_lines.is_empty() {
        bail!("no changed lines found");
    }

    // Read the full file
    let content = std::fs::read_to_string(file)?;
    let all_lines: Vec<&str> = content.lines().collect();
    let total = all_lines.len();

    // Determine which lines to include: changed + 2 lines context + enclosing signatures
    let mut include: Vec<bool> = vec![false; total + 1]; // 1-indexed

    let context_radius = 2;
    for &line_num in &changed_lines {
        let start = line_num.saturating_sub(context_radius).max(1);
        let end = (line_num + context_radius).min(total);
        for l in start..=end {
            include[l] = true;
        }
    }

    // Find enclosing function/class signatures for each changed region
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Lang::from_ext(ext);
    let sym_regex = if !lang.patterns().is_empty() {
        regex::Regex::new(lang.patterns()).ok()
    } else {
        None
    };

    for &line_num in &changed_lines {
        // Look backward for nearest signature
        for i in (0..line_num.min(total)).rev() {
            let line = all_lines[i];
            if let Some(ref re) = sym_regex {
                if re.is_match(line) {
                    include[i + 1] = true; // 1-indexed
                    break;
                }
            }
        }
    }

    // Build output with gap markers
    let changed_set: HashSet<usize> = changed_lines.iter().copied().collect();
    let mut result_lines: Vec<(usize, String)> = Vec::new();
    let mut last_included: Option<usize> = None;

    for i in 1..=total {
        if include[i] {
            if let Some(last) = last_included {
                if i > last + 1 {
                    // Gap — insert marker
                    let skipped = i - last - 1;
                    result_lines.push((0, format!("    // ... [{} lines]", skipped)));
                }
            }

            // Mark changed lines with a prefix
            let line_text = all_lines[i - 1];
            let marker = if changed_set.contains(&i) {
                ">"
            } else {
                " "
            };
            result_lines.push((i, format!("{} {}", marker, line_text)));
            last_included = Some(i);
        }
    }

    // Add header
    let mut final_lines = vec![(
        0,
        format!(
            "# delta: {} changed lines in {}",
            changed_lines.len(),
            file.display()
        ),
    )];
    final_lines.extend(result_lines);

    Ok(ReadResult {
        file: file.to_string_lossy().to_string(),
        lines: final_lines,
    })
}

#[cfg(test)]
mod tests {
    // Delta tests require git state, so integration tests are more appropriate
}
