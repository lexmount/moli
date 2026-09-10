//! Convenience owner for callers without a shared network runtime.

use super::{CurlWebSocketConnection, CurlWebSocketConnector, CurlWebSocketRequest};
use crate::{CurlMultiRuntime, CurlMultiRuntimeConfig};
use anyhow::Result;
use std::time::Duration;

/// Standalone owner for callers without an existing HTTP runtime. Uses the
/// same Multi driver as HTTP, with no HTTP submissions.
#[derive(Debug)]
pub struct CurlWebSocketRuntime {
    runtime: CurlMultiRuntime<StandaloneHandler, ()>,
}

#[derive(Debug)]
struct StandaloneHandler;
impl curl::easy::Handler for StandaloneHandler {}

impl CurlWebSocketRuntime {
    pub fn new() -> Result<Self> {
        let (runtime, _) = CurlMultiRuntime::new(CurlMultiRuntimeConfig {
            thread_name: "moli-curl-websocket".to_owned(),
            poll_interval: Duration::from_secs(1),
            ..CurlMultiRuntimeConfig::default()
        })?;
        Ok(Self { runtime })
    }

    pub fn connector(&self) -> CurlWebSocketConnector {
        self.runtime.websocket_connector()
    }

    pub fn connect(&self, request: CurlWebSocketRequest) -> Result<CurlWebSocketConnection> {
        self.connector().connect(request)
    }
}
