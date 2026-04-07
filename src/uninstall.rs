//! `ig uninstall` — removes all ig artifacts from the system.
//!
//! Reverses everything done by `ig setup` and `install.sh`:
//! hook files, settings.json entries, CLAUDE.md sections, agent configs,
//! daemons, launchd plists, tracking data, and the binary itself.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use crate::setup::resolve_real_home;

// ─── Markers for identifying ig-installed hooks ─────────────────────────────

/// Markers in hook commands that identify ig-installed entries.
/// If a hook's `command` field contains any of these, it was installed by ig.
const IG_HOOK_MARKERS: &[&str] = &[
    "ig-guard.sh",
    "format.sh",
    "session-start.sh",
    "Use ig via Bash",  // Grep blocker
    "Destructive git",  // destructive git blocker
    "bun/bunx instead", // npm/npx blocker
    "secret detected",  // secret detection
    ".env",             // .env warning
];

const IG_HOOK_FILES: &[&str] = &["ig-guard.sh", "session-start.sh", "format.sh"];

const IG_ENV_VARS: &[&str] = &[
    "CLAUDE_CODE_EFFORT_LEVEL",
    "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE",
];

const IG_PERMISSION: &str = "Bash(ig *)";

// ─── Result type ────────────────────────────────────────────────────────────

enum RemoveResult {
    Removed(String),
    NotFound(String),
}

// ─── Public entry point ─────────────────────────────────────────────────────

pub fn run_uninstall(dry_run: bool, yes: bool) {
    let label = if dry_run { "[DRY RUN] " } else { "" };
    eprintln!("\x1b[1m🗑  ig uninstall {label}— Removing all ig artifacts...\x1b[0m\n");

    let home = match resolve_real_home() {
        Some(h) => h,
        None => {
            eprintln!("✗ Could not determine HOME directory");
            return;
        }
    };

    // Confirmation prompt (unless --yes or --dry-run)
    if !dry_run && !yes {
        eprint!("This will remove ig and all its configuration. Type 'yes' to confirm: ");
        io::stderr().flush().ok();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() || input.trim() != "yes" {
            eprintln!("Aborted.");
            return;
        }
        eprintln!();
    }

    let mut sections = 0u32;

    // 1. Stop daemons
    let results = stop_all_daemons(&home, dry_run);
    if !results.is_empty() {
        print_section("Daemons", &results);
        sections += 1;
    }

    // 2. Claude Code
    let claude_dir = home.join(".claude");
    if claude_dir.is_dir() {
        let results = remove_claude_config(&claude_dir, dry_run);
        print_section("Claude Code", &results);
        sections += 1;
    }

    // 3. Codex CLI
    let codex_dir = home.join(".codex");
    if codex_dir.is_dir() {
        let results = remove_codex_config(&codex_dir, dry_run);
        print_section("Codex CLI", &results);
        sections += 1;
    }

    // 4. OpenCode
    let opencode_dir = home.join(".config/opencode");
    if opencode_dir.is_dir() {
        let results = remove_opencode_config(&opencode_dir, dry_run);
        print_section("OpenCode", &results);
        sections += 1;
    }

    // 5. Cursor
    let cursor_dir = home.join(".cursor");
    if cursor_dir.is_dir() {
        let results = remove_cursor_config(&cursor_dir, dry_run);
        if results
            .iter()
            .any(|r| matches!(r, RemoveResult::Removed(_)))
        {
            print_section("Cursor", &results);
            sections += 1;
        }
    }

    // 6. Tracking data
    let result = remove_tracking_data(&home, dry_run);
    if matches!(result, RemoveResult::Removed(_)) {
        print_section("Tracking", &[result]);
        sections += 1;
    }

    // 7. Binary (last — we're running from it!)
    let result = remove_binary(&home, dry_run);
    if matches!(result, RemoveResult::Removed(_)) {
        print_section("Binary", &[result]);
        sections += 1;
    }

    eprintln!(
        "\n\x1b[1m{label}Done!\x1b[0m Removed ig artifacts from {} location(s).",
        sections
    );
}

