use std::fs;
use std::path::{Path, PathBuf};

use crate::hooks::copilot;

/// Sentinel markers for a re-writable section. Anything between these tags
/// is fully owned by `ig setup` and will be replaced on every run.
const IG_MANAGED_BEGIN: &str = "<!-- IG-MANAGED-BLOCK:BEGIN -->";
const IG_MANAGED_END: &str = "<!-- IG-MANAGED-BLOCK:END -->";

/// Pre-formatted section as it appears in agent rule files (`CLAUDE.md`,
/// `AGENTS.md`, `GEMINI.md`, `kilorules.md`). Wrapped with managed-block
/// sentinels so `ig setup` can find-and-replace it idempotently across
/// version bumps.
const IG_SEARCH_TOOLS_SECTION: &str = "\n<!-- IG-MANAGED-BLOCK:BEGIN -->\n\
## Search Tools (`ig` — instant-grep)\n\n\
- **Code search**: prefer `ig` (instant-grep) over `rg` or `grep`. Trigram-indexed, daemon-backed, sub-ms hot path. Match parity with `rg` is verified per release on real codebases.\n\
- Usage: `ig \"pattern\" [path]` or `ig search \"pattern\" [path]`.\n\
- **Indexes are NEVER in-project** since v1.15.0. They live under the XDG cache: `~/.cache/ig/projects/<hash>/` (or `~/Library/Caches/ig/...` on macOS). Don't add `.ig/` to `.gitignore` — it's not there.\n\
- **First search on a new project** auto-spawns the global daemon (one process for all projects on the machine, ~6 MB idle) and auto-builds the index in the background. You don't run `ig index`.\n\
- **Editing a file**: the daemon's filesystem watcher rebuilds the overlay invisibly (50-200 ms) and reloads via the seal protocol.\n\
- **Inspecting a project's cache by name**: `ls ~/.cache/ig/by-name/` shows all warmed projects as symlinks. Useful for debugging only — never modify these by hand.\n\n\
### Smart reads (token-compressed for agent context)\n\
- `ig read <file> -s` — imports + function signatures only.\n\
- `ig smart [path]` — 2-line summary per file.\n\
- `ig pack [path]` — generate a full project map under the cache dir.\n\n\
### Daemon control (rare, normally invisible)\n\
- `ig daemon status` — PID, socket path, active projects.\n\
- `ig warm [path]` — add a project to the active set (called automatically by shell hooks on `cd`).\n\
- `ig projects list` / `ig projects forget <root>` — manage warmed projects.\n\
- `ig daemon install` — systemd-user (Linux) or launchd (macOS) auto-start. **Do this once per new machine.**\n\n\
### Cache hygiene (rare)\n\
- `ig gc [--dry-run]` — prune orphan / stale cache entries.\n\
- `ig cache-ls` — list per-project cache size + last-used.\n\n\
Fall back to `rg` only if `ig --version` errors out. Never use plain `grep` for code search.\n\
<!-- IG-MANAGED-BLOCK:END -->\n";

/// Full managed section as injected into a markdown file. Wrapped with
/// sentinel markers so `ig setup` can find-and-replace it idempotently.
/// Returns owned String so callers don't have to clone the const.
fn ig_managed_section() -> String {
    IG_SEARCH_TOOLS_SECTION.to_string()
}

/// Content of `~/.claude/rules/tools/ig.md` — the deep-dive rule file
/// referenced by the global CLAUDE.md. Owned entirely by `ig setup`:
/// rewritten on every invocation so users always have current commands.
const IG_RULES_TOOLS_IG_MD: &str = "# ig (instant-grep) — current\n\n\
Trigram-indexed regex search CLI, daemon-backed. Replaces `rg` and `grep` for code search. Sub-ms warm queries, byte-identical match parity with `rg`.\n\n\
## Search\n\n\
```bash\n\
ig \"pattern\" [path]        # content search (recommended shortcut)\n\
ig search \"pattern\" [path] # explicit form\n\
ig -i \"pattern\"            # case-insensitive\n\
ig -t rs \"pattern\"         # filter by file type (rs, ts, py, …)\n\
ig -l \"pattern\"             # file paths only (like rg -l)\n\
ig -c \"pattern\"             # match count per file\n\
ig --top 5 \"pattern\"       # BM25 ranking, return top-5 files only\n\
ig --semantic \"throw\"      # PMI synonym expansion, no ML model\n\
ig --json \"pattern\"        # JSON output\n\
```\n\n\
## Where the index lives\n\n\
`~/.cache/ig/projects/<hash>/` (or `~/Library/Caches/ig/...` on macOS).\n\
**Never** in the project tree — no `.ig/` folder to gitignore since v1.15.0.\n\n\
Inspect by human-readable name: `ls ~/.cache/ig/by-name/<project>/`.\n\n\
## First time in a project\n\n\
Just run `ig \"pattern\" [path]`. The daemon auto-spawns and the index auto-builds in background on first search. **Do not run `ig index` manually** unless something is wrong — the daemon's filesystem watcher keeps the index synced via overlay rebuilds.\n\n\
## Smart reads (token-compressed for agent context)\n\n\
```bash\n\
ig read <file> -s          # imports + signatures only\n\
ig smart [path]            # 2-line file summaries\n\
ig pack [path]             # full project map → cache_dir/context.md\n\
ig files [path]            # project file list (compact)\n\
ig symbols [path]          # symbol definitions (functions/classes)\n\
ig context <file> <line>   # enclosing block at file:line\n\
```\n\n\
## Daemon (one global process for all projects)\n\n\
```bash\n\
ig daemon status           # check health (PID, socket, active projects)\n\
ig daemon install          # systemd-user (Linux) or launchd (macOS) auto-start\n\
ig warm [path]             # explicitly add project to active set (idempotent)\n\
ig projects list           # active projects + idle seconds\n\
```\n\n\
Socket: `~/.cache/ig/daemon/daemon.sock`. Don't touch.\n\n\
## Cache hygiene (rare)\n\n\
```bash\n\
ig gc                      # prune orphan / unused entries\n\
ig gc --days 30 --dry-run  # what would be pruned (no delete)\n\
ig cache-ls                # list cached projects with size + last-used\n\
```\n\n\
## Subagent snippet\n\n\
> Use `ig \"pattern\" [path]` via Bash for code search. Trigram-indexed, daemon-backed, sub-ms hot path. Binary at `~/.local/bin/ig`. Index lives in `~/.cache/ig/`, never in the project. Auto-builds on first search; never run `ig index` manually unless instructed. Match parity with `rg` is verified per release.\n\n\
## What NOT to do\n\n\
- `rg` for code search → use `ig` (rg only as fallback if `ig --version` errors).\n\
- Add `.ig/` to `.gitignore` — pre-v1.15.0 leftover, no longer relevant.\n\
- Run `ig index` proactively — daemon does it.\n\
- Touch files inside `~/.cache/ig/projects/` — read-only territory.\n\
- `rm -rf ~/.cache/ig/` — use `ig gc` for partial clean.\n\n\
*This file is auto-managed by `ig setup`. Manual edits are overwritten on the next run.*\n";

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

