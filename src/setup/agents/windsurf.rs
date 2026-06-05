//! Windsurf installer — writes a project-local `.windsurfrules`.

use std::fs;
use std::path::{Path, PathBuf};

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, configure_windsurf,
    results_to_report, sha256_of,
};

pub struct Windsurf;
const NAME: &str = "Windsurf";

impl AgentInstaller for Windsurf {
    fn id(&self) -> &'static str {
        "windsurf"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".windsurf").is_dir() || super::super::which_exists("windsurf")
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        if ctx.hook_only {
            return InstallReport {
                agent: NAME.to_string(),
                skipped: Some("rules only (--hook-only)".to_string()),
                ..Default::default()
            };
        }
        results_to_report(NAME, configure_windsurf(home, ctx.dry_run))
    }
    fn uninstall(&self, _home: &Path, ctx: &InstallContext) -> InstallReport {
        let p = PathBuf::from(".windsurfrules");
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
        let p = PathBuf::from(".windsurfrules");
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
