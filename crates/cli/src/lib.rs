//! CLI entry and trusted local registration, shared with isolated native tests.
mod entry;
pub use entry::{registration::resolve, run};
