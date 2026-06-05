//! Claude Code installer.
//!
//! Wires:
//! - `~/.claude/CLAUDE.md` (managed-block with search-tools section)
//! - `~/.claude/rules/tools/ig.md` (deep-dive rule file, fully owned)
//! - `~/.claude/settings.json` (`Bash(ig *)` permission, hooks)
//! - `~/.claude/hooks/{ig-guard,format,subagent-context}.sh`
//! - `~/.claude/agents/explorer.md`

use std::fs;
use std::path::Path;

use super::super::{
    AgentInstaller, ConfigResult, IG_MANAGED_BEGIN, IG_MANAGED_END, InstallContext, InstallReport,
    ShowItem, ShowReport, configure_claude_hooks_full,
    configure_claude_md, configure_claude_rules_ig_md, configure_claude_settings,
    results_to_report, sha256_of,
};

pub struct Claude;

const ID: &str = "claude";
const NAME: &str = "Claude Code";

impl AgentInstaller for Claude {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        NAME
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".claude").is_dir() || super::super::which_exists("claude")
    }

    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let claude_dir = home.join(".claude");
        if !claude_dir.is_dir() {
            if !ctx.auto_patch && !ctx.dry_run {
                return InstallReport {
                    agent: NAME.to_string(),
                    skipped: Some(format!(
                        "{} not present (use --auto-patch)",
                        claude_dir.display()
                    )),
                    ..Default::default()
                };
            }
            if !ctx.dry_run {
                let _ = fs::create_dir_all(&claude_dir);
            }
        }

        let mut all: Vec<ConfigResult> = Vec::new();

        // Helpers that don't take a dry_run flag (legacy API); we short-
        // circuit them by reporting a synthetic "would configure" entry
        // when dry-run is on, so the disk stays clean.
        if !ctx.no_patch {
            if ctx.dry_run {
                all.push(ConfigResult::Configured(
                    "Would update ~/.claude/settings.json".to_string(),
                ));
            } else {
                all.push(configure_claude_settings(&claude_dir));
            }
        }
        if !ctx.hook_only {
            if ctx.dry_run {
                all.push(ConfigResult::Configured(
                    "Would update ~/.claude/CLAUDE.md".to_string(),
                ));
                all.push(ConfigResult::Configured(
                    "Would update ~/.claude/rules/tools/ig.md".to_string(),
                ));
            } else {
                all.push(configure_claude_md(&claude_dir));
                all.push(configure_claude_rules_ig_md(&claude_dir));
            }
        }
        if !ctx.no_patch {
            all.extend(configure_claude_hooks_full(&claude_dir, ctx.dry_run));
        }

        results_to_report(NAME, all)
    }

    fn uninstall(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let claude_dir = home.join(".claude");
        let mut report = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        if !claude_dir.is_dir() {
            report.skipped = Some(format!("{} not present", claude_dir.display()));
            return report;
        }

        // Strip the managed-block from CLAUDE.md (preserve user content).
        let md = claude_dir.join("CLAUDE.md");
        if md.exists() {
            let content = fs::read_to_string(&md).unwrap_or_default();
            if let Some(stripped) = strip_managed_block(&content) {
                if !ctx.dry_run {
                    if let Err(e) = fs::write(&md, stripped) {
                        report.errors.push(format!("CLAUDE.md: {}", e));
                    } else {
                        report.configured += 1;
                    }
                } else {
                    report.configured += 1;
                }
            }
        }

        // Drop the rules file and hook scripts (fully owned).
        for rel in [
            "rules/tools/ig.md",
            "hooks/ig-guard.sh",
            "hooks/format.sh",
            "hooks/subagent-context.sh",
            "agents/explorer.md",
        ] {
            let p = claude_dir.join(rel);
            if p.exists() {
                if !ctx.dry_run {
                    let _ = fs::remove_file(&p);
                }
                report.configured += 1;
            }
        }

        // Prune our hook entries from settings.json (preserve other keys).
        let settings = claude_dir.join("settings.json");
        if settings.exists() {
            let content = fs::read_to_string(&settings).unwrap_or_default();
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&content) {
                let markers = [
                    "ig-guard.sh",
                    "format.sh",
                    "subagent-context.sh",
                    "Destructive git",
                    "bun/bunx instead",
                    "Use ig via Bash",
                    ".env",
                    "secret detected",
                ];
                let mut changed = false;
                if let Some(hooks) = v.get_mut("hooks").and_then(|h| h.as_object_mut()) {
                    for (_evt, matchers) in hooks.iter_mut() {
                        if let Some(arr) = matchers.as_array_mut() {
                            for matcher in arr.iter_mut() {
                                if let Some(list) =
                                    matcher.get_mut("hooks").and_then(|h| h.as_array_mut())
                                {
                                    let before = list.len();
                                    list.retain(|h| {
                                        let cmd =
                                            h.get("command").and_then(|c| c.as_str()).unwrap_or("");
                                        !markers.iter().any(|m| cmd.contains(m))
                                    });
                                    if list.len() != before {
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
                // Drop the Bash(ig *) permission.
                if let Some(allow) = v
                    .get_mut("permissions")
                    .and_then(|p| p.get_mut("allow"))
                    .and_then(|a| a.as_array_mut())
                {
                    let before = allow.len();
                    allow.retain(|v| v.as_str() != Some("Bash(ig *)"));
                    if allow.len() != before {
                        changed = true;
                    }
                }
                if changed && !ctx.dry_run {
                    let formatted = serde_json::to_string_pretty(&v).unwrap_or_default();
                    let _ = fs::write(&settings, format!("{}\n", formatted));
                    report.configured += 1;
                } else if changed {
                    report.configured += 1;
                }
            }
        }

        report
    }

    fn show(&self, home: &Path) -> ShowReport {
        let claude_dir = home.join(".claude");
        let items = [
            "CLAUDE.md",
            "rules/tools/ig.md",
            "hooks/ig-guard.sh",
            "hooks/format.sh",
            "hooks/subagent-context.sh",
            "agents/explorer.md",
            "settings.json",
        ]
        .iter()
        .map(|rel| {
            let p = claude_dir.join(rel);
            ShowItem {
                path: p.display().to_string(),
                present: p.exists(),
                sha256: sha256_of(&p),
            }
        })
        .collect();
        ShowReport {
            agent: NAME.to_string(),
            items,
        }
    }
}

/// Remove the IG-MANAGED-BLOCK region from `content`. Returns the new content
/// only if the block was present.
pub(crate) fn strip_managed_block(content: &str) -> Option<String> {
    let begin = content.find(IG_MANAGED_BEGIN)?;
    let end_marker = content.find(IG_MANAGED_END)?;
    if end_marker < begin {
        return None;
    }
    let end = end_marker + IG_MANAGED_END.len();
    let mut after = end;
    if content[after..].starts_with('\n') {
        after += 1;
    }
    // Also eat the leading newline we add before the block.
    let mut start = begin;
    if start > 0 && &content[start - 1..start] == "\n" {
        start -= 1;
    }
    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..start]);
    out.push_str(&content[after..]);
    Some(out)
}
