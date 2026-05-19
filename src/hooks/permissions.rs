//! Permission engine — built-in deny/ask rules with optional user/project overrides.
//!
//! Ported from RTK (`rtk:src/hooks/permissions.rs`), adapted to ig's needs:
//! - Built-in rules are hardcoded regexes compiled once via `OnceLock`.
//! - User and project rule files (`~/.config/ig/permissions.toml`,
//!   `<project>/.ig/permissions.toml`) may only contribute `deny` and `ask`
//!   entries. Any `allow` from those sources is downgraded to `ask`, with a
//!   one-shot stderr warning per process.
//! - Verdict precedence: Deny > Ask > Allow > Default (no rule matched).
//!
//! See `docs/specs/SPEC-rtk-iso-plan.md` § "PR 2".
//
// Some helpers are only reachable from the binary's main() (e.g.
// `PermissionEngine::load`, `user_rules_path`), but the integration tests
// `#[path]`-include this file and never touch them. Silence the resulting
// dead-code lints crate-wide rather than peppering each item.
#![allow(dead_code)]

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Verdict for a single command lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// An explicit allow rule (built-in only) matched — safe to auto-allow.
    Allow,
    /// An ask rule matched — surface to the user before executing.
    Ask,
    /// A deny rule matched — block the command.
    Deny,
    /// No rule matched. Callers MAY treat this as "ask" or "allow" depending
    /// on whether the command is otherwise familiar.
    Default,
}

/// Where a rule came from. Used for diagnostics and to enforce the "user/project
/// may not allow" invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSource {
    Builtin,
    User,
    Project,
}

/// A compiled permission rule.
#[derive(Debug, Clone)]
pub struct PermissionRule {
    pub pattern: Regex,
    pub verdict: Verdict,
    #[allow(dead_code)]
    pub source: RuleSource,
}

/// The full ordered rule list. First-match wins on `check`.
#[derive(Debug, Clone)]
pub struct PermissionEngine {
    rules: Vec<PermissionRule>,
}

// ---------------------------------------------------------------------------
// Built-in rules.
// ---------------------------------------------------------------------------

const BUILTIN_DENY: &[&str] = &[
    r"^rm\s+-rf?\s+/(\s|$)",
    r"^rm\s+-rf?\s+/\*",
    r"^rm\s+-rf?\s+\$HOME",
    r"^rm\s+-rf?\s+~",
    r"^git\s+reset\s+--hard(\s|$)",
    r"^git\s+clean\s+-[fd]+",
    r"^git\s+checkout\s+\.",
    r"^git\s+restore\s+\.",
    r"^dd\s+if=.*\sof=/dev/",
    r"^mkfs\.",
    r"^:\(\)\s*\{\s*:\|:&\s*\}\s*;:",
];

const BUILTIN_ASK: &[&str] = &[
    r"^git\s+push\s+--force",
    r"^git\s+push\s+-f(\s|$)",
    r"^git\s+commit\s+--amend.*--no-edit.*--force",
    r"^npm\s+publish",
    r"^cargo\s+publish",
    r"^docker\s+system\s+prune.*-a.*-f",
    r"^kubectl\s+delete\s+(ns|namespace)\s+",
    r"^terraform\s+destroy",
    r"^aws\s+iam\s+delete-",
];

/// Compiled, cached built-in rules. Built lazily on first `load()`.
fn builtin_rules() -> &'static [PermissionRule] {
    static CACHE: OnceLock<Vec<PermissionRule>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut out = Vec::with_capacity(BUILTIN_DENY.len() + BUILTIN_ASK.len());
        for src in BUILTIN_DENY {
            out.push(PermissionRule {
                pattern: Regex::new(src).expect("invalid built-in deny regex"),
                verdict: Verdict::Deny,
                source: RuleSource::Builtin,
            });
        }
        for src in BUILTIN_ASK {
            out.push(PermissionRule {
                pattern: Regex::new(src).expect("invalid built-in ask regex"),
                verdict: Verdict::Ask,
                source: RuleSource::Builtin,
            });
        }
        out
    })
}

/// Warn at most once per process when a user/project file tried to `allow`
/// something — that is silently demoted to `ask`.
fn warn_allow_downgrade_once(path: &Path) {
    static WARNED: OnceLock<()> = OnceLock::new();
    if WARNED.set(()).is_ok() {
        eprintln!(
            "[ig] note: `allow` rules in {} are not honored — downgraded to `ask`. \
             User/project files may only deny or ask; allows would create a privilege \
             escalation path. Use `~/.config/ig/permissions.toml` for ask-rules.",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// TOML file format
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Deserialize)]
struct FileRules {
    #[serde(default)]
    deny: Vec<String>,
    #[serde(default)]
    ask: Vec<String>,
    #[serde(default)]
    allow: Vec<String>,
}

/// Test-only re-export of `load_file_rules` so integration tests can verify
/// the user/project file format and the allow-downgrade contract.
#[doc(hidden)]
pub fn test_only_load_file_rules(path: &Path, source: RuleSource) -> Vec<PermissionRule> {
    load_file_rules(path, source)
}

fn load_file_rules(path: &Path, source: RuleSource) -> Vec<PermissionRule> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let file: FileRules = match toml::from_str(&content) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "[ig] warning: failed to parse permissions from {}: {}",
                path.display(),
                e
            );
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for src in &file.deny {
        if let Some(r) = compile(src, Verdict::Deny, source, path) {
            out.push(r);
        }
    }
    for src in &file.ask {
        if let Some(r) = compile(src, Verdict::Ask, source, path) {
            out.push(r);
        }
    }
    // `allow` from user/project is downgraded to ask.
    if !file.allow.is_empty() {
        warn_allow_downgrade_once(path);
        for src in &file.allow {
            if let Some(r) = compile(src, Verdict::Ask, source, path) {
                out.push(r);
            }
        }
    }
    out
}

