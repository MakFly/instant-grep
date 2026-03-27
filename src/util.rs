use std::path::{Path, PathBuf};

const BINARY_CHECK_LEN: usize = 8192;

/// Check if file content looks like binary (contains null bytes in first 8KB).
pub fn is_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(BINARY_CHECK_LEN);
    data[..check_len].contains(&0)
}

/// Find the project root by walking up until we find .git/ or use the given path.
pub fn find_root(start: &Path) -> PathBuf {
    let start = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };

    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return current;
        }
        if !current.pop() {
            // No .git found, use the original start path
            return start.to_path_buf();
        }
    }
}

/// Get the .ig index directory path for a given root.
pub fn ig_dir(root: &Path) -> PathBuf {
    root.join(".ig")
}

/// Detect if colored output should be used.
pub fn use_color(json: bool) -> bool {
    use std::io::IsTerminal;
    !json && std::io::stdout().is_terminal() && std::env::var("NO_COLOR").is_err()
}

/// Format a byte count as a human-readable string (B / K / M).
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Returns true for lines that are preamble/boilerplate (imports, directives, etc.)
/// and should be skipped when looking for a file's semantic role.
/// Union of smart.rs and read.rs detection logic.
pub fn is_preamble_line(trimmed: &str) -> bool {
    trimmed.starts_with("use ")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("require(")
        || (trimmed.starts_with("const ") && trimmed.contains("require("))
        || trimmed.starts_with("#include")
        || trimmed.starts_with("package ")
        || trimmed.starts_with("module ")
        || trimmed.starts_with("#!")
        || trimmed.starts_with("#[")
        || trimmed.starts_with("mod ")
        || trimmed.starts_with("extern ")
        // React/Next.js directives — not useful as role description
        || trimmed == "\"use client\""
        || trimmed == "\"use client\";"
        || trimmed == "'use client'"
        || trimmed == "'use client';"
        || trimmed == "\"use server\""
        || trimmed == "\"use server\";"
        || trimmed == "'use server'"
        || trimmed == "'use server';"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(5120), "5.0K");
        assert_eq!(format_bytes(1_048_576), "1.0M");
    }

    #[test]
    fn test_is_preamble_line() {
        assert!(is_preamble_line("use std::io;"));
        assert!(is_preamble_line("import { foo } from 'bar';"));
        assert!(is_preamble_line("from os import path"));
        assert!(is_preamble_line("require(\"module\")"));
        assert!(is_preamble_line("const x = require(\"mod\")"));
        assert!(is_preamble_line("#include <stdio.h>"));
        assert!(is_preamble_line("\"use client\";"));
        assert!(is_preamble_line("'use server'"));
        assert!(!is_preamble_line("pub fn main() {"));
        assert!(!is_preamble_line("struct Config {"));
    }
}