fn print_section(name: &str, results: &[RemoveResult]) {
    eprintln!("\x1b[32m✓ {}\x1b[0m", name);
    for r in results {
        match r {
            RemoveResult::Removed(msg) => eprintln!("  → {}", msg),
            RemoveResult::NotFound(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
        }
    }
}

// ─── Daemons ────────────────────────────────────────────────────────────────

fn stop_all_daemons(home: &Path, dry_run: bool) -> Vec<RemoveResult> {
    let mut results = Vec::new();

    // Kill daemon sockets
    if let Ok(entries) = fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("ig-") && name.ends_with(".sock") {
                if !dry_run {
                    let _ = fs::remove_file(entry.path());
                }
                results.push(RemoveResult::Removed(format!("Removed socket /tmp/{name}")));
            }
        }
    }

    // Remove launchd plists (macOS)
    let launch_agents = home.join("Library/LaunchAgents");
    if launch_agents.is_dir()
        && let Ok(entries) = fs::read_dir(&launch_agents)
    {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !(name.starts_with("com.ig.daemon.") && name.ends_with(".plist")) {
                continue;
            }
            if !dry_run {
                let _ = std::process::Command::new("launchctl")
                    .args(["unload", &entry.path().to_string_lossy()])
                    .output();
                let _ = fs::remove_file(entry.path());
            }
            results.push(RemoveResult::Removed(format!("Removed plist {name}")));
        }
    }

    results
}

// ─── Claude Code ────────────────────────────────────────────────────────────

fn remove_claude_config(claude_dir: &Path, dry_run: bool) -> Vec<RemoveResult> {
    let mut results = Vec::new();

    // 1. Delete hook files
    let hooks_dir = claude_dir.join("hooks");
    for hook_file in IG_HOOK_FILES {
        let path = hooks_dir.join(hook_file);
        if !path.exists() {
            continue;
        }
        if !dry_run {
            let _ = fs::remove_file(&path);
        }
        results.push(RemoveResult::Removed(format!("Removed hooks/{hook_file}")));
    }

    // 2. Clean settings.json
    let settings_path = claude_dir.join("settings.json");
    if let Some(mut parsed) = fs::read_to_string(&settings_path)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        && clean_settings_json(&mut parsed)
    {
        if !dry_run {
            let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
            let _ = fs::write(&settings_path, format!("{formatted}\n"));
        }
        results.push(RemoveResult::Removed(
            "Cleaned ig entries from settings.json".to_string(),
        ));
    }

    // 3. Clean CLAUDE.md
    let md_path = claude_dir.join("CLAUDE.md");
    if let Some(cleaned) = fs::read_to_string(&md_path)
        .ok()
        .and_then(|c| remove_search_tools_section(&c))
    {
        if !dry_run {
            let _ = fs::write(&md_path, &cleaned);
        }
        results.push(RemoveResult::Removed(
            "Removed Search Tools section from CLAUDE.md".to_string(),
        ));
    }

    results
}

/// Remove all ig-related entries from a parsed settings.json.
/// Returns `true` if any changes were made.
fn clean_settings_json(parsed: &mut serde_json::Value) -> bool {
    let mut changed = false;

    // Remove "Bash(ig *)" from permissions.allow
    if let Some(allow) = parsed
        .get_mut("permissions")
        .and_then(|p| p.get_mut("allow"))
        .and_then(|a| a.as_array_mut())
    {
        let before = allow.len();
        allow.retain(|v| v.as_str() != Some(IG_PERMISSION));
        if allow.len() != before {
            changed = true;
        }
    }

    // Remove ig hooks from all hook categories
    if let Some(hooks) = parsed.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (_key, matchers) in hooks.iter_mut() {
            if let Some(matchers_arr) = matchers.as_array_mut() {
                for matcher in matchers_arr.iter_mut() {
                    if let Some(hook_list) = matcher.get_mut("hooks").and_then(|h| h.as_array_mut())
                    {
                        let before = hook_list.len();
                        hook_list.retain(|hook| {
                            let cmd = hook.get("command").and_then(|c| c.as_str()).unwrap_or("");
                            !IG_HOOK_MARKERS.iter().any(|marker| cmd.contains(marker))
                        });
                        if hook_list.len() != before {
                            changed = true;
                        }
                    }
                }
                // Remove matchers with empty hook lists
                let before = matchers_arr.len();
                matchers_arr.retain(|m| {
                    m.get("hooks")
                        .and_then(|h| h.as_array())
                        .is_none_or(|a| !a.is_empty())
                });
                if matchers_arr.len() != before {
                    changed = true;
                }
            }
        }

        // Remove hook categories that are now empty arrays
        hooks.retain(|_k, v| v.as_array().is_none_or(|a| !a.is_empty()));
    }

    // Remove ig env vars
    if let Some(env) = parsed.get_mut("env").and_then(|e| e.as_object_mut()) {
        for var in IG_ENV_VARS {
            if env.remove(*var).is_some() {
                changed = true;
            }
        }
        // Remove env object if empty
        if env.is_empty() {
            // Can't remove here, mark for outer cleanup
        }
    }
    // Clean up empty env object
    if parsed
        .get("env")
        .and_then(|e| e.as_object())
        .is_some_and(|o| o.is_empty())
    {
        if let Some(obj) = parsed.as_object_mut() {
            obj.remove("env");
        }
        changed = true;
    }

    changed
}

