use std::fs;
use std::path::{Path, PathBuf};

use crate::hooks::copilot;

const IG_SEARCH_TOOLS_SECTION: &str = "\n## Search Tools\n\
- **Code search**: prefer `ig` (instant-grep) over `rg` or `grep` for searching code.\n\
- Usage: `ig \"pattern\" [path]` or `ig search \"pattern\" [path]` — trigram-indexed regex search.\n\
- If the project has no `.ig/` index yet, `ig` auto-builds one on first search.\n\
- **Project overview**: read `.ig/context.md` for a complete project map (tree + file summaries + symbols).\n\
- **Smart read**: `ig read <file> --signatures` for imports and function signatures only.\n\
- **Smart summary**: `ig smart [path]` for 2-line file summaries.\n\
- Fall back to `rg` only if `ig` is not installed.\n";

const IG_PERMISSION: &str = "Bash(ig *)";

const IG_GUARD_HOOK: &str = include_str!("../hooks/ig-guard.sh");
const IG_SESSION_START_HOOK: &str = include_str!("../hooks/session-start.sh");
const IG_FORMAT_HOOK: &str = include_str!("../hooks/format.sh");
const IG_SUBAGENT_CONTEXT_HOOK: &str = include_str!("../hooks/subagent-context.sh");
const IG_CURSORRULES_SNIPPET: &str = include_str!("../hooks/cursorrules-snippet.txt");

const IG_EXPLORER_AGENT: &str = "\
---
name: explorer
description: Explores codebases to answer questions, find patterns, and map dependencies. Read-only, never modifies files. Use to understand unfamiliar code or find specific implementations. Replaces the built-in Explore subagent with git-aware capabilities and sonnet model.
model: sonnet
effort: medium
tools: Read, Glob, Bash(ig *), Bash(git log *), Bash(git show *), Bash(git blame *), Bash(git diff *), Bash(wc *), Bash(ls *), Bash(tree *), Bash(find *), Bash(cat *), Bash(head *), Bash(tail *), Bash(jq *), WebFetch, WebSearch
initialPrompt: |
  SEARCH STRATEGY (mandatory order):
  1. ig symbols | grep KEYWORD — find all class/function definitions matching the concept
  2. ig -l \"KEYWORD\" — list all files containing the keyword
  3. Read the KEY files (config, controllers, models, services) — not all of them
  4. Only then do targeted ig searches for specific patterns

  This 4-step approach covers 100% of a concept in <500ms instead of 69 sequential reads.
  Use ig for ALL code search. Never use grep, rg, or find. Never use the Grep tool.
---
";

enum ConfigResult {
    Configured(String),
    AlreadyDone(String),
    Error(String),
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn write_if_not_dry(path: &Path, content: &[u8], dry_run: bool) -> Result<(), String> {
    if dry_run {
        Ok(())
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
        }
        fs::write(path, content).map_err(|e| format!("write {}: {}", path.display(), e))
    }
}

fn install_hook_file(hooks_dir: &Path, name: &str, content: &str, dry_run: bool) -> ConfigResult {
    let hook_path = hooks_dir.join(name);
    if hook_path.exists() {
        return ConfigResult::AlreadyDone(format!("{} already installed", name));
    }
    match write_if_not_dry(&hook_path, content.as_bytes(), dry_run) {
        Ok(_) => {
            if !dry_run {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755));
                }
            }
            ConfigResult::Configured(format!("Installed {}", name))
        }
        Err(e) => ConfigResult::Error(format!("Could not install {}: {}", name, e)),
    }
}

/// Ensure a specific hook command is registered under a specific matcher in settings.json.
/// Returns true if a new entry was added.
fn ensure_hook_registered(
    parsed: &mut serde_json::Value,
    event: &str,
    matcher: &str,
    hook_cmd: &str,
    marker: &str,
    timeout: Option<u32>,
) -> bool {
    // Ensure hooks.{event} exists
    if parsed.get("hooks").is_none() {
        parsed["hooks"] = serde_json::json!({});
    }
    if parsed["hooks"].get(event).is_none() {
        parsed["hooks"][event] = serde_json::json!([]);
    }

    let event_arr = match parsed["hooks"][event].as_array_mut() {
        Some(arr) => arr,
        None => return false,
    };

    // Find or create matcher entry
    let matcher_idx = event_arr
        .iter()
        .position(|e| e.get("matcher").and_then(|m| m.as_str()) == Some(matcher));

    if matcher_idx.is_none() {
        event_arr.push(serde_json::json!({"matcher": matcher, "hooks": []}));
    }

    let matcher_idx = event_arr
        .iter()
        .position(|e| e.get("matcher").and_then(|m| m.as_str()) == Some(matcher))
        .unwrap();

    let entry = &mut event_arr[matcher_idx];
    if entry.get("hooks").is_none() {
        entry["hooks"] = serde_json::json!([]);
    }

    let hooks = match entry["hooks"].as_array_mut() {
        Some(arr) => arr,
        None => return false,
    };

    // Check if already present
    let already = hooks.iter().any(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .contains(marker)
    });

    if already {
        return false;
    }

    // Add the hook
    let mut hook_obj = serde_json::json!({"type": "command", "command": hook_cmd});
    if let Some(t) = timeout {
        hook_obj["timeout"] = serde_json::json!(t);
    }
    hooks.push(hook_obj);
    true
}

