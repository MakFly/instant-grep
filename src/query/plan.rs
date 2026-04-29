use crate::index::ngram::NgramKey;

#[derive(Debug, Clone)]
pub enum NgramQuery {
    /// All n-grams must be present (intersection)
    And(Vec<NgramQuery>),
    /// Any n-gram branch can match (union)
    Or(Vec<NgramQuery>),
    /// A single n-gram key to look up
    #[allow(dead_code)]
    Ngram(NgramKey),
    /// A single n-gram with optional per-document mask constraints.
    MaskedNgram {
        key: NgramKey,
        next_mask: u8,
        rel_pos: u16,
        exact_pos: bool,
    },
    /// Cannot extract n-grams — must scan all files
    All,
}

impl NgramQuery {
    pub fn is_all(&self) -> bool {
        matches!(self, NgramQuery::All)
    }
}
