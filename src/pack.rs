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

    // Section 2: Dependencies
    if let Some(deps_section) = build_dependencies_section(root) {
        output.push_str(&deps_section);
    }

    // Section 3: API Routes (Next.js app/api/)
    if let Some(routes_section) = build_api_routes_section(root) {
        output.push_str(&routes_section);
    }

    // Section 4: Environment variables
    if let Some(env_section) = build_env_section(root) {
        output.push_str(&env_section);
    }

    // Section 5: Smart summaries (filtered + grouped)
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

// ---------------------------------------------------------------------------
// Metadata sections
// ---------------------------------------------------------------------------

/// Parse `"dependencies"` and `"devDependencies"` blocks from package.json text.
/// Returns a flat list of package names (no versions).
fn parse_package_json_deps(content: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();

    for block_key in &["\"dependencies\"", "\"devDependencies\""] {
        let Some(start) = content.find(block_key) else { continue };
        // Find the opening '{' after the key
        let Some(brace_start) = content[start..].find('{') else { continue };
        let block_offset = start + brace_start + 1;

        // Walk character-by-character to find the matching closing '}'
        let mut depth = 1usize;
        let mut end = block_offset;
        for (i, ch) in content[block_offset..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = block_offset + i;
                        break;
                    }
                }
                _ => {}
            }
        }

        // Extract package names from either pretty-printed or compact JSON.
        // Split on commas to handle both `"pkg": "ver"` per-line and inline objects.
        for chunk in content[block_offset..end].split(',') {
            let chunk = chunk.trim();
            if let Some(colon_pos) = chunk.find(':') {
                let key_part = chunk[..colon_pos].trim().trim_matches('"');
                if !key_part.is_empty() && !key_part.starts_with('/') {
                    names.push(key_part.to_string());
                }
            }
        }
    }

    names
}

/// Parse `[dependencies]` section from Cargo.toml text.
fn parse_cargo_toml_deps(content: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" {
            in_deps = true;
            continue;
        }
        // Stop at any other section header
        if trimmed.starts_with('[') {
            in_deps = false;
            continue;
        }
        if in_deps && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some(eq_pos) = trimmed.find('=') {
                let name = trimmed[..eq_pos].trim().to_string();
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
    }

    names
}

/// Parse `"require"` block from composer.json text.
fn parse_composer_json_deps(content: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();

    let Some(start) = content.find("\"require\"") else { return names };
    let Some(brace_start) = content[start..].find('{') else { return names };
    let block_offset = start + brace_start + 1;

    let mut depth = 1usize;
    let mut end = block_offset;
    for (i, ch) in content[block_offset..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = block_offset + i;
                    break;
                }
            }
            _ => {}
        }
    }

    for chunk in content[block_offset..end].split(',') {
        let chunk = chunk.trim();
        if let Some(colon_pos) = chunk.find(':') {
            let key = chunk[..colon_pos].trim().trim_matches('"');
            // Skip PHP version constraint ("php": ">=8.1")
            if !key.is_empty() && key != "php" && !key.starts_with('/') {
                names.push(key.to_string());
            }
        }
    }

    names
}

/// Build the `## Dependencies` section string, or `None` if no manifest found.
fn build_dependencies_section(root: &Path) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();

    // package.json
    let pkg_path = root.join("package.json");
    if pkg_path.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_path) {
            let deps = parse_package_json_deps(&content);
            if !deps.is_empty() {
                let capped: Vec<&str> = deps.iter().map(|s| s.as_str()).take(20).collect();
                lines.push(format!("**package.json**: {}", capped.join(", ")));
            }
        }
    }

    // Cargo.toml
    let cargo_path = root.join("Cargo.toml");
    if cargo_path.exists() {
        if let Ok(content) = fs::read_to_string(&cargo_path) {
            let deps = parse_cargo_toml_deps(&content);
            if !deps.is_empty() {
                let capped: Vec<&str> = deps.iter().map(|s| s.as_str()).take(20).collect();
                lines.push(format!("**Cargo.toml**: {}", capped.join(", ")));
            }
        }
    }

    // composer.json
    let composer_path = root.join("composer.json");
    if composer_path.exists() {
        if let Ok(content) = fs::read_to_string(&composer_path) {
            let deps = parse_composer_json_deps(&content);
            if !deps.is_empty() {
                let capped: Vec<&str> = deps.iter().map(|s| s.as_str()).take(20).collect();
                lines.push(format!("**composer.json**: {}", capped.join(", ")));
            }
        }
    }

    if lines.is_empty() {
        return None;
    }

    let mut out = String::from("## Dependencies\n\n");
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    Some(out)
}

