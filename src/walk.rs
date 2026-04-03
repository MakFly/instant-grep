use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

/// Directories excluded by default (in addition to .gitignore).
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    ".git",
    ".hg",
    ".svn",
    ".ig",
    "coverage",
    ".cache",
    ".turbo",
    ".output",
    ".vercel",
    "tmp",
    ".temp",
    ".gradle",
    ".idea",
    ".vscode",
    ".terraform",
    ".pants.d",
    "bazel-out",
    ".mypy_cache",
    ".ruff_cache",
    ".pytest_cache",
    ".tox",
    "bower_components",
    ".dart_tool",
    ".pub-cache",
    ".cargo",
    "Pods",
];

/// Default max file size: 1 MB.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 1_048_576;

/// Walk a directory tree, respecting .gitignore and default exclusions.
/// Returns a sorted list of file paths.
pub fn walk_files(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true);

    // Add type filter if specified
    if let Some(file_type) = type_filter {
        let mut types_builder = ignore::types::TypesBuilder::new();
        types_builder.add_defaults();
        // Map short aliases to the names the ignore crate recognizes
        let canonical = match file_type {
            "rs" => "rust",
            "ts" => "ts",
            "js" => "js",
            "py" => "python",
            "rb" => "ruby",
            "yml" => "yaml",
            "md" => "markdown",
            "sh" => "sh",
            other => other,
        };
        types_builder.select(canonical);
        if let Ok(types) = types_builder.build() {
            builder.types(types);
        }
    }

    // Add glob filter if specified
    if let Some(glob) = glob_filter {
        let mut overrides = ignore::overrides::OverrideBuilder::new(root);
        let _ = overrides.add(glob);
        if let Ok(ov) = overrides.build() {
            builder.overrides(ov);
        }
    }

    let exclude_set: std::collections::HashSet<&str> = if use_default_excludes {
        DEFAULT_EXCLUDES.iter().copied().collect()
    } else {
        // Still always exclude .ig
        [".ig"].iter().copied().collect()
    };

    builder.filter_entry(move |entry| {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && let Some(name) = entry.path().file_name().and_then(|n| n.to_str())
            && exclude_set.contains(name)
        {
            return false;
        }
        true
    });

    let mut paths = Vec::new();

    for entry in builder.build() {
        let entry = entry?;
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        // Skip files larger than max_file_size
        if max_file_size > 0
            && let Ok(meta) = entry.metadata()
            && meta.len() > max_file_size
        {
            continue;
        }
        paths.push(entry.into_path());
    }

    paths.sort();
    Ok(paths)
}