// ─── AgentSetup trait ─────────────────────────────────────────────────────────

trait AgentSetup {
    fn name(&self) -> &str;
    fn is_present(&self, home: &Path) -> bool;
    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult>;
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

    // Compare existing content (if any) against shipped content.
    // - Identical → AlreadyDone (no-op)
    // - Different → back up to `<name>.bak-<timestamp>` and overwrite
    // - Missing → install fresh
    let existing = fs::read_to_string(&hook_path).ok();
    match &existing {
        Some(s) if s == content => {
            return ConfigResult::AlreadyDone(format!("{} already up-to-date", name));
        }
        Some(_) if !dry_run => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup = hooks_dir.join(format!("{}.bak-{}", name, ts));
            let _ = fs::rename(&hook_path, &backup);
        }
        _ => {}
    }

    let action = if existing.is_some() {
        "Updated"
    } else {
        "Installed"
    };

    match write_if_not_dry(&hook_path, content.as_bytes(), dry_run) {
        Ok(_) => {
            if !dry_run {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755));
                }
            }
            ConfigResult::Configured(format!("{} {}", action, name))
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

    // Find an existing entry by marker.
    // - If marker matches AND command is byte-identical → no-op (already ok).
    // - If marker matches but command differs (new binary shipped a fixed
    //   one-liner, see v1.9.1 CLAUDE_BASH_COMMAND fallback) → overwrite.
    // - No match → append fresh.
    let existing_idx = hooks.iter().position(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .contains(marker)
    });

    if let Some(idx) = existing_idx {
        let current_cmd = hooks[idx]
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if current_cmd == hook_cmd {
            return false; // identical, no update needed
        }
        // Update in place, preserving other fields (e.g. `type`, `timeout`).
        hooks[idx]["command"] = serde_json::json!(hook_cmd);
        if let Some(t) = timeout {
            hooks[idx]["timeout"] = serde_json::json!(t);
        }
        return true;
    }

    // Add a new hook entry
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

fn print_results(agent_name: &str, results: &[ConfigResult], configured_count: &mut u32) {
    eprintln!("\x1b[32m✓ {}\x1b[0m", agent_name);
    for action in results {
        match action {
            ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
            ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
            ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
        }
    }
    *configured_count += 1;
}

// ─── Claude Code ──────────────────────────────────────────────────────────────

struct ClaudeCodeAgent;

impl AgentSetup for ClaudeCodeAgent {
    fn name(&self) -> &str {
        "Claude Code"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".claude").is_dir() || which_exists("claude")
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        let claude_dir = home.join(".claude");
        if !claude_dir.is_dir() {
            let _ = fs::create_dir_all(&claude_dir);
        }
        let mut actions = Vec::new();
        actions.push(configure_claude_settings(&claude_dir));
        actions.push(configure_claude_md(&claude_dir));
        actions.push(configure_claude_rules_ig_md(&claude_dir));
        actions.extend(configure_claude_hooks_full(&claude_dir, dry_run));
        actions.extend(configure_claude_env_vars(&claude_dir, dry_run));
        actions
    }
}

// ─── Codex CLI ────────────────────────────────────────────────────────────────

struct CodexAgent;

impl AgentSetup for CodexAgent {
    fn name(&self) -> &str {
        "Codex CLI"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".codex").is_dir() || which_exists("codex")
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        let codex_dir = home.join(".codex");
        if !codex_dir.is_dir() {
            let _ = fs::create_dir_all(&codex_dir);
        }
        let result = configure_codex_agents_md(&codex_dir);
        // configure_codex_agents_md does not use dry_run yet — consistent with original
        let _ = dry_run;
        vec![result]
    }
}

// ─── OpenCode ─────────────────────────────────────────────────────────────────

struct OpenCodeAgent;

impl AgentSetup for OpenCodeAgent {
    fn name(&self) -> &str {
        "OpenCode"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".config/opencode").is_dir() || which_exists("opencode")
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_opencode(home, dry_run)
    }
}

