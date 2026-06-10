//! Per-tool build modules — thin wrappers that spawn the underlying
//! tool, run its merged output through the TOML filter pipeline, and
//! emit via the shared `finish::emit` helper.

pub mod next;
pub mod prisma;