fn which_exists(binary: &str) -> bool {
    std::process::Command::new("which")
        .arg(binary)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ─── Claude Code — full hook suite ───────────────────────────────────────────

fn configure_claude_hooks_full(claude_dir: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let mut results = Vec::new();
    let hooks_dir = claude_dir.join("hooks");

    // Migrate: remove old split hooks (replaced by ig-guard.sh in v2.1)
    let old_hooks = ["ig-rewrite.sh", "prefer-ig.sh", "find-rewrite.sh"];
    for old in &old_hooks {
        let old_path = hooks_dir.join(old);
        if old_path.exists() {
            let _ = fs::remove_file(&old_path);
            results.push(ConfigResult::Configured(format!(
                "Migrated: removed old {}",
                old
            )));
        }
    }

    // Install hook files
    results.push(install_hook_file(
        &hooks_dir,
        "ig-guard.sh",
        IG_GUARD_HOOK,
        dry_run,
    ));
    results.push(install_hook_file(
        &hooks_dir,
        "session-start.sh",
        IG_SESSION_START_HOOK,
        dry_run,
    ));
    results.push(install_hook_file(
        &hooks_dir,
        "format.sh",
        IG_FORMAT_HOOK,
        dry_run,
    ));
    results.push(install_hook_file(
        &hooks_dir,
        "subagent-context.sh",
        IG_SUBAGENT_CONTEXT_HOOK,
        dry_run,
    ));

    // Register all hooks in settings.json
    let settings_path = claude_dir.join("settings.json");
    let content = fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string());
    let mut parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            results.push(ConfigResult::Error(
                "Could not parse settings.json for hooks".to_string(),
            ));
            return results;
        }
    };

    let mut changes = 0u32;

    // Migrate: remove old hook entries from settings.json
    let old_markers = ["ig-rewrite.sh", "prefer-ig.sh", "find-rewrite.sh"];
    if let Some(hooks_obj) = parsed.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (_key, matchers) in hooks_obj.iter_mut() {
            if let Some(matchers_arr) = matchers.as_array_mut() {
                for matcher in matchers_arr.iter_mut() {
                    if let Some(hook_list) = matcher.get_mut("hooks").and_then(|h| h.as_array_mut())
                    {
                        let before = hook_list.len();
                        hook_list.retain(|hook| {
                            let cmd = hook.get("command").and_then(|c| c.as_str()).unwrap_or("");
                            !old_markers.iter().any(|m| cmd.contains(m))
                        });
                        if hook_list.len() != before {
                            changes += 1;
                        }
                    }
                }
            }
        }
    }

    // PreToolUse/Bash — destructive git blocker
    let destructive_git_cmd = r#"echo "$CLAUDE_BASH_COMMAND" | grep -qE '(git reset --hard|git checkout \.|git clean -f|--force|--no-verify)' && echo 'BLOCK: Destructive git command detected. Confirm with user first.' >&2 && exit 2 || exit 0"#;
    if ensure_hook_registered(
        &mut parsed,
        "PreToolUse",
        "Bash",
        destructive_git_cmd,
        "Destructive git",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered destructive git blocker".to_string(),
        ));
        changes += 1;
    }

    // PreToolUse/Bash — npm/npx blocker
    let npm_cmd = r#"echo "$CLAUDE_BASH_COMMAND" | grep -qE '^(npm |npx )' && echo 'BLOCK: Use bun/bunx instead of npm/npx (global rule).' >&2 && exit 2 || exit 0"#;
    if ensure_hook_registered(
        &mut parsed,
        "PreToolUse",
        "Bash",
        npm_cmd,
        "bun/bunx instead",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered npm/npx blocker".to_string(),
        ));
        changes += 1;
    }

    // PreToolUse/Bash — ig-guard.sh (blocking + rewriting)
    if ensure_hook_registered(
        &mut parsed,
        "PreToolUse",
        "Bash",
        "~/.claude/hooks/ig-guard.sh",
        "ig-guard.sh",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered ig-guard.sh hook".to_string(),
        ));
        changes += 1;
    }

    // PreToolUse/Grep — blocker
    let grep_cmd = "echo 'BLOCK: Use ig via Bash instead of the Grep tool. Examples:' >&2 && echo '  ig \"pattern\" [path]        # content search' >&2 && echo '  ig -i \"pattern\"            # case-insensitive' >&2 && echo '  ig -t rs \"pattern\"         # filter by file type' >&2 && echo '  ig -l \"pattern\"            # file paths only' >&2 && echo '  ig -c \"pattern\"            # match count' >&2 && exit 2";
    if ensure_hook_registered(
        &mut parsed,
        "PreToolUse",
        "Grep",
        grep_cmd,
        "Use ig via Bash",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered Grep blocker".to_string(),
        ));
        changes += 1;
    }

    // PostToolUse/Write|Edit — format.sh
    if ensure_hook_registered(
        &mut parsed,
        "PostToolUse",
        "Write|Edit",
        "~/.claude/hooks/format.sh",
        "format.sh",
        Some(10),
    ) {
        results.push(ConfigResult::Configured(
            "Registered format.sh hook".to_string(),
        ));
        changes += 1;
    }

    // PostToolUse/Write|Edit — .env warning
    let env_cmd = r#"echo "$CLAUDE_FILE_PATH" | grep -qE '\.env' && echo 'WARNING: Modifying .env file. Ensure no secrets are hardcoded and file is in .gitignore.' >&2 || true"#;
    if ensure_hook_registered(
        &mut parsed,
        "PostToolUse",
        "Write|Edit",
        env_cmd,
        ".env",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered .env warning hook".to_string(),
        ));
        changes += 1;
    }

    // SessionStart — session-start.sh
    if ensure_hook_registered(
        &mut parsed,
        "SessionStart",
        "*",
        "~/.claude/hooks/session-start.sh",
        "session-start.sh",
        Some(5),
    ) {
        results.push(ConfigResult::Configured(
            "Registered session-start.sh hook".to_string(),
        ));
        changes += 1;
    }

    // UserPromptSubmit — secret detection
    let secret_cmd = r#"printf '%s' "$CLAUDE_USER_PROMPT" | grep -qiE '(sk-[a-zA-Z0-9]{20,}|AKIA[A-Z0-9]{16}|ghp_[a-zA-Z0-9]{36})' && echo 'BLOCK: Potential secret detected in prompt. Remove credentials before sending.' >&2 && exit 2 || exit 0"#;
    if ensure_hook_registered(
        &mut parsed,
        "UserPromptSubmit",
        "*",
        secret_cmd,
        "secret detected",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered secret detection hook".to_string(),
        ));
        changes += 1;
    }

    // SubagentStart — subagent-context.sh
    if ensure_hook_registered(
        &mut parsed,
        "SubagentStart",
        "*",
        "~/.claude/hooks/subagent-context.sh",
        "subagent-context.sh",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered subagent-context.sh hook".to_string(),
        ));
        changes += 1;
    }

    // Write settings.json if changes were made
    if changes > 0 && !dry_run {
        let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
        if fs::write(&settings_path, format!("{}\n", formatted)).is_err() {
            results.push(ConfigResult::Error(
                "Could not write settings.json".to_string(),
            ));
        }
    }

    if changes == 0 {
        results.push(ConfigResult::AlreadyDone(
            "All hooks already registered".to_string(),
        ));
    }

    // Install explorer agent definition
    results.push(install_explorer_agent(claude_dir, dry_run));

    results
}

