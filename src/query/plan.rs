use crate::index::trigram::Trigram;

#[derive(Debug, Clone)]
pub enum TrigramQuery {
    /// All trigrams must be present (intersection)
    And(Vec<TrigramQuery>),
    /// Any trigram branch can match (union)
    Or(Vec<TrigramQuery>),
    /// A single trigram to look up
    Trigram(Trigram),
    /// Cannot extract trigrams — must scan all files
    All,
}

impl TrigramQuery {
    /// Returns true if this query requires scanning all files.
    pub fn is_all(&self) -> bool {
        matches!(self, TrigramQuery::All)
    }
}