// ─── Cursor ───────────────────────────────────────────────────────────────────

struct CursorAgent;

impl AgentSetup for CursorAgent {
    fn name(&self) -> &str {
        "Cursor"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".cursor").is_dir()
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_cursor(home, dry_run)
    }
}

// ─── GitHub Copilot ───────────────────────────────────────────────────────────

struct CopilotAgent;

impl AgentSetup for CopilotAgent {
    fn name(&self) -> &str {
        "GitHub Copilot"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".github").is_dir() || PathBuf::from(".github").is_dir()
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_copilot(home, dry_run)
    }
}

// ─── Windsurf ─────────────────────────────────────────────────────────────────

struct WindsurfAgent;

impl AgentSetup for WindsurfAgent {
    fn name(&self) -> &str {
        "Windsurf"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".windsurf").is_dir() || PathBuf::from(".windsurf").is_dir()
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_windsurf(home, dry_run)
    }
}

// ─── Cline ────────────────────────────────────────────────────────────────────

struct ClineAgent;

impl AgentSetup for ClineAgent {
    fn name(&self) -> &str {
        "Cline"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".cline").is_dir() || PathBuf::from(".cline").is_dir()
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_cline(home, dry_run)
    }
}

// ─── Gemini CLI ───────────────────────────────────────────────────────────────

struct GeminiAgent;

impl AgentSetup for GeminiAgent {
    fn name(&self) -> &str {
        "Gemini CLI"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".gemini").is_dir() || which_exists("gemini")
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_gemini(home, dry_run)
    }
}

// ─── Aider ────────────────────────────────────────────────────────────────────

struct AiderAgent;

impl AgentSetup for AiderAgent {
    fn name(&self) -> &str {
        "Aider"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".aider.conf.yml").exists() || which_exists("aider")
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_aider(home, dry_run)
    }
}

// ─── Continue ─────────────────────────────────────────────────────────────────

struct ContinueAgent;

impl AgentSetup for ContinueAgent {
    fn name(&self) -> &str {
        "Continue"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".continue").is_dir()
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_continue(home, dry_run)
    }
}

// ─── Zed ──────────────────────────────────────────────────────────────────────

struct ZedAgent;

impl AgentSetup for ZedAgent {
    fn name(&self) -> &str {
        "Zed"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".config/zed").is_dir()
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_zed(home, dry_run)
    }
}

// ─── Kilo ─────────────────────────────────────────────────────────────────────

struct KiloAgent;

impl AgentSetup for KiloAgent {
    fn name(&self) -> &str {
        "Kilo"
    }

    fn is_present(&self, home: &Path) -> bool {
        home.join(".kilo").is_dir() || PathBuf::from(".kilo").is_dir()
    }

