//! Hermes installer — drops a small Python plugin under
//! `~/.hermes/plugins/ig-rewrite/__init__.py`.
//!
//! Best-effort detection: if `~/.hermes/` is missing, we skip silently
//! unless `--auto-patch` is set, in which case the directory is created.

use std::fs;
use std::path::Path;

use super::super::{
    AgentInstaller, InstallContext, InstallReport, ShowItem, ShowReport, sha256_of,
};

pub struct Hermes;
const NAME: &str = "Hermes";
const PLUGIN_SRC: &str = include_str!("../../../hooks/hermes/__init__.py");

fn plugin_path(home: &Path) -> std::path::PathBuf {
    home.join(".hermes/plugins/ig-rewrite/__init__.py")
}

impl AgentInstaller for Hermes {
    fn id(&self) -> &'static str {
        "hermes"
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".hermes").is_dir()
    }
    fn install(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let mut r = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        let hermes_dir = home.join(".hermes");
        if !hermes_dir.is_dir() && !ctx.auto_patch {
            r.skipped = Some(format!(
                "{} not present (use --auto-patch)",
                hermes_dir.display()
            ));
            return r;
        }
        let path = plugin_path(home);
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if existing == PLUGIN_SRC {
            r.already_done += 1;
            return r;
        }
        if !ctx.dry_run {
            if let Some(parent) = path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    r.errors.push(format!("mkdir: {}", e));
                    return r;
                }
            }
            if let Err(e) = fs::write(&path, PLUGIN_SRC) {
                r.errors.push(format!("write: {}", e));
                return r;
            }
        }
        r.configured += 1;
        r
    }
    fn uninstall(&self, home: &Path, ctx: &InstallContext) -> InstallReport {
        let mut r = InstallReport {
            agent: NAME.to_string(),
            ..Default::default()
        };
        let path = plugin_path(home);
        if path.exists() {
            if !ctx.dry_run {
                let _ = fs::remove_file(&path);
            }
            r.configured = 1;
        }
        r
    }
    fn show(&self, home: &Path) -> ShowReport {
        let p = plugin_path(home);
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
