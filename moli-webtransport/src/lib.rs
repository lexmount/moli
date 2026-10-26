//! Native WebTransport sessions on the caller's selected Tokio executor.
//!
//! Browser bindings own policy, realms and task delivery. This crate owns real
//! QUIC/HTTP3 connections, byte streams and datagrams, with the same DNS,
//! address admission, host overrides and certificate trust as HTTP requests.
//! Dropping the unique owner cancels existing sessions and in-flight handshakes;
//! cloneable connectors and streams cannot prolong that owner's lifetime.

mod client;
mod config;
mod session;
mod tls;

pub use client::{Connector, Owner};
pub use config::{CertificateHash, Config, CongestionControl, ConnectOptions};
pub use session::{CloseInfo, CloseReason, RecvStream, SendStream, Session};
pub use web_transport_quinn::{ReadError, SessionError, WriteError};

#[cfg(test)]
mod tests;
