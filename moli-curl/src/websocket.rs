//! Native WebSocket frames on the shared libcurl runtime.
//!
//! This layer transports frames. Browser handshake policy, message assembly and
//! close-handshake semantics belong to the caller. Dropping the receiver cancels
//! its session, including DNS and handshake work, independently of queue capacity.
//!
//! Connector submits work; connection holds the caller/owner I/O endpoints;
//! registry owns DNS and attached sessions; session drives handshake and frame
//! I/O. Readiness maps the runtime's poll results back to sessions. Returning
//! AGAIN parks that I/O until its socket is signalled. Application wakeups only
//! resume paused work, such as a new frame or restored receive capacity.

mod connection;
mod connection_pool;
mod connector;
mod readiness;
pub(crate) mod registry;
mod request;
mod session;
mod standalone;
#[cfg(test)]
mod tests;

use crate::{CurlDnsResolution, CurlTlsConfig};
use anyhow::{Result, bail};
use std::time::Duration;

use connection::SessionIo;
pub use connection::{CurlWebSocketConnection, CurlWebSocketSender};
pub use connector::CurlWebSocketConnector;
pub(crate) use connector::Submission;
pub use curl::easy::{WsFlags, WsFrame};
pub use standalone::CurlWebSocketRuntime;

/// Fragment large messages above this layer to bound native write residence.
pub const MAX_SEND_FRAME_BYTES: usize = 64 * 1024;
const MAX_PENDING_EVENTS: usize = 8;
const SESSION_CAPACITY: usize = 255;

#[derive(Debug)]
pub struct CurlWebSocketRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// An already resolved proxy policy. None explicitly disables environment proxies.
    pub proxy: Option<String>,
    pub proxy_headers: Vec<(String, String)>,
    pub tls: CurlTlsConfig,
    pub dns_resolution: CurlDnsResolution,
    pub handshake_timeout: Duration,
}

impl CurlWebSocketRequest {
    pub fn new(url: String) -> Self {
        Self {
            url,
            headers: Vec::new(),
            proxy: None,
            proxy_headers: Vec::new(),
            tls: CurlTlsConfig::default(),
            dns_resolution: CurlDnsResolution::curl_managed(),
            handshake_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub enum CurlWebSocketEvent {
    Handshake {
        /// Actual outgoing GET request, excluding proxy CONNECT headers.
        request: Vec<u8>,
        /// Final HTTP response block, preserving duplicate headers.
        response: Vec<u8>,
        result: std::result::Result<(), String>,
    },
    /// Decoded payload from libcurl with raw mode and automatic Pong disabled.
    /// Invalid opcodes/RSV, masking, fragmentation and control sizes fail at the
    /// native decoder. Each chunk belongs to one frame: len equals data.len(),
    /// offsets advance from zero, and bytes_left counts the remaining payload.
    /// Empty frames yield an empty chunk. Continuations retain TEXT/BINARY;
    /// CONT marks every non-final fragment, including empty ones.
    /// UTF-8, Close contents and application size limits belong to the caller.
    Chunk { data: Vec<u8>, frame: WsFrame },
    /// Emitted once, after all previously admitted events. Ok means TCP EOF;
    /// it does not assert that a WebSocket close handshake was completed.
    Closed {
        result: std::result::Result<(), String>,
    },
}

#[derive(Debug)]
pub struct CurlWebSocketSend {
    pub flags: WsFlags,
    pub data: Vec<u8>,
}

impl CurlWebSocketSend {
    fn is_control(&self) -> bool {
        [WsFlags::PING, WsFlags::PONG, WsFlags::CLOSE].contains(&self.flags)
    }

    fn validate(&self) -> Result<()> {
        let data_flags = WsFlags::from_bits(self.flags.bits() & !WsFlags::CONT.bits());
        if !self.is_control() && data_flags != WsFlags::TEXT && data_flags != WsFlags::BINARY {
            bail!("invalid WebSocket send flags");
        }
        let limit = if self.is_control() {
            125
        } else {
            MAX_SEND_FRAME_BYTES
        };
        if self.data.len() > limit {
            bail!("WebSocket send frame exceeds {limit} bytes");
        }
        Ok(())
    }
}