/// Build the `## API Routes` section for Next.js `app/api/` directories.
/// Returns `None` if `root/app/api/` does not exist.
fn build_api_routes_section(root: &Path) -> Option<String> {
    let api_root = root.join("app").join("api");
    if !api_root.is_dir() {
        return None;
    }

    // Walk app/api/ looking for route.ts / route.js files
    let mut routes: Vec<String> = Vec::new();
    collect_api_routes(&api_root, &api_root, &mut routes);

    if routes.is_empty() {
        return None;
    }

    routes.sort();

    // Group by first segment after /api/
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for route in &routes {
        // route looks like "/api/auth/session"
        let parts: Vec<&str> = route.trim_start_matches('/').splitn(3, '/').collect();
        // parts[0] = "api", parts[1] = segment, parts[2..] = rest
        let segment = if parts.len() >= 2 { parts[1] } else { "" };
        let leaf = if parts.len() >= 3 {
            parts[2].to_string()
        } else {
            String::new()
        };
        groups.entry(segment.to_string()).or_default().push(leaf);
    }

    let mut out = String::from("## API Routes\n\n");
    for (segment, leaves) in &groups {
        let leaves_clean: Vec<&str> = leaves.iter().map(|s| s.as_str()).collect();
        out.push_str(&format!("- `/api/{}/` — {}\n", segment, leaves_clean.join(", ")));
    }
    out.push('\n');
    Some(out)
}

/// Recursively walk `dir`, collecting route paths relative to `api_root`.
fn collect_api_routes(api_root: &Path, dir: &Path, routes: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_api_routes(api_root, &path, routes);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == "route.ts" || name == "route.js" {
                // Build route path: strip api_root prefix, remove filename
                if let Some(parent) = path.parent() {
                    let rel = parent.strip_prefix(api_root).unwrap_or(parent);
                    let route = format!("/api/{}", rel.to_string_lossy());
                    routes.push(route);
                }
            }
        }
    }
}

/// Build the `## Environment` section from `.env.example` or `.env.local.example`.
/// Returns `None` if neither file exists.
fn build_env_section(root: &Path) -> Option<String> {
    let candidates = [".env.example", ".env.local.example"];
    let content = candidates
        .iter()
        .find_map(|name| fs::read_to_string(root.join(name)).ok())?;

    let mut vars: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // Match VAR_NAME= or VAR_NAME=value (uppercase + underscore + digits)
        let is_var = trimmed
            .split('=')
            .next()
            .map(|k| {
                !k.is_empty()
                    && k.chars()
                        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            })
            .unwrap_or(false);
        if is_var {
            if let Some(name) = trimmed.split('=').next() {
                vars.push(name.to_string());
            }
        }
    }

    if vars.is_empty() {
        return None;
    }

    // Group by prefix (segment before first underscore that is a common prefix)
    let mut prefix_counts: BTreeMap<String, usize> = BTreeMap::new();
    for var in &vars {
        let prefix = extract_env_prefix(var);
        *prefix_counts.entry(prefix).or_default() += 1;
    }

    // Build "Groups" line
    let groups_str = prefix_counts
        .iter()
        .map(|(p, c)| format!("{} ({})", p, c))
        .collect::<Vec<_>>()
        .join(", ");

    // Key vars: up to 6, prioritizing DATABASE_URL, AUTH_SECRET, *_URL, *_KEY
    let priority_keywords = ["DATABASE_URL", "AUTH_SECRET", "SECRET", "URL", "KEY", "TOKEN"];
    let mut key_vars: Vec<&str> = Vec::new();
    // First pass: exact/contains priority keywords
    for kw in &priority_keywords {
        for var in &vars {
            if var.contains(kw) && !key_vars.contains(&var.as_str()) && key_vars.len() < 6 {
                key_vars.push(var.as_str());
            }
        }
    }
    // Fill up to 6 with any remaining
    for var in &vars {
        if !key_vars.contains(&var.as_str()) && key_vars.len() < 6 {
            key_vars.push(var.as_str());
        }
    }

    let mut out = String::from("## Environment\n\n");
    out.push_str(&format!("**Groups**: {}\n", groups_str));
    out.push_str(&format!("**Key vars**: {}\n", key_vars.join(", ")));
    out.push('\n');
    Some(out)
}