/// Remove the `## Search Tools` section from markdown content.
/// Returns `Some(cleaned)` if found and removed, `None` if not found.
fn remove_search_tools_section(content: &str) -> Option<String> {
    let marker = "## Search Tools";
    let start = content.find(marker)?;

    // Find the end: next ## heading or end of file
    let after_marker = start + marker.len();
    let end = content[after_marker..]
        .find("\n## ")
        .map(|pos| after_marker + pos)
        .unwrap_or(content.len());

    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..start]);
    result.push_str(&content[end..]);

    // Clean up multiple consecutive newlines
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    Some(result.trim_end().to_string() + "\n")
}

// ─── Codex CLI ──────────────────────────────────────────────────────────────

fn remove_codex_config(codex_dir: &Path, dry_run: bool) -> Vec<RemoveResult> {
    let mut results = Vec::new();
    let md_path = codex_dir.join("AGENTS.md");

    if let Some(cleaned) = fs::read_to_string(&md_path)
        .ok()
        .and_then(|c| remove_search_tools_section(&c))
    {
        if !dry_run {
            let _ = fs::write(&md_path, &cleaned);
        }
        results.push(RemoveResult::Removed(
            "Removed Search Tools section from AGENTS.md".to_string(),
        ));
    }

    results
}

// ─── OpenCode ───────────────────────────────────────────────────────────────

fn remove_opencode_config(opencode_dir: &Path, dry_run: bool) -> Vec<RemoveResult> {
    let mut results = Vec::new();

    // AGENTS.md
    let md_path = opencode_dir.join("AGENTS.md");
    if let Some(cleaned) = fs::read_to_string(&md_path)
        .ok()
        .and_then(|c| remove_search_tools_section(&c))
    {
        if !dry_run {
            let _ = fs::write(&md_path, &cleaned);
        }
        results.push(RemoveResult::Removed(
            "Removed Search Tools section from AGENTS.md".to_string(),
        ));
    }

    // opencode.json — remove AGENTS.md from instructions array
    let json_path = opencode_dir.join("opencode.json");
    if let Some(mut parsed) = fs::read_to_string(&json_path)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        && let Some(instructions) = parsed
            .get_mut("instructions")
            .and_then(|i| i.as_array_mut())
    {
        let before = instructions.len();
        instructions.retain(|v| v.as_str().is_none_or(|s| !s.contains("AGENTS.md")));
        if instructions.len() != before {
            if !dry_run {
                let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
                let _ = fs::write(&json_path, format!("{formatted}\n"));
            }
            results.push(RemoveResult::Removed(
                "Removed AGENTS.md from opencode.json instructions".to_string(),
            ));
        }
    }

    results
}

// ─── Cursor ─────────────────────────────────────────────────────────────────

fn remove_cursor_config(cursor_dir: &Path, dry_run: bool) -> Vec<RemoveResult> {
    let mut results = Vec::new();
    let rule = cursor_dir.join("rules/ig-search.mdc");

    if rule.exists() {
        if !dry_run {
            let _ = fs::remove_file(&rule);
        }
        results.push(RemoveResult::Removed(
            "Removed rules/ig-search.mdc".to_string(),
        ));
    }

    results
}

// ─── Tracking data ──────────────────────────────────────────────────────────

fn remove_tracking_data(home: &Path, dry_run: bool) -> RemoveResult {
    let data_dir = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/ig")
    } else {
        home.join(".local/share/ig")
    };

    if !data_dir.is_dir() {
        return RemoveResult::NotFound("No tracking data found".to_string());
    }
    if !dry_run {
        let _ = fs::remove_dir_all(&data_dir);
    }
    RemoveResult::Removed(format!("Removed {}", data_dir.display()))
}

// ─── Binary ─────────────────────────────────────────────────────────────────

