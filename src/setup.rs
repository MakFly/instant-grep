use std::fs;
use std::path::{Path, PathBuf};

const IG_SEARCH_TOOLS_SECTION: &str = "\n## Search Tools\n\
- **Code search**: prefer `ig` (instant-grep) over `rg` or `grep` for searching code.\n\
- Usage: `ig \"pattern\" [path]` or `ig search \"pattern\" [path]` — trigram-indexed regex search.\n\
- If the project has no `.ig/` index yet, `ig` auto-builds one on first search.\n\
- **Project overview**: read `.ig/context.md` for a complete project map (tree + file summaries + symbols).\n\
- **Smart read**: `ig read <file> --signatures` for imports and function signatures only.\n\
- **Smart summary**: `ig smart [path]` for 2-line file summaries.\n\
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
        match install_rewrite_hook(&claude_dir) {
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

const IG_REWRITE_HOOK: &str = include_str!("../hooks/ig-rewrite.sh");
const IG_HOOK_MARKER: &str = "ig-rewrite.sh";

fn install_rewrite_hook(claude_dir: &Path) -> ConfigResult {
    let hooks_dir = claude_dir.join("hooks");
    let hook_path = hooks_dir.join("ig-rewrite.sh");

    // Write hook file if absent
    let file_installed = if hook_path.exists() {
        false
    } else {
        if fs::create_dir_all(&hooks_dir).is_err() {
            return ConfigResult::Error("Could not create ~/.claude/hooks/".to_string());
        }
        if fs::write(&hook_path, IG_REWRITE_HOOK).is_err() {
            return ConfigResult::Error("Could not write ig-rewrite.sh".to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755));
        }
        true
    };

    // Register in settings.json (idempotent)
    let reg = register_hook_in_settings(claude_dir);

    match reg {
        ConfigResult::Configured(msg) => {
            if file_installed {
                ConfigResult::Configured(format!("Installed ig-rewrite.sh + {}", msg))
            } else {
                ConfigResult::Configured(msg)
            }
        }
        ConfigResult::AlreadyDone(msg) => {
            if file_installed {
                ConfigResult::Configured(format!("Installed ig-rewrite.sh ({})", msg))
            } else {
                ConfigResult::AlreadyDone(msg)
            }
        }
        ConfigResult::Error(e) => {
            if file_installed {
                ConfigResult::Configured(format!(
                    "Installed ig-rewrite.sh but failed to register: {}", e
                ))
            } else {
                ConfigResult::Error(e)
            }
        }
    }
}

/// Register ig-rewrite.sh hook in settings.json PreToolUse.
/// Removes old prefer-ig.sh if present. Idempotent.
fn register_hook_in_settings(claude_dir: &Path) -> ConfigResult {
    let settings_path = claude_dir.join("settings.json");

    let content = fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string());
    let mut parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return ConfigResult::Error("Could not parse settings.json".to_string()),
    };

    // Ensure hooks.PreToolUse exists as array
    if parsed.get("hooks").is_none() {
        parsed["hooks"] = serde_json::json!({});
    }
    if parsed["hooks"].get("PreToolUse").is_none() {
        parsed["hooks"]["PreToolUse"] = serde_json::json!([]);
    }

    let pre_tool_use = match parsed["hooks"]["PreToolUse"].as_array_mut() {
        Some(arr) => arr,
        None => return ConfigResult::Error("PreToolUse is not an array".to_string()),
    };

    // Find or create the Bash matcher entry
    let bash_idx = pre_tool_use.iter().position(|entry| {
        entry.get("matcher").and_then(|m| m.as_str()) == Some("Bash")
    });

    if bash_idx.is_none() {
        pre_tool_use.push(serde_json::json!({"matcher": "Bash", "hooks": []}));
    }

    let bash_idx = pre_tool_use.iter().position(|entry| {
        entry.get("matcher").and_then(|m| m.as_str()) == Some("Bash")
    }).unwrap();

    let bash_entry = &mut pre_tool_use[bash_idx];
    if bash_entry.get("hooks").is_none() {
        bash_entry["hooks"] = serde_json::json!([]);
    }

    let hooks = match bash_entry["hooks"].as_array_mut() {
        Some(arr) => arr,
        None => return ConfigResult::Error("Bash hooks is not an array".to_string()),
    };

    // Remove prefer-ig.sh
    let before_len = hooks.len();
    hooks.retain(|hook| {
        let cmd = hook.get("command").and_then(|c| c.as_str()).unwrap_or("");
        !cmd.contains("prefer-ig.sh")
    });
    let removed_prefer_ig = hooks.len() != before_len;

    // Check if already registered
    let already_registered = hooks.iter().any(|hook| {
        let cmd = hook.get("command").and_then(|c| c.as_str()).unwrap_or("");
        cmd.contains(IG_HOOK_MARKER)
    });

    if already_registered && !removed_prefer_ig {
        return ConfigResult::AlreadyDone(
            "ig-rewrite.sh already registered in settings.json".to_string(),
        );
    }

    if !already_registered {
        hooks.push(serde_json::json!({
            "type": "command",
            "command": "~/.claude/hooks/ig-rewrite.sh"
        }));
    }

    // Write back
    let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
    if fs::write(&settings_path, formatted.as_bytes()).is_err() {
        return ConfigResult::Error("Could not write settings.json".to_string());
    }

    let mut msg = "Registered ig-rewrite.sh in settings.json".to_string();
    if removed_prefer_ig {
        msg.push_str(" (removed old prefer-ig.sh)");
    }
    ConfigResult::Configured(msg)
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

    // --- register_hook_in_settings tests ---

    #[test]
    fn test_register_hook_fresh_settings() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo destructive check"}]}]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = register_hook_in_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("ig-rewrite.sh"), "should add ig-rewrite.sh");
        assert!(content.contains("destructive check"), "should preserve existing hooks");
    }

    #[test]
    fn test_register_hook_removes_prefer_ig() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"~/.claude/hooks/prefer-ig.sh"}]}]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = register_hook_in_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!content.contains("prefer-ig.sh"), "should remove prefer-ig.sh");
        assert!(content.contains("ig-rewrite.sh"), "should add ig-rewrite.sh");
    }

    #[test]
    fn test_register_hook_idempotent() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"~/.claude/hooks/ig-rewrite.sh"}]}]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = register_hook_in_settings(dir.path());
        assert!(matches!(result, ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_register_hook_no_pretooluse() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"permissions":{"allow":[]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = register_hook_in_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("ig-rewrite.sh"));
        assert!(content.contains("PreToolUse"));
    }

    #[test]
    fn test_register_hook_preserves_grep_blocker() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[]},{"matcher":"Grep","hooks":[{"type":"command","command":"echo BLOCK"}]}]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = register_hook_in_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("ig-rewrite.sh"), "should add ig hook");
        assert!(content.contains("Grep"), "should preserve Grep matcher");
        assert!(content.contains("BLOCK"), "should preserve Grep blocker content");
    }

    #[test]
    fn test_install_rewrite_hook_full_flow() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"~/.claude/hooks/prefer-ig.sh"}]}]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = install_rewrite_hook(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        // Hook file should exist
        assert!(dir.path().join("hooks/ig-rewrite.sh").exists());

        // settings.json should have ig-rewrite.sh and not prefer-ig.sh
        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("ig-rewrite.sh"));
        assert!(!content.contains("prefer-ig.sh"));
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
