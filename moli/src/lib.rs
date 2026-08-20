//! Application support crate for the Moli CLI binary.
//!
//! This crate groups CLI parsing, application configuration, telemetry setup,
//! and command wiring for the `moli` executable.

pub mod app;
pub mod cli;
pub mod config;
pub mod cookie_cache;
pub mod fetch_dump;
mod network_trace;
mod robots;
pub mod telemetry;

/// Compatibility namespace for callers that used the embedded server through
/// the `moli` support crate before it became an independent crate.
pub mod protocol_server {
    pub use moli_protocol_server::{ProtocolServer, ServerConfig};
}

pub use moli_protocol_server::runtime_thread_budget;
