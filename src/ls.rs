//! Compact directory listing — token-optimized output for AI agents.
//! Groups directories first, then files with sizes. Summary line at end.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};

use crate::util::format_bytes;

pub struct LsResult {
    pub dirs: Vec<String>,
    pub files: Vec<(String, u64)>,
    pub total_dirs: usize,
    pub total_files: usize,
}

pub fn compact_ls(path: &Path) -> Result<LsResult> {
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };

    let entries =
        fs::read_dir(path).with_context(|| format!("reading directory {}", path.display()))?;

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files
        if name.starts_with('.') {
            continue;
        }

        let meta = entry.metadata()?;
        if meta.is_dir() {
            dirs.push(name);
        } else {
            files.push((name, meta.len()));
        }
    }

    dirs.sort();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let total_dirs = dirs.len();
    let total_files = files.len();

    Ok(LsResult {
        dirs,
        files,
        total_dirs,
        total_files,
    })
}

pub fn format_ls(result: &LsResult) -> String {
    let mut output = String::new();

    // Directories on grouped lines (4 per line)
    if !result.dirs.is_empty() {
        let dir_names: Vec<String> = result.dirs.iter().map(|d| format!("{}/", d)).collect();
        for chunk in dir_names.chunks(4) {
            output.push_str(&chunk.join("  "));
            output.push('\n');
        }
    }

    // Files with sizes
    if !result.files.is_empty() {
        if !result.dirs.is_empty() {
            // No extra separator — keep compact
        }
        for (name, size) in &result.files {
            output.push_str(&format!("{}  {}\n", name, format_bytes(*size)));
        }
    }

    // Summary
    let parts: Vec<String> = [
        if result.total_files > 0 {
            Some(format!("{} files", result.total_files))
        } else {
            None
        },
        if result.total_dirs > 0 {
            Some(format!("{} dirs", result.total_dirs))
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
    .collect();

    // Skip footer when listing is tiny: a 4-entry dir doesn't need a summary.
    let total_entries = result.total_dirs + result.total_files;
    if !parts.is_empty() && total_entries > 8 {
        output.push_str(&format!("\n{}\n", parts.join(", ")));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_compact_ls() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::create_dir(dir.path().join("docs")).unwrap();
        let mut f = fs::File::create(dir.path().join("README.md")).unwrap();
        writeln!(f, "# Hello").unwrap();
        fs::File::create(dir.path().join("Cargo.toml")).unwrap();

        let result = compact_ls(dir.path()).unwrap();
        assert_eq!(result.total_dirs, 2);
        assert_eq!(result.total_files, 2);
        assert!(result.dirs.contains(&"src".to_string()));
        assert!(result.dirs.contains(&"docs".to_string()));
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(1024), "1.0K");
        assert_eq!(format_bytes(1536), "1.5K");
        assert_eq!(format_bytes(1_048_576), "1.0M");
    }

    #[test]
    fn test_compact_ls_hides_dotfiles() {
        let dir = TempDir::new().unwrap();
        fs::File::create(dir.path().join(".hidden")).unwrap();
        fs::File::create(dir.path().join("visible")).unwrap();

        let result = compact_ls(dir.path()).unwrap();
        assert_eq!(result.total_files, 1);
        assert_eq!(result.files[0].0, "visible");
    }

    #[test]
    fn test_format_ls_output() {
        // Footer only appears when total entries > 8.
        let result = LsResult {
            dirs: (0..6).map(|i| format!("d{}", i)).collect(),
            files: (0..6).map(|i| (format!("f{}.md", i), 1024)).collect(),
            total_dirs: 6,
            total_files: 6,
        };
        let output = format_ls(&result);
        assert!(output.contains("d0/"));
        assert!(output.contains("f0.md  1.0K"));
        assert!(output.contains("6 files, 6 dirs"));
    }
}