    fn configure(&self, home: &Path, dry_run: bool) -> Vec<ConfigResult> {
        configure_kilo(home, dry_run)
    }
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
    // Reads command from $CLAUDE_BASH_COMMAND (legacy) or stdin JSON (Claude
    // Code 2.1+). Works with both harness versions.
    let destructive_git_cmd = r#"CMD="${CLAUDE_BASH_COMMAND:-}"; [[ -z "$CMD" && ! -t 0 ]] && CMD="$(jq -r '.tool_input.command // empty' 2>/dev/null)"; echo "$CMD" | grep -qE '(git reset --hard|git checkout \.|git clean -f|--force|--no-verify)' && echo 'BLOCK: Destructive git command detected. Confirm with user first.' >&2 && exit 2 || exit 0"#;
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
    // Same env-var/stdin-JSON dual source as the destructive git blocker.
    let npm_cmd = r#"CMD="${CLAUDE_BASH_COMMAND:-}"; [[ -z "$CMD" && ! -t 0 ]] && CMD="$(jq -r '.tool_input.command // empty' 2>/dev/null)"; echo "$CMD" | grep -qE '^(npm |npx )' && echo 'BLOCK: Use bun/bunx instead of npm/npx (global rule).' >&2 && exit 2 || exit 0"#;
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

// ─── Gemini CLI ───────────────────────────────────────────────────────────────

fn configure_gemini(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let gemini_dir = home.join(".gemini");
    let md_path = gemini_dir.join("GEMINI.md");

    let content = fs::read_to_string(&md_path).unwrap_or_default();

    if content.contains("## Search Tools") && content.contains("ig") {
        return vec![ConfigResult::AlreadyDone(
            "GEMINI.md already has ig instructions".to_string(),
        )];
    }

    let new_content = if content.is_empty() {
        format!("# GEMINI.md\n{}", IG_SEARCH_TOOLS_SECTION)
    } else {
        format!("{}\n{}", content.trim_end(), IG_SEARCH_TOOLS_SECTION)
    };

    match write_if_not_dry(&md_path, new_content.as_bytes(), dry_run) {
        Ok(_) => vec![ConfigResult::Configured(
            "Added ig instructions to ~/.gemini/GEMINI.md".to_string(),
        )],
        Err(e) => vec![ConfigResult::Error(e)],
    }
}

// ─── Aider ────────────────────────────────────────────────────────────────────

fn configure_aider(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let mut results = Vec::new();

    // Write IG.md under ~/.aider/
    let aider_dir = home.join(".aider");
    let ig_md_path = aider_dir.join("IG.md");

    if ig_md_path.exists() {
        results.push(ConfigResult::AlreadyDone(
            "~/.aider/IG.md already exists".to_string(),
        ));
    } else {
        let ig_md_content = format!("# ig (instant-grep)\n{}", IG_SEARCH_TOOLS_SECTION);
        match write_if_not_dry(&ig_md_path, ig_md_content.as_bytes(), dry_run) {
            Ok(_) => results.push(ConfigResult::Configured(
                "Created ~/.aider/IG.md".to_string(),
            )),
            Err(e) => results.push(ConfigResult::Error(e)),
        }
    }

    // Write/merge ~/.aider.conf.yml
    let conf_path = home.join(".aider.conf.yml");
    let ig_md_str = ig_md_path.to_string_lossy().to_string();

    if conf_path.exists() {
        let existing = fs::read_to_string(&conf_path).unwrap_or_default();
        if existing.contains("IG.md") {
            results.push(ConfigResult::AlreadyDone(
                "~/.aider.conf.yml already references IG.md".to_string(),
            ));
        } else {
            // Append read entry
            let appended = format!("{}\nread:\n  - \"{}\"\n", existing.trim_end(), ig_md_str);
            match write_if_not_dry(&conf_path, appended.as_bytes(), dry_run) {
                Ok(_) => results.push(ConfigResult::Configured(
                    "Added IG.md to ~/.aider.conf.yml read list".to_string(),
                )),
                Err(e) => results.push(ConfigResult::Error(e)),
            }
        }
    } else {
        let conf_content = format!("read:\n  - \"{}\"\n", ig_md_str);
        match write_if_not_dry(&conf_path, conf_content.as_bytes(), dry_run) {
            Ok(_) => results.push(ConfigResult::Configured(
                "Created ~/.aider.conf.yml with IG.md read entry".to_string(),
            )),
            Err(e) => results.push(ConfigResult::Error(e)),
        }
    }

    results
}

// ─── Continue ─────────────────────────────────────────────────────────────────

fn configure_continue(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let continue_dir = home.join(".continue");
    let config_path = continue_dir.join("config.json");

    let content = fs::read_to_string(&config_path).unwrap_or_else(|_| "{}".to_string());
    let mut parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            return vec![ConfigResult::Error(
                "Could not parse ~/.continue/config.json".to_string(),
            )];
        }
    };

    if parsed.get("customCommands").is_none() {
        parsed["customCommands"] = serde_json::json!([]);
    }

    let commands = parsed["customCommands"].as_array_mut().unwrap();
    let already = commands
        .iter()
        .any(|c| c.get("name").and_then(|n| n.as_str()).unwrap_or("") == "ig");

    if already {
        return vec![ConfigResult::AlreadyDone(
            "ig command already in ~/.continue/config.json customCommands".to_string(),
        )];
    }

    commands.push(serde_json::json!({
        "name": "ig",
        "description": "trigger ig search/read",
        "prompt": "Use `ig \"pattern\" [path]` for code search. Trigram-indexed, fast, project-aware."
    }));

    if !dry_run {
        let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
        match write_if_not_dry(&config_path, format!("{}\n", formatted).as_bytes(), dry_run) {
            Ok(_) => vec![ConfigResult::Configured(
                "Added ig command to ~/.continue/config.json customCommands".to_string(),
            )],
            Err(e) => vec![ConfigResult::Error(e)],
        }
    } else {
        vec![ConfigResult::Configured(
            "Would add ig command to ~/.continue/config.json customCommands".to_string(),
        )]
    }
}

// ─── Zed ──────────────────────────────────────────────────────────────────────

fn configure_zed(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    let zed_dir = home.join(".config/zed");
    let settings_path = zed_dir.join("settings.json");

    let content = fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string());
    let mut parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            return vec![ConfigResult::Error(
                "Could not parse ~/.config/zed/settings.json".to_string(),
            )];
        }
    };

    // Check if already configured
    let already = parsed
        .get("assistant")
        .and_then(|a| a.get("context_servers"))
        .and_then(|cs| cs.get("ig"))
        .is_some();

    if already {
        return vec![ConfigResult::AlreadyDone(
            "ig context server already in ~/.config/zed/settings.json".to_string(),
        )];
    }

    // Ensure assistant.context_servers.ig exists
    if parsed.get("assistant").is_none() {
        parsed["assistant"] = serde_json::json!({});
    }
    if parsed["assistant"].get("context_servers").is_none() {
        parsed["assistant"]["context_servers"] = serde_json::json!({});
    }
    parsed["assistant"]["context_servers"]["ig"] = serde_json::json!({
        "command": "ig",
        "args": ["rewrite"]
    });

    if !dry_run {
        let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_default();
        match write_if_not_dry(
            &settings_path,
            format!("{}\n", formatted).as_bytes(),
            dry_run,
        ) {
            Ok(_) => vec![ConfigResult::Configured(
                "Added ig context server to ~/.config/zed/settings.json".to_string(),
            )],
            Err(e) => vec![ConfigResult::Error(e)],
        }
    } else {
        vec![ConfigResult::Configured(
            "Would add ig context server to ~/.config/zed/settings.json".to_string(),
        )]
    }
}

// ─── Kilo ─────────────────────────────────────────────────────────────────────

