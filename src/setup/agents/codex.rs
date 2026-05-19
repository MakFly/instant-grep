//! Codex CLI installer.
//!
//! Codex has no native hook system (per 2026-05). We only manage the
//! `~/.codex/AGENTS.md` rules file.

use std::fs;
use std::path::Path;

use super::super::{
    AgentInstaller, ConfigResult, InstallContext, InstallReport, ShowItem, ShowReport,
    configure_codex_agents_md, results_to_report, sha256_of, which_exists,
};
use super::claude::strip_managed_block;

pub struct Codex;
const NAME: &str = "Codex CLI";

impl AgentInstaller for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn name(&self) -> &'static str {
        NAME
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".codex").is_dir() || which_exists("codex")
    }

    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let codex_dir = home.join(".codex");
        if !codex_dir.is_dir() {
            if !ctx.auto_patch && !ctx.dry_run {
                return InstallReport {
                    agent: NAME.to_string(),
                    skipped: Some(format!("{} not present", codex_dir.display())),
                    ..Default::default()
                };
            }
            if !ctx.dry_run {
                let _ = fs::create_dir_all(&codex_dir);
            }
        }
        if ctx.hook_only {
            // Codex has no hook surface; hook-only is a no-op.
            return InstallReport {
                agent: NAME.to_string(),
                skipped: Some("no hook surface (--hook-only)".to_string()),
                ..Default::default()
            };
        }
        let r: Vec<ConfigResult> = vec![configure_codex_agents_md(&codex_dir, ctx.dry_run)];
        results_to_report(NAME, r)
    }

    fn uninstall(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let path = home.join(".codex/AGENTS.md");
        let mut report = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        if !path.exists() {
            report.skipped = Some("AGENTS.md not present".to_string());
            return report;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Some(stripped) = strip_managed_block(&content) {
                if !ctx.dry_run {
                    if let Err(e) = fs::write(&path, stripped) {
                        report.errors.push(e.to_string());
                    } else {
                        report.configured += 1;
                    }
                } else {
                    report.configured += 1;
                }
            }
        }
        report
    }

    fn show(&self, home: &Path) -> ShowReport {
        let p = home.join(".codex/AGENTS.md");
        ShowReport {
            agent: NAME.to_string(),
            items: vec![ShowItem {
                path: p.display().to_string(),
                present: p.exists(),
                sha256: sha256_of(&p),
            }],
        }
    }
}
