//! Cline / Roo installer — writes a project-local `.clinerules`.

use std::fs;
use std::path::{Path, PathBuf};

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, configure_cline,
    results_to_report, sha256_of,
};

pub struct Cline;
const NAME: &str = "Cline";

impl AgentInstaller for Cline {
    fn id(&self) -> &'static str {
        "cline"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".cline").is_dir() || PathBuf::from(".cline").is_dir()
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        if ctx.hook_only {
            return InstallReport {
                agent: NAME.to_string(),
                skipped: Some("rules only (--hook-only)".to_string()),
                ..Default::default()
            };
        }
        results_to_report(NAME, configure_cline(home, ctx.dry_run))
    }
    fn uninstall(&self, _home: &Path, ctx: &InstallContext) -> InstallReport {
        let p = PathBuf::from(".clinerules");
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
        let p = PathBuf::from(".clinerules");
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