fn configure_kilo(home: &Path, dry_run: bool) -> Vec<ConfigResult> {
    // Prefer ~/.kilo/, fall back to ./.kilo/ detection but always write to ~/.kilo/
    let kilo_dir = home.join(".kilo");
    let md_path = kilo_dir.join("kilorules.md");

    let content = fs::read_to_string(&md_path).unwrap_or_default();

    if content.contains("## Search Tools") && content.contains("ig") {
        return vec![ConfigResult::AlreadyDone(
            "kilorules.md already has ig instructions".to_string(),
        )];
    }

    let new_content = if content.is_empty() {
        format!("# kilorules.md\n{}", IG_SEARCH_TOOLS_SECTION)
    } else {
        format!("{}\n{}", content.trim_end(), IG_SEARCH_TOOLS_SECTION)
    };

    match write_if_not_dry(&md_path, new_content.as_bytes(), dry_run) {
        Ok(_) => vec![ConfigResult::Configured(
            "Added ig instructions to ~/.kilo/kilorules.md".to_string(),
        )],
        Err(e) => vec![ConfigResult::Error(e)],
    }
}

/// Resolve the real user's home directory, even when running under sudo.
pub(crate) fn resolve_real_home() -> Option<PathBuf> {
    // If SUDO_USER is set, we're running under sudo — use the real user's home
    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        // Strict username validation: POSIX portable filename set minus '.',
        // since '.' allows usernames like `a.$(id)` which would still be passed
        // through to external processes.
        if !sudo_user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            eprintln!("Warning: SUDO_USER contains invalid characters, ignoring");
        } else if let Some(home_dir) = std::process::Command::new("getent")
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
        // No shell fallback: if getent can't resolve the user, we refuse to
        // guess. The caller will fall back to $HOME below.
    }
    // Default: use HOME
    std::env::var("HOME").ok().map(PathBuf::from)
}

// ─── Public entry point ───────────────────────────────────────────────────────

const SHELL_HOOK_OPEN: &str = "# >>> ig managed >>>";
const SHELL_HOOK_CLOSE: &str = "# <<< ig managed <<<";

const SHELL_HOOK_ZSH: &str = r#"# >>> ig managed >>>
_ig_autostart() {
    command -v ig >/dev/null 2>&1 || return 0
    ig warm --silent "$PWD" >/dev/null 2>&1 &!
    if [[ -d .ig && ! -f .ignore ]]; then
        ig autoignore "$PWD" >/dev/null 2>&1 &!
    fi
}
if [[ -n "$ZSH_VERSION" ]]; then
    chpwd_functions+=(_ig_autostart)
fi
# <<< ig managed <<<"#;

const SHELL_HOOK_BASH: &str = r#"# >>> ig managed >>>
_ig_autostart() {
    command -v ig >/dev/null 2>&1 || return 0
    ig warm --silent "$PWD" >/dev/null 2>&1 &
    if [[ -d .ig && ! -f .ignore ]]; then
        ig autoignore "$PWD" >/dev/null 2>&1 &
    fi
}
PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }_ig_autostart"
# <<< ig managed <<<"#;

const SHELL_HOOK_FISH: &str = r#"# >>> ig managed >>>
function _ig_autostart --on-variable PWD
    command -q ig; or return
    ig warm --silent $PWD >/dev/null 2>&1 &
    if test -d .ig; and not test -f .ignore
        ig autoignore $PWD >/dev/null 2>&1 &
    end
end
# <<< ig managed <<<"#;

fn install_shell_hook(home: &Path, dry_run: bool) -> ConfigResult {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let (rc_path, hook_content) = if shell.contains("zsh") {
        (home.join(".zshrc"), SHELL_HOOK_ZSH)
    } else if shell.contains("fish") {
        (home.join(".config/fish/config.fish"), SHELL_HOOK_FISH)
    } else {
        (home.join(".bashrc"), SHELL_HOOK_BASH)
    };

    let existing = fs::read_to_string(&rc_path).unwrap_or_default();
    if existing.contains(SHELL_HOOK_OPEN)
        && existing.contains(SHELL_HOOK_CLOSE)
        && let Some(updated) = replace_managed_shell_hook(&existing, hook_content)
    {
        if updated == existing {
            return ConfigResult::AlreadyDone(format!(
                "Shell hook already present in {}",
                rc_path.display()
            ));
        }
        if dry_run {
            return ConfigResult::Configured(format!(
                "Would update shell hook in {}",
                rc_path.display()
            ));
        }
        return match fs::write(&rc_path, updated.as_bytes()) {
            Ok(_) => {
                ConfigResult::Configured(format!("Shell hook updated in {}", rc_path.display()))
            }
            Err(e) => ConfigResult::Error(format!("Could not write {}: {}", rc_path.display(), e)),
        };
    }

    if dry_run {
        return ConfigResult::Configured(format!(
            "Would install shell hook in {}",
            rc_path.display()
        ));
    }

    if let Some(parent) = rc_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let new_content = if existing.is_empty() {
        format!("{}\n", hook_content)
    } else {
        format!("{}\n{}\n", existing.trim_end_matches('\n'), hook_content)
    };

    match fs::write(&rc_path, new_content.as_bytes()) {
        Ok(_) => ConfigResult::Configured(format!("Shell hook installed in {}", rc_path.display())),
        Err(e) => ConfigResult::Error(format!("Could not write {}: {}", rc_path.display(), e)),
    }
}

