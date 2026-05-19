//! `ig summary <file>` — word count + first 5 / last 3 lines. For markdown,
//! also surfaces the heading outline (`#` / `##`).

use std::path::Path;

use anyhow::{Context, Result};

use crate::RunOptions;

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig summary <file>");
    }
    let path = Path::new(&args[0]);
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let out = summarise(&text, path);
    print!("{}", out);
    Ok(0)
}

pub fn summarise(text: &str, path: &Path) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let line_count = lines.len();
    let word_count = text.split_whitespace().count();
    let byte_count = text.len();
    let mut out = format!(
        "{} — {} lines, {} words, {} bytes\n",
        path.display(),
        line_count,
        word_count,
        byte_count
    );

    if is_markdown(path) {
        let outline = outline(&lines);
        if !outline.is_empty() {
            out.push_str("\noutline:\n");
            out.push_str(&outline);
        }
    }

    let head_n = 5.min(lines.len());
    out.push_str("\nfirst lines:\n");
    for l in lines.iter().take(head_n) {
        out.push_str(l);
        out.push('\n');
    }

    if lines.len() > head_n + 3 {
        out.push_str("\nlast lines:\n");
        for l in lines.iter().skip(lines.len().saturating_sub(3)) {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

fn is_markdown(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("md") | Some("markdown")
    )
}

fn outline(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        let t = line.trim_start();
        if (t.starts_with("# ") || t.starts_with("## ")) && !out.contains(line) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn counts_words_and_lines() {
        let s = "alpha beta\nthird line\n";
        let out = summarise(s, &PathBuf::from("x.txt"));
        assert!(out.contains("2 lines"));
        assert!(out.contains("4 words"));
    }

    #[test]
    fn extracts_markdown_outline() {
        let s = "# Title\n\nbody\n\n## Section\n\nmore\n";
        let out = summarise(s, &PathBuf::from("a.md"));
        assert!(out.contains("outline"));
        assert!(out.contains("# Title"));
        assert!(out.contains("## Section"));
    }

    #[test]
    fn surfaces_first_and_last_lines() {
        let mut s = String::new();
        for i in 0..30 {
            s.push_str(&format!("line {}\n", i));
        }
        let out = summarise(&s, &PathBuf::from("x.txt"));
        assert!(out.contains("line 0"));
        assert!(out.contains("line 29"));
    }
}
