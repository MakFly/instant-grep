use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::smart;
use crate::util::ig_dir;
use crate::walk;

/// Max lines for tree section in context.md
const MAX_TREE_LINES: usize = 150;

/// Extensions to exclude from smart summaries (noise for AI agents)
const EXCLUDED_EXTENSIONS: &[&str] = &[
    "json", "lock", "css", "scss", "svg", "png", "jpg", "jpeg", "gif", "ico",
    "woff", "woff2", "ttf", "eot", "map", "min.js",
    "md", "mdx", "txt", "yml", "yaml", "toml", "xml", "sh",
    "mjs", "cjs",
];

/// Filename patterns to exclude (boilerplate/config)
const EXCLUDED_NAMES: &[&str] = &[
    "layout.tsx", "layout.ts", "layout.jsx", "layout.js",
    "error.tsx", "error.ts",
    "loading.tsx", "loading.ts",
    "not-found.tsx", "not-found.ts",
    "global-error.tsx",
    "middleware.ts", "middleware.js",
    ".eslintrc", ".prettierrc",
    "tailwind.config.ts", "tailwind.config.js",
    "postcss.config.mjs", "postcss.config.js",
    "next.config.ts", "next.config.js", "next.config.mjs",
    "tsconfig.json", "tsconfig.tsbuildinfo",
    "Dockerfile", "docker-compose.yml",
    "components.json",
    "global-error.tsx",
];

/// Directory patterns to exclude entirely
const EXCLUDED_DIRS: &[&str] = &[
    "__tests__", "tests", "test", "e2e", "__mocks__",
    ".storybook", "stories",
    "public", "static", "assets",
    "_components", "components",
    "scripts", "notes", "docs",
];

/// Generate `.ig/context.md` — a compact project context file for AI agents.
pub fn generate_context(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
) -> Result<String> {
    let ig = ig_dir(root);
    fs::create_dir_all(&ig)?;

    let output = build_context_string(&ig, root, use_default_excludes, max_file_size)?;

    let context_path = ig.join("context.md");
    fs::write(&context_path, &output)?;

    Ok(output)
}

/// Quick context generation during index build — best-effort, no error propagation.
pub fn generate_context_quiet(root: &Path, ig: &Path) {
    let output = match build_context_string(ig, root, true, 0) {
        Ok(o) => o,
        Err(_) => return,
    };
    let _ = fs::write(ig.join("context.md"), output.as_bytes());
}