/// Extract a grouping prefix from an env var name.
/// E.g. `NEXT_PUBLIC_API_URL` → `NEXT_PUBLIC`, `DATABASE_URL` → `DATABASE`.
fn extract_env_prefix(var: &str) -> String {
    // Check for two-word known prefixes first
    let two_word_prefixes = ["NEXT_PUBLIC", "RESEND_", "STRIPE_"];
    for pfx in &two_word_prefixes {
        let pfx_clean = pfx.trim_end_matches('_');
        if var.starts_with(pfx_clean) {
            return pfx_clean.to_string();
        }
    }
    // Otherwise use the first segment before the first underscore
    var.split('_').next().unwrap_or(var).to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Dependencies section tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_package_json_deps_basic() {
        let content = r#"{
  "name": "my-app",
  "dependencies": {
    "react": "^18.0.0",
    "next": "^14.0.0",
    "zod": "^3.0.0"
  },
  "devDependencies": {
    "typescript": "^5.0.0",
    "eslint": "^8.0.0"
  }
}"#;
        let deps = parse_package_json_deps(content);
        assert!(deps.contains(&"react".to_string()), "missing react");
        assert!(deps.contains(&"next".to_string()), "missing next");
        assert!(deps.contains(&"zod".to_string()), "missing zod");
        assert!(deps.contains(&"typescript".to_string()), "missing typescript");
        assert!(deps.contains(&"eslint".to_string()), "missing eslint");
    }

    #[test]
    fn test_parse_package_json_deps_empty() {
        let content = r#"{"name": "my-app"}"#;
        let deps = parse_package_json_deps(content);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_cargo_toml_deps() {
        let content = r#"
[package]
name = "ig"
version = "1.0.0"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
tokio = "1"

[dev-dependencies]
tempfile = "3"
"#;
        let deps = parse_cargo_toml_deps(content);
        assert!(deps.contains(&"anyhow".to_string()), "missing anyhow");
        assert!(deps.contains(&"clap".to_string()), "missing clap");
        assert!(deps.contains(&"serde".to_string()), "missing serde");
        assert!(deps.contains(&"tokio".to_string()), "missing tokio");
        // dev-dependencies should not be included
        assert!(!deps.contains(&"tempfile".to_string()), "tempfile should not appear");
    }

    #[test]
    fn test_parse_composer_json_deps() {
        let content = r#"{
  "require": {
    "php": ">=8.1",
    "laravel/framework": "^10.0",
    "guzzlehttp/guzzle": "^7.0"
  }
}"#;
        let deps = parse_composer_json_deps(content);
        assert!(deps.contains(&"laravel/framework".to_string()), "missing laravel/framework");
        assert!(deps.contains(&"guzzlehttp/guzzle".to_string()), "missing guzzlehttp/guzzle");
        // php version constraint should be excluded
        assert!(!deps.contains(&"php".to_string()), "php should be excluded");
    }

    #[test]
    fn test_build_dependencies_section_package_json() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"dependencies":{"react":"^18","next":"^14","tailwindcss":"^3"}}"#,
        )
        .unwrap();

        let section = build_dependencies_section(dir.path()).unwrap();
        assert!(section.contains("## Dependencies"), "missing header");
        assert!(section.contains("**package.json**"), "missing manifest label");
        assert!(section.contains("react"), "missing react");
        assert!(section.contains("next"), "missing next");
        assert!(section.contains("tailwindcss"), "missing tailwindcss");
    }

    #[test]
    fn test_build_dependencies_section_cargo_toml() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[dependencies]\nanyhow = \"1\"\ntokio = \"1\"\n",
        )
        .unwrap();

        let section = build_dependencies_section(dir.path()).unwrap();
        assert!(section.contains("## Dependencies"));
        assert!(section.contains("**Cargo.toml**"));
        assert!(section.contains("anyhow"));
        assert!(section.contains("tokio"));
    }

    #[test]
    fn test_build_dependencies_section_none() {
        let dir = TempDir::new().unwrap();
        assert!(build_dependencies_section(dir.path()).is_none());
    }

    #[test]
    fn test_build_dependencies_section_limits_to_20() {
        let dir = TempDir::new().unwrap();
        // Create 25 dependencies
        let mut deps_obj = String::from(r#"{"dependencies":{"#);
        for i in 0..25 {
            if i > 0 {
                deps_obj.push(',');
            }
            deps_obj.push_str(&format!("\"pkg-{}\": \"^1.0\"", i));
        }
        deps_obj.push_str("}}");
        fs::write(dir.path().join("package.json"), &deps_obj).unwrap();

        let section = build_dependencies_section(dir.path()).unwrap();
        // Count comma-separated names — should be at most 20
        let line = section
            .lines()
            .find(|l| l.starts_with("**package.json**"))
            .unwrap();
        let after_colon = line.split(": ").nth(1).unwrap_or("");
        let count = after_colon.split(", ").count();
        assert!(count <= 20, "should cap at 20 deps, got {}", count);
    }

    // -----------------------------------------------------------------------
    // API Routes section tests
    // -----------------------------------------------------------------------

    fn setup_api_routes(dir: &TempDir) {
        let api = dir.path().join("app").join("api");
        // /api/auth/session/route.ts
        let auth_session = api.join("auth").join("session");
        fs::create_dir_all(&auth_session).unwrap();
        fs::write(auth_session.join("route.ts"), "export async function GET() {}").unwrap();

        // /api/auth/refresh/route.ts
        let auth_refresh = api.join("auth").join("refresh");
        fs::create_dir_all(&auth_refresh).unwrap();
        fs::write(auth_refresh.join("route.ts"), "export async function POST() {}").unwrap();

        // /api/payments/checkout/route.ts
        let payments_checkout = api.join("payments").join("checkout");
        fs::create_dir_all(&payments_checkout).unwrap();
        fs::write(payments_checkout.join("route.ts"), "").unwrap();

        // /api/payments/webhook/route.js (js variant)
        let payments_webhook = api.join("payments").join("webhook");
        fs::create_dir_all(&payments_webhook).unwrap();
        fs::write(payments_webhook.join("route.js"), "").unwrap();
    }

    #[test]
    fn test_build_api_routes_section() {
        let dir = TempDir::new().unwrap();
        setup_api_routes(&dir);

        let section = build_api_routes_section(dir.path()).unwrap();
        assert!(section.contains("## API Routes"), "missing header");
        assert!(section.contains("/api/auth/"), "missing auth group");
        assert!(section.contains("/api/payments/"), "missing payments group");
        assert!(section.contains("session"), "missing session route");
        assert!(section.contains("refresh"), "missing refresh route");
        assert!(section.contains("checkout"), "missing checkout route");
        assert!(section.contains("webhook"), "missing webhook route");
    }

    #[test]
    fn test_build_api_routes_section_no_api_dir() {
        let dir = TempDir::new().unwrap();
        assert!(build_api_routes_section(dir.path()).is_none());
    }

    #[test]
    fn test_build_api_routes_section_empty_api_dir() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("app").join("api")).unwrap();
        assert!(build_api_routes_section(dir.path()).is_none());
    }

    // -----------------------------------------------------------------------
    // Environment section tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_env_section_env_example() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".env.example"),
            "DATABASE_URL=postgres://localhost/db\nAUTH_SECRET=changeme\nNEXT_PUBLIC_API_URL=https://api.example.com\nSTRIPE_SECRET_KEY=sk_test_xxx\n",
        )
        .unwrap();

        let section = build_env_section(dir.path()).unwrap();
        assert!(section.contains("## Environment"), "missing header");
        assert!(section.contains("**Groups**"), "missing groups line");
        assert!(section.contains("**Key vars**"), "missing key vars line");
        assert!(section.contains("DATABASE_URL"), "missing DATABASE_URL");
        assert!(section.contains("AUTH_SECRET"), "missing AUTH_SECRET");
    }

    #[test]
    fn test_build_env_section_local_example() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".env.local.example"),
            "DATABASE_URL=postgres://localhost/db\nSECRET_KEY=abc\n",
        )
        .unwrap();

        let section = build_env_section(dir.path()).unwrap();
        assert!(section.contains("## Environment"));
        assert!(section.contains("DATABASE_URL"));
    }

    #[test]
    fn test_build_env_section_none() {
        let dir = TempDir::new().unwrap();
        assert!(build_env_section(dir.path()).is_none());
    }

    #[test]
    fn test_build_env_section_groups() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".env.example"),
            "NEXT_PUBLIC_URL=https://example.com\nNEXT_PUBLIC_CDN=https://cdn.example.com\nDATABASE_URL=postgres://\nDATABASE_HOST=localhost\nDATABASE_PORT=5432\nSTRIPE_SECRET_KEY=sk\nSTRIPE_WEBHOOK_SECRET=wh\n",
        )
        .unwrap();

        let section = build_env_section(dir.path()).unwrap();
        // NEXT_PUBLIC group should show count 2
        assert!(section.contains("NEXT_PUBLIC (2)"), "missing NEXT_PUBLIC group count");
        // DATABASE group should show count 3
        assert!(section.contains("DATABASE (3)"), "missing DATABASE group count");
        // STRIPE group should show count 2
        assert!(section.contains("STRIPE (2)"), "missing STRIPE group count");
    }

    #[test]
    fn test_build_env_section_ignores_comments_and_blank_lines() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".env.example"),
            "# This is a comment\n\nDATABASE_URL=postgres://\n# Another comment\nAUTH_SECRET=x\n",
        )
        .unwrap();

        let section = build_env_section(dir.path()).unwrap();
        assert!(section.contains("DATABASE_URL"));
        assert!(section.contains("AUTH_SECRET"));
        // Comments should not appear as variable names
        assert!(!section.contains("This is a comment"));
    }

    // -----------------------------------------------------------------------
    // Integration: sections appear between Structure and Files
    // -----------------------------------------------------------------------

    #[test]
    fn test_sections_ordering_in_context() {
        let dir = setup_test_project();

        // Add package.json
        fs::write(
            dir.path().join("package.json"),
            r#"{"dependencies":{"react":"^18"}}"#,
        )
        .unwrap();

        // Add .env.example
        fs::write(dir.path().join(".env.example"), "DATABASE_URL=x\n").unwrap();

        // Add app/api route
        let route_dir = dir.path().join("app").join("api").join("health");
        fs::create_dir_all(&route_dir).unwrap();
        fs::write(route_dir.join("route.ts"), "").unwrap();

        let output = generate_context(dir.path(), true, 1_048_576).unwrap();

        let struct_pos = output.find("## Structure").unwrap();
        let deps_pos = output.find("## Dependencies").unwrap();
        let routes_pos = output.find("## API Routes").unwrap();
        let env_pos = output.find("## Environment").unwrap();
        let files_pos = output.find("## Files").unwrap();

        assert!(struct_pos < deps_pos, "Dependencies must come after Structure");
        assert!(deps_pos < routes_pos, "API Routes must come after Dependencies");
        assert!(routes_pos < env_pos, "Environment must come after API Routes");
        assert!(env_pos < files_pos, "Files must come after Environment");
    }
}
