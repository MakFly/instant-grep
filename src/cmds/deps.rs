//! `ig deps` — read and display dependency files from the current directory.
//!
//! Supports: Cargo.toml, package.json, go.mod, requirements.txt,
//! pyproject.toml, Gemfile, composer.json.

use std::path::Path;
use anyhow::Result;

/// Run the deps command — scan CWD for dependency manifests and display them.
pub fn run(_args: &[String]) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let mut found = false;

    if cwd.join("Cargo.toml").exists() {
        found = true;
        print_cargo_deps(&cwd.join("Cargo.toml"))?;
    }
    if cwd.join("package.json").exists() {
        found = true;
        print_node_deps(&cwd.join("package.json"))?;
    }
    if cwd.join("go.mod").exists() {
        found = true;
        print_go_deps(&cwd.join("go.mod"))?;
    }
    if cwd.join("requirements.txt").exists() {
        found = true;
        print_requirements_deps(&cwd.join("requirements.txt"))?;
    }
    if cwd.join("pyproject.toml").exists() {
        found = true;
        print_pyproject_deps(&cwd.join("pyproject.toml"))?;
    }
    if cwd.join("Gemfile").exists() {
        found = true;
        print_gemfile_deps(&cwd.join("Gemfile"))?;
    }
    if cwd.join("composer.json").exists() {
        found = true;
        print_composer_deps(&cwd.join("composer.json"))?;
    }

    if !found {
        println!("No dependency files found in current directory.");
    }

    Ok(0)
}

fn print_cargo_deps(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    println!("Rust (Cargo.toml):");

    let deps = extract_toml_section(&content, "[dependencies]");
    let dev_deps = extract_toml_section(&content, "[dev-dependencies]");

    if !deps.is_empty() {
        let formatted: Vec<String> = deps.iter().map(|(k, v)| format!("{k} ({v})")).collect();
        println!("  Dependencies ({}): {}", deps.len(), formatted.join(", "));
    }
    if !dev_deps.is_empty() {
        let formatted: Vec<String> = dev_deps.iter().map(|(k, v)| format!("{k} ({v})")).collect();
        println!("  Dev Dependencies ({}): {}", dev_deps.len(), formatted.join(", "));
    }
    println!();
    Ok(())
}

/// Simple TOML section parser — extracts `key = "value"` or `key = { version = "value" }` pairs.
fn extract_toml_section(content: &str, section: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == section {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with('[') {
            break;
        }
        if in_section && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((key, rest)) = trimmed.split_once('=') {
                let key = key.trim().to_string();
                let rest = rest.trim();
                let version = if rest.starts_with('"') {
                    rest.trim_matches('"').to_string()
                } else if rest.starts_with('{') {
                    // Parse { version = "..." }
                    extract_inline_version(rest)
                } else {
                    rest.to_string()
                };
                result.push((key, version));
            }
        }
    }
    result
}

fn extract_inline_version(s: &str) -> String {
    // Look for version = "..."
    let re = regex::Regex::new(r#"version\s*=\s*"([^"]+)""#).unwrap();
    re.captures(s)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "*".into())
}

fn print_node_deps(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;

    println!("Node.js (package.json):");

    if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
        let formatted: Vec<String> = deps
            .iter()
            .map(|(k, v)| format!("{k} ({})", v.as_str().unwrap_or("*")))
            .collect();
        println!("  Dependencies ({}): {}", deps.len(), formatted.join(", "));
    }
    if let Some(deps) = json.get("devDependencies").and_then(|d| d.as_object()) {
        let formatted: Vec<String> = deps
            .iter()
            .map(|(k, v)| format!("{k} ({})", v.as_str().unwrap_or("*")))
            .collect();
        println!("  Dev Dependencies ({}): {}", deps.len(), formatted.join(", "));
    }
    println!();
    Ok(())
}

fn print_go_deps(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    println!("Go (go.mod):");

    let mut deps = Vec::new();
    let mut in_require = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "require (" {
            in_require = true;
            continue;
        }
        if in_require && trimmed == ")" {
            in_require = false;
            continue;
        }
        if in_require && !trimmed.is_empty() && !trimmed.starts_with("//") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                deps.push(format!("{} ({})", parts[0], parts[1]));
            }
        }
    }

    if !deps.is_empty() {
        println!("  Dependencies ({}): {}", deps.len(), deps.join(", "));
    }
    println!();
    Ok(())
}

fn print_requirements_deps(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    println!("Python (requirements.txt):");

    let deps: Vec<String> = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .map(|l| l.trim().to_string())
        .collect();

    if !deps.is_empty() {
        println!("  Dependencies ({}): {}", deps.len(), deps.join(", "));
    }
    println!();
    Ok(())
}

fn print_pyproject_deps(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    println!("Python (pyproject.toml):");

    // Look for dependencies = [...] in [project] section
    let mut in_deps = false;
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("dependencies") && trimmed.contains('[') {
            in_deps = true;
            // Check for inline list
            if let Some(start) = trimmed.find('[') {
                if let Some(end) = trimmed.find(']') {
                    let items = &trimmed[start + 1..end];
                    for item in items.split(',') {
                        let dep = item.trim().trim_matches('"').trim().to_string();
                        if !dep.is_empty() {
                            deps.push(dep);
                        }
                    }
                    in_deps = false;
                }
            }
            continue;
        }
        if in_deps {
            if trimmed.starts_with(']') {
                in_deps = false;
                continue;
            }
            let dep = trimmed.trim_matches('"').trim_matches(',').trim().to_string();
            if !dep.is_empty() {
                deps.push(dep);
            }
        }
    }

    if !deps.is_empty() {
        println!("  Dependencies ({}): {}", deps.len(), deps.join(", "));
    }
    println!();
    Ok(())
}

fn print_gemfile_deps(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    println!("Ruby (Gemfile):");

    let gem_re = regex::Regex::new(r#"^\s*gem\s+['"]([^'"]+)['"](?:,\s*['"]([^'"]+)['"])?"#).unwrap();
    let deps: Vec<String> = content
        .lines()
        .filter_map(|line| {
            gem_re.captures(line).map(|c| {
                let name = c.get(1).unwrap().as_str();
                match c.get(2) {
                    Some(ver) => format!("{name} ({})", ver.as_str()),
                    None => name.to_string(),
                }
            })
        })
        .collect();

    if !deps.is_empty() {
        println!("  Dependencies ({}): {}", deps.len(), deps.join(", "));
    }
    println!();
    Ok(())
}

fn print_composer_deps(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;

    println!("PHP (composer.json):");

    if let Some(deps) = json.get("require").and_then(|d| d.as_object()) {
        let formatted: Vec<String> = deps
            .iter()
            .map(|(k, v)| format!("{k} ({})", v.as_str().unwrap_or("*")))
            .collect();
        println!("  Dependencies ({}): {}", deps.len(), formatted.join(", "));
    }
    if let Some(deps) = json.get("require-dev").and_then(|d| d.as_object()) {
        let formatted: Vec<String> = deps
            .iter()
            .map(|(k, v)| format!("{k} ({})", v.as_str().unwrap_or("*")))
            .collect();
        println!("  Dev Dependencies ({}): {}", deps.len(), formatted.join(", "));
    }
    println!();
    Ok(())
}
