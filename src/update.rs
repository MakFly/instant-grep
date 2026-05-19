use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const GITHUB_API_URL: &str = "https://api.github.com/repos/MakFly/instant-grep/releases/latest";
const CHECK_INTERVAL_SECS: u64 = 86400; // 24h
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Spawn a background thread that checks for updates (non-blocking).
/// All errors are silently ignored — this must never break the CLI.
pub fn check_update_background() {
    std::thread::spawn(|| {
        let _ = check_update();
    });
}

/// Interactive self-update with progress bar.
pub fn run_update() -> Result<()> {
    // v2.0+: this build no longer ships a daemon. If an old daemon from a
    // pre-v2.0 install is still running (manually or via launchd/systemd
    // user agent), tear it down here so the upgrade leaves no zombie
    // background process behind. Best-effort, never fatal.
    cleanup_legacy_daemon();

    eprint!("  Checking latest version... ");
    let response: serde_json::Value = ureq::get(GITHUB_API_URL)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call()
        .context("failed to reach GitHub API")?
        .body_mut()
        .read_json()
        .context("failed to parse release info")?;

    let tag = response
        .get("tag_name")
        .and_then(|t| t.as_str())
        .context("no tag_name in release")?;
    let latest = tag.strip_prefix('v').unwrap_or(tag);

    if !is_newer(latest) {
        eprintln!("✓");
        eprintln!("\n  Already up to date (v{}).", CURRENT_VERSION);
        return Ok(());
    }

    eprintln!("v{} → v{}", CURRENT_VERSION, latest);

    // v1.20+ single-binary layout: one artifact per arch, installed in-place
    // over the user's existing `ig`. `current_exe()` is the install target.
    let artifact = detect_artifact()?;
    let target = std::env::current_exe()
        .context("cannot determine binary path")?
        .canonicalize()
        .context("cannot canonicalize current binary path")?;

    if let Some(dir) = target.parent() {
        check_writable(dir)?;
    }

    let url = format!(
        "https://github.com/MakFly/instant-grep/releases/download/{}/{}",
        tag, artifact
    );

    eprint!("  Downloading {}...", artifact);
    io::stderr().flush().ok();
    let bytes = download_artifact(&url, &artifact)?;
    let size_mb = bytes.len() as f64 / 1_048_576.0;
    eprintln!(
        "\r  Downloading {}... [{}] {:.1} MB ✓",
        artifact,
        "█".repeat(20),
        size_mb
    );

    verify_checksums(tag, &[(&artifact, &bytes)]);

    eprint!("  Installing... ");
    io::stderr().flush().ok();
    atomic_install(&bytes, &target).context("failed to install ig")?;
    eprintln!("✓");

    // Migration from the pre-v1.20 shim+backend layout. If the new binary
    // landed at the C-shim path (~/.local/bin/ig) the backend at
    // ~/.local/share/ig/bin/ig-rust is now dead weight. Remove it + its
    // empty share dirs. Best-effort, silent on failure. `target` is passed
    // through so the sweep never deletes the binary we just installed (in
    // the legacy shim layout the install target *is* an ig-rust path).
    clean_legacy_backend(&target);

    eprintln!("\n  ✓ Updated: {} → {}", tag, target.display());

    eprintln!();
    post_update_rewarm()?;

    update_cache(latest);

    Ok(())
}

/// Pre-v1.20 `ig-rust` backend paths that become dead weight once the single
/// binary is installed. `installed` — the path `ig update` just wrote to — is
/// **excluded** from the list: in the legacy shim layout `current_exe()`
/// resolves to `~/.local/share/ig/bin/ig-rust`, so the install target *is* a
/// legacy path. Deleting it would nuke the binary we just installed and leave
/// the surviving C shim pointing at nothing.
fn legacy_backend_candidates(installed: &Path) -> Vec<PathBuf> {
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from("/usr/local/share/ig/bin/ig-rust"),
        PathBuf::from("/usr/local/bin/ig-rust"),
        PathBuf::from("/opt/homebrew/share/ig/bin/ig-rust"),
    ];
    if let Some(h) = home.as_ref() {
        candidates.extend([
            h.join(".local/share/ig/bin/ig-rust"),
            h.join(".local/bin/ig-rust"),
            h.join(".cargo/bin/ig-rust"),
        ]);
    }
    let installed_canon = installed
        .canonicalize()
        .unwrap_or_else(|_| installed.to_path_buf());
    candidates
        .into_iter()
        .filter(|p| {
            let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
            canon != installed_canon
        })
        .collect()
}

