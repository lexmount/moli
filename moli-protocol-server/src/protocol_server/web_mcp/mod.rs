//! MCP transport for the native WebMCP tools on a managed CDP page.

mod catalog;
mod page;
#[cfg(test)]
mod tests;
mod transport;

use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use axum::serve::ListenerExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::{AppState, ProtocolServer, build_router, tcp_options};
use page::WebMcpPage;
use transport::{McpTransportConfig, mcp_router};

/// Behavior of the WebMCP-to-MCP adapter. CDP remains available on the same port.
#[derive(Debug, Clone)]
pub struct WebMcpConfig {
    pub tool_timeout: Duration,
    /// Submit manual tool forms through their normal submit button.
    pub auto_submit: bool,
    /// Allow asynchronously registered tools to appear before initial discovery.
    pub discovery_wait: Duration,
}

impl Default for WebMcpConfig {
    fn default() -> Self {
        Self {
            tool_timeout: Duration::from_secs(60),
            auto_submit: false,
            discovery_wait: Duration::from_secs(1),
        }
    }
}

impl ProtocolServer {
    fn web_mcp_app_state(&self, addr: SocketAddr) -> Result<AppState> {
        AppState::new_with_storage_partition_and_runtime_config(
            addr,
            self.storage_partition.clone(),
            self.navigation_runtime_config.clone(),
            self.config.screencast_interval_ms,
        )
    }

    async fn open_web_mcp_page(
        &self,
        state: &AppState,
        url: &str,
        wait: Duration,
    ) -> Result<WebMcpPage> {
        let page = tokio::time::timeout(
            Duration::from_secs(u64::from(self.config.timeout_secs)),
            WebMcpPage::open(state.cdp_owner_registry.shared_owner()?, url),
        )
        .await
        .context("timed out opening WebMCP page")??;
        tokio::time::sleep(wait).await;
        Ok(page)
    }

    /// Open a URL and return its MCP-compatible native tool catalog as JSON.
    pub async fn list_web_mcp_tools(&self, url: &str, wait: Duration) -> Result<Value> {
        let state = self.web_mcp_app_state("127.0.0.1:0".parse().unwrap())?;
        let result = async {
            let page = self.open_web_mcp_page(&state, url, wait).await?;
            Ok(json!(page.tools()?))
        }
        .await;
        state.cdp_owner_registry.shutdown().await;
        result
    }

    /// Serve native page tools over Streamable HTTP at `/mcp`, alongside CDP.
    pub async fn serve_web_mcp(&self, url: &str, config: WebMcpConfig) -> Result<()> {
        let listener = TcpListener::bind(self.config.bind_target())
            .await
            .context("failed to bind WebMCP protocol server")?;
        let addr = listener.local_addr()?;
        let state = self.web_mcp_app_state(addr)?;
        let result = async {
            let page = self
                .open_web_mcp_page(&state, url, config.discovery_wait)
                .await?;
            let cancellation = CancellationToken::new();
            let _shutdown = cancellation.clone().drop_guard();
            let hosts = vec![
                self.config.host.clone(),
                addr.ip().to_string(),
                "localhost".to_owned(),
                "127.0.0.1".to_owned(),
                "::1".to_owned(),
            ];
            let transport = McpTransportConfig::new(hosts, addr.port(), cancellation.clone());
            let app =
                build_router(state.clone()).merge(mcp_router(page.clone(), config, transport));
            tracing::info!(mcp = %format!("http://{addr}/mcp"),
                cdp = %format!("http://{addr}"), target_id = page.target_id(),
                url, "WebMCP service ready");
            let listener = listener.tap_io(|stream| {
                tcp_options::configure_accepted_protocol_stream(stream);
            });
            let result = axum::serve(listener, app).await;
            cancellation.cancel();
            page.closed().cancel();
            result.context("WebMCP protocol server failed")
        }
        .await;
        state.cdp_owner_registry.shutdown().await;
        result
    }
}