fn build_context_string(
    ig: &Path,
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
) -> Result<String> {
    let mut output = String::with_capacity(16_384);
    output.push_str("# Project Context\n\n");

    // Section 1: Tree (depth 2 only — keep compact)
    let tree_path = ig.join("tree.txt");
    if let Ok(tree) = fs::read_to_string(&tree_path) {
        output.push_str("## Structure\n\n```\n");
        let mut count = 0;
        for line in tree.lines() {
            // Depth 2 filter: count '/' separators in the path
            let depth = line.matches('/').count();
            // Include depth 0 (root files), depth 1 (first level dirs/files),
            // and directories at depth 2 (ending with /)
            if depth <= 1 || (depth == 2 && line.ends_with('/')) {
                output.push_str(line);
                output.push('\n');
                count += 1;
                if count >= MAX_TREE_LINES {
                    output.push_str("... (truncated)\n");
                    break;
                }
            }
        }
        output.push_str("```\n\n");
    }

    // Section 2: Smart summaries (filtered + grouped)
    let files = walk::walk_files(root, use_default_excludes, max_file_size, None, None)?;

    // Filter files
    let filtered: Vec<_> = files
        .iter()
        .filter(|path| {
            let rel = path.strip_prefix(root).unwrap_or(path);
            let rel_str = rel.to_string_lossy();

            // Exclude by directory
            for dir in EXCLUDED_DIRS {
                if rel_str.starts_with(dir) || rel_str.contains(&format!("/{}/", dir)) {
                    return false;
                }
            }

            // Exclude by filename
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if EXCLUDED_NAMES.contains(&name) {
                    return false;
                }
            }

            // Exclude by extension
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if EXCLUDED_EXTENSIONS.contains(&ext) {
                    return false;
                }
                // Exclude .d.ts and .test.* and .spec.*
                if rel_str.ends_with(".d.ts")
                    || rel_str.contains(".test.")
                    || rel_str.contains(".spec.")
                {
                    return false;
                }
            }

            true
        })
        .collect();

    // Separate page.tsx files (group by directory) from regular files
    let mut page_dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut regular_files: Vec<&std::path::PathBuf> = Vec::new();

    for path in &filtered {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if name == "page.tsx" || name == "page.ts" || name == "page.jsx" || name == "page.js" {
            // Group by parent directory (2 levels up from page.tsx for route groups)
            let parent = rel.parent().unwrap_or(rel);
            let grandparent = parent.parent().unwrap_or(parent);
            let group_key = grandparent.to_string_lossy().to_string();
            let route_name = parent
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            page_dirs.entry(group_key).or_default().push(route_name);
        } else {
            regular_files.push(path);
        }
    }

    output.push_str("## Files\n\n");

    // Write grouped routes first
    if !page_dirs.is_empty() {
        output.push_str("### Routes\n");
        for (group, routes) in &page_dirs {
            let routes_str = routes.join(", ");
            output.push_str(&format!("- `{}/` — {}\n", group, routes_str));
        }
        output.push('\n');
    }

    // Group regular files by top-level directory for compactness
    let mut by_dir: BTreeMap<String, Vec<smart::SmartSummary>> = BTreeMap::new();
    let mut root_files: Vec<smart::SmartSummary> = Vec::new();

    for path in &regular_files {
        if let Ok(s) = smart::smart_summarize_file(path, root) {
            let first_dir = s.file.split('/').next().unwrap_or("");
            if s.file.contains('/') {
                by_dir.entry(first_dir.to_string()).or_default().push(s);
            } else {
                root_files.push(s);
            }
        }
    }

    // Root files individually (few)
    if !root_files.is_empty() {
        output.push_str("### Root\n");
        for s in &root_files {
            if s.public_api.is_empty() {
                output.push_str(&format!("- `{}` — {}\n", s.file, s.role));
            } else {
                output.push_str(&format!("- `{}` — {} / {}\n", s.file, s.role, s.public_api));
            }
        }
        output.push('\n');
    }

    // Grouped directories: if > 10 files, summarize as group; otherwise list individually
    output.push_str("### Modules\n");
    for (dir, files) in &by_dir {
        if files.len() > 10 {
            // Group summary: dir name + count + key exports
            let exports: Vec<&str> = files
                .iter()
                .filter(|s| !s.public_api.is_empty())
                .flat_map(|s| s.public_api.split(", "))
                .take(8)
                .collect();
            let exports_str = if exports.is_empty() {
                String::new()
            } else {
                format!(" / {}", exports.join(", "))
            };
            output.push_str(&format!(
                "- `{}/` — {} files{}\n",
                dir,
                files.len(),
                exports_str,
            ));
        } else {
            // List individually
            for s in files {
                if s.public_api.is_empty() {
                    output.push_str(&format!("- `{}` — {}\n", s.file, s.role));
                } else {
                    output.push_str(&format!("- `{}` — {} / {}\n", s.file, s.role, s.public_api));
                }
            }
        }
    }
    output.push('\n');

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_test_project() -> TempDir {
        let dir = TempDir::new().unwrap();
        let ig_dir = dir.path().join(".ig");
        fs::create_dir_all(&ig_dir).unwrap();

        fs::write(
            ig_dir.join("tree.txt"),
            "src/\nsrc/main.rs\nsrc/lib.rs\n",
        )
        .unwrap();

        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();

        let mut main_rs = fs::File::create(src.join("main.rs")).unwrap();
        writeln!(main_rs, "/// Application entry point").unwrap();
        writeln!(main_rs, "pub fn main() {{}}").unwrap();

        let mut lib_rs = fs::File::create(src.join("lib.rs")).unwrap();
        writeln!(lib_rs, "/// Core library").unwrap();
        writeln!(lib_rs, "pub fn greet(name: &str) -> String {{").unwrap();
        writeln!(lib_rs, "    format!(\"hello {{}}\", name)").unwrap();
        writeln!(lib_rs, "}}").unwrap();

        dir
    }

    #[test]
    fn test_generate_context() {
        let dir = setup_test_project();
        let output = generate_context(dir.path(), true, 1_048_576).unwrap();

        assert!(output.contains("# Project Context"), "missing header");
        assert!(output.contains("## Structure"), "missing structure section");
        assert!(output.contains("src/main.rs"), "missing file in tree");
        assert!(output.contains("## Files"), "missing files section");

        let context_path = dir.path().join(".ig/context.md");
        assert!(context_path.exists(), "context.md not written");
    }

    #[test]
    fn test_generate_context_quiet() {
        let dir = setup_test_project();
        generate_context_quiet(dir.path(), &dir.path().join(".ig"));

        let context_path = dir.path().join(".ig/context.md");
        assert!(context_path.exists());
        let content = fs::read_to_string(&context_path).unwrap();
        assert!(content.contains("# Project Context"));
    }

    #[test]
    fn test_excluded_extensions() {
        assert!(EXCLUDED_EXTENSIONS.contains(&"json"));
        assert!(EXCLUDED_EXTENSIONS.contains(&"css"));
        assert!(!EXCLUDED_EXTENSIONS.contains(&"ts"));
        assert!(!EXCLUDED_EXTENSIONS.contains(&"rs"));
    }
}
