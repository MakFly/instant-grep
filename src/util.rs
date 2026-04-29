use std::path::{Path, PathBuf};

const BINARY_CHECK_LEN: usize = 8192;

/// Check if file content looks like binary (contains null bytes in first 8KB).
pub fn is_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(BINARY_CHECK_LEN);
    data[..check_len].contains(&0)
}

/// Files whose presence at the top of a directory marks it as a project root,
/// in addition to `.git/`. Lets `find_root` resolve correctly inside
/// non-versioned projects (e.g. a Next.js scratch dir without a git history).
const PROJECT_MARKERS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "deno.json",
    "deno.jsonc",
    "composer.json",
    "pnpm-workspace.yaml",
    "bun.lock",
    "Gemfile",
    "build.gradle",
    "build.gradle.kts",
    "pom.xml",
    "mix.exs",
    "Pipfile",
    "requirements.txt",
];

fn has_git(p: &Path) -> bool {
    p.join(".git").exists()
}

fn has_marker(p: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|m| p.join(m).exists())
}

/// Find the project root by walking up.
///
/// Resolution order (highest match wins on each pass):
///   1. Highest ancestor with BOTH `.git/` and a project marker.
///   2. Highest ancestor with `.git/`.
///   3. Highest ancestor with a project marker (handles non-versioned projects
///      that previously created stray `.ig/` in subdirs).
///   4. Fallback: the start path itself.
///
/// Walking stops *before* `$HOME` — we never want to root searches at the
/// home directory (a stray `~/package.json` should not anchor the index).
pub fn find_root(start: &Path) -> PathBuf {
    let start = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };

    let abs_start = if start.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(start))
            .unwrap_or_else(|_| start.to_path_buf())
    } else {
        start.to_path_buf()
    };

    let home = dirs::home_dir();

    let mut chain: Vec<PathBuf> = Vec::new();
    let mut current = abs_start.clone();
    loop {
        if let Some(ref h) = home
            && &current == h
        {
            break;
        }
        chain.push(current.clone());
        if !current.pop() {
            break;
        }
    }

    // chain[0] is the deepest, chain[last] is the shallowest (closest to /).
    // We want the *highest* (shallowest) match, so we keep overwriting as we
    // walk up.
    let mut highest_combined: Option<PathBuf> = None;
    let mut highest_git: Option<PathBuf> = None;
    let mut highest_marker: Option<PathBuf> = None;
    for p in &chain {
        let g = has_git(p);
        let m = has_marker(p);
        if g && m {
            highest_combined = Some(p.clone());
        }
        if g {
            highest_git = Some(p.clone());
        }
        if m {
            highest_marker = Some(p.clone());
        }
    }

    highest_combined
        .or(highest_git)
        .or(highest_marker)
        .unwrap_or(abs_start)
}

/// Get the index directory for a given project root.
///
/// Resolution order:
///   1. `<root>/.ig/` if it already exists (legacy / opt-in).
///   2. `<root>/.ig/` if `IG_LOCAL_INDEX=1`.
///   3. XDG cache: `~/.cache/ig/<hash>/` (default — keeps projects clean).
pub fn ig_dir(root: &Path) -> PathBuf {
    let local = root.join(".ig");
    if local.exists() {
        return local;
    }
    if std::env::var("IG_LOCAL_INDEX").as_deref() == Ok("1") {
        return local;
    }
    crate::cache::cache_index_dir(root)
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
    use std::fs;
    use tempfile::tempdir;

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

    fn canon(p: &Path) -> PathBuf {
        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
    }

    #[test]
    fn find_root_walks_up_to_git() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let nested = root.join("src/sub");
        fs::create_dir_all(&nested).unwrap();

        let found = find_root(&nested);
        assert_eq!(canon(&found), canon(root));
    }

    #[test]
    fn find_root_uses_project_marker_when_no_git() {
        // Reproduces the ux-ui-unified bug: a Next.js project with no .git/
        // used to anchor `.ig/` in subdirs.
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("package.json"), "{}").unwrap();
        let app = root.join("app");
        fs::create_dir(&app).unwrap();
        fs::write(app.join("page.tsx"), "").unwrap();

        let found_from_app = find_root(&app);
        let found_from_root = find_root(root);
        assert_eq!(canon(&found_from_app), canon(&found_from_root));
        assert_eq!(canon(&found_from_app), canon(root));
    }

    #[test]
    fn find_root_prefers_outer_git_over_inner_node_modules_git() {
        // Reproduces the node_modules/<pkg>/.git/ trap.
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join("package.json"), "{}").unwrap();

        let inner = root.join("node_modules/foo");
        fs::create_dir_all(&inner).unwrap();
        fs::create_dir(inner.join(".git")).unwrap();
        fs::write(inner.join("package.json"), "{}").unwrap();
        let deep = inner.join("src");
        fs::create_dir_all(&deep).unwrap();

        let found = find_root(&deep);
        assert_eq!(canon(&found), canon(root));
    }

    #[test]
    fn find_root_falls_back_to_start_when_nothing_found() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("orphan");
        fs::create_dir(&dir).unwrap();
        let found = find_root(&dir);
        // No marker, no .git -> falls back to the start path (possibly canonicalized).
        assert_eq!(canon(&found), canon(&dir));
    }
}
