//! Lightweight, tools-only MCP over Streamable HTTP.
//!
//! A [`ToolService`] supplies definitions, execution and catalog changes. The
//! transport owns JSON-RPC framing, version negotiation, isolated legacy
//! sessions, cancellation and SSE notifications. It has no browser dependencies.

mod http;
#[cfg(test)]
mod tests;

use std::future::Future;

use serde_json::Value;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub use http::{HttpConfig, ServerInfo, router};

/// Application-owned tools and their lifecycle.
pub trait ToolService: Send + Sync + 'static {
    /// A tool snapshot. `as_ref()` exposes its MCP definition, including its
    /// `name` and `inputSchema`; the remaining handle belongs to the service.
    type Tool: AsRef<Value> + Send + 'static;

    /// The current MCP tool definitions. Errors become JSON-RPC internal errors.
    fn tools(&self) -> Result<Vec<Value>, String>;

    /// Resolve an exposed name to a tool snapshot before executing author code.
    fn tool(&self, name: &str) -> Option<Self::Tool>;

    /// Execute the snapshot and return JSON output or a tool execution error.
    /// Objects become `structuredContent`; other output becomes text content.
    /// The transport drops this future on cancellation, modern HTTP disconnect
    /// or shutdown. Dropping it must also stop any application-owned invocation.
    fn call(
        &self,
        tool: Self::Tool,
        arguments: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// Subscribe to changes in the tool definitions. Notifications may coalesce;
    /// clients fetch `tools/list` to read the current catalog.
    fn changes(&self) -> watch::Receiver<u64>;

    /// End notification streams when the underlying service closes.
    fn closed(&self) -> &CancellationToken;
}
