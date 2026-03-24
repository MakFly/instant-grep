use std::fs;
use std::path::{Path, PathBuf};

const IG_SEARCH_TOOLS_SECTION: &str = "\n## Search Tools\n\
- **Code search**: prefer `ig` (instant-grep) over `rg` or `grep` for searching code.\n\
- Usage: `ig \"pattern\" [path]` or `ig search \"pattern\" [path]` — trigram-indexed regex search.\n\
- If the project has no `.ig/` index yet, `ig` auto-builds one on first search.\n\
- Fall back to `rg` only if `ig` is not installed.\n";

const IG_PERMISSION: &str = "Bash(ig *)";

pub fn run_setup() {
    eprintln!("\x1b[1m🔧 ig setup — Configuring AI CLI agents...\x1b[0m\n");

    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => {
            eprintln!("✗ Could not determine HOME directory");
            return;
        }
    };

    let mut auto_configured = 0u32;
    let mut manual_needed = 0u32;

    // --- Claude Code ---
    let claude_dir = home.join(".claude");
    if claude_dir.is_dir() {
        let mut actions = Vec::new();
        match configure_claude_settings(&claude_dir) {
            ConfigResult::Configured(msg) => actions.push(msg),
            ConfigResult::AlreadyDone(msg) => actions.push(msg),
            ConfigResult::Error(msg) => actions.push(msg),
        }
        match configure_claude_md(&claude_dir) {
            ConfigResult::Configured(msg) => actions.push(msg),
            ConfigResult::AlreadyDone(msg) => actions.push(msg),
            ConfigResult::Error(msg) => actions.push(msg),
        }
        eprintln!("\x1b[32m✓ Claude Code\x1b[0m");
        for action in &actions {
            eprintln!("  → {}", action);
        }
        auto_configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ Claude Code — not detected (~/.claude/)\x1b[0m");
    }

    // --- Codex CLI ---
    let codex_dir = home.join(".codex");
    if codex_dir.is_dir() {
        match configure_codex_agents_md(&codex_dir) {
            ConfigResult::Configured(msg) | ConfigResult::AlreadyDone(msg) => {
                eprintln!("\x1b[32m✓ Codex CLI\x1b[0m");
                eprintln!("  → {}", msg);
                auto_configured += 1;
            }
            ConfigResult::Error(msg) => {
                eprintln!("\x1b[31m✗ Codex CLI\x1b[0m");
                eprintln!("  → {}", msg);
            }
        }
    } else {
        eprintln!("\x1b[2m⊘ Codex CLI — not detected (~/.codex/)\x1b[0m");
    }

    // --- Gemini CLI ---
    let gemini_dir = home.join(".gemini");
    if gemini_dir.is_dir() {
        eprintln!("\n\x1b[33mℹ Gemini CLI\x1b[0m (detected at ~/.gemini/)");
        eprintln!("  Add this to ~/.gemini/settings.json:");
        eprintln!("  \x1b[36m\"tools\": {{ \"shell\": {{ \"allowed\": [\"ig *\"] }} }}\x1b[0m");
        eprintln!();
        eprintln!("  Or add to your project GEMINI.md:");
        eprintln!("  \x1b[36mPrefer `ig \"pattern\"` over `rg` or `grep` for code search.\x1b[0m");
        manual_needed += 1;
    }

    // --- Other agents ---
    eprintln!("\n\x1b[33mℹ Other agents\x1b[0m (Cursor, Windsurf, OpenCode, Aider...)");
    eprintln!("  Add to your rules/instructions file:");
    eprintln!("  \x1b[36mPrefer `ig \"pattern\"` over `rg` or `grep` for code search.");
    eprintln!("  Usage: ig \"pattern\" [path] — trigram-indexed regex search.\x1b[0m");

    // --- Summary ---
    eprintln!();
    if manual_needed > 0 {
        eprintln!(
            "\x1b[1mDone!\x1b[0m ig configured for {} agent(s) ({} manual setup needed).",
            auto_configured, manual_needed
        );
    } else {
        eprintln!(
            "\x1b[1mDone!\x1b[0m ig configured for {} agent(s).",
            auto_configured
        );
    }
}

enum ConfigResult {
    Configured(String),
    AlreadyDone(String),
    Error(String),
}

fn configure_claude_settings(claude_dir: &Path) -> ConfigResult {
    let settings_path = claude_dir.join("settings.json");

    let content = match fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(_) => {
            return ConfigResult::Error(
                "Could not read ~/.claude/settings.json".to_string(),
            );
        }
    };

    if content.contains(IG_PERMISSION) {
        return ConfigResult::AlreadyDone(
            "Bash(ig *) permission already present in settings.json".to_string(),
        );
    }

    // Insert "Bash(ig *)" into the permissions.allow array
    let mut parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            return ConfigResult::Error("Could not parse settings.json".to_string());
        }
    };

    let inserted = if let Some(allow) = parsed
        .get_mut("permissions")
        .and_then(|p| p.get_mut("allow"))
        .and_then(|a| a.as_array_mut())
    {
        allow.push(serde_json::Value::String(IG_PERMISSION.to_string()));
        true
    } else {
        false
    };

    if !inserted {
        return ConfigResult::Error(
            "Could not find permissions.allow array in settings.json".to_string(),
        );
    }

    let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
    if fs::write(&settings_path, formatted.as_bytes()).is_err() {
        return ConfigResult::Error("Could not write settings.json".to_string());
    }

    ConfigResult::Configured(
        "Added Bash(ig *) permission to ~/.claude/settings.json".to_string(),
    )
}