/// Sweep pre-v1.20 `ig-rust` backend artifacts. The new binary is
/// self-contained at the shim path, so any `ig-rust` sibling is stale
/// and would be misleading to anyone debugging their PATH.
fn clean_legacy_backend(installed: &Path) {
    for p in legacy_backend_candidates(installed) {
        if p.exists() {
            match fs::remove_file(&p) {
                Ok(_) => eprintln!("  → Removed legacy backend: {}", p.display()),
                Err(e) => eprintln!("  ⚠ Could not remove {}: {}", p.display(), e),
            }
        }
    }
    // Tidy empty share dirs left behind. `remove_dir` only removes empty
    // directories, so a share dir still holding the install target (legacy
    // shim layout) is left untouched.
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let share_dirs: Vec<PathBuf> = {
        let mut v = vec![
            PathBuf::from("/usr/local/share/ig/bin"),
            PathBuf::from("/usr/local/share/ig"),
            PathBuf::from("/opt/homebrew/share/ig/bin"),
            PathBuf::from("/opt/homebrew/share/ig"),
        ];
        if let Some(h) = home.as_ref() {
            v.push(h.join(".local/share/ig/bin"));
            v.push(h.join(".local/share/ig"));
        }
        v
    };
    for d in share_dirs {
        let _ = fs::remove_dir(&d);
    }
}

/// v2.0 upgrade cleanup: tear down any pre-v2.0 daemon left behind by the
/// previous installation. Best-effort — every step is silent on failure so
/// the update path can never abort here.
fn cleanup_legacy_daemon() {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };

    // 1. macOS launchd plist (bootout + remove).
    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library/LaunchAgents/com.ig.daemon.global.plist");
        if plist.exists() {
            // Best-effort: get uid for `bootout gui/<uid>` and fall back to
            // legacy `unload` if `bootout` isn't available.
            let uid = unsafe { libc::getuid() };
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &format!("gui/{}/com.ig.daemon.global", uid)])
                .output();
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist.to_string_lossy()])
                .output();
            let _ = fs::remove_file(&plist);
        }
    }

    // 2. Linux systemd-user unit (disable + remove).
    #[cfg(target_os = "linux")]
    {
        let cfg = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let unit = cfg.join("systemd/user/ig-daemon.service");
        if unit.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "ig-daemon.service"])
                .output();
            let _ = fs::remove_file(&unit);
        }
    }

    // 3. Kill any still-running `ig daemon` process from an old binary.
    //    `pkill -f` matches against the full command line.
    let _ = std::process::Command::new("pkill")
        .args(["-TERM", "-f", "ig daemon"])
        .output();

    // 4. Remove daemon socket / pid / log files under the XDG cache.
    let cache_root = if cfg!(target_os = "macos") {
        home.join("Library/Caches/ig")
    } else if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("ig")
    } else {
        home.join(".cache/ig")
    };
    let daemon_dir = cache_root.join("daemon");
    if daemon_dir.is_dir() {
        for entry in fs::read_dir(&daemon_dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // Sweep daemon.sock, daemon.pid, daemon.log, daemon.log.1..5, memory.cooldown.json
            if name.starts_with("daemon.") || name.starts_with("memory.") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

fn post_update_rewarm() -> Result<()> {
    eprintln!("  Refreshing ig ecosystem...");
    // Quiet mode: only surface the agent rule files that actually drifted
    // since the previous binary version. Most users have a stable agent
    // setup; printing "already up-to-date" for every entry is noise.
    crate::setup::run_setup_with_options(false, true);
    Ok(())
}

/// Determine where to install the shim and the Rust backend, and whether a
/// legacy ig-rust binary should be cleaned up after migration.
///
/// Layout convention:
///   - shim    → first `ig` found in $PATH (or `$HOME/.local/bin/ig` as fallback)
///   - backend → `$HOME/.local/share/ig/bin/ig-rust` (or unchanged if already there)
///
/// If the current binary lives next to a shim (legacy ~/.local/bin layout), it
/// will be migrated to the share directory and the old file flagged for removal.
fn check_writable(dir: &std::path::Path) -> Result<()> {
    let probe = dir.join(".ig_write_probe");
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            anyhow::bail!(
                "Permission denied writing to {}. Try: sudo ig update",
                dir.display()
            )
        }
        Err(_) => Ok(()), // other errors handled later at actual write time
    }
}

