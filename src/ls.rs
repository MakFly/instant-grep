/// Compact directory listing — token-optimized output for AI agents.
/// Groups directories first, then files with sizes. Summary line at end.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};

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

    let entries = fs::read_dir(path)
        .with_context(|| format!("reading directory {}", path.display()))?;

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
            output.push_str(&format!("{}  {}\n", name, format_size(*size)));
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

    if !parts.is_empty() {
        output.push_str(&format!("\n{}\n", parts.join(", ")));
    }

    output
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
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
    fn test_format_size() {
        assert_eq!(format_size(500), "500B");
        assert_eq!(format_size(1024), "1.0K");
        assert_eq!(format_size(1536), "1.5K");
        assert_eq!(format_size(1_048_576), "1.0M");
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
        let result = LsResult {
            dirs: vec!["src".into(), "docs".into(), "tests".into()],
            files: vec![
                ("README.md".into(), 1536),
                ("Cargo.toml".into(), 512),
            ],
            total_dirs: 3,
            total_files: 2,
        };
        let output = format_ls(&result);
        assert!(output.contains("src/"));
        assert!(output.contains("docs/"));
        assert!(output.contains("README.md  1.5K"));
        assert!(output.contains("2 files, 3 dirs"));
    }
}
