use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

/// Global ig configuration, loaded from ~/.config/ig/config.toml.
#[derive(Deserialize, Default)]
pub struct IgConfig {
    // TODO(audit 2026-06): tracking.retention_days, filters.user_dir,
    // limits.grep_max_results and limits.head_default are documented config
    // keys that no code path reads today — wire them up or drop them from
    // the public config surface.
    #[serde(default)]
    #[allow(dead_code)]
    pub tracking: TrackingConfig,
    #[serde(default)]
    #[allow(dead_code)]
    pub filters: FilterConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub cache: CacheConfig,
}

#[derive(Deserialize)]
pub struct TrackingConfig {
    #[serde(default = "default_retention_days")]
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub user_dir: Option<PathBuf>,
}

#[derive(Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_grep_max")]
    #[allow(dead_code)]
    pub grep_max_results: usize,
    #[serde(default = "default_head")]
    #[allow(dead_code)]
    pub head_default: usize,
    #[serde(default = "default_index_memory_mb")]
    pub index_memory_mb: usize,
    #[serde(default = "default_index_batch_size")]
    pub index_batch_size: usize,
    #[serde(default = "default_semantic_index")]
    pub semantic_index: bool,
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
            index_memory_mb: default_index_memory_mb(),
            index_batch_size: default_index_batch_size(),
            semantic_index: default_semantic_index(),
        }
    }
}

fn default_grep_max() -> usize {
    1000
}

fn default_head() -> usize {
    250
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
    config().limits.semantic_index
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
        assert_eq!(cfg.limits.index_memory_mb, 64);
        assert_eq!(cfg.limits.index_batch_size, 250);
        assert!(cfg.limits.semantic_index);
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
# Legacy v1.x daemon keys — must still parse (ignored) so old user
# configs don't break after the daemon removal.
daemon_soft_rss_mb = 256
daemon_semantic_index = false
index_memory_mb = 32
index_batch_size = 100
semantic_index = false

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
        assert_eq!(cfg.limits.index_memory_mb, 32);
        assert_eq!(cfg.limits.index_batch_size, 100);
        assert!(!cfg.limits.semantic_index);
        assert!(!cfg.cache.auto_gc);
        assert_eq!(cfg.cache.auto_gc_interval_secs, 120);
        assert_eq!(cfg.cache.auto_gc_days, 14);
        assert_eq!(cfg.cache.auto_gc_max_size_mb, 2048);
    }
}
