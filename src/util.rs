use std::path::{Path, PathBuf};

const BINARY_CHECK_LEN: usize = 8192;

/// Check if file content looks like binary (contains null bytes in first 8KB).
pub fn is_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(BINARY_CHECK_LEN);
    data[..check_len].contains(&0)
}

/// Find the project root by walking up until we find .git/ or use the given path.
pub fn find_root(start: &Path) -> PathBuf {
    let start = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };

    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return current;
        }
        if !current.pop() {
            // No .git found, use the original start path
            return start.to_path_buf();
        }
    }
}

/// Get the .ig index directory path for a given root.
pub fn ig_dir(root: &Path) -> PathBuf {
    root.join(".ig")
}
