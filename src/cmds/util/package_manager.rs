//! Detect the active Node package manager based on lockfile presence.

use std::path::Path;

/// Node package manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageManager {
    Pnpm,
    Yarn,
    Npm,
    Bun,
}

impl PackageManager {
    /// Bin name used to invoke the manager (`pnpm`, `yarn`, `npm`, `bun`).
    pub fn bin(self) -> &'static str {
        match self {
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Npm => "npm",
            PackageManager::Bun => "bun",
        }
    }

    /// `<bin> exec` prefix used to run a local node-module binary
    /// (`pnpm exec vitest`, `yarn exec vitest`, …).
    /// For npm we use `npx`, for bun we use `bunx`.
    pub fn exec_argv(self) -> Vec<&'static str> {
        match self {
            PackageManager::Pnpm => vec!["pnpm", "exec"],
            PackageManager::Yarn => vec!["yarn", "exec", "--"],
            PackageManager::Npm => vec!["npx", "--no-install"],
            PackageManager::Bun => vec!["bunx"],
        }
    }
}

/// Detect the package manager by looking for a lockfile in `cwd`.
/// Falls back to `Npm` when none is found.
pub fn detect_pm(cwd: &Path) -> PackageManager {
    if cwd.join("pnpm-lock.yaml").exists() {
        PackageManager::Pnpm
    } else if cwd.join("bun.lockb").exists() || cwd.join("bun.lock").exists() {
        PackageManager::Bun
    } else if cwd.join("yarn.lock").exists() {
        PackageManager::Yarn
    } else {
        // Either package-lock.json (npm) or no lockfile — default to npm.
        PackageManager::Npm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn defaults_to_npm() {
        let d = TempDir::new().unwrap();
        assert_eq!(detect_pm(d.path()), PackageManager::Npm);
    }

    #[test]
    fn detects_pnpm() {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_pm(d.path()), PackageManager::Pnpm);
    }

    #[test]
    fn detects_yarn() {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join("yarn.lock"), "").unwrap();
        assert_eq!(detect_pm(d.path()), PackageManager::Yarn);
    }

    #[test]
    fn detects_bun() {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join("bun.lockb"), "").unwrap();
        assert_eq!(detect_pm(d.path()), PackageManager::Bun);
    }

    #[test]
    fn pnpm_lock_wins_over_npm_lock() {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(d.path().join("package-lock.json"), "").unwrap();
        assert_eq!(detect_pm(d.path()), PackageManager::Pnpm);
    }
}