// ─── Claude Code — env vars ───────────────────────────────────────────────────

fn configure_claude_env_vars(claude_dir: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let mut results = Vec::new();
    let settings_path = claude_dir.join("settings.json");

    let content = fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string());
    let mut parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            results.push(ConfigResult::Error(
                "Could not parse settings.json for env vars".to_string(),
            ));
            return results;
        }
    };

    if parsed.get("env").is_none() {
        parsed["env"] = serde_json::json!({});
    }

    let env = parsed["env"].as_object_mut().unwrap();
    let mut changed = false;

    if !env.contains_key("CLAUDE_CODE_EFFORT_LEVEL") {
        env.insert(
            "CLAUDE_CODE_EFFORT_LEVEL".to_string(),
            serde_json::json!("medium"),
        );
        results.push(ConfigResult::Configured(
            "Set CLAUDE_CODE_EFFORT_LEVEL=medium".to_string(),
        ));
        changed = true;
    } else {
        results.push(ConfigResult::AlreadyDone(
            "CLAUDE_CODE_EFFORT_LEVEL already set".to_string(),
        ));
    }

    if !env.contains_key("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE") {
        env.insert(
            "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE".to_string(),
            serde_json::json!("70"),
        );
        results.push(ConfigResult::Configured(
            "Set CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=70".to_string(),
        ));
        changed = true;
    } else {
        results.push(ConfigResult::AlreadyDone(
            "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE already set".to_string(),
        ));
    }

    if changed && !dry_run {
        let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
        if fs::write(&settings_path, format!("{}\n", formatted)).is_err() {
            results.push(ConfigResult::Error(
                "Could not write settings.json".to_string(),
            ));
        }
    }

    results
}

// ─── Claude Code — explorer agent ────────────────────────────────────────────

fn install_explorer_agent(claude_dir: &Path, dry_run: bool) -> ConfigResult {
    let agents_dir = claude_dir.join("agents");
    let agent_path = agents_dir.join("explorer.md");

    if agent_path.exists() {
        // Check if the tools: line contains the outdated Grep tool reference
        let content = fs::read_to_string(&agent_path).unwrap_or_default();
        let has_grep_tool = content
            .lines()
            .any(|line| line.starts_with("tools:") && line.contains("Grep"));
        if has_grep_tool {
            // Update: remove Grep from tools (blocked by hook, wastes agent turns)
            let updated = content.replace(", Grep,", ",").replace("Grep, ", "");
            match write_if_not_dry(&agent_path, updated.as_bytes(), dry_run) {
                Ok(_) => {
                    return ConfigResult::Configured(
                        "Updated explorer.md: removed Grep tool (blocked by hook)".to_string(),
                    );
                }
                Err(e) => return ConfigResult::Error(e),
            }
        }
        return ConfigResult::AlreadyDone("agents/explorer.md already installed".to_string());
    }

    match write_if_not_dry(&agent_path, IG_EXPLORER_AGENT.as_bytes(), dry_run) {
        Ok(_) => ConfigResult::Configured("Installed agents/explorer.md".to_string()),
        Err(e) => ConfigResult::Error(e),
    }
}

// ─── OpenCode ─────────────────────────────────────────────────────────────────

