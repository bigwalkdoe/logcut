//! Shared, UI-free core for the `logcut` binary.

#[cfg(feature = "cli")]
pub mod agent;
pub mod core;
#[cfg(feature = "mcp")]
pub mod mcp;
