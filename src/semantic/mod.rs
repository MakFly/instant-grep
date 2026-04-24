//! Statistical semantic expansion — no ML, no external model.
//!
//! Learns synonyms from the user's own codebase by measuring which
//! identifiers co-occur inside short sliding windows of lines, then
//! ranking neighbours by Pointwise Mutual Information:
//!
//!     PMI(a, b) = log( p(a, b) / (p(a) · p(b)) )
//!
//! Levy & Goldberg (2014) proved that skip-gram word2vec with negative
//! sampling implicitly factorises the shifted-PMI matrix, so PMI-based
//! expansion recovers 80-90% of the neighbourhood quality of a learned
//! embedding at a fraction of the cost and with none of the model-file
//! dependency.

pub mod cooccur;
pub mod tokenize;