fn configure_opencode(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let mut results = Vec::new();
    let config_dir = home.join(".config/opencode");

    // Write AGENTS.md
    let agents_md_path = config_dir.join("AGENTS.md");
    let agents_content = fs::read_to_string(&agents_md_path).unwrap_or_default();

    if agents_content.contains("## Search Tools") && agents_content.contains("ig") {
        results.push(ConfigResult::AlreadyDone(
            "AGENTS.md already has ig instructions".to_string(),
        ));
    } else {
        let new_content = if agents_content.is_empty() {
            format!("# AGENTS.md\n{}", IG_SEARCH_TOOLS_SECTION)
        } else {
            format!("{}\n{}", agents_content.trim_end(), IG_SEARCH_TOOLS_SECTION)
        };
        match write_if_not_dry(&agents_md_path, new_content.as_bytes(), dry_run) {
            Ok(_) => results.push(ConfigResult::Configured(
                "Added ig instructions to AGENTS.md".to_string(),
            )),
            Err(e) => results.push(ConfigResult::Error(e)),
        }
    }

    // Update opencode.json instructions array
    let json_path = config_dir.join("opencode.json");
    let json_content = fs::read_to_string(&json_path).unwrap_or_else(|_| "{}".to_string());

    match serde_json::from_str::<serde_json::Value>(&json_content) {
        Ok(mut parsed) => {
            if parsed.get("instructions").is_none() {
                parsed["instructions"] = serde_json::json!([]);
            }

            let instructions = parsed["instructions"].as_array_mut().unwrap();
            let agents_md_str = agents_md_path.to_string_lossy().to_string();
            let already = instructions
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("AGENTS.md"));

            if already {
                results.push(ConfigResult::AlreadyDone(
                    "AGENTS.md already in opencode.json instructions".to_string(),
                ));
            } else {
                instructions.push(serde_json::json!(agents_md_str));
                if !dry_run {
                    let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
                    if fs::write(&json_path, format!("{}\n", formatted)).is_err() {
                        results.push(ConfigResult::Error(
                            "Could not write opencode.json".to_string(),
                        ));
                    } else {
                        results.push(ConfigResult::Configured(
                            "Added AGENTS.md to opencode.json instructions".to_string(),
                        ));
                    }
                } else {
                    results.push(ConfigResult::Configured(
                        "Would add AGENTS.md to opencode.json".to_string(),
                    ));
                }
            }
        }
        Err(_) => results.push(ConfigResult::Error(
            "Could not parse opencode.json".to_string(),
        )),
    }

    results
}

// ─── Cursor ───────────────────────────────────────────────────────────────────

fn configure_cursor(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let cursor_dir = home.join(".cursor");
    let rules_dir = cursor_dir.join("rules");
    let mdc_path = rules_dir.join("ig-search.mdc");

    if mdc_path.exists() {
        return vec![ConfigResult::AlreadyDone(
            "ig-search.mdc already exists".to_string(),
        )];
    }

    match write_if_not_dry(&mdc_path, IG_CURSORRULES_SNIPPET.as_bytes(), dry_run) {
        Ok(_) => vec![ConfigResult::Configured(
            "Created ~/.cursor/rules/ig-search.mdc".to_string(),
        )],
        Err(e) => vec![ConfigResult::Error(e)],
    }
}

// ─── Copilot ────────────────────────────────────────────────────────────────

fn configure_copilot(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let config_path = home.join(".github/copilot-instructions.md");
    let project_path = PathBuf::from(".github/copilot-instructions.md");

    // Use project-local if .github/ exists, otherwise user-level
    let target = if PathBuf::from(".github").is_dir() {
        &project_path
    } else {
        &config_path
    };

    if target.exists() {
        return vec![ConfigResult::AlreadyDone(format!(
            "{} already exists",
            target.display()
        ))];
    }

    let content = copilot::copilot_instructions();
    match write_if_not_dry(target, content.as_bytes(), dry_run) {
        Ok(_) => vec![ConfigResult::Configured(format!(
            "Created {}",
            target.display()
        ))],
        Err(e) => vec![ConfigResult::Error(e)],
    }
}

// ─── Windsurf ───────────────────────────────────────────────────────────────

fn configure_windsurf(_home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let target = PathBuf::from(".windsurfrules");

    if target.exists() {
        return vec![ConfigResult::AlreadyDone(
            ".windsurfrules already exists".to_string(),
        )];
    }

    let content = copilot::windsurf_rules();
    match write_if_not_dry(&target, content.as_bytes(), dry_run) {
        Ok(_) => vec![ConfigResult::Configured(
            "Created .windsurfrules".to_string(),
        )],
        Err(e) => vec![ConfigResult::Error(e)],
    }
}

// ─── Cline ──────────────────────────────────────────────────────────────────

fn configure_cline(_home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let target = PathBuf::from(".clinerules");

    if target.exists() {
        return vec![ConfigResult::AlreadyDone(
            ".clinerules already exists".to_string(),
        )];
    }

    let content = copilot::cline_rules();
    match write_if_not_dry(&target, content.as_bytes(), dry_run) {
        Ok(_) => vec![ConfigResult::Configured("Created .clinerules".to_string())],
        Err(e) => vec![ConfigResult::Error(e)],
    }
}

