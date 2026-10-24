use std::{num::NonZeroUsize, time::Duration};

use anyhow::{Context, Result, anyhow};
use curl::multi::Multi;
use tracing::debug;

const DEFAULT_RUNTIME_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Configuration for a multi-request curl runtime.
#[derive(Debug, Clone)]
pub struct CurlMultiRuntimeConfig {
    pub max_active: NonZeroUsize,
    /// Scheduler-side per-origin active transfer cap.
    ///
    /// Keep this separate from `max_host_connections`: the latter is a curl
    /// transport connection-pool cap and should not throttle HTTP/2 streams.
    pub max_host_active: Option<NonZeroUsize>,
    /// Libcurl per-host socket cap for HTTP/1 and HTTP/2.
    /// Requests needing a new connection wait when the cap is reached; an
    /// existing HTTP/2 connection can still carry additional streams.
    pub max_host_connections: Option<NonZeroUsize>,
    /// HTTP socket cap across all hosts on this runtime.
    pub max_total_connections: Option<NonZeroUsize>,
    /// Socket cap for the separate WebSocket pool. Native session admission
    /// remains independently bounded, including pending handshakes.
    pub max_websocket_connections: NonZeroUsize,
    pub max_concurrent_streams: Option<NonZeroUsize>,
    pub poll_interval: Duration,
    pub multiplex: bool,
    pub thread_name: String,
}

impl Default for CurlMultiRuntimeConfig {
    fn default() -> Self {
        Self {
            max_active: NonZeroUsize::new(8).expect("default active transfer cap is non-zero"),
            max_host_active: None,
            max_host_connections: None,
            max_total_connections: None,
            max_websocket_connections: NonZeroUsize::new(crate::websocket::SESSION_CAPACITY)
                .expect("WebSocket session capacity is non-zero"),
            max_concurrent_streams: None,
            poll_interval: DEFAULT_RUNTIME_POLL_INTERVAL,
            multiplex: true,
            thread_name: "lm-curl-multi".to_owned(),
        }
    }
}

impl CurlMultiRuntimeConfig {
    pub fn validate(&self) -> Result<()> {
        if self.poll_interval.is_zero() {
            return Err(anyhow!("curl multi runtime poll interval must be non-zero"));
        }
        if self.thread_name.is_empty() {
            return Err(anyhow!("curl multi runtime thread name must not be empty"));
        }
        Ok(())
    }
}

pub(super) fn make_runtime_multi(config: &CurlMultiRuntimeConfig) -> Multi {
    let mut multi = Multi::new();
    if let Some(max_host_connections) = config.max_host_connections
        && let Err(error) = multi.set_max_host_connections(max_host_connections.get())
    {
        debug!("failed to configure curl multi max_host_connections: {error}");
    }
    if let Some(max_total_connections) = config.max_total_connections {
        let max_total_connections = max_total_connections.get();
        if let Err(error) = multi.set_max_total_connections(max_total_connections) {
            debug!("failed to configure curl multi max_total_connections: {error}");
        }
        if let Err(error) = multi.set_max_connects(max_total_connections) {
            debug!("failed to configure curl multi max_connects: {error}");
        }
    }
    let max_concurrent_streams = config.max_concurrent_streams.map(NonZeroUsize::get);
    if let Some(max_concurrent_streams) = max_concurrent_streams
        && let Err(error) = multi.set_max_concurrent_streams(max_concurrent_streams)
    {
        debug!("failed to configure curl multi max_concurrent_streams: {error}");
    }
    if config.multiplex
        && let Err(error) = multi.pipelining(false, true)
    {
        debug!("failed to enable curl multi multiplexing: {error}");
    }
    multi
}

pub(super) fn make_websocket_multi(limit: NonZeroUsize) -> Multi {
    let mut multi = Multi::new();
    // Upgraded connections stay attached for curl_ws_send/recv. Giving them
    // their own pool prevents HTTP's six-socket default from blocking upgrades.
    if let Err(error) = multi.set_max_total_connections(limit.get()) {
        debug!("failed to configure curl WebSocket max_total_connections: {error}");
    }
    if let Err(error) = multi.set_max_connects(limit.get()) {
        debug!("failed to configure curl WebSocket max_connects: {error}");
    }
    multi
}

pub(super) fn runtime_wait_timeout(multi: &Multi, poll_interval: Duration) -> Result<Duration> {
    let curl_timeout = multi
        .get_timeout()
        .context("failed to read curl multi timeout")?;
    Ok(curl_timeout
        .map(|timeout| timeout.min(poll_interval))
        .unwrap_or(poll_interval))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_config_rejects_zero_poll_interval() {
        let config = CurlMultiRuntimeConfig {
            poll_interval: Duration::ZERO,
            ..CurlMultiRuntimeConfig::default()
        };

        let error = config
            .validate()
            .expect_err("zero runtime poll interval should fail")
            .to_string();

        assert!(
            error.contains("poll interval must be non-zero"),
            "unexpected error: {error}"
        );
    }
}
