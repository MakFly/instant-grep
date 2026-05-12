#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

/// Global ig configuration, loaded from ~/.config/ig/config.toml.
#[derive(Deserialize, Default)]
pub struct IgConfig {
    #[serde(default)]
    pub tracking: TrackingConfig,
    #[serde(default)]
    pub filters: FilterConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub cache: CacheConfig,
}

#[derive(Deserialize)]
pub struct TrackingConfig {
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

impl Default for TrackingConfig {
    fn default() -> Self {
        Self {
            retention_days: default_retention_days(),
        }
    }
}

fn default_retention_days() -> u32 {
    90
}

#[derive(Deserialize, Default)]
pub struct FilterConfig {
    /// Override the user filter directory (default: ~/.config/ig/filters/)
    pub user_dir: Option<PathBuf>,
}

#[derive(Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_grep_max")]
    pub grep_max_results: usize,
    #[serde(default = "default_head")]
    pub head_default: usize,
    #[serde(default = "default_daemon_soft_rss_mb")]
    pub daemon_soft_rss_mb: usize,
    #[serde(default = "default_daemon_hard_rss_mb")]
    pub daemon_hard_rss_mb: usize,
    #[serde(default = "default_daemon_cooldown_secs")]
    pub daemon_cooldown_secs: u64,
    #[serde(default = "default_daemon_max_active_projects")]
    pub daemon_max_active_projects: usize,
    #[serde(default = "default_daemon_project_idle_secs")]
    pub daemon_project_idle_secs: u64,
    #[serde(default = "default_index_memory_mb")]
    pub index_memory_mb: usize,
    #[serde(default = "default_index_batch_size")]
    pub index_batch_size: usize,
    #[serde(default = "default_semantic_index")]
    pub semantic_index: bool,
    #[serde(default = "default_daemon_semantic_index")]
    pub daemon_semantic_index: bool,
}

#[derive(Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_auto_gc")]
    pub auto_gc: bool,
    #[serde(default = "default_auto_gc_interval_secs")]
    pub auto_gc_interval_secs: u64,
    #[serde(default = "default_auto_gc_days")]
    pub auto_gc_days: u64,
    #[serde(default = "default_auto_gc_max_size_mb")]
    pub auto_gc_max_size_mb: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            auto_gc: default_auto_gc(),
            auto_gc_interval_secs: default_auto_gc_interval_secs(),
            auto_gc_days: default_auto_gc_days(),
            auto_gc_max_size_mb: default_auto_gc_max_size_mb(),
        }
    }
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            grep_max_results: default_grep_max(),
            head_default: default_head(),
            daemon_soft_rss_mb: default_daemon_soft_rss_mb(),
            daemon_hard_rss_mb: default_daemon_hard_rss_mb(),
            daemon_cooldown_secs: default_daemon_cooldown_secs(),
            daemon_max_active_projects: default_daemon_max_active_projects(),
            daemon_project_idle_secs: default_daemon_project_idle_secs(),
            index_memory_mb: default_index_memory_mb(),
            index_batch_size: default_index_batch_size(),
            semantic_index: default_semantic_index(),
            daemon_semantic_index: default_daemon_semantic_index(),
        }
    }
}

fn default_grep_max() -> usize {
    1000
}

fn default_head() -> usize {
    250
}

fn default_daemon_soft_rss_mb() -> usize {
    768
}

fn default_daemon_hard_rss_mb() -> usize {
    1024
}

fn default_daemon_cooldown_secs() -> u64 {
    60
}

fn default_daemon_max_active_projects() -> usize {
    8
}

fn default_daemon_project_idle_secs() -> u64 {
    5 * 60
}

fn default_index_memory_mb() -> usize {
    64
}

fn default_index_batch_size() -> usize {
    250
}

fn default_semantic_index() -> bool {
    true
}

fn default_daemon_semantic_index() -> bool {
    false
}

fn default_auto_gc() -> bool {
    true
}

fn default_auto_gc_interval_secs() -> u64 {
    60 * 60
}

fn default_auto_gc_days() -> u64 {
    30
}

