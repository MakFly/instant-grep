//! `ig test [args...]` — auto-detect test framework and run with filter.
//!
//! Detects the project's test framework from marker files in the current
//! directory, builds the appropriate command, and delegates to `ig run`.

use anyhow::Result;
use std::path::Path;

/// Auto-detect the test framework and run tests with filtering.
///
/// Detection order:
/// 1. `Cargo.toml` → `cargo test`
/// 2. `package.json` → vitest or jest (checks scripts)
/// 3. `pyproject.toml` or `setup.py` → `pytest`
/// 4. `go.mod` → `go test ./...`
/// 5. `Gemfile` → `bundle exec rspec`
/// 6. `*.csproj` or `*.sln` → `dotnet test`
pub fn run(extra_args: &[String]) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let cmd_parts = detect_test_command(&cwd, extra_args)?;
    crate::cmds::run::run(&cmd_parts)
}

fn detect_test_command(cwd: &Path, extra_args: &[String]) -> Result<Vec<String>> {
    // 1. Rust / Cargo
    if cwd.join("Cargo.toml").exists() {
        let mut cmd = vec!["cargo".into(), "test".into()];
        cmd.extend(extra_args.iter().cloned());
        return Ok(cmd);
    }

    // 2. Node.js / package.json
    if cwd.join("package.json").exists() {
        let framework = detect_js_test_framework(cwd);
        let mut cmd = match framework.as_str() {
            "vitest" => vec!["npx".into(), "vitest".into(), "run".into()],
            "jest" => vec!["npx".into(), "jest".into()],
            _ => vec!["npx".into(), "vitest".into(), "run".into()],
        };
        cmd.extend(extra_args.iter().cloned());
        return Ok(cmd);
    }

    // 3. Python
    if cwd.join("pyproject.toml").exists() || cwd.join("setup.py").exists() {
        let mut cmd = vec!["pytest".into()];
        cmd.extend(extra_args.iter().cloned());
        return Ok(cmd);
    }

    // 4. Go
    if cwd.join("go.mod").exists() {
        let mut cmd = vec!["go".into(), "test".into(), "./...".into()];
        cmd.extend(extra_args.iter().cloned());
        return Ok(cmd);
    }

    // 5. Ruby
    if cwd.join("Gemfile").exists() {
        let mut cmd = vec!["bundle".into(), "exec".into(), "rspec".into()];
        cmd.extend(extra_args.iter().cloned());
        return Ok(cmd);
    }

    // 6. .NET
    if has_dotnet_project(cwd) {
        let mut cmd = vec!["dotnet".into(), "test".into()];
        cmd.extend(extra_args.iter().cloned());
        return Ok(cmd);
    }

    anyhow::bail!(
        "No test framework detected. Supported: Cargo.toml, package.json, \
         pyproject.toml, go.mod, Gemfile, *.csproj/*.sln"
    );
}

/// Check package.json scripts for vitest or jest.
fn detect_js_test_framework(cwd: &Path) -> String {
    let pkg_path = cwd.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_path)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            // Check scripts.test
            if let Some(test_script) = json
                .get("scripts")
                .and_then(|s| s.get("test"))
                .and_then(|t| t.as_str())
            {
                if test_script.contains("vitest") {
                    return "vitest".into();
                }
                if test_script.contains("jest") {
                    return "jest".into();
                }
            }
            // Check devDependencies
            if let Some(dev_deps) = json.get("devDependencies") {
                if dev_deps.get("vitest").is_some() {
                    return "vitest".into();
                }
                if dev_deps.get("jest").is_some() {
                    return "jest".into();
                }
            }
        }
    "vitest".into() // Default
}

/// Check if there are .csproj or .sln files in the directory.
fn has_dotnet_project(cwd: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(cwd) {
        for entry in entries.flatten() {
            if let Some(ext) = entry.path().extension() {
                let ext = ext.to_string_lossy();
                if ext == "csproj" || ext == "sln" {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn detects_cargo_project() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        let cmd = detect_test_command(dir.path(), &[]).unwrap();
        assert_eq!(cmd, vec!["cargo", "test"]);
    }

    #[test]
    fn detects_go_project() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("go.mod"), "module example.com/test").unwrap();
        let cmd = detect_test_command(dir.path(), &[]).unwrap();
        assert_eq!(cmd, vec!["go", "test", "./..."]);
    }

    #[test]
    fn no_project_returns_error() {
        let dir = TempDir::new().unwrap();
        let result = detect_test_command(dir.path(), &[]);
        assert!(result.is_err());
    }

    #[test]
    fn extra_args_appended() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let cmd = detect_test_command(
            dir.path(),
            &["--release".into(), "--".into(), "my_test".into()],
        )
        .unwrap();
        assert_eq!(cmd, vec!["cargo", "test", "--release", "--", "my_test"]);
    }
}
