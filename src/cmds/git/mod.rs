//! Git platform wrappers (gh, glab, gt). Distinct from the existing
//! `crate::git` module which serves `Commands::Git`.
//!
//! Introduced in PR #5 of the RTK-iso plan.

#![allow(dead_code)]

pub mod gh;
pub mod glab;
pub mod gt;
