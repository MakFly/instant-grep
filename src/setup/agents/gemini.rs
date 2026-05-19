//! Gemini CLI installer — `~/.gemini/GEMINI.md`.

use std::fs;
use std::path::Path;

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, configure_gemini,
    results_to_report, sha256_of, which_exists,
};
use super::claude::strip_managed_block;

pub struct Gemini;
const NAME: &str = "Gemini CLI";

impl AgentInstaller for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".gemini").is_dir() || which_exists("gemini")
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        if ctx.hook_only {
            return InstallReport {
                agent: NAME.to_string(),
                skipped: Some("rules only (--hook-only)".to_string()),
                ..Default::default()
            };
        }
        results_to_report(NAME, configure_gemini(home, ctx.dry_run))
    }
    fn uninstall(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let p = home.join(".gemini/GEMINI.md");
        let mut r = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        if p.exists() {
            if let Ok(content) = fs::read_to_string(&p) {
                if let Some(stripped) = strip_managed_block(&content) {
                    if !ctx.dry_run {
                        let _ = fs::write(&p, stripped);
                    }
                    r.configured = 1;
                }
            }
        }
        r
    }
    fn show(&self, home: &Path) -> ShowReport {
        let p = home.join(".gemini/GEMINI.md");
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
