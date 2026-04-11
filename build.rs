use std::fs;
use std::path::Path;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("builtin_filters.toml");

    let filters_dir = Path::new("filters");
    let mut combined = String::new();

    if filters_dir.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(filters_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "toml").unwrap_or(false))
            .collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let content = fs::read_to_string(entry.path()).unwrap();
            combined.push_str(&format!("# --- {} ---\n", entry.path().display()));
            combined.push_str(&content);
            combined.push('\n');
        }
    }

    fs::write(&dest, &combined).unwrap();

    // Tell cargo to re-run if any filter file changes
    println!("cargo:rerun-if-changed=filters/");
}
