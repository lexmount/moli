//! MCP transport for the native WebMCP tools on a managed CDP page.

mod catalog;
mod page;
#[cfg(test)]
mod tests;

use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::Request,
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::Response,
    serve::ListenerExt,
};
use parking_lot::Mutex;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities,
        ServerConfig as McpServerConfig, SubscriptionFilter, Tool,
    },
    service::{NotificationContext, RequestContext, SubscriptionContext},
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::{AppState, ProtocolServer, build_router, tcp_options};
use page::{WebMcpPage, response_error};

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
            let origins = hosts
                .iter()
                .map(|host| {
                    let host = if host.contains(':') {
                        format!("[{host}]")
                    } else {
                        host.clone()
                    };
                    format!("http://{host}:{}", addr.port())
                })
                .collect::<Vec<_>>();
            let transport = StreamableHttpServerConfig::default()
                .with_allowed_hosts(hosts)
                .with_allowed_origins(origins)
                .enforce_origin_validation()
                .with_json_response(true)
                .with_cancellation_token(cancellation.clone());
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

fn mcp_router(
    page: WebMcpPage,
    config: WebMcpConfig,
    transport: StreamableHttpServerConfig,
) -> Router {
    Router::new()
        .nest_service("/mcp", mcp_service(page, config, transport))
        .layer(middleware::from_fn(session_delete_status))
}

async fn session_delete_status(request: Request, next: Next) -> Response {
    let deleting = request.method() == Method::DELETE;
    let mut response = next.run(request).await;
    // rmcp returns 202 after close_session has completed; the Python SDK
    // expects 200 for successful legacy session termination.
    if deleting && response.status() == StatusCode::ACCEPTED {
        *response.status_mut() = StatusCode::OK;
    }
    response
}

fn mcp_service(
    page: WebMcpPage,
    config: WebMcpConfig,
    transport: StreamableHttpServerConfig,
) -> StreamableHttpService<McpService, LocalSessionManager> {
    StreamableHttpService::new(
        move || {
            Ok(McpService {
                page: page.clone(),
                config: config.clone(),
                notifications: Mutex::new(None),
            })
        },
        Default::default(),
        transport,
    )
}

struct McpService {
    page: WebMcpPage,
    config: WebMcpConfig,
    notifications: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for McpService {
    fn drop(&mut self) {
        if let Some(task) = self.notifications.get_mut().take() {
            task.abort();
        }
    }
}

impl ServerHandler for McpService {
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.page.find(name).map(|tool| tool.mcp)
    }

    fn get_info(&self) -> McpServerConfig {
        McpServerConfig::new(ServerCapabilities::builder()
            .enable_tools().enable_tool_list_changed().build())
            .with_server_info(Implementation::new("moli-webmcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("Tools run in one live Moli page shared with CDP. Tool _meta contains the native name, frame and target. Manual forms await page confirmation unless the service was started with --auto-submit.")
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|request| request.cursor.is_some()) {
            return Err(ErrorData::invalid_params(
                "this catalog has no pagination cursor",
                None,
            ));
        }
        let tools = self
            .page
            .tools()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = self.page.find(&request.name).ok_or_else(|| {
            ErrorData::invalid_params(format!("unknown WebMCP tool: {}", request.name), None)
        })?;
        let invocation = self.page.invoke(
            tool,
            Value::Object(request.arguments.unwrap_or_default()),
            self.config.auto_submit,
        );
        let outcome = tokio::select! {
            biased;
            _ = context.ct.cancelled() => Err(anyhow::anyhow!("WebMCP invocation canceled")),
            result = tokio::time::timeout(self.config.tool_timeout, invocation) => {
                result.context("WebMCP invocation timed out")
                    .and_then(|result| result)
            }
        };
        let result = match outcome {
            Ok(response) if response["status"] == "Completed" => {
                let output = response["output"].clone();
                if let Some(text) = output.as_str() {
                    CallToolResult::success(vec![ContentBlock::text(text)])
                } else if output.is_object() {
                    CallToolResult::structured(output)
                } else {
                    CallToolResult::success(vec![ContentBlock::text(output.to_string())])
                }
            }
            Ok(response) => {
                CallToolResult::error(vec![ContentBlock::text(response_error(&response))])
            }
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        };
        Ok(result.into())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        let page = self.page.clone();
        let mut changes = page.changes();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = page.closed().cancelled() => break,
                    result = changes.changed() => if result.is_err() { break; },
                }
                if context.peer.notify_tool_list_changed().await.is_err() {
                    break;
                }
            }
        });
        if let Some(previous) = self.notifications.lock().replace(task) {
            previous.abort();
        }
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&self.get_info().capabilities))
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let mut changes = self.page.changes();
        loop {
            tokio::select! {
                _ = context.cancelled() => return Ok(()),
                _ = self.page.closed().cancelled() => return Ok(()),
                result = changes.changed() => if result.is_err() { return Ok(()); },
            }
            if context.sink().notify_tool_list_changed().await.is_err() {
                return Ok(());
            }
        }
    }
}
