//! GitHub Copilot installer — writes `.github/copilot-instructions.md`.

use std::fs;
use std::path::{Path, PathBuf};

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, configure_copilot,
    results_to_report, sha256_of,
};

pub struct Copilot;
const NAME: &str = "GitHub Copilot";

fn target_path(home: &Path) -> PathBuf {
    if PathBuf::from(".github").is_dir() {
        PathBuf::from(".github/copilot-instructions.md")
    } else {
        home.join(".github/copilot-instructions.md")
    }
}

impl AgentInstaller for Copilot {
    fn id(&self) -> &'static str {
        "copilot"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".github").is_dir() || PathBuf::from(".github").is_dir()
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        if ctx.hook_only {
            return InstallReport {
                agent: NAME.to_string(),
                skipped: Some("rules only (--hook-only)".to_string()),
                ..Default::default()
            };
        }
        results_to_report(NAME, configure_copilot(home, ctx.dry_run))
    }
    fn uninstall(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let p = target_path(home);
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
    fn show(&self, home: &Path) -> ShowReport {
        let p = target_path(home);
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