fn replace_managed_shell_hook(existing: &str, hook_content: &str) -> Option<String> {
    let start = existing.find(SHELL_HOOK_OPEN)?;
    let close_start = existing[start..].find(SHELL_HOOK_CLOSE)? + start;
    let close_end = close_start + SHELL_HOOK_CLOSE.len();
    let mut updated = String::new();
    updated.push_str(&existing[..start]);
    updated.push_str(hook_content);
    updated.push_str(&existing[close_end..]);
    Some(updated)
}

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

    let agents: &[&dyn AgentSetup] = &[
        &ClaudeCodeAgent,
        &CodexAgent,
        &OpenCodeAgent,
        &CursorAgent,
        &CopilotAgent,
        &WindsurfAgent,
        &ClineAgent,
        &GeminiAgent,
        &AiderAgent,
        &ContinueAgent,
        &ZedAgent,
        &KiloAgent,
    ];

    for agent in agents {
        if agent.is_present(&home) {
            let results = agent.configure(&home, dry_run);
            print_results(agent.name(), &results, &mut configured);
        } else {
            eprintln!("\x1b[2m⊘ {} — not detected\x1b[0m", agent.name());
        }
    }

    // Install shell auto-warmup hook
    eprintln!("\x1b[32m✓ Shell hook\x1b[0m");
    let hook_result = install_shell_hook(&home, dry_run);
    match &hook_result {
        ConfigResult::Configured(msg) => eprintln!("  → {}", msg),
        ConfigResult::AlreadyDone(msg) => eprintln!("  \x1b[2m→ {}\x1b[0m", msg),
        ConfigResult::Error(msg) => eprintln!("  \x1b[31m✗ {}\x1b[0m", msg),
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
    let target_section = ig_managed_section();

    let (new_content, status) = upsert_managed_block(&content, &target_section, "# Global Rules");

    if let UpsertStatus::Unchanged = status {
        return ConfigResult::AlreadyDone(
            "Search Tools section already up-to-date in CLAUDE.md".to_string(),
        );
    }

    let initial = if new_content.starts_with("# CLAUDE.md") || new_content.starts_with("# ") {
        new_content
    } else {
        format!("# CLAUDE.md\n{}", new_content)
    };

    if fs::write(&md_path, initial.as_bytes()).is_err() {
        return ConfigResult::Error("Could not write CLAUDE.md".to_string());
    }

    let msg = match status {
        UpsertStatus::Inserted => "Added Search Tools section to ~/.claude/CLAUDE.md",
        UpsertStatus::Updated => "Updated Search Tools section in ~/.claude/CLAUDE.md",
        UpsertStatus::ReplacedLegacy => {
            "Replaced legacy Search Tools section in ~/.claude/CLAUDE.md"
        }
        UpsertStatus::Unchanged => unreachable!(),
    };
    ConfigResult::Configured(msg.to_string())
}

