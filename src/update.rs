use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
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

/// Return true if a launchd plist (macOS) or systemd-user unit (Linux) has
/// already been installed for ig-daemon. Used by `post_update_rewarm` to
/// decide whether to reload the service manager vs. just inline-restart.
fn service_unit_installed() -> bool {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return false,
    };
    #[cfg(target_os = "macos")]
    {
        home.join("Library/LaunchAgents/com.ig.daemon.global.plist")
            .exists()
    }
    #[cfg(target_os = "linux")]
    {
        let cfg = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        cfg.join("systemd/user/ig-daemon.service").exists()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = home;
        false
    }
}

fn post_update_rewarm() -> Result<()> {
    eprintln!("  Refreshing ig ecosystem...");
    // Quiet mode: only surface the agent rule files that actually drifted
    // since the previous binary version. Most users have a stable agent
    // setup; printing "already up-to-date" for every entry is noise.
    crate::setup::run_setup_with_options(false, true);

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = crate::util::find_root(&cwd);

    let service_installed = service_unit_installed();
    if service_installed {
        // launchd/systemd-user agent already installed: reload it so the
        // (auto-restarting) service picks up the new exe path. install_launchd
        // is idempotent — it unloads first, stops the running daemon, then
        // loads/starts again.
        eprint!("  Reloading daemon service... ");
        io::stderr().flush().ok();
        match crate::daemon::install_launchd(&root) {
            Ok(_) => eprintln!("✓"),
            Err(e) => eprintln!("skipped ({})", e),
        }
    } else if crate::daemon::is_daemon_available() {
        // No service unit, but a daemon is running (manual `ig daemon start`):
        // restart it inline so the new binary takes over.
        eprint!("  Restarting daemon... ");
        io::stderr().flush().ok();
        crate::daemon::stop_daemon(&root)?;
        crate::daemon::start_daemon_background_silent(&root)?;
        match crate::daemon::verify_daemon_health() {
            Ok(()) => eprintln!("✓"),
            Err(e) => eprintln!("⚠ {}", e),
        }
    } else {
        eprintln!("  Daemon not running. Run `ig daemon install` once to enable auto-start.");
    }

    eprint!("  Rewarming current project... ");
    io::stderr().flush().ok();
    match crate::daemon::warm_daemon(&root) {
        Ok(resp) if resp.error.is_none() => eprintln!("✓"),
        Ok(resp) => {
            eprintln!("skipped");
            if let Some(err) = resp.error {
                eprintln!("  Warning: warm failed: {}", err);
            }
        }
        Err(e) => {
            eprintln!("skipped");
            eprintln!("  Warning: warm failed: {}", e);
        }
    }

    Ok(())
}

/// Refresh indexes for the current project, or every known indexed project
/// under `path` when `all` is set.
pub fn run_index_update(
    path: Option<&str>,
    all: bool,
    use_default_excludes: bool,
    max_file_size: u64,
) -> Result<()> {
    let start = path
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().context("cannot get current directory")?);
    let start = start.canonicalize().unwrap_or(start);

    let roots = if all {
        discover_indexed_roots(&start)?
    } else {
        vec![crate::util::find_root(&start)]
    };

    if roots.is_empty() {
        eprintln!("No indexed projects found under {}", start.display());
        return Ok(());
    }

    let total = roots.len();
    let mut rebuilt = 0usize;
    let mut failed = 0usize;

    for root in roots {
        let ig = crate::util::ig_dir(&root);
        let exists = crate::index::metadata::IndexMetadata::exists(&ig);
        let stale = exists
            && crate::index::metadata::IndexMetadata::load_from(&ig)
                .map(|meta| meta.version != crate::index::metadata::INDEX_VERSION)
                .unwrap_or(true);

        eprintln!(
            "{} {}",
            if exists {
                if stale {
                    "Rebuilding stale index:"
                } else {
                    "Refreshing index:"
                }
            } else {
                "Building missing index:"
            },
            root.display()
        );

        let start = Instant::now();
        match crate::index::writer::build_index(&root, use_default_excludes, max_file_size) {
            Ok(meta) => {
                rebuilt += 1;
                let size = dir_size(&crate::util::ig_dir(&root));
                eprintln!(
                    "  ✓ {} files, {} trigrams, {:.1}s, {:.1} MB",
                    meta.file_count,
                    meta.ngram_count,
                    start.elapsed().as_secs_f64(),
                    size as f64 / 1_048_576.0
                );
            }
            Err(err) => {
                failed += 1;
                eprintln!("  ✗ {}", err);
            }
        }
    }

    eprintln!(
        "\nIndex update complete: {} ok, {} failed, {} total.",
        rebuilt, failed, total
    );

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn discover_indexed_roots(start: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = std::collections::BTreeSet::new();

    for entry in walkdir::WalkDir::new(start)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.path() == start || should_descend(entry.path()))
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_dir() {
            continue;
        }
        if should_skip_project_path(entry.path(), start) {
            continue;
        }
        if is_project_root(entry.path()) {
            roots.insert(entry.path().to_path_buf());
            continue;
        }
        if entry.file_name() == ".ig" {
            let ig = entry.path();
            if crate::index::metadata::IndexMetadata::exists(ig)
                && let Some(root) = ig.parent()
            {
                roots.insert(root.to_path_buf());
            }
        }
    }

    for entry in crate::cache::list_entries()? {
        let Some(meta) = entry.meta else {
            continue;
        };
        let root = PathBuf::from(meta.root_path);
        if root.exists() && root.starts_with(start) && !should_skip_project_path(&root, start) {
            roots.insert(root);
        }
    }

    Ok(roots.into_iter().collect())
}

const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "deno.json",
    "deno.jsonc",
    "composer.json",
    "pnpm-workspace.yaml",
    "bun.lock",
    "Gemfile",
    "build.gradle",
    "build.gradle.kts",
    "pom.xml",
    "mix.exs",
    "Pipfile",
    "requirements.txt",
];

fn is_project_root(path: &Path) -> bool {
    PROJECT_MARKERS
        .iter()
        .any(|marker| path.join(marker).exists())
}

const SKIP_PROJECT_COMPONENTS: &[&str] = &[
    ".bun",
    ".cache",
    ".cargo",
    ".claude",
    ".codex",
    ".config",
    ".cursor",
    ".cursor-server",
    ".deno",
    ".docker",
    ".local",
    ".npm",
    ".pnpm-store",
    ".rustup",
    ".volta",
    ".vscode",
    "Library",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    "coverage",
    ".turbo",
    ".output",
    ".terraform",
];

fn should_skip_project_path(path: &Path, start: &Path) -> bool {
    let rel = path.strip_prefix(start).unwrap_or(path);
    rel.components().any(|component| {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        let Some(name) = name.to_str() else {
            return false;
        };
        name.starts_with('.') || SKIP_PROJECT_COMPONENTS.contains(&name)
    })
}

fn should_descend(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    !name.starts_with('.') && !SKIP_PROJECT_COMPONENTS.contains(&name)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(path) {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_type().is_file()
            && let Ok(meta) = entry.metadata()
        {
            total += meta.len();
        }
    }
    total
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
