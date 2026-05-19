//! OpenCode installer — `~/.config/opencode/AGENTS.md` + opencode.json
//! instructions array.

use std::fs;
use std::path::Path;

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, configure_opencode,
    results_to_report, sha256_of, which_exists,
};

pub struct OpenCode;
const NAME: &str = "OpenCode";

impl AgentInstaller for OpenCode {
    fn id(&self) -> &'static str {
        "opencode"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".config/opencode").is_dir() || which_exists("opencode")
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        if ctx.hook_only {
            return InstallReport {
                agent: NAME.to_string(),
                skipped: Some("rules only (--hook-only)".to_string()),
                ..Default::default()
            };
        }
        let dir = home.join(".config/opencode");
        if !dir.is_dir() {
            if !ctx.auto_patch && !ctx.dry_run {
                return InstallReport {
                    agent: NAME.to_string(),
                    skipped: Some(format!("{} not present", dir.display())),
                    ..Default::default()
                };
            }
            if !ctx.dry_run {
                let _ = fs::create_dir_all(&dir);
                let cfg = dir.join("opencode.json");
                if !cfg.exists() {
                    let _ = fs::write(&cfg, "{}");
                }
            }
        }
        results_to_report(NAME, configure_opencode(home, ctx.dry_run))
    }
    fn uninstall(&self, _home: &Path, _ctx: &InstallContext) -> InstallReport {
        // OpenCode rules are concatenated into AGENTS.md — same strip helper
        // as gemini/codex would apply, but AGENTS.md here is per-user-owned
        // (we appended). Be conservative: report no-op.
        InstallReport {
            agent: NAME.to_string(),
            skipped: Some(
                "AGENTS.md is user-owned; remove the Search Tools block manually".to_string(),
            ),
            ..Default::default()
        }
    }
    fn show(&self, home: &Path) -> ShowReport {
        let dir = home.join(".config/opencode");
        ShowReport {
            agent: NAME.to_string(),
            items: vec![
                ShowItem {
                    path: dir.join("AGENTS.md").display().to_string(),
                    present: dir.join("AGENTS.md").exists(),
                    sha256: sha256_of(&dir.join("AGENTS.md")),
                },
                ShowItem {
                    path: dir.join("opencode.json").display().to_string(),
                    present: dir.join("opencode.json").exists(),
                    sha256: sha256_of(&dir.join("opencode.json")),
                },
            ],
        }
    }
}