/// Resolve the real user's home directory, even when running under sudo.
pub(crate) fn resolve_real_home() -> Option<PathBuf> {
    // If SUDO_USER is set, we're running under sudo — use the real user's home
    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        // Validate username to prevent shell injection
        if !sudo_user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            eprintln!("Warning: SUDO_USER contains invalid characters, ignoring");
        } else {
            // Try /etc/passwd lookup via getent (Linux)
            if let Some(home_dir) = std::process::Command::new("getent")
                .args(["passwd", &sudo_user])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    let line = String::from_utf8_lossy(&o.stdout).to_string();
                    line.split(':').nth(5).map(|h| PathBuf::from(h.trim()))
                })
            {
                return Some(home_dir);
            }
            // Fallback: expand ~user via shell (safe — username validated above)
            if let Some(home) = std::process::Command::new("sh")
                .args(["-c", &format!("eval echo ~{}", sudo_user)])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|h| !h.is_empty())
            {
                return Some(PathBuf::from(home));
            }
        }
    }
    // Default: use HOME
    std::env::var("HOME").ok().map(PathBuf::from)
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run_setup(dry_run: bool) {
    if dry_run {
        eprintln!("\x1b[1;33m🔧 ig setup [DRY RUN] — Showing what would be configured...\x1b[0m\n");
    } else {
        eprintln!("\x1b[1m🔧 ig setup — Configuring AI CLI agents...\x1b[0m\n");
    }

    // When running under sudo, resolve the real user's home directory
    let home = match resolve_real_home() {
        Some(h) => h,
        None => {
            eprintln!("✗ Could not determine HOME directory");
            return;
        }
    };

    let mut configured = 0u32;

    // --- Claude Code ---
    let claude_dir = home.join(".claude");
    if claude_dir.is_dir() || which_exists("claude") {
        if !claude_dir.is_dir() {
            let _ = fs::create_dir_all(&claude_dir);
        }
        let mut actions = Vec::new();

        // Existing: permissions + CLAUDE.md
        actions.push(configure_claude_settings(&claude_dir));
        actions.push(configure_claude_md(&claude_dir));

        // NEW: Full hook suite
        actions.extend(configure_claude_hooks_full(&claude_dir, dry_run));

        // NEW: Env vars
        actions.extend(configure_claude_env_vars(&claude_dir, dry_run));

        eprintln!("\x1b[32m✓ Claude Code\x1b[0m");
        for action in &actions {
            match action {
                ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
                ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
                ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
            }
        }
        configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ Claude Code — not detected\x1b[0m");
    }

    // --- Codex CLI ---
    let codex_dir = home.join(".codex");
    if codex_dir.is_dir() || which_exists("codex") {
        if !codex_dir.is_dir() {
            let _ = fs::create_dir_all(&codex_dir);
        }
        let result = configure_codex_agents_md(&codex_dir);
        eprintln!("\x1b[32m✓ Codex CLI\x1b[0m");
        match &result {
            ConfigResult::Configured(msg) | ConfigResult::AlreadyDone(msg) => {
                eprintln!("  → {}", msg)
            }
            ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
        }
        configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ Codex CLI — not detected\x1b[0m");
    }

    // --- OpenCode ---
    let opencode_dir = home.join(".config/opencode");
    if opencode_dir.is_dir() || which_exists("opencode") {
        let actions = configure_opencode(&home, dry_run);
        eprintln!("\x1b[32m✓ OpenCode\x1b[0m");
        for action in &actions {
            match action {
                ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
                ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
                ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
            }
        }
        configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ OpenCode — not detected\x1b[0m");
    }

    // --- Cursor ---
    let cursor_dir = home.join(".cursor");
    if cursor_dir.is_dir() {
        let actions = configure_cursor(&home, dry_run);
        eprintln!("\x1b[32m✓ Cursor\x1b[0m");
        for action in &actions {
            match action {
                ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
                ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
                ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
            }
        }
        configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ Cursor — not detected\x1b[0m");
    }

    // --- Copilot ---
    let github_dir = home.join(".github");
    let project_github = PathBuf::from(".github");
    if github_dir.is_dir() || project_github.is_dir() {
        let actions = configure_copilot(&home, dry_run);
        eprintln!("\x1b[32m✓ GitHub Copilot\x1b[0m");
        for action in &actions {
            match action {
                ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
                ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
                ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
            }
        }
        configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ GitHub Copilot — not detected\x1b[0m");
    }

    // --- Windsurf ---
    let windsurf_dir = home.join(".windsurf");
    let project_windsurf = PathBuf::from(".windsurf");
    if windsurf_dir.is_dir() || project_windsurf.is_dir() {
        let actions = configure_windsurf(&home, dry_run);
        eprintln!("\x1b[32m✓ Windsurf\x1b[0m");
        for action in &actions {
            match action {
                ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
                ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
                ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
            }
        }
        configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ Windsurf — not detected\x1b[0m");
    }

    // --- Cline ---
    let cline_dir = home.join(".cline");
    let project_cline = PathBuf::from(".cline");
    if cline_dir.is_dir() || project_cline.is_dir() {
        let actions = configure_cline(&home, dry_run);
        eprintln!("\x1b[32m✓ Cline\x1b[0m");
        for action in &actions {
            match action {
                ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
                ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
                ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
            }
        }
        configured += 1;
    } else {
        eprintln!("\x1b[2m⊘ Cline — not detected\x1b[0m");
    }

    // --- Gemini CLI ---
    let gemini_dir = home.join(".gemini");
    if gemini_dir.is_dir() || which_exists("gemini") {
        eprintln!("\n\x1b[33mℹ Gemini CLI\x1b[0m (manual setup needed)");
        eprintln!("  Add to ~/.gemini/GEMINI.md or GEMINI.md in your project:");
        eprintln!("  \x1b[36mPrefer `ig \"pattern\"` over `rg` or `grep` for code search.\x1b[0m");
    }

    // --- Summary ---
    eprintln!();
    let prefix = if dry_run { "[DRY RUN] " } else { "" };
    eprintln!(
        "\x1b[1m{}Done!\x1b[0m ig configured for {} agent(s).",
        prefix, configured
    );
}

fn configure_claude_settings(claude_dir: &Path) -> ConfigResult {
    let settings_path = claude_dir.join("settings.json");

    let content = fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string());

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

    // Ensure permissions.allow array exists, creating it if needed
    if parsed.get("permissions").is_none() {
        parsed["permissions"] = serde_json::json!({ "allow": [] });
    } else if parsed["permissions"].get("allow").is_none() {
        parsed["permissions"]["allow"] = serde_json::json!([]);
    }

    if let Some(allow) = parsed
        .get_mut("permissions")
        .and_then(|p| p.get_mut("allow"))
        .and_then(|a| a.as_array_mut())
    {
        allow.push(serde_json::Value::String(IG_PERMISSION.to_string()));
    }

    let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
    if fs::write(&settings_path, formatted.as_bytes()).is_err() {
        return ConfigResult::Error("Could not write settings.json".to_string());
    }

    ConfigResult::Configured("Added Bash(ig *) permission to ~/.claude/settings.json".to_string())
}

fn configure_claude_md(claude_dir: &Path) -> ConfigResult {
    let md_path = claude_dir.join("CLAUDE.md");

    let content = fs::read_to_string(&md_path).unwrap_or_default();

    if content.contains("ig")
        && (content.contains("## Search Tools") || content.contains("## Search & Token"))
    {
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

    ConfigResult::Configured("Added Search Tools section to ~/.claude/CLAUDE.md".to_string())
}

fn configure_codex_agents_md(codex_dir: &Path) -> ConfigResult {
    let md_path = codex_dir.join("AGENTS.md");

    let content = fs::read_to_string(&md_path).unwrap_or_default();

    if content.contains("ig")
        && (content.contains("## Search Tools") || content.contains("## Search & Token"))
    {
        return ConfigResult::AlreadyDone(
            "Search Tools section already present in AGENTS.md".to_string(),
        );
    }

    let section = format!("# AGENTS.md\n{}", IG_SEARCH_TOOLS_SECTION);

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

    ConfigResult::Configured("Added Search Tools section to ~/.codex/AGENTS.md".to_string())
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
        assert!(
            content.contains("Bash(ig *)"),
            "should contain ig permission"
        );
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
    fn test_claude_settings_missing_file_creates_it() {
        let dir = TempDir::new().unwrap();
        // No settings.json written — file does not exist, should create with defaults
        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));
        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("Bash(ig *)"));
    }

    #[test]
    fn test_claude_settings_invalid_json_returns_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), "not json").unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Error(_)));
    }

    #[test]
    fn test_claude_settings_missing_allow_array_creates_it() {
        let dir = TempDir::new().unwrap();
        // Valid JSON but no permissions.allow array
        fs::write(dir.path().join("settings.json"), r#"{"other": "value"}"#).unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("Bash(ig *)"));
    }

    #[test]
    fn test_claude_settings_missing_allow_key_creates_it() {
        let dir = TempDir::new().unwrap();
        // Has permissions but no allow key
        fs::write(
            dir.path().join("settings.json"),
            r#"{"permissions":{"deny":[]}}"#,
        )
        .unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("Bash(ig *)"));
    }

    #[test]
    fn test_claude_settings_empty_json_creates_structure() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), r#"{}"#).unwrap();

        let result = configure_claude_settings(&dir.path().to_path_buf());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("Bash(ig *)"));
        assert!(content.contains("permissions"));
        assert!(content.contains("allow"));
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
        assert!(
            content.contains("## Search Tools"),
            "should contain Search Tools section"
        );
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
        assert!(
            content.contains("Some existing content."),
            "should preserve existing content"
        );
        assert!(
            content.contains("## Search Tools"),
            "should add Search Tools section"
        );
    }

    // --- new helper tests ---

    #[test]
    fn test_write_if_not_dry_dry_run() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let result = write_if_not_dry(&path, b"hello", true);
        assert!(result.is_ok());
        assert!(!path.exists(), "dry_run should not create file");
    }

    #[test]
    fn test_write_if_not_dry_writes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let result = write_if_not_dry(&path, b"hello", false);
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn test_write_if_not_dry_creates_parents() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a/b/c/test.txt");
        let result = write_if_not_dry(&path, b"hello", false);
        assert!(result.is_ok());
        assert!(path.exists());
    }

    #[test]
    fn test_install_hook_file_creates_and_marks_executable() {
        let dir = TempDir::new().unwrap();
        let result = install_hook_file(dir.path(), "myhook.sh", "#!/bin/sh\necho ok", false);
        assert!(matches!(result, ConfigResult::Configured(_)));
        assert!(dir.path().join("myhook.sh").exists());
    }

    #[test]
    fn test_install_hook_file_idempotent() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("myhook.sh"), "existing").unwrap();
        let result = install_hook_file(dir.path(), "myhook.sh", "new content", false);
        assert!(matches!(result, ConfigResult::AlreadyDone(_)));
        // Content should be unchanged
        assert_eq!(
            fs::read_to_string(dir.path().join("myhook.sh")).unwrap(),
            "existing"
        );
    }

    #[test]
    fn test_install_hook_file_dry_run() {
        let dir = TempDir::new().unwrap();
        let result = install_hook_file(dir.path(), "myhook.sh", "content", true);
        assert!(matches!(result, ConfigResult::Configured(_)));
        assert!(
            !dir.path().join("myhook.sh").exists(),
            "dry_run should not write file"
        );
    }

    #[test]
    fn test_ensure_hook_registered_adds_new() {
        let mut parsed = serde_json::json!({});
        let added = ensure_hook_registered(
            &mut parsed,
            "PreToolUse",
            "Bash",
            "echo hello",
            "hello",
            None,
        );
        assert!(added);
        let cmd = parsed["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(cmd, "echo hello");
    }

    #[test]
    fn test_ensure_hook_registered_idempotent() {
        let mut parsed = serde_json::json!({
            "hooks": {
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo hello"}]}]
            }
        });
        let added = ensure_hook_registered(
            &mut parsed,
            "PreToolUse",
            "Bash",
            "echo hello again",
            "hello",
            None,
        );
        assert!(!added, "should not add duplicate (marker already present)");
    }

    #[test]
    fn test_ensure_hook_registered_with_timeout() {
        let mut parsed = serde_json::json!({});
        ensure_hook_registered(
            &mut parsed,
            "SessionStart",
            "*",
            "~/.claude/hooks/session-start.sh",
            "session-start.sh",
            Some(5),
        );
        let hook = &parsed["hooks"]["SessionStart"][0]["hooks"][0];
        assert_eq!(hook["timeout"], 5);
        assert_eq!(hook["command"], "~/.claude/hooks/session-start.sh");
    }

    #[test]
    fn test_ensure_hook_registered_different_matchers() {
        let mut parsed = serde_json::json!({});
        ensure_hook_registered(
            &mut parsed,
            "PreToolUse",
            "Bash",
            "cmd-bash",
            "cmd-bash",
            None,
        );
        ensure_hook_registered(
            &mut parsed,
            "PreToolUse",
            "Grep",
            "cmd-grep",
            "cmd-grep",
            None,
        );
        ensure_hook_registered(
            &mut parsed,
            "PostToolUse",
            "Write|Edit",
            "cmd-write",
            "cmd-write",
            None,
        );

        let pre = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "should have Bash and Grep matchers");

        let post = parsed["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post.len(), 1, "should have Write|Edit matcher");
    }

    #[test]
    fn test_configure_claude_hooks_full_dry_run() {
        let dir = TempDir::new().unwrap();
        // No settings.json — start from scratch
        let results = configure_claude_hooks_full(dir.path(), true);
        // Should not fail, should produce Configured results for each hook
        let has_configured = results
            .iter()
            .any(|r| matches!(r, ConfigResult::Configured(_)));
        assert!(has_configured);
        // In dry_run mode, no files should be written
        assert!(!dir.path().join("hooks/ig-guard.sh").exists());
    }

    #[test]
    fn test_configure_claude_hooks_full_writes_files() {
        let dir = TempDir::new().unwrap();
        let results = configure_claude_hooks_full(dir.path(), false);
        let errors: Vec<_> = results
            .iter()
            .filter(|r| matches!(r, ConfigResult::Error(_)))
            .collect();
        assert!(errors.is_empty(), "no errors expected: {:?}", errors.len());
        assert!(dir.path().join("hooks/ig-guard.sh").exists());
        assert!(dir.path().join("hooks/session-start.sh").exists());
        assert!(dir.path().join("hooks/format.sh").exists());
    }

    #[test]
    fn test_configure_claude_hooks_full_idempotent() {
        let dir = TempDir::new().unwrap();
        // Run twice
        configure_claude_hooks_full(dir.path(), false);
        let results = configure_claude_hooks_full(dir.path(), false);
        // All results should be AlreadyDone (hooks installed + registered)
        let configured: Vec<_> = results
            .iter()
            .filter(|r| matches!(r, ConfigResult::Configured(_)))
            .collect();
        assert!(
            configured.is_empty(),
            "second run should produce no Configured results"
        );
    }

    #[test]
    fn test_configure_claude_env_vars_sets_keys() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), "{}").unwrap();
        let results = configure_claude_env_vars(dir.path(), false);
        let configured: Vec<_> = results
            .iter()
            .filter_map(|r| {
                if let ConfigResult::Configured(m) = r {
                    Some(m.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            configured
                .iter()
                .any(|m| m.contains("CLAUDE_CODE_EFFORT_LEVEL"))
        );
        assert!(
            configured
                .iter()
                .any(|m| m.contains("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE"))
        );

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("CLAUDE_CODE_EFFORT_LEVEL"));
        assert!(content.contains("medium"));
        assert!(content.contains("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE"));
        assert!(content.contains("70"));
    }

    #[test]
    fn test_configure_claude_env_vars_idempotent() {
        let dir = TempDir::new().unwrap();
        let settings =
            r#"{"env":{"CLAUDE_CODE_EFFORT_LEVEL":"high","CLAUDE_AUTOCOMPACT_PCT_OVERRIDE":"80"}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let results = configure_claude_env_vars(dir.path(), false);
        let already: Vec<_> = results
            .iter()
            .filter(|r| matches!(r, ConfigResult::AlreadyDone(_)))
            .collect();
        assert_eq!(already.len(), 2);
    }

    #[test]
    fn test_configure_claude_env_vars_dry_run() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), "{}").unwrap();
        configure_claude_env_vars(dir.path(), true);
        // File should remain unchanged
        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert_eq!(content, "{}");
    }

    #[test]
    fn test_configure_cursor_creates_mdc() {
        let dir = TempDir::new().unwrap();
        let results = configure_cursor(dir.path(), false);
        assert!(matches!(results[0], ConfigResult::Configured(_)));
        let mdc = dir.path().join(".cursor/rules/ig-search.mdc");
        assert!(mdc.exists());
        let content = fs::read_to_string(&mdc).unwrap();
        assert!(content.contains("ig"));
    }

    #[test]
    fn test_configure_cursor_idempotent() {
        let dir = TempDir::new().unwrap();
        configure_cursor(dir.path(), false);
        let results = configure_cursor(dir.path(), false);
        assert!(matches!(results[0], ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_configure_cursor_dry_run() {
        let dir = TempDir::new().unwrap();
        let results = configure_cursor(dir.path(), true);
        assert!(matches!(results[0], ConfigResult::Configured(_)));
        assert!(
            !dir.path().join(".cursor/rules/ig-search.mdc").exists(),
            "dry_run should not write"
        );
    }

    #[test]
    fn test_configure_opencode_creates_agents_md() {
        let dir = TempDir::new().unwrap();
        // Simulate ~/.config/opencode/ existing with empty opencode.json
        let opencode_dir = dir.path().join(".config/opencode");
        fs::create_dir_all(&opencode_dir).unwrap();
        fs::write(opencode_dir.join("opencode.json"), "{}").unwrap();

        let results = configure_opencode(dir.path(), false);
        let errors: Vec<_> = results
            .iter()
            .filter(|r| matches!(r, ConfigResult::Error(_)))
            .collect();
        assert!(errors.is_empty(), "no errors expected");

        let agents_md = opencode_dir.join("AGENTS.md");
        assert!(agents_md.exists());
        let content = fs::read_to_string(&agents_md).unwrap();
        assert!(content.contains("Search Tools"));
    }

    #[test]
    fn test_configure_opencode_dry_run() {
        let dir = TempDir::new().unwrap();
        let opencode_dir = dir.path().join(".config/opencode");
        fs::create_dir_all(&opencode_dir).unwrap();
        fs::write(opencode_dir.join("opencode.json"), "{}").unwrap();

        configure_opencode(dir.path(), true);
        // AGENTS.md should NOT be created in dry_run
        assert!(!opencode_dir.join("AGENTS.md").exists());
    }

    // --- explorer agent tests ---

    #[test]
    fn test_install_explorer_agent_creates_file() {
        let dir = TempDir::new().unwrap();
        let result = install_explorer_agent(dir.path(), false);
        assert!(matches!(result, ConfigResult::Configured(_)));
        let agent = dir.path().join("agents/explorer.md");
        assert!(agent.exists());
        let content = fs::read_to_string(&agent).unwrap();
        assert!(content.contains("name: explorer"));
        assert!(content.contains("Bash(ig *)"));
        // Must NOT have Grep in tools line
        let tools_line = content.lines().find(|l| l.starts_with("tools:")).unwrap();
        assert!(!tools_line.contains("Grep"), "tools must not contain Grep");
    }

    #[test]
    fn test_install_explorer_agent_idempotent() {
        let dir = TempDir::new().unwrap();
        install_explorer_agent(dir.path(), false);
        let result = install_explorer_agent(dir.path(), false);
        assert!(matches!(result, ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_install_explorer_agent_migrates_grep() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        // Write old version with Grep in tools
        fs::write(
            agents_dir.join("explorer.md"),
            "---\nname: explorer\ntools: Read, Grep, Glob, Bash(ig *)\ninitialPrompt: |\n  Use ig.\n---\n",
        )
        .unwrap();

        let result = install_explorer_agent(dir.path(), false);
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(agents_dir.join("explorer.md")).unwrap();
        let tools_line = content.lines().find(|l| l.starts_with("tools:")).unwrap();
        assert!(
            !tools_line.contains("Grep"),
            "Grep should be removed from tools after migration"
        );
    }

    #[test]
    fn test_install_explorer_agent_dry_run() {
        let dir = TempDir::new().unwrap();
        let result = install_explorer_agent(dir.path(), true);
        assert!(matches!(result, ConfigResult::Configured(_)));
        assert!(
            !dir.path().join("agents/explorer.md").exists(),
            "dry_run should not create file"
        );
    }

    // --- subagent-context hook tests ---

    #[test]
    fn test_subagent_context_hook_installed() {
        let dir = TempDir::new().unwrap();
        let results = configure_claude_hooks_full(dir.path(), false);
        assert!(dir.path().join("hooks/subagent-context.sh").exists());
        // Verify SubagentStart registered in settings.json
        let settings = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(settings.contains("SubagentStart"));
        assert!(settings.contains("subagent-context.sh"));
        let has_subagent = results.iter().any(|r| match r {
            ConfigResult::Configured(m) => m.contains("subagent-context"),
            _ => false,
        });
        assert!(has_subagent);
    }
}
