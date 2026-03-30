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

const IG_REWRITE_HOOK: &str = include_str!("../hooks/ig-rewrite.sh");
#[allow(dead_code)]
const IG_HOOK_MARKER: &str = "ig-rewrite.sh";

const IG_PREFER_HOOK: &str = include_str!("../hooks/prefer-ig.sh");
const IG_SESSION_START_HOOK: &str = include_str!("../hooks/session-start.sh");
const IG_FORMAT_HOOK: &str = include_str!("../hooks/format.sh");
const IG_CURSORRULES_SNIPPET: &str = include_str!("../hooks/cursorrules-snippet.txt");

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

    // Install hook files
    results.push(install_hook_file(
        &hooks_dir,
        "ig-rewrite.sh",
        IG_REWRITE_HOOK,
        dry_run,
    ));
    results.push(install_hook_file(
        &hooks_dir,
        "prefer-ig.sh",
        IG_PREFER_HOOK,
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

    // PreToolUse/Bash — ig-rewrite.sh
    if ensure_hook_registered(
        &mut parsed,
        "PreToolUse",
        "Bash",
        "~/.claude/hooks/ig-rewrite.sh",
        "ig-rewrite.sh",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered ig-rewrite.sh hook".to_string(),
        ));
        changes += 1;
    }

    // PreToolUse/Bash — prefer-ig.sh
    if ensure_hook_registered(
        &mut parsed,
        "PreToolUse",
        "Bash",
        "~/.claude/hooks/prefer-ig.sh",
        "prefer-ig.sh",
        None,
    ) {
        results.push(ConfigResult::Configured(
            "Registered prefer-ig.sh hook".to_string(),
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

/// Resolve the real user's home directory, even when running under sudo.
pub(crate) fn resolve_real_home() -> Option<PathBuf> {
    // If SUDO_USER is set, we're running under sudo — use the real user's home
    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
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
        // Fallback: expand ~user via shell
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

// ─── Existing functions (kept for backward compat + tests) ────────────────────

#[allow(dead_code)]
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
                    "Installed ig-rewrite.sh but failed to register: {}",
                    e
                ))
            } else {
                ConfigResult::Error(e)
            }
        }
    }
}

/// Register ig-rewrite.sh hook in settings.json PreToolUse.
/// Removes old prefer-ig.sh if present. Idempotent.
#[allow(dead_code)]
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
    let bash_idx = pre_tool_use
        .iter()
        .position(|entry| entry.get("matcher").and_then(|m| m.as_str()) == Some("Bash"));

    if bash_idx.is_none() {
        pre_tool_use.push(serde_json::json!({"matcher": "Bash", "hooks": []}));
    }

    let bash_idx = pre_tool_use
        .iter()
        .position(|entry| entry.get("matcher").and_then(|m| m.as_str()) == Some("Bash"))
        .unwrap();

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

fn configure_claude_settings(claude_dir: &Path) -> ConfigResult {
    let settings_path = claude_dir.join("settings.json");

    let content = match fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(_) => {
            return ConfigResult::Error("Could not read ~/.claude/settings.json".to_string());
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

    // --- register_hook_in_settings tests ---

    #[test]
    fn test_register_hook_fresh_settings() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo destructive check"}]}]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = register_hook_in_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(
            content.contains("ig-rewrite.sh"),
            "should add ig-rewrite.sh"
        );
        assert!(
            content.contains("destructive check"),
            "should preserve existing hooks"
        );
    }

    #[test]
    fn test_register_hook_removes_prefer_ig() {
        let dir = TempDir::new().unwrap();
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"~/.claude/hooks/prefer-ig.sh"}]}]}}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();

        let result = register_hook_in_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(
            !content.contains("prefer-ig.sh"),
            "should remove prefer-ig.sh"
        );
        assert!(
            content.contains("ig-rewrite.sh"),
            "should add ig-rewrite.sh"
        );
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
        assert!(
            content.contains("BLOCK"),
            "should preserve Grep blocker content"
        );
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
        assert!(!dir.path().join("hooks/ig-rewrite.sh").exists());
        assert!(!dir.path().join("hooks/prefer-ig.sh").exists());
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
        assert!(dir.path().join("hooks/ig-rewrite.sh").exists());
        assert!(dir.path().join("hooks/prefer-ig.sh").exists());
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
}
