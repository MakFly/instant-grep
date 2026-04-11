use crate::filter::FilterEngine;

/// Verify that all TOML filters (builtin + user + project) load and compile correctly.
pub fn run_verify() {
    let engine = FilterEngine::new();
    // FilterEngine::new() loads and compiles all filters (builtin + user + project).
    // If we reach this point without panic, all TOML files parse and all regexes compile.
    let count = engine.filter_count();
    eprintln!("Verified: {} filters loaded and compiled successfully.", count);
}
