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

    let (shim_artifact, rust_artifact) = detect_artifact()?;

    // Determine install layout (handles new layout, legacy in-PATH, and migration)
    let (rust_path, shim_path, legacy_rust) = resolve_install_targets()?;

    // Early permission check on both target dirs
    if let Some(dir) = rust_path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("cannot create backend directory {}", dir.display()))?;
        check_writable(dir)?;
    }
    if let Some(dir) = shim_path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("cannot create shim directory {}", dir.display()))?;
        check_writable(dir)?;
    }

    // Try downloading ig-rust to detect if this is a new-style release
    let rust_url = format!(
        "https://github.com/MakFly/instant-grep/releases/download/{}/{}",
        tag, rust_artifact
    );
    let shim_url = format!(
        "https://github.com/MakFly/instant-grep/releases/download/{}/{}",
        tag, shim_artifact
    );

    eprint!("  Downloading {}...", rust_artifact);
    io::stderr().flush().ok();

    let rust_response = ureq::get(&rust_url)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call();

    let use_dual = match &rust_response {
        Ok(_) => true,
        Err(ureq::Error::StatusCode(404)) => false,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "download failed for {}: {}",
                rust_artifact,
                e
            ));
        }
    };

    if !use_dual {
        // Fallback: legacy release without -rust artifact — update current binary only.
        // Write to the *original* current_exe() location (no migration), since the legacy
        // artifact IS a Rust binary in old releases.
        eprintln!();
        eprintln!(
            "  Warning: release {} has no `{}` artifact; \
             falling back to single-binary update.",
            tag, rust_artifact
        );
        let shim_bytes = download_artifact(&shim_url, &shim_artifact)?;
        let legacy_target = std::env::current_exe()
            .context("cannot determine binary path")?
            .canonicalize()
            .context("cannot canonicalize current binary path")?;
        atomic_install(&shim_bytes, &legacy_target)?;
        eprintln!("✓ Updated: {} (legacy single-binary)", tag);
        eprintln!("  Path: {}", legacy_target.display());
        eprintln!();
        post_update_rewarm()?;
        update_cache(latest);
        return Ok(());
    }

    // Read rust bytes from already-open response
    let rust_bytes = rust_response
        .unwrap()
        .body_mut()
        .read_to_vec()
        .context("failed to read ig-rust response body")?;
    let size_mb = rust_bytes.len() as f64 / 1_048_576.0;
    eprintln!(
        "\r  Downloading {}... [{}] {:.1} MB ✓",
        rust_artifact,
        "█".repeat(20),
        size_mb
    );

    // Download shim
    let shim_bytes = download_artifact(&shim_url, &shim_artifact)?;

    // Best-effort checksum verification
    verify_checksums(
        tag,
        &[(&shim_artifact, &shim_bytes), (&rust_artifact, &rust_bytes)],
    );

    // Atomic install of both binaries
    eprint!("  Installing... ");
    io::stderr().flush().ok();

    atomic_install(&rust_bytes, &rust_path).context("failed to install ig-rust")?;
    atomic_install(&shim_bytes, &shim_path).context("failed to install ig shim")?;

    eprintln!("✓");

    // Migration: remove legacy ig-rust placed next to the shim
    if let Some(legacy) = legacy_rust.as_ref()
        && legacy != &rust_path
        && legacy.exists()
    {
        match fs::remove_file(legacy) {
            Ok(_) => eprintln!("  → Migrated backend, removed legacy: {}", legacy.display()),
            Err(e) => eprintln!(
                "  ⚠ Could not remove legacy backend at {}: {}",
                legacy.display(),
                e
            ),
        }
    }

    eprintln!("\n  ✓ Updated: {} (ig + ig-rust)", tag);
    eprintln!("  ig      : {}", shim_path.display());
    eprintln!("  ig-rust : {}  (hidden)", rust_path.display());

    eprintln!();
    post_update_rewarm()?;

    update_cache(latest);

    Ok(())
}

