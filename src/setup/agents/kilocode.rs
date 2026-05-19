//! Kilocode installer — file-based rules under `~/.kilo/kilorules.md`.
//! Mirrors rtk's `kilocode` mode.

use std::fs;
use std::path::Path;

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, configure_kilo,
    results_to_report, sha256_of,
};
use super::claude::strip_managed_block;

pub struct Kilocode;
const NAME: &str = "Kilocode";

impl AgentInstaller for Kilocode {
    fn id(&self) -> &'static str {
        "kilocode"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".kilo").is_dir() || std::path::PathBuf::from(".kilo").is_dir()
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let mut r = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        if !self.detect(home) && !ctx.auto_patch {
            r.skipped = Some("~/.kilo not present (use --auto-patch)".to_string());
            return r;
        }
        if ctx.hook_only {
            r.skipped = Some("rules only (--hook-only)".to_string());
            return r;
        }
        results_to_report(NAME, configure_kilo(home, ctx.dry_run))
    }
    fn uninstall(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let mut r = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        let p = home.join(".kilo/kilorules.md");
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
        let p = home.join(".kilo/kilorules.md");
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
