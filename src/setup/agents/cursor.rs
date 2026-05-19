//! Cursor installer — writes a `~/.cursor/rules/ig-search.mdc` snippet.

use std::fs;
use std::path::Path;

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, configure_cursor,
    results_to_report, sha256_of,
};

pub struct Cursor;
const NAME: &str = "Cursor";

impl AgentInstaller for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".cursor").is_dir()
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        if ctx.hook_only {
            return InstallReport {
                agent: NAME.to_string(),
                skipped: Some("rules only (--hook-only)".to_string()),
                ..Default::default()
            };
        }
        results_to_report(NAME, configure_cursor(home, ctx.dry_run))
    }
    fn uninstall(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let p = home.join(".cursor/rules/ig-search.mdc");
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
        let p = home.join(".cursor/rules/ig-search.mdc");
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