/// Configure the `~/.claude/rules/tools/ig.md` deep-dive file. Fully owned
/// by ig — overwritten on every `ig setup` invocation so the rule content
/// always matches the current binary's commands.
fn configure_claude_rules_ig_md(claude_dir: &Path) -> ConfigResult {
    let path = claude_dir.join("rules").join("tools").join("ig.md");
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return ConfigResult::Error(format!("Could not create {}", parent.display()));
    }

    let existing = fs::read_to_string(&path).unwrap_or_default();
    if existing == IG_RULES_TOOLS_IG_MD {
        return ConfigResult::AlreadyDone(
            "~/.claude/rules/tools/ig.md already up-to-date".to_string(),
        );
    }

    if fs::write(&path, IG_RULES_TOOLS_IG_MD.as_bytes()).is_err() {
        return ConfigResult::Error(format!("Could not write {}", path.display()));
    }

    let msg = if existing.is_empty() {
        "Created ~/.claude/rules/tools/ig.md"
    } else {
        "Updated ~/.claude/rules/tools/ig.md"
    };
    ConfigResult::Configured(msg.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpsertStatus {
    /// File had no Search Tools section at all — block appended.
    Inserted,
    /// Managed-block markers were found and the body matched — no write.
    Unchanged,
    /// Managed-block markers were found but the body differed — replaced.
    Updated,
    /// Legacy `## Search Tools` heading found (no markers) — region from
    /// that heading to the next `## ` (or EOF) replaced with the managed
    /// block.
    ReplacedLegacy,
}

/// Insert or update the managed block in `content`. Returns the new content
/// and a status flag describing what happened.
///
/// Strategy (in order):
/// 1. If the begin/end markers are already present → replace what's between
///    them. If the new section equals the existing one, return Unchanged.
/// 2. If a legacy `## Search Tools` heading exists (no markers) → replace
///    from that heading up to the next `## ` heading or EOF.
/// 3. Else: append before `anchor` (e.g. `# Global Rules`) if present, or at
///    EOF.
fn upsert_managed_block(
    content: &str,
    target_section: &str,
    anchor: &str,
) -> (String, UpsertStatus) {
    if let (Some(begin), Some(end_marker_start)) =
        (content.find(IG_MANAGED_BEGIN), content.find(IG_MANAGED_END))
        && begin < end_marker_start
    {
        let end = end_marker_start + IG_MANAGED_END.len();
        let mut after_end = end;
        // Eat one trailing newline so we don't accumulate blank lines.
        if content[after_end..].starts_with('\n') {
            after_end += 1;
        }
        let mut new_content = String::with_capacity(content.len() + target_section.len());
        new_content.push_str(&content[..begin]);
        // Keep the trailing newline if the prefix didn't already supply it.
        let trimmed_section = target_section.trim_start_matches('\n');
        if !content[..begin].ends_with('\n') && !content[..begin].is_empty() {
            new_content.push('\n');
        }
        new_content.push_str(trimmed_section);
        new_content.push_str(&content[after_end..]);

        if new_content == content {
            (new_content, UpsertStatus::Unchanged)
        } else {
            (new_content, UpsertStatus::Updated)
        }
    } else if let Some(legacy_pos) = content.find("\n## Search Tools") {
        let region_start = legacy_pos + 1;
        let after_heading = region_start + "## Search Tools".len();
        let region_end = content[after_heading..]
            .find("\n## ")
            .map(|off| after_heading + off + 1)
            .unwrap_or(content.len());

        let mut new_content = String::with_capacity(content.len() + target_section.len());
        new_content.push_str(&content[..region_start]);
        let trimmed_section = target_section.trim_start_matches('\n');
        new_content.push_str(trimmed_section);
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(&content[region_end..]);
        (new_content, UpsertStatus::ReplacedLegacy)
    } else if content.is_empty() {
        (target_section.to_string(), UpsertStatus::Inserted)
    } else if let Some(anchor_pos) = content.find(anchor) {
        let mut new_content = String::with_capacity(content.len() + target_section.len());
        new_content.push_str(&content[..anchor_pos]);
        new_content.push_str(target_section.trim_start_matches('\n'));
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(&content[anchor_pos..]);
        (new_content, UpsertStatus::Inserted)
    } else {
        let mut new_content = content.trim_end().to_string();
        new_content.push_str(target_section);
        (new_content, UpsertStatus::Inserted)
    }
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

        let result = configure_claude_settings(dir.path());
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

        let result = configure_claude_settings(dir.path());
        assert!(matches!(result, ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_claude_settings_missing_file_creates_it() {
        let dir = TempDir::new().unwrap();
        // No settings.json written — file does not exist, should create with defaults
        let result = configure_claude_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));
        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("Bash(ig *)"));
    }

    #[test]
    fn test_claude_settings_invalid_json_returns_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), "not json").unwrap();

        let result = configure_claude_settings(dir.path());
        assert!(matches!(result, ConfigResult::Error(_)));
    }

    #[test]
    fn test_claude_settings_missing_allow_array_creates_it() {
        let dir = TempDir::new().unwrap();
        // Valid JSON but no permissions.allow array
        fs::write(dir.path().join("settings.json"), r#"{"other": "value"}"#).unwrap();

        let result = configure_claude_settings(dir.path());
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

        let result = configure_claude_settings(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(content.contains("Bash(ig *)"));
    }

    #[test]
    fn test_claude_settings_empty_json_creates_structure() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), r#"{}"#).unwrap();

        let result = configure_claude_settings(dir.path());
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

        let result = configure_claude_md(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(
            content.contains("## Search Tools"),
            "should contain Search Tools section"
        );
        assert!(content.contains("ig"), "should mention ig");
    }

    #[test]
    fn test_claude_md_legacy_section_is_upgraded() {
        // A pre-v1.19 CLAUDE.md without managed-block markers must be
        // upgraded (legacy section replaced) — NOT silently skipped.
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("CLAUDE.md"),
            "# CLAUDE.md\n\n## Search Tools\n- Use ig\n",
        )
        .unwrap();

        let result = configure_claude_md(dir.path());
        assert!(
            matches!(result, ConfigResult::Configured(_)),
            "legacy section must be upgraded"
        );
        let content = fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("IG-MANAGED-BLOCK:BEGIN"));
        assert!(content.contains("IG-MANAGED-BLOCK:END"));
        assert!(
            !content.contains("- Use ig\n"),
            "stale bullet should be gone"
        );
    }

    #[test]
    fn test_claude_md_idempotent_after_managed_install() {
        // Second run on an already-managed file must be a no-op.
        let dir = TempDir::new().unwrap();
        let _ = configure_claude_md(dir.path()); // first run: Configured
        let result = configure_claude_md(dir.path()); // second run: AlreadyDone
        assert!(
            matches!(result, ConfigResult::AlreadyDone(_)),
            "second invocation must be AlreadyDone"
        );
    }

    #[test]
    fn test_claude_md_missing_file_creates_content() {
        let dir = TempDir::new().unwrap();
        // CLAUDE.md does not exist — configure_claude_md starts from empty string
        let result = configure_claude_md(dir.path());
        assert!(matches!(result, ConfigResult::Configured(_)));

        let content = fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("## Search Tools"));
        assert!(content.contains("ig"));
    }

    #[test]
    fn test_codex_agents_md_injection() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "").unwrap();

        let result = configure_codex_agents_md(dir.path());
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

        let result = configure_codex_agents_md(dir.path());
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

        let result = configure_codex_agents_md(dir.path());
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
    fn test_replace_managed_shell_hook_updates_existing_block() {
        let existing = "# before\n# >>> ig managed >>>\nold\n# <<< ig managed <<<\n# after\n";
        let updated = replace_managed_shell_hook(existing, SHELL_HOOK_ZSH).unwrap();
        assert!(updated.contains("# before"));
        assert!(updated.contains("ig warm --silent"));
        assert!(!updated.contains("\nold\n"));
        assert!(updated.contains("# after"));
    }

    #[test]
    fn test_install_hook_file_creates_and_marks_executable() {
        let dir = TempDir::new().unwrap();
        let result = install_hook_file(dir.path(), "myhook.sh", "#!/bin/sh\necho ok", false);
        assert!(matches!(result, ConfigResult::Configured(_)));
        assert!(dir.path().join("myhook.sh").exists());
    }

    #[test]
    fn test_install_hook_file_identical_is_noop() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("myhook.sh"), "same content").unwrap();
        let result = install_hook_file(dir.path(), "myhook.sh", "same content", false);
        assert!(
            matches!(result, ConfigResult::AlreadyDone(_)),
            "identical content should be AlreadyDone"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("myhook.sh")).unwrap(),
            "same content"
        );
    }

    #[test]
    fn test_install_hook_file_updates_when_content_differs() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("myhook.sh"), "old content").unwrap();
        let result = install_hook_file(dir.path(), "myhook.sh", "new content", false);
        assert!(
            matches!(result, ConfigResult::Configured(_)),
            "changed content should be Configured (updated)"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("myhook.sh")).unwrap(),
            "new content"
        );
        // A backup should exist alongside the updated file.
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("myhook.sh.bak-")
            })
            .collect();
        assert_eq!(backups.len(), 1, "expected one backup file");
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
    fn test_ensure_hook_registered_identical_is_noop() {
        // Same marker AND byte-identical command → no-op, returns false.
        let mut parsed = serde_json::json!({
            "hooks": {
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo hello"}]}]
            }
        });
        let changed = ensure_hook_registered(
            &mut parsed,
            "PreToolUse",
            "Bash",
            "echo hello",
            "hello",
            None,
        );
        assert!(!changed, "identical command should be no-op");
    }

    #[test]
    fn test_ensure_hook_registered_updates_when_command_differs() {
        // Same marker BUT different command (e.g. v1.9.1 fixed one-liner
        // replaces v1.9.0 broken one) → update in place, returns true.
        let mut parsed = serde_json::json!({
            "hooks": {
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo hello OLD"}]}]
            }
        });
        let changed = ensure_hook_registered(
            &mut parsed,
            "PreToolUse",
            "Bash",
            "echo hello NEW",
            "hello",
            None,
        );
        assert!(changed, "changed command should trigger an update");
        assert_eq!(
            parsed["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap(),
            "echo hello NEW"
        );
        // Only one entry — didn't duplicate.
        assert_eq!(
            parsed["hooks"]["PreToolUse"][0]["hooks"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
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

    // ─── 5 new agent tests ────────────────────────────────────────────────────

    #[test]
    fn test_configure_gemini_creates_gemini_md() {
        let dir = TempDir::new().unwrap();
        let results = configure_gemini(dir.path(), false);
        assert!(matches!(results[0], ConfigResult::Configured(_)));
        let md = dir.path().join(".gemini/GEMINI.md");
        assert!(md.exists());
        let content = fs::read_to_string(&md).unwrap();
        assert!(content.contains("ig"));
        assert!(content.contains("Search Tools"));
        // Idempotence
        let results2 = configure_gemini(dir.path(), false);
        assert!(matches!(results2[0], ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_configure_aider_creates_conf_and_ig_md() {
        let dir = TempDir::new().unwrap();
        let results = configure_aider(dir.path(), false);
        let errors: Vec<_> = results
            .iter()
            .filter(|r| matches!(r, ConfigResult::Error(_)))
            .collect();
        assert!(errors.is_empty(), "no errors expected");
        assert!(dir.path().join(".aider/IG.md").exists());
        assert!(dir.path().join(".aider.conf.yml").exists());
        let conf = fs::read_to_string(dir.path().join(".aider.conf.yml")).unwrap();
        assert!(conf.contains("IG.md"));
        // Idempotence
        let results2 = configure_aider(dir.path(), false);
        let already: Vec<_> = results2
            .iter()
            .filter(|r| matches!(r, ConfigResult::AlreadyDone(_)))
            .collect();
        assert_eq!(
            already.len(),
            2,
            "both IG.md and conf should be AlreadyDone"
        );
    }

    #[test]
    fn test_configure_continue_creates_custom_command() {
        let dir = TempDir::new().unwrap();
        let continue_dir = dir.path().join(".continue");
        fs::create_dir_all(&continue_dir).unwrap();
        fs::write(continue_dir.join("config.json"), "{}").unwrap();

        let results = configure_continue(dir.path(), false);
        assert!(matches!(results[0], ConfigResult::Configured(_)));
        let config = fs::read_to_string(continue_dir.join("config.json")).unwrap();
        assert!(config.contains("\"ig\""));
        // Idempotence
        let results2 = configure_continue(dir.path(), false);
        assert!(matches!(results2[0], ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_configure_zed_creates_context_server() {
        let dir = TempDir::new().unwrap();
        let zed_dir = dir.path().join(".config/zed");
        fs::create_dir_all(&zed_dir).unwrap();
        fs::write(zed_dir.join("settings.json"), "{}").unwrap();

        let results = configure_zed(dir.path(), false);
        assert!(matches!(results[0], ConfigResult::Configured(_)));
        let settings = fs::read_to_string(zed_dir.join("settings.json")).unwrap();
        assert!(settings.contains("context_servers"));
        assert!(settings.contains("\"ig\""));
        // Idempotence
        let results2 = configure_zed(dir.path(), false);
        assert!(matches!(results2[0], ConfigResult::AlreadyDone(_)));
    }

    #[test]
    fn test_configure_kilo_creates_kilorules_md() {
        let dir = TempDir::new().unwrap();
        let results = configure_kilo(dir.path(), false);
        assert!(matches!(results[0], ConfigResult::Configured(_)));
        let md = dir.path().join(".kilo/kilorules.md");
        assert!(md.exists());
        let content = fs::read_to_string(&md).unwrap();
        assert!(content.contains("ig"));
        assert!(content.contains("Search Tools"));
        // Idempotence
        let results2 = configure_kilo(dir.path(), false);
        assert!(matches!(results2[0], ConfigResult::AlreadyDone(_)));
    }
}
