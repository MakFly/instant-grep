//! Antigravity installer — stub.
//!
//! Antigravity has no published config-dir convention; we write a project-
//! local `.antigravity-rules.md` only when the user explicitly asks (either
//! by selecting `--agent antigravity` or passing `--auto-patch`).

use std::fs;
use std::path::{Path, PathBuf};

use super::super::{
    AgentInstaller, IG_SEARCH_TOOLS_SECTION, InstallContext, InstallReport, ShowItem, ShowReport,
    sha256_of,
};

pub struct Antigravity;
const NAME: &str = "Antigravity";

fn target() -> PathBuf {
    PathBuf::from(".antigravity-rules.md")
}

impl AgentInstaller for Antigravity {
    fn id(&self) -> &'static str {
        "antigravity"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, _home: &Path) -> bool {
        target().exists()
    }
    fn install(&self, _home: &Path, ctx: &InstallContext) -> InstallReport {
        let mut r = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        if !self.detect(_home) && !ctx.auto_patch {
            r.skipped = Some("no .antigravity-rules.md and --auto-patch not set".to_string());
            return r;
        }
        let p = target();
        let existing = fs::read_to_string(&p).unwrap_or_default();
        let body = format!("# .antigravity-rules.md\n{}", IG_SEARCH_TOOLS_SECTION);
        if existing == body {
            r.already_done += 1;
            return r;
        }
        if !ctx.dry_run {
            if let Err(e) = fs::write(&p, &body) {
                r.errors.push(e.to_string());
                return r;
            }
        }
        r.configured += 1;
        r
    }
    fn uninstall(&self, _home: &Path, ctx: &InstallContext) -> InstallReport {
        let p = target();
        let mut r = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        if p.exists() {
            if !ctx.dry_run {
                let _ = fs::remove_file(&p);
            }
            r.configured = 1;
        }
        r
    }
    fn show(&self, _home: &Path) -> ShowReport {
        let p = target();
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