fn compile(src: &str, verdict: Verdict, source: RuleSource, path: &Path) -> Option<PermissionRule> {
    match Regex::new(src) {
        Ok(pattern) => Some(PermissionRule {
            pattern,
            verdict,
            source,
        }),
        Err(e) => {
            eprintln!(
                "[ig] warning: invalid regex {:?} in {}: {}",
                src,
                path.display(),
                e
            );
            None
        }
    }
}

fn user_rules_path() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("ig").join("permissions.toml"))
}

/// Walk up from CWD looking for `.ig/permissions.toml`. Returns the first
/// match found.
fn project_rules_path() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(".ig").join("permissions.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

impl PermissionEngine {
    /// Load built-in rules + user file + project file, in that order.
    /// Returns an engine that is safe to query repeatedly.
    pub fn load() -> Self {
        let mut rules: Vec<PermissionRule> = builtin_rules().to_vec();
        if let Some(p) = user_rules_path() {
            rules.extend(load_file_rules(&p, RuleSource::User));
        }
        if let Some(p) = project_rules_path() {
            rules.extend(load_file_rules(&p, RuleSource::Project));
        }
        Self { rules }
    }

    /// Construct an engine from a custom rule list. Used by tests.
    #[allow(dead_code)]
    pub fn from_rules(rules: Vec<PermissionRule>) -> Self {
        Self { rules }
    }

    /// Construct an engine with only the built-in rules.
    #[allow(dead_code)]
    pub fn builtin_only() -> Self {
        Self {
            rules: builtin_rules().to_vec(),
        }
    }

    /// Check a command. First-match wins. Returns `(Default, None)` if no
    /// rule applied. Deny is checked first across all rules (precedence
    /// Deny > Ask > Allow).
    pub fn check(&self, cmd: &str) -> (Verdict, Option<&PermissionRule>) {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return (Verdict::Default, None);
        }

        // Precedence pass 1: any deny wins.
        for rule in &self.rules {
            if rule.verdict == Verdict::Deny && rule.pattern.is_match(trimmed) {
                return (Verdict::Deny, Some(rule));
            }
        }
        // Precedence pass 2: any ask wins.
        for rule in &self.rules {
            if rule.verdict == Verdict::Ask && rule.pattern.is_match(trimmed) {
                return (Verdict::Ask, Some(rule));
            }
        }
        // Precedence pass 3: any allow wins.
        for rule in &self.rules {
            if rule.verdict == Verdict::Allow && rule.pattern.is_match(trimmed) {
                return (Verdict::Allow, Some(rule));
            }
        }
        (Verdict::Default, None)
    }

    /// Total number of compiled rules.
    #[allow(dead_code)]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_compiles() {
        let _ = builtin_rules();
    }

    #[test]
    fn builtin_only_engine_loads() {
        let eng = PermissionEngine::builtin_only();
        assert!(eng.rule_count() >= BUILTIN_DENY.len() + BUILTIN_ASK.len());
    }

    #[test]
    fn deny_rm_rf_root() {
        let eng = PermissionEngine::builtin_only();
        let (v, _) = eng.check("rm -rf /");
        assert_eq!(v, Verdict::Deny);
        let (v, _) = eng.check("rm -rf / extra");
        assert_eq!(v, Verdict::Deny);
    }

    #[test]
    fn deny_git_reset_hard() {
        let eng = PermissionEngine::builtin_only();
        assert_eq!(eng.check("git reset --hard").0, Verdict::Deny);
        assert_eq!(eng.check("git reset --hard HEAD~1").0, Verdict::Deny);
    }

    #[test]
    fn deny_mkfs() {
        let eng = PermissionEngine::builtin_only();
        assert_eq!(eng.check("mkfs.ext4 /dev/sda1").0, Verdict::Deny);
    }

    #[test]
    fn ask_git_push_force() {
        let eng = PermissionEngine::builtin_only();
        assert_eq!(eng.check("git push --force origin main").0, Verdict::Ask);
        assert_eq!(eng.check("git push -f origin").0, Verdict::Ask);
    }

    #[test]
    fn ask_publish() {
        let eng = PermissionEngine::builtin_only();
        assert_eq!(eng.check("npm publish").0, Verdict::Ask);
        assert_eq!(eng.check("cargo publish").0, Verdict::Ask);
    }

    #[test]
    fn default_when_unmatched() {
        let eng = PermissionEngine::builtin_only();
        assert_eq!(eng.check("ls -la").0, Verdict::Default);
        assert_eq!(eng.check("").0, Verdict::Default);
    }

    #[test]
    fn deny_precedes_ask() {
        // Build manually: an ask rule that would also match a denied command.
        let rules = vec![
            PermissionRule {
                pattern: Regex::new("^git push").unwrap(),
                verdict: Verdict::Ask,
                source: RuleSource::Builtin,
            },
            PermissionRule {
                pattern: Regex::new("^git push --force").unwrap(),
                verdict: Verdict::Deny,
                source: RuleSource::Builtin,
            },
        ];
        let eng = PermissionEngine::from_rules(rules);
        assert_eq!(eng.check("git push --force origin main").0, Verdict::Deny);
    }
}
