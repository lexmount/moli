# moli-protocol-mcp

A lightweight, tools-only MCP server over Streamable HTTP. The crate depends on
HTTP, JSON and async utilities already used by Moli; it has no browser or V8
dependencies.

Implement `ToolService` to provide the current tool definitions, resolve tool
handles, execute calls, and signal catalog changes and service closure. A tool
handle exposes its MCP definition through `AsRef<serde_json::Value>` and can carry
application-specific execution data.

```rust
use moli_protocol_mcp::{HttpConfig, ServerInfo, ToolService, router};
use tokio_util::sync::CancellationToken;

pub fn tool_router<S: ToolService>(service: S, shutdown: CancellationToken) -> axum::Router {
    router(service, HttpConfig {
        allowed_hosts: vec!["127.0.0.1".to_owned(), "localhost".to_owned()],
        port: 9222,
        server_info: ServerInfo {
            name: "my-tool-service".to_owned(),
            version: "0.1.0".to_owned(),
            instructions: None,
        },
        shutdown,
    })
}
```

Serve this router with Axum, or merge it into an existing router. The MCP endpoint
is `/mcp`. Configure `allowed_hosts` and `port` for the HTTP listener: requests
must have an allowed Host, and browser requests must also have a matching Origin.
Callers control listener setup and graceful shutdown.

The transport handles JSON-RPC validation, version negotiation, session
isolation, cancellation, SSE tool-change notifications, and bounded request and
session state. It supports legacy MCP versions `2025-03-26`, `2025-06-18` and
`2025-11-25`, plus the stateless `2026-07-28` transport. Modern clients use
`server/discover` and `subscriptions/listen`; legacy clients initialize a session
and use GET for notifications. `ping` is available only to legacy clients.

Modern subscriptions send an acknowledgment before any catalog changes. When
the service closes or shuts down gracefully, each stream sends one final result
with `resultType: "complete"`, the original request ID, and subscription and
server metadata, then ends. An HTTP disconnect drops the listener without a final
response; legacy notification streams end without a result.

Calls return application JSON. Objects become both `structuredContent` and text
content; other values become text content. Execution errors are MCP tool results
with `isError: true`. JSON-RPC errors describe malformed requests or missing tools.

Cancellation drops the application's call future. Implementations must stop any
invocation owned by that future when it is dropped. Modern HTTP disconnects also
drop the call; legacy calls continue until completion, explicit cancellation,
session closure or service shutdown. Application timeouts belong in `ToolService`.

Moli's native WebMCP adapter lives in
[`moli-protocol-server`](../moli-protocol-server/src/protocol_server/web_mcp/service.rs).
It supplies page tools and native invocation cleanup; CDP, navigation and form
submission remain there. This crate's release tests use a standalone service
without a browser:

```sh
cargo nextest run --release -p moli-protocol-mcp --no-fail-fast
```