fn remove_binary(home: &Path, dry_run: bool) -> RemoveResult {
    let bin = home.join(".local/bin/ig");

    if !bin.exists() {
        return RemoveResult::NotFound("Binary not found at ~/.local/bin/ig".to_string());
    }
    // On Unix, removing a running binary is safe (inode stays until process exits)
    if !dry_run {
        let _ = fs::remove_file(&bin);
    }
    RemoveResult::Removed(format!("Removed {}", bin.display()))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_remove_search_tools_section_middle() {
        let content = "# CLAUDE.md\n\nSome intro.\n\n## Search Tools\n- Use ig for search.\n- More stuff.\n\n## Other Section\n\nKeep this.\n";
        let result = remove_search_tools_section(content).unwrap();
        assert!(!result.contains("Search Tools"));
        assert!(!result.contains("Use ig"));
        assert!(result.contains("Some intro."));
        assert!(result.contains("## Other Section"));
        assert!(result.contains("Keep this."));
    }

    #[test]
    fn test_remove_search_tools_section_at_end() {
        let content = "# CLAUDE.md\n\nSome intro.\n\n## Search Tools\n- Use ig for search.\n";
        let result = remove_search_tools_section(content).unwrap();
        assert!(!result.contains("Search Tools"));
        assert!(result.contains("Some intro."));
    }

    #[test]
    fn test_remove_search_tools_section_not_found() {
        let content = "# CLAUDE.md\n\nNo tools here.\n";
        assert!(remove_search_tools_section(content).is_none());
    }

    #[test]
    fn test_clean_settings_json_removes_permission() {
        let json = r#"{"permissions":{"allow":["Bash(git *)","Bash(ig *)","Read(*)"]}}"#;
        let mut parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(clean_settings_json(&mut parsed));

        let allow = parsed["permissions"]["allow"].as_array().unwrap();
        assert_eq!(allow.len(), 2);
        assert!(allow.iter().all(|v| v.as_str() != Some("Bash(ig *)")));
        assert!(allow.iter().any(|v| v.as_str() == Some("Bash(git *)")));
    }

    #[test]
    fn test_clean_settings_json_removes_hooks() {
        let json = r#"{
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {"type": "command", "command": "~/.claude/hooks/ig-guard.sh"},
                            {"type": "command", "command": "echo custom hook"}
                        ]
                    },
                    {
                        "matcher": "Grep",
                        "hooks": [
                            {"type": "command", "command": "echo 'BLOCK: Use ig via Bash instead'"}
                        ]
                    }
                ]
            }
        }"#;
        let mut parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(clean_settings_json(&mut parsed));

        // The custom hook should remain
        let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1); // Grep matcher removed (empty after cleanup)
        let bash_hooks = pre[0]["hooks"].as_array().unwrap();
        assert_eq!(bash_hooks.len(), 1);
        assert!(
            bash_hooks[0]["command"]
                .as_str()
                .unwrap()
                .contains("custom hook")
        );
    }

    #[test]
    fn test_clean_settings_json_removes_env_vars() {
        let json = r#"{
            "env": {
                "CLAUDE_CODE_EFFORT_LEVEL": "medium",
                "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE": "70",
                "MY_CUSTOM_VAR": "keep"
            }
        }"#;
        let mut parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(clean_settings_json(&mut parsed));

        let env = parsed["env"].as_object().unwrap();
        assert_eq!(env.len(), 1);
        assert!(env.contains_key("MY_CUSTOM_VAR"));
    }

    #[test]
    fn test_clean_settings_json_removes_empty_env() {
        let json = r#"{
            "env": {
                "CLAUDE_CODE_EFFORT_LEVEL": "medium"
            }
        }"#;
        let mut parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(clean_settings_json(&mut parsed));
        assert!(parsed.get("env").is_none());
    }

    #[test]
    fn test_clean_settings_json_no_changes() {
        let json = r#"{"permissions":{"allow":["Bash(git *)"]}}"#;
        let mut parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(!clean_settings_json(&mut parsed));
    }

    #[test]
    fn test_remove_claude_config_full() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path();

        // Create hooks
        let hooks = claude.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        for f in IG_HOOK_FILES {
            fs::write(hooks.join(f), "#!/bin/bash\necho hook").unwrap();
        }

        // Create settings.json with ig entries
        fs::write(
            claude.join("settings.json"),
            r#"{
                "permissions": {"allow": ["Bash(git *)", "Bash(ig *)"]},
                "hooks": {
                    "PreToolUse": [{
                        "matcher": "Bash",
                        "hooks": [
                            {"type": "command", "command": "~/.claude/hooks/ig-guard.sh"}
                        ]
                    }]
                },
                "env": {"CLAUDE_CODE_EFFORT_LEVEL": "medium"}
            }"#,
        )
        .unwrap();

        // Create CLAUDE.md
        fs::write(
            claude.join("CLAUDE.md"),
            "# CLAUDE.md\n\n## Search Tools\n- Use ig.\n\n## Other\nKeep.\n",
        )
        .unwrap();

        let results = remove_claude_config(claude, false);
        assert!(results.len() >= 3); // hooks + settings + md

        // Verify hooks deleted
        for f in IG_HOOK_FILES {
            assert!(!hooks.join(f).exists(), "{} should be deleted", f);
        }
        // Hooks directory should still exist
        assert!(hooks.exists());

        // Verify settings.json cleaned
        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(claude.join("settings.json")).unwrap())
                .unwrap();
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        assert_eq!(allow.len(), 1);
        assert!(
            settings.get("hooks").is_none() || settings["hooks"].as_object().unwrap().is_empty()
        );

        // Verify CLAUDE.md cleaned
        let md = fs::read_to_string(claude.join("CLAUDE.md")).unwrap();
        assert!(!md.contains("Search Tools"));
        assert!(md.contains("## Other"));
    }

    #[test]
    fn test_remove_codex_config() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("AGENTS.md"),
            "# AGENTS.md\n\n## Search Tools\n- Use ig.\n\n## Other\nKeep.\n",
        )
        .unwrap();

        let results = remove_codex_config(dir.path(), false);
        assert_eq!(results.len(), 1);

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(!content.contains("Search Tools"));
        assert!(content.contains("## Other"));
    }

    #[test]
    fn test_remove_opencode_config() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("AGENTS.md"),
            "# AGENTS.md\n\n## Search Tools\n- Use ig.\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("opencode.json"),
            r#"{"instructions":["AGENTS.md","other.md"]}"#,
        )
        .unwrap();

        let results = remove_opencode_config(dir.path(), false);
        assert_eq!(results.len(), 2);

        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("opencode.json")).unwrap())
                .unwrap();
        let instructions = json["instructions"].as_array().unwrap();
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0].as_str().unwrap(), "other.md");
    }

    #[test]
    fn test_remove_cursor_config() {
        let dir = TempDir::new().unwrap();
        let rules = dir.path().join("rules");
        fs::create_dir_all(&rules).unwrap();
        fs::write(rules.join("ig-search.mdc"), "ig rules").unwrap();

        let results = remove_cursor_config(dir.path(), false);
        assert_eq!(results.len(), 1);
        assert!(!rules.join("ig-search.mdc").exists());
        assert!(rules.exists()); // directory preserved
    }

    #[test]
    fn test_remove_tracking_data() {
        let dir = TempDir::new().unwrap();
        let data = if cfg!(target_os = "macos") {
            dir.path().join("Library/Application Support/ig")
        } else {
            dir.path().join(".local/share/ig")
        };
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("history.jsonl"), "{}").unwrap();

        let result = remove_tracking_data(dir.path(), false);
        assert!(matches!(result, RemoveResult::Removed(_)));
        assert!(!data.exists());
    }

    #[test]
    fn test_remove_binary() {
        let dir = TempDir::new().unwrap();
        let bin_dir = dir.path().join(".local/bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("ig"), "binary").unwrap();

        let result = remove_binary(dir.path(), false);
        assert!(matches!(result, RemoveResult::Removed(_)));
        assert!(!bin_dir.join("ig").exists());
    }

    #[test]
    fn test_dry_run_makes_no_changes() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path();

        // Setup full config
        let hooks = claude.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(hooks.join("ig-guard.sh"), "hook").unwrap();
        fs::write(
            claude.join("settings.json"),
            r#"{"permissions":{"allow":["Bash(ig *)"]}}"#,
        )
        .unwrap();
        fs::write(
            claude.join("CLAUDE.md"),
            "# CLAUDE.md\n\n## Search Tools\n- ig.\n",
        )
        .unwrap();

        // Run with dry_run = true
        let _ = remove_claude_config(claude, true);

        // Everything should still exist unchanged
        assert!(hooks.join("ig-guard.sh").exists());
        let settings = fs::read_to_string(claude.join("settings.json")).unwrap();
        assert!(settings.contains("Bash(ig *)"));
        let md = fs::read_to_string(claude.join("CLAUDE.md")).unwrap();
        assert!(md.contains("Search Tools"));
    }
}
