//! User/project permissions.toml overrides — `allow` rules must be silently
//! downgraded to `ask` with a one-shot warning.

use std::fs;

#[path = "../src/hooks/permissions.rs"]
mod permissions;

use permissions::{PermissionEngine, RuleSource, Verdict};

#[test]
fn user_allow_in_toml_is_downgraded_to_ask() {
    // We can't easily inject a config path into `PermissionEngine::load()`
    // from a test, so we exercise the file-loader directly through the
    // engine's public surface by emulating its behaviour: write a temp file,
    // parse it, build an engine from those rules.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("permissions.toml");
    fs::write(
        &path,
        r#"
allow = ["^my-safe-script\\b"]
deny = ["^secret-leak\\b"]
ask = ["^maybe-bad\\b"]
"#,
    )
    .unwrap();

    // Build rules via the same loader the engine uses internally.
    let rules = permissions::test_only_load_file_rules(&path, RuleSource::User);

    // Allow downgraded → ask.
    let allow_demoted = rules
        .iter()
        .find(|r| r.pattern.as_str() == r"^my-safe-script\b")
        .expect("allow rule present after parse");
    assert_eq!(
        allow_demoted.verdict,
        Verdict::Ask,
        "user-supplied allow rule must be demoted to ask"
    );

    let deny = rules
        .iter()
        .find(|r| r.pattern.as_str() == r"^secret-leak\b")
        .expect("deny rule present");
    assert_eq!(deny.verdict, Verdict::Deny);

    let ask = rules
        .iter()
        .find(|r| r.pattern.as_str() == r"^maybe-bad\b")
        .expect("ask rule present");
    assert_eq!(ask.verdict, Verdict::Ask);

    // Sanity: an engine built from those rules respects the demotion.
    let eng = PermissionEngine::from_rules(rules);
    assert_eq!(eng.check("my-safe-script arg").0, Verdict::Ask);
    assert_eq!(eng.check("secret-leak").0, Verdict::Deny);
}
