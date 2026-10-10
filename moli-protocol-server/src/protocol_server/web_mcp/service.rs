use std::time::Duration;

use anyhow::Context;
use axum::Router;
use moli_protocol_mcp::{HttpConfig, ServerInfo, ToolService, router};
use serde_json::Value;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{
    WebMcpConfig,
    catalog::SiteTool,
    page::{WebMcpPage, response_error},
};

pub(super) fn transport_config(
    hosts: Vec<String>,
    port: u16,
    shutdown: CancellationToken,
) -> HttpConfig {
    HttpConfig {
        allowed_hosts: hosts,
        port,
        shutdown,
        server_info: ServerInfo {
            name: "moli-webmcp".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            instructions: Some("Tools run in one live Moli page shared with CDP. Tool _meta contains the native name, frame and target. Manual forms await page confirmation unless the service was started with --auto-submit.".to_owned()),
        },
    }
}

pub(super) fn mcp_router(page: WebMcpPage, config: WebMcpConfig, transport: HttpConfig) -> Router {
    router(
        WebMcpService {
            page,
            auto_submit: config.auto_submit,
            tool_timeout: config.tool_timeout,
        },
        transport,
    )
}

struct WebMcpService {
    page: WebMcpPage,
    auto_submit: bool,
    tool_timeout: Duration,
}

impl ToolService for WebMcpService {
    type Tool = SiteTool;

    fn tools(&self) -> Result<Vec<Value>, String> {
        self.page.tools().map_err(|error| error.to_string())
    }

    fn tool(&self, name: &str) -> Option<SiteTool> {
        self.page.find(name)
    }

    async fn call(&self, tool: SiteTool, arguments: Value) -> Result<Value, String> {
        let response = tokio::time::timeout(
            self.tool_timeout,
            self.page.invoke(tool, arguments, self.auto_submit),
        )
        .await
        .context("WebMCP invocation timed out")
        .and_then(|response| response)
        .map_err(|error| error.to_string())?;
        if response["status"] == "Completed" {
            Ok(response["output"].clone())
        } else {
            Err(response_error(&response))
        }
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.page.changes()
    }

    fn closed(&self) -> &CancellationToken {
        self.page.closed()
    }
}
