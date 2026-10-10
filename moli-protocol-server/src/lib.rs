//! HTTP and WebSocket host for Moli's automation protocols.
//!
//! This crate owns the transport, frontend routing, and protocol actor
//! scheduling used by CDP, WebDriver BiDi, and WebDriver Classic.

mod cdp_frontend;
mod cdp_frontend_router;
mod cdp_scheduler;
mod cdp_writer;
mod config;
pub mod protocol_server;
pub mod runtime_thread_budget;

pub use config::ServerConfig;
pub use protocol_server::{ProtocolServer, WebMcpConfig};
