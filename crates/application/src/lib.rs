//! Application services shared by the desktop, CLI and future MCP adapter.
mod access;
pub mod input;
pub mod ports;
pub mod result;
pub use access::{RequestContext, Session};
pub use result::ErrorCode;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub mod projects;
pub mod workspaces;