fn default_auto_gc_max_size_mb() -> u64 {
    5 * 1024
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn env_bool(name: &str) -> Option<bool> {
    std::env::var(name).ok().and_then(|s| match s.as_str() {
        "1" | "true" | "TRUE" | "yes" | "on" => Some(true),
        "0" | "false" | "FALSE" | "no" | "off" => Some(false),
        _ => None,
    })
}

pub fn daemon_soft_rss_mb() -> usize {
    env_usize("IG_DAEMON_SOFT_RSS_MB").unwrap_or(config().limits.daemon_soft_rss_mb)
}

pub fn daemon_hard_rss_mb() -> usize {
    env_usize("IG_DAEMON_HARD_RSS_MB").unwrap_or(config().limits.daemon_hard_rss_mb)
}

pub fn daemon_cooldown_secs() -> u64 {
    env_u64("IG_DAEMON_COOLDOWN_SECS").unwrap_or(config().limits.daemon_cooldown_secs)
}

pub fn daemon_max_active_projects() -> usize {
    env_usize("IG_DAEMON_TENANTS_MAX")
        .or_else(|| env_usize("IG_DAEMON_MAX_ACTIVE_PROJECTS"))
        .unwrap_or(config().limits.daemon_max_active_projects)
}

pub fn daemon_project_idle_secs() -> u64 {
    env_u64("IG_DAEMON_PROJECT_IDLE_SECS").unwrap_or(config().limits.daemon_project_idle_secs)
}

pub fn index_memory_budget_bytes() -> usize {
    let mb = env_usize("IG_INDEX_MEMORY_MB").unwrap_or(config().limits.index_memory_mb);
    mb.max(1) * 1024 * 1024
}

pub fn index_batch_size() -> usize {
    env_usize("IG_INDEX_BATCH_SIZE")
        .unwrap_or(config().limits.index_batch_size)
        .max(1)
}

pub fn semantic_index_enabled() -> bool {
    if let Some(v) = env_bool("IG_SEMANTIC") {
        return v;
    }
    if std::env::var_os("IG_DAEMON_FOREGROUND").is_some() {
        config().limits.daemon_semantic_index
    } else {
        config().limits.semantic_index
    }
}

pub fn cache_auto_gc_enabled() -> bool {
    env_bool("IG_AUTO_GC").unwrap_or(config().cache.auto_gc)
}

pub fn cache_auto_gc_interval_secs() -> u64 {
    env_u64("IG_CACHE_GC_INTERVAL_SECS")
        .unwrap_or(config().cache.auto_gc_interval_secs)
        .max(60)
}

pub fn cache_auto_gc_days() -> Option<u64> {
    let days = env_u64("IG_CACHE_GC_DAYS").unwrap_or(config().cache.auto_gc_days);
    (days > 0).then_some(days)
}

pub fn cache_auto_gc_max_size_bytes() -> Option<u64> {
    let mb = env_u64("IG_CACHE_MAX_SIZE_MB").unwrap_or(config().cache.auto_gc_max_size_mb);
    (mb > 0).then_some(mb.saturating_mul(1024 * 1024))
}

static CONFIG: OnceLock<IgConfig> = OnceLock::new();

/// Get the global config singleton. Loads from disk on first call.
pub fn config() -> &'static IgConfig {
    CONFIG.get_or_init(load_config)
}

/// Load config from ~/.config/ig/config.toml, falling back to defaults.
fn load_config() -> IgConfig {
    let Some(config_dir) = dirs::config_dir() else {
        return IgConfig::default();
    };

    let path = config_dir.join("ig").join("config.toml");
    if !path.exists() {
        return IgConfig::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("ig: warn: failed to parse {}: {}", path.display(), e);
                IgConfig::default()
            }
        },
        Err(e) => {
            eprintln!("ig: warn: failed to read {}: {}", path.display(), e);
            IgConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let cfg = IgConfig::default();
        assert_eq!(cfg.tracking.retention_days, 90);
        assert_eq!(cfg.limits.grep_max_results, 1000);
        assert_eq!(cfg.limits.head_default, 250);
        assert_eq!(cfg.limits.daemon_soft_rss_mb, 768);
        assert_eq!(cfg.limits.daemon_hard_rss_mb, 1024);
        assert_eq!(cfg.limits.daemon_max_active_projects, 8);
        assert_eq!(cfg.limits.daemon_project_idle_secs, 300);
        assert_eq!(cfg.limits.index_memory_mb, 64);
        assert_eq!(cfg.limits.index_batch_size, 250);
        assert!(cfg.limits.semantic_index);
        assert!(!cfg.limits.daemon_semantic_index);
        assert!(cfg.filters.user_dir.is_none());
        assert!(cfg.cache.auto_gc);
        assert_eq!(cfg.cache.auto_gc_interval_secs, 3600);
        assert_eq!(cfg.cache.auto_gc_days, 30);
        assert_eq!(cfg.cache.auto_gc_max_size_mb, 5120);
    }

    #[test]
    fn test_config_accessor_does_not_panic() {
        let cfg = config();
        assert!(cfg.tracking.retention_days > 0);
    }

    #[test]
    fn test_deserialize_partial_config() {
        let toml = r#"
[tracking]
retention_days = 30
"#;
        let cfg: IgConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.tracking.retention_days, 30);
        // Other fields use defaults
        assert_eq!(cfg.limits.grep_max_results, 1000);
    }

    #[test]
    fn test_deserialize_full_config() {
        let toml = r#"
[tracking]
retention_days = 60

[filters]
user_dir = "/custom/filters"

[limits]
grep_max_results = 500
head_default = 100
daemon_soft_rss_mb = 256
daemon_hard_rss_mb = 512
daemon_cooldown_secs = 30
daemon_max_active_projects = 3
daemon_project_idle_secs = 120
index_memory_mb = 32
index_batch_size = 100
semantic_index = false
daemon_semantic_index = false

[cache]
auto_gc = false
auto_gc_interval_secs = 120
auto_gc_days = 14
auto_gc_max_size_mb = 2048
"#;
        let cfg: IgConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.tracking.retention_days, 60);
        assert_eq!(
            cfg.filters.user_dir.as_deref(),
            Some(std::path::Path::new("/custom/filters"))
        );
        assert_eq!(cfg.limits.grep_max_results, 500);
        assert_eq!(cfg.limits.head_default, 100);
        assert_eq!(cfg.limits.daemon_soft_rss_mb, 256);
        assert_eq!(cfg.limits.daemon_hard_rss_mb, 512);
        assert_eq!(cfg.limits.daemon_cooldown_secs, 30);
        assert_eq!(cfg.limits.daemon_max_active_projects, 3);
        assert_eq!(cfg.limits.daemon_project_idle_secs, 120);
        assert_eq!(cfg.limits.index_memory_mb, 32);
        assert_eq!(cfg.limits.index_batch_size, 100);
        assert!(!cfg.limits.semantic_index);
        assert!(!cfg.limits.daemon_semantic_index);
        assert!(!cfg.cache.auto_gc);
        assert_eq!(cfg.cache.auto_gc_interval_secs, 120);
        assert_eq!(cfg.cache.auto_gc_days, 14);
        assert_eq!(cfg.cache.auto_gc_max_size_mb, 2048);
    }
}