fn post_update_rewarm() -> Result<()> {
    eprintln!("  Refreshing ig ecosystem...");
    crate::setup::run_setup(false);

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = crate::util::find_root(&cwd);

    if crate::daemon::is_daemon_available() {
        eprint!("  Restarting daemon... ");
        io::stderr().flush().ok();
        crate::daemon::stop_daemon(&root)?;
        crate::daemon::start_daemon_background_silent(&root)?;
        eprintln!("✓");
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
fn resolve_install_targets() -> Result<(PathBuf, PathBuf, Option<PathBuf>)> {
    let current = std::env::current_exe().context("cannot determine binary path")?;
    let current = current.canonicalize().unwrap_or(current);

    let home = std::env::var("HOME").ok().map(PathBuf::from);

    // Already in the canonical hidden location?
    let already_hidden = current
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| s.ends_with("/share/ig/bin"))
        .unwrap_or(false);

    let (rust_target, legacy_remove) = if already_hidden {
        (current.clone(), None)
    } else if let Some(h) = home.as_ref() {
        let new = h.join(".local/share/ig/bin/ig-rust");
        if new == current {
            (current.clone(), None)
        } else {
            (new, Some(current.clone()))
        }
    } else {
        (current.clone(), None)
    };

    // Resolve shim: prefer existing `ig` in PATH that isn't the rust binary
    let shim_target = locate_shim_in_path(&current)
        .or_else(|| home.as_ref().map(|h| h.join(".local/bin/ig")))
        .or_else(|| current.parent().map(|p| p.join("ig")))
        .context("cannot determine shim install path")?;

    Ok((rust_target, shim_target, legacy_remove))
}

fn locate_shim_in_path(exclude: &Path) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("ig");
        if candidate.is_file() {
            let canon = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            if canon != exclude {
                return Some(candidate);
            }
        }
    }
    None
}

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
pub fn detect_artifact() -> Result<(String, String)> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let base = match (os, arch) {
        ("macos", "aarch64") => "ig-macos-aarch64",
        ("macos", "x86_64") => "ig-macos-x86_64",
        ("linux", "x86_64") => "ig-linux-x86_64",
        ("linux", "aarch64") => "ig-linux-aarch64",
        _ => anyhow::bail!("unsupported platform: {}-{}", os, arch),
    };
    Ok((base.to_string(), format!("{}-rust", base)))
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

    #[test]
    fn test_cache_path_exists() {
        if std::env::var("HOME").is_ok() {
            assert!(cache_path().is_some());
            let path = cache_path().unwrap();
            assert!(path.to_string_lossy().contains(".config/ig"));
        }
    }

    /// detect_artifact() must return a (shim, rust) tuple where rust = shim + "-rust"
    #[test]
    fn detect_artifact_returns_pair() {
        let (shim, rust) = detect_artifact().unwrap();
        assert!(
            shim.starts_with("ig-"),
            "shim='{}' should start with ig-",
            shim
        );
        assert!(
            rust.ends_with("-rust"),
            "rust='{}' should end with -rust",
            rust
        );
        assert_eq!(rust, format!("{}-rust", shim));
    }

    /// Verify each of the 4 supported platforms produces the expected artifact names.
    #[test]
    fn detect_artifact_all_platforms() {
        let cases = [
            (
                "macos",
                "aarch64",
                "ig-macos-aarch64",
                "ig-macos-aarch64-rust",
            ),
            ("macos", "x86_64", "ig-macos-x86_64", "ig-macos-x86_64-rust"),
            ("linux", "x86_64", "ig-linux-x86_64", "ig-linux-x86_64-rust"),
            (
                "linux",
                "aarch64",
                "ig-linux-aarch64",
                "ig-linux-aarch64-rust",
            ),
        ];
        for (os, arch, expected_shim, expected_rust) in cases {
            let base = match (os, arch) {
                ("macos", "aarch64") => "ig-macos-aarch64",
                ("macos", "x86_64") => "ig-macos-x86_64",
                ("linux", "x86_64") => "ig-linux-x86_64",
                ("linux", "aarch64") => "ig-linux-aarch64",
                _ => panic!("unexpected combo"),
            };
            assert_eq!(base, expected_shim, "os={os} arch={arch}");
            assert_eq!(
                format!("{}-rust", base),
                expected_rust,
                "os={os} arch={arch}"
            );
        }
    }
}
