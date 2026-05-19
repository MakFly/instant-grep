//! Cloud / data CLI wrappers (aws, kubectl, psql, curl, wget).
//!
//! Introduced in PR #5 of the RTK-iso plan.

#![allow(dead_code)]

pub mod aws;
pub mod curl;
pub mod kubectl;
pub mod psql;
pub mod wget;