fn configure_claude_md(claude_dir: &Path) -> ConfigResult {
    let md_path = claude_dir.join("CLAUDE.md");

    let content = fs::read_to_string(&md_path).unwrap_or_default();

    if content.contains("## Search Tools") && content.contains("ig") {
        return ConfigResult::AlreadyDone(
            "Search Tools section already present in CLAUDE.md".to_string(),
        );
    }

    let new_content = if content.is_empty() {
        format!("# CLAUDE.md\n{}", IG_SEARCH_TOOLS_SECTION)
    } else if content.contains("# Global Rules") {
        // Insert before "# Global Rules"
        content.replacen(
            "# Global Rules",
            &format!("{}\n# Global Rules", IG_SEARCH_TOOLS_SECTION),
            1,
        )
    } else {
        // Append at the end
        format!("{}\n{}", content.trim_end(), IG_SEARCH_TOOLS_SECTION)
    };

    if fs::write(&md_path, new_content.as_bytes()).is_err() {
        return ConfigResult::Error("Could not write CLAUDE.md".to_string());
    }

    ConfigResult::Configured(
        "Added Search Tools section to ~/.claude/CLAUDE.md".to_string(),
    )
}

fn configure_codex_agents_md(codex_dir: &Path) -> ConfigResult {
    let md_path = codex_dir.join("AGENTS.md");

    let content = fs::read_to_string(&md_path).unwrap_or_default();

    if content.contains("## Search Tools") && content.contains("ig") {
        return ConfigResult::AlreadyDone(
            "Search Tools section already present in AGENTS.md".to_string(),
        );
    }

    let section = format!(
        "# AGENTS.md\n{}",
        IG_SEARCH_TOOLS_SECTION
    );

    let new_content = if content.is_empty() || content.trim().is_empty() {
        section
    } else if content.contains("## Search Tools") {
        // Section exists but without ig — append ig instructions
        content.replacen(
            "## Search Tools",
            "## Search Tools\n\
- **Code search**: prefer `ig` (instant-grep) over `rg` or `grep` for searching code.\n\
- Usage: `ig \"pattern\" [path]` or `ig search \"pattern\" [path]` — trigram-indexed regex search.\n\
- If the project has no `.ig/` index yet, `ig` auto-builds one on first search.\n\
- Fall back to `rg` only if `ig` is not installed.\n",
            1,
        )
    } else {
        // Append section
        format!("{}\n{}", content.trim_end(), IG_SEARCH_TOOLS_SECTION)
    };

    if fs::write(&md_path, new_content.as_bytes()).is_err() {
        return ConfigResult::Error("Could not write AGENTS.md".to_string());
    }

    ConfigResult::Configured(
        "Added Search Tools section to ~/.codex/AGENTS.md".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_claude_settings_injection() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"permissions":{"allow":["Bash(git *)"]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("Bash(ig *)"), "should contain ig permission");
    }

    #[test]
    fn test_claude_settings_idempotent() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"permissions":{"allow":["Bash(git *)","Bash(ig *)"]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_claude_settings_missing_file_returns_error() {
        let dir = TempDir::new().unwrap();
        // No settings.json written — file does not exist
        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Error(_)));
    }

    #[test]
    fn test_claude_settings_invalid_json_returns_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), "not json").unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Error(_)));
    }

    #[test]
    fn test_claude_settings_missing_allow_array_returns_error() {
        let dir = TempDir::new().unwrap();
        // Valid JSON but no permissions.allow array
        fs::write(dir.path().join("settings.json"), r#"{"other": "value"}"#).unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Error(_)));
    }

    #[test]
    fn test_claude_md_injection() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("CLAUDE.md"),
            "# CLAUDE.md\n\n# Global Rules\n\n- Some rule\n",
        )
        .unwrap();

        let result = configure_claude_md(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("## Search Tools"), "should contain Search Tools section");
        assert!(content.contains("ig"), "should mention ig");
    }

    #[test]
    fn test_claude_md_idempotent() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("CLAUDE.md"),
            "# CLAUDE.md\n\n## Search Tools\n- Use ig\n",
        )
        .unwrap();

        let result = configure_claude_md(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_claude_md_missing_file_creates_content() {
        let dir = TempDir::new().unwrap();
        // CLAUDE.md does not exist — configure_claude_md starts from empty string
        let result = configure_claude_md(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("## Search Tools"));
        assert!(content.contains("ig"));
    }

    #[test]
    fn test_codex_agents_md_injection() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "").unwrap();

        let result = configure_codex_agents_md(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("Search Tools"));
        assert!(content.contains("ig"));
    }

    #[test]
    fn test_codex_agents_md_idempotent() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("AGENTS.md"),
            "# AGENTS.md\n\n## Search Tools\n- Use ig for search\n",
        )
        .unwrap();

        let result = configure_codex_agents_md(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_codex_agents_md_appends_to_existing_content() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("AGENTS.md"),
            "# AGENTS.md\n\nSome existing content.\n",
        )
        .unwrap();

        let result = configure_codex_agents_md(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("Some existing content."), "should preserve existing content");
        assert!(content.contains("## Search Tools"), "should add Search Tools section");
    }
}
