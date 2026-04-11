pub mod loader;
pub mod pipeline;

pub use loader::load_filters;
pub use pipeline::{CompiledFilter, apply_filter};

/// Engine that holds compiled filters and matches them against commands.
pub struct FilterEngine {
    filters: Vec<CompiledFilter>,
}

impl FilterEngine {
    pub fn new() -> Self {
        let filters = load_filters();
        Self { filters }
    }

    /// Return the number of loaded filters.
    pub fn filter_count(&self) -> usize {
        self.filters.len()
    }

    /// Find the first filter matching a command string.
    pub fn find(&self, command: &str) -> Option<&CompiledFilter> {
        self.filters.iter().find(|f| f.matches(command))
    }

    /// Apply a filter to raw output, return filtered output.
    #[allow(dead_code)]
    pub fn apply(&self, filter: &CompiledFilter, raw: &str) -> String {
        apply_filter(filter, raw)
    }

    /// Convenience: find and apply in one call. Returns None if no filter matches.
    #[allow(dead_code)]
    pub fn filter_output(&self, command: &str, raw: &str) -> Option<String> {
        self.find(command).map(|f| apply_filter(f, raw))
    }
}

impl Default for FilterEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_returns_none_for_unknown_command() {
        let engine = FilterEngine::new();
        assert!(engine.find("some-unknown-command --flag").is_none());
    }

    #[test]
    fn engine_filter_output_returns_none_when_no_match() {
        let engine = FilterEngine::new();
        assert!(engine.filter_output("unknown", "some output").is_none());
    }
}