fn download_artifact(url: &str, name: &str) -> Result<Vec<u8>> {
    let bytes = ureq::get(url)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call()
        .with_context(|| format!("download failed for {}", name))?
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("failed to read response body for {}", name))?;
    let size_mb = bytes.len() as f64 / 1_048_576.0;
    eprintln!(
        "\r  Downloading {}... [{}] {:.1} MB ✓",
        name,
        "█".repeat(20),
        size_mb
    );
    Ok(bytes)
}

fn atomic_install(bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let dir = dest
        .parent()
        .context("destination has no parent directory")?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).context("cannot create temporary file")?;
    tmp.write_all(bytes)
        .context("cannot write to temporary file")?;
    tmp.flush().context("cannot flush temporary file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755))
            .context("cannot set executable permissions")?;
    }

    // persist() does an atomic rename; falls back to copy on cross-device
    if let Err(e) = tmp.persist(dest) {
        fs::copy(e.file.path(), dest).context("failed to copy binary into place")?;
    }

    // macOS: re-sign with a stable identifier so TCC and BTM stop treating
    // every `ig update` as a brand-new app. Without `-i`, codesign embeds
    // the binary hash into the identifier (`ig-<sha256-prefix>`), which
    // forces a fresh permission prompt on every rebuild. Best-effort: a
    // failed sign-attempt isn't fatal — the binary remains usable, the user
    // will just see TCC prompts again until the next update.
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-", "--identifier", "dev.makfly.ig"])
            .arg(dest)
            .status();
    }

    Ok(())
}

/// Best-effort SHA-256 checksum verification against the release checksums.txt.
/// Logs warnings on mismatch but never blocks the update.
fn verify_checksums(tag: &str, artifacts: &[(&str, &[u8])]) {
    use sha2::{Digest, Sha256};

    let checksum_url = format!(
        "https://github.com/MakFly/instant-grep/releases/download/{}/checksums.txt",
        tag
    );
    let body = match ureq::get(&checksum_url)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
    {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "  Warning: checksums.txt not available for {}; skipping verification.",
                tag
            );
            return;
        }
    };

    for (name, data) in artifacts {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let computed = format!("{:x}", hasher.finalize());
        let matched = body.lines().any(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next().unwrap_or("");
            let fname = parts.next().unwrap_or("").trim_start_matches('*');
            fname == *name && hash == computed
        });
        if !matched {
            eprintln!(
                "  Warning: checksum mismatch for {} — proceeding anyway.",
                name
            );
        }
    }
}

/// Returns `(shim_artifact, rust_artifact)` for the current platform.
pub fn detect_artifact() -> Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let base = match (os, arch) {
        ("macos", "aarch64") => "ig-macos-aarch64",
        ("macos", "x86_64") => "ig-macos-x86_64",
        ("linux", "x86_64") => "ig-linux-x86_64",
        ("linux", "aarch64") => "ig-linux-aarch64",
        _ => anyhow::bail!("unsupported platform: {}-{}", os, arch),
    };
    Ok(base.to_string())
}

fn update_cache(latest: &str) {
    if let Some(cache) = cache_path() {
        if let Some(dir) = cache.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(mut f) = fs::File::create(&cache) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = writeln!(f, "{}", now);
            let _ = writeln!(f, "{}", latest);
        }
    }
}

fn cache_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("ig"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("ig"))
}

fn cache_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("last_update_check"))
}

fn check_update() -> Option<()> {
    let cache = cache_path()?;

    if let Ok(contents) = fs::read_to_string(&cache) {
        let mut lines = contents.lines();
        if let Some(timestamp_str) = lines.next()
            && let Ok(last_check) = timestamp_str.parse::<u64>()
        {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
            if now.saturating_sub(last_check) < CHECK_INTERVAL_SECS {
                if let Some(cached_version) = lines.next()
                    && is_newer(cached_version)
                {
                    print_update_message(cached_version);
                }
                return Some(());
            }
        }
    }

    let response: serde_json::Value = ureq::get(GITHUB_API_URL)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call()
        .ok()?
        .body_mut()
        .read_json()
        .ok()?;

    let tag = response.get("tag_name")?.as_str()?;
    let latest = tag.strip_prefix('v').unwrap_or(tag);

    if let Some(dir) = cache_dir() {
        let _ = fs::create_dir_all(&dir);
        if let Ok(mut f) = fs::File::create(&cache) {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
            let _ = writeln!(f, "{}", now);
            let _ = writeln!(f, "{}", latest);
        }
    }

    if is_newer(latest) {
        print_update_message(latest);
    }

    Some(())
}

