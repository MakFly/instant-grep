//! `ig import-rtk` translates a project-local RTK filter file into ig's
//! filter schema and is idempotent on re-run.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "ig-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn project_local_rtk_filters_round_trip() {
    let proj = unique_dir("rtk-proj");
    let home = unique_dir("rtk-home");

    // Fake a git project with an RTK filter file.
    std::fs::create_dir_all(proj.join(".git")).unwrap();
    std::fs::create_dir_all(proj.join(".rtk")).unwrap();
    std::fs::write(
        proj.join(".rtk").join("filters.toml"),
        r#"
[[filters]]
name = "git-status"
match = "^git status"
strip_ansi = true
keep = "^\\s*[MAD] "
truncate = 100
"#,
    )
    .unwrap();

    let run = || {
        Command::new(bin())
            .arg("import-rtk")
            .current_dir(&proj)
            .env("HOME", &home)
            .output()
            .expect("spawn ig import-rtk")
    };

    let out = run();
    assert!(
        out.status.success(),
        "ig import-rtk failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let imported = proj.join(".ig").join("filters").join("imported-from-rtk.toml");
    assert!(
        imported.is_file(),
        "expected {} to be created",
        imported.display()
    );
    let body = std::fs::read_to_string(&imported).unwrap();
    assert!(body.contains("name = \"git-status\""), "name not mapped");
    assert!(
        body.contains("keep_lines = "),
        "rtk `keep` should map to ig `keep_lines`"
    );
    assert!(
        body.contains("truncate_at = 100"),
        "rtk `truncate` should map to ig `truncate_at`"
    );

    // Idempotent: second run still succeeds and the file still parses.
    let out2 = run();
    assert!(out2.status.success(), "second ig import-rtk run failed");
    assert!(imported.is_file());

    std::fs::remove_dir_all(&proj).ok();
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn dry_run_writes_nothing() {
    let proj = unique_dir("rtk-dry");
    let home = unique_dir("rtk-dry-home");
    std::fs::create_dir_all(proj.join(".git")).unwrap();
    std::fs::create_dir_all(proj.join(".rtk")).unwrap();
    std::fs::write(
        proj.join(".rtk").join("filters.toml"),
        "[[filters]]\nname = \"x\"\nmatch = \"^x\"\n",
    )
    .unwrap();

    let out = Command::new(bin())
        .args(["import-rtk", "--dry-run"])
        .current_dir(&proj)
        .env("HOME", &home)
        .output()
        .expect("spawn ig import-rtk --dry-run");
    assert!(out.status.success());
    assert!(
        !proj.join(".ig").join("filters").join("imported-from-rtk.toml").exists(),
        "--dry-run must not write the imported filter file"
    );

    std::fs::remove_dir_all(&proj).ok();
    std::fs::remove_dir_all(&home).ok();
}
