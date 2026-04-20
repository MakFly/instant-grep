//! Integration tests: golden file verification for built-in filters.
//!
//! Each test fixture in tests/filter_goldens/ consists of three files:
//!   <name>.cmd      — the command string (one line)
//!   <name>.raw      — raw tool output fed to the filter engine
//!   <name>.expected — expected output after filtering
//!
//! Since instant-grep is a binary crate (no lib target), we include the
//! filter modules directly via #[path] and provide a stub for `trust`.

// Stub for crate::trust (only the parts used by loader.rs).
mod trust {
    use std::path::Path;

    #[allow(dead_code)]
    pub enum TrustStatus {
        Trusted,
        Untrusted,
        ContentChanged,
    }

    pub fn check_trust(_path: &Path) -> TrustStatus {
        TrustStatus::Untrusted
    }
}

#[path = "../src/filter/pipeline.rs"]
mod pipeline;

#[allow(clippy::duplicate_mod)]
#[path = "../src/filter/loader.rs"]
mod loader;

#[allow(clippy::duplicate_mod)]
#[path = "../src/filter/mod.rs"]
mod filter;

use std::path::Path;

use filter::FilterEngine;

fn goldens_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/filter_goldens"))
}

#[test]
fn run_all_goldens() {
    let dir = goldens_dir();
    assert!(dir.is_dir(), "tests/filter_goldens/ directory missing");

    let engine = FilterEngine::new();
    assert!(engine.filter_count() > 0, "no filters loaded");
    let mut tested = 0;
    let mut failures: Vec<String> = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("read filter_goldens dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "cmd").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in &entries {
        let cmd_path = entry.path();
        let stem = cmd_path.file_stem().unwrap().to_string_lossy().to_string();

        let raw_path = dir.join(format!("{}.raw", stem));
        let expected_path = dir.join(format!("{}.expected", stem));

        let cmd = std::fs::read_to_string(&cmd_path)
            .unwrap_or_else(|_| panic!("cannot read {}", cmd_path.display()))
            .trim()
            .to_string();

        let raw = std::fs::read_to_string(&raw_path)
            .unwrap_or_else(|_| panic!("cannot read {}", raw_path.display()));

        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("cannot read {}", expected_path.display()));

        match engine.filter_output(&cmd, &raw) {
            None => {
                failures.push(format!(
                    "[{}] no filter matched command: {:?}",
                    stem, cmd
                ));
            }
            Some(actual) => {
                let actual = actual.trim_end().to_string();
                let expected = expected.trim_end().to_string();
                if actual != expected {
                    failures.push(format!(
                        "[{}] output mismatch\n  CMD: {}\n  EXPECTED:\n{}\n  ACTUAL:\n{}",
                        stem, cmd, expected, actual
                    ));
                }
            }
        }

        tested += 1;
    }

    assert!(tested > 0, "no golden test fixtures found");

    if !failures.is_empty() {
        panic!(
            "{}/{} golden tests FAILED:\n\n{}",
            failures.len(),
            tested,
            failures.join("\n\n---\n\n")
        );
    }

    println!("{} golden tests passed", tested);
}