fn is_newer(latest: &str) -> bool {
    let parse = |v: &str| -> Option<(u64, u64, u64)> {
        let mut parts = v.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        Some((major, minor, patch))
    };

    match (parse(CURRENT_VERSION), parse(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

fn print_update_message(latest: &str) {
    eprintln!(
        "\x1b[33mig v{} available (current: v{}). Run `ig update` to upgrade.\x1b[0m",
        latest, CURRENT_VERSION,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_true() {
        assert!(is_newer("99.0.0"));
        assert!(is_newer("99.99.99"));
    }

    #[test]
    fn test_is_newer_false() {
        assert!(!is_newer("0.0.1"));
        assert!(!is_newer("0.1.0"));
        assert!(!is_newer(CURRENT_VERSION));
    }

    #[test]
    fn test_is_newer_invalid() {
        assert!(!is_newer("not-a-version"));
        assert!(!is_newer(""));
        assert!(!is_newer("1.0"));
    }

    /// The binary `ig update` just installed must never appear in the legacy
    /// sweep — in the pre-v1.20 shim layout the install target *is* an
    /// `ig-rust` path, and deleting it would strand the surviving C shim.
    #[test]
    fn legacy_backend_candidates_excludes_install_target() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let target = PathBuf::from(&home).join(".local/share/ig/bin/ig-rust");
        let candidates = legacy_backend_candidates(&target);
        assert!(
            !candidates.contains(&target),
            "install target {} must be excluded from the legacy sweep",
            target.display()
        );
        // Other legacy paths are still swept.
        assert!(candidates.contains(&PathBuf::from("/usr/local/bin/ig-rust")));
    }

    /// When the install target is unrelated to any legacy `ig-rust` path,
    /// every candidate is kept.
    #[test]
    fn legacy_backend_candidates_keeps_all_when_target_unrelated() {
        let target = PathBuf::from("/opt/ig/bin/ig");
        let candidates = legacy_backend_candidates(&target);
        assert!(candidates.contains(&PathBuf::from("/usr/local/bin/ig-rust")));
        assert!(candidates.contains(&PathBuf::from("/usr/local/share/ig/bin/ig-rust")));
    }

    #[test]
    fn test_cache_path_exists() {
        if std::env::var("HOME").is_ok() {
            assert!(cache_path().is_some());
            let path = cache_path().unwrap();
            assert!(path.to_string_lossy().contains(".config/ig"));
        }
    }

    /// detect_artifact() must return a single string starting with `ig-` for
    /// the current platform.
    #[test]
    fn detect_artifact_returns_single_string() {
        let artifact = detect_artifact().unwrap();
        assert!(
            artifact.starts_with("ig-"),
            "artifact='{}' should start with ig-",
            artifact
        );
        // No more "-rust" suffix in v1.20+ — single-binary layout.
        assert!(
            !artifact.ends_with("-rust"),
            "artifact='{}' should NOT end with -rust",
            artifact
        );
    }

    /// Verify each of the 4 supported platforms produces the expected artifact name.
    #[test]
    fn detect_artifact_all_platforms() {
        let cases = [
            ("macos", "aarch64", "ig-macos-aarch64"),
            ("macos", "x86_64", "ig-macos-x86_64"),
            ("linux", "x86_64", "ig-linux-x86_64"),
            ("linux", "aarch64", "ig-linux-aarch64"),
        ];
        for (os, arch, expected) in cases {
            let base = match (os, arch) {
                ("macos", "aarch64") => "ig-macos-aarch64",
                ("macos", "x86_64") => "ig-macos-x86_64",
                ("linux", "x86_64") => "ig-linux-x86_64",
                ("linux", "aarch64") => "ig-linux-aarch64",
                _ => panic!("unexpected combo"),
            };
            assert_eq!(base, expected, "os={os} arch={arch}");
        }
    }
}
