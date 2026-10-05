//! Tools-only Streamable HTTP, using the server's existing Axum/JSON stack.
//! Legacy clients get isolated sessions; modern requests are self-contained.

use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Context;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::post,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use futures_util::stream;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{
    WebMcpConfig,
    catalog::SiteTool,
    page::{WebMcpPage, response_error},
};

const LEGACY_VERSION: &str = "2025-11-25";
const MODERN_VERSION: &str = "2026-07-28";
const LEGACY_VERSIONS: &[&str] = &[LEGACY_VERSION, "2025-06-18", "2025-03-26"];
const MAX_SESSIONS: usize = 256;
const MAX_PENDING_CALLS: usize = 128;
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const INSTRUCTIONS: &str = "Tools run in one live Moli page shared with CDP. Tool _meta contains the native name, frame and target. Manual forms await page confirmation unless the service was started with --auto-submit.";

pub(super) struct McpTransportConfig {
    hosts: Vec<String>,
    port: u16,
    shutdown: CancellationToken,
}

impl McpTransportConfig {
    pub(super) fn new(hosts: Vec<String>, port: u16, shutdown: CancellationToken) -> Self {
        Self {
            hosts,
            port,
            shutdown,
        }
    }

    fn allows(&self, url: &Url) -> bool {
        url.scheme() == "http"
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port_or_known_default() == Some(self.port)
            && url.host_str().is_some_and(|host| {
                self.hosts.iter().any(|allowed| {
                    allowed
                        .trim_matches(['[', ']'])
                        .eq_ignore_ascii_case(host.trim_matches(['[', ']']))
                })
            })
    }
}

struct HttpMcp {
    page: WebMcpPage,
    config: WebMcpConfig,
    transport: McpTransportConfig,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

struct Session {
    version: String,
    initialized: AtomicBool,
    last_used: Mutex<Instant>,
    closed: CancellationToken,
    pending: Mutex<HashMap<String, CancellationToken>>,
    changes: Mutex<tokio::sync::watch::Receiver<u64>>,
    stream: Mutex<Option<CancellationToken>>,
}

impl HttpMcp {
    fn session(&self, headers: &HeaderMap) -> Result<Arc<Session>, StatusCode> {
        let id = header(headers, "mcp-session-id").ok_or(StatusCode::BAD_REQUEST)?;
        let mut sessions = self.sessions.lock();
        expire_sessions(&mut sessions);
        let session = sessions.get(id).cloned().ok_or(StatusCode::NOT_FOUND)?;
        let version = match header(headers, "mcp-protocol-version") {
            Some(version) => version,
            None if !headers.contains_key("mcp-protocol-version") => &session.version,
            None => return Err(StatusCode::BAD_REQUEST),
        };
        if version != session.version {
            return Err(StatusCode::BAD_REQUEST);
        }
        *session.last_used.lock() = Instant::now();
        Ok(session)
    }
}

pub(super) fn mcp_router(
    page: WebMcpPage,
    config: WebMcpConfig,
    transport: McpTransportConfig,
) -> Router {
    let state = Arc::new(HttpMcp {
        page,
        config,
        transport,
        sessions: Mutex::default(),
    });
    Router::new()
        .route(
            "/mcp",
            post(post_message)
                .get(get_notifications)
                .delete(delete_session),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), check_origin))
        .with_state(state)
}

async fn check_origin(State(state): State<Arc<HttpMcp>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    if headers.contains_key("origin")
        && !header(headers, "origin")
            .and_then(|origin| Url::parse(origin).ok())
            .is_some_and(|url| state.transport.allows(&url))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !header(headers, "host")
        .and_then(|host| Url::parse(&format!("http://{host}")).ok())
        .is_some_and(|url| state.transport.allows(&url))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if state.transport.shutdown.is_cancelled() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    next.run(request).await
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

fn accepts(headers: &HeaderMap, media: &str) -> bool {
    header(headers, "accept").is_some_and(|value| {
        value.split(',').any(|part| {
            let mut parts = part.trim().split(';').map(str::trim);
            parts
                .next()
                .is_some_and(|value| value.eq_ignore_ascii_case(media))
                && !parts.any(|param| {
                    param
                        .strip_prefix("q=")
                        .is_some_and(|q| q.parse::<f32>().is_ok_and(|q| q <= 0.0))
                })
        })
    })
}

fn error(code: i32, message: impl Into<String>) -> Value {
    json!({"code":code, "message":message.into()})
}

fn rpc_response(id: &Value, result: Result<Value, Value>, modern: bool) -> Response {
    Json(rpc_value(id, result, modern)).into_response()
}

fn bad_request(id: &Value, error: Value) -> Response {
    let mut response = rpc_response(id, Err(error), false);
    *response.status_mut() = StatusCode::BAD_REQUEST;
    response
}

fn valid_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}

fn server_info() -> Value {
    json!({"name":"moli-webmcp", "version":env!("CARGO_PKG_VERSION")})
}

fn capabilities() -> Value {
    json!({"tools":{"listChanged":true}})
}

fn supported_versions() -> Value {
    json!([MODERN_VERSION, LEGACY_VERSION, "2025-06-18", "2025-03-26"])
}

async fn post_message(
    State(state): State<Arc<HttpMcp>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !header(&headers, "content-type")
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    if !accepts(&headers, "application/json") || !accepts(&headers, "text/event-stream") {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(_) => return bad_request(&Value::Null, error(-32700, "invalid JSON")),
    };
    let id = message.get("id").unwrap_or(&Value::Null);
    let Some(method) = message["method"]
        .as_str()
        .filter(|_| message["jsonrpc"] == "2.0" && message.get("id").is_none_or(valid_id))
    else {
        return bad_request(
            &Value::Null,
            error(-32600, "expected a JSON-RPC request or notification"),
        );
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() || params.get("_meta").is_some_and(|meta| !meta.is_object()) {
        return bad_request(id, error(-32602, "params must be an object"));
    }
    let modern = params["_meta"]
        .get("io.modelcontextprotocol/protocolVersion")
        .is_some()
        || header(&headers, "mcp-protocol-version") == Some(MODERN_VERSION);
    let session = if modern {
        if let Err(error) = validate_modern(&headers, method, &params) {
            return bad_request(id, error);
        }
        None
    } else if method == "initialize" && message.get("id").is_some() {
        return initialize(&state, id, &params, &headers);
    } else {
        match state.session(&headers) {
            Ok(session) => Some(session),
            Err(status) => return status.into_response(),
        }
    };
    if message.get("id").is_none() {
        let Some(session) = session else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        return match method {
            "notifications/initialized" => {
                session.changes.lock().borrow_and_update();
                session.initialized.store(true, Ordering::Release);
                StatusCode::ACCEPTED.into_response()
            }
            "notifications/cancelled" if valid_id(&params["requestId"]) => {
                if let Some(pending) = session.pending.lock().get(&params["requestId"].to_string())
                {
                    pending.cancel();
                }
                StatusCode::ACCEPTED.into_response()
            }
            _ => StatusCode::BAD_REQUEST.into_response(),
        };
    }
    if session
        .as_ref()
        .is_some_and(|s| !s.initialized.load(Ordering::Acquire))
        && method != "ping"
    {
        return bad_request(id, error(-32600, "session is not initialized"));
    }
    match method {
        "ping" => rpc_response(id, Ok(json!({})), modern),
        "server/discover" if modern => rpc_response(
            id,
            Ok(json!({
                "supportedVersions": supported_versions(), "capabilities":capabilities(), "instructions":INSTRUCTIONS
            })),
            true,
        ),
        "tools/list" => {
            let result = if params.get("cursor").is_some() {
                Err(error(-32602, "this catalog has no pagination cursor"))
            } else {
                state
                    .page
                    .tools()
                    .map(|tools| json!({"tools":tools}))
                    .map_err(|err| error(-32603, err.to_string()))
            };
            rpc_response(id, result, modern)
        }
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return rpc_response(id, Err(error(-32602, "tool name must be a string")), modern);
            };
            if params
                .get("arguments")
                .is_some_and(|value| !value.is_object())
            {
                return rpc_response(
                    id,
                    Err(error(-32602, "tool arguments must be an object")),
                    modern,
                );
            }
            let Some(tool) = state.page.find(name) else {
                return rpc_response(
                    id,
                    Err(error(-32602, format!("unknown WebMCP tool: {name}"))),
                    modern,
                );
            };
            if modern
                && let Err(err) =
                    validate_tool_headers(&headers, &tool.mcp["inputSchema"], &params["arguments"])
            {
                return bad_request(id, err);
            }
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let id = id.clone();
            if let Some(session) = session {
                let key = id.to_string();
                let cancellation = session.closed.child_token();
                {
                    let mut pending = session.pending.lock();
                    if pending.contains_key(&key) {
                        return bad_request(&id, error(-32600, "request ID is already in use"));
                    }
                    if pending.len() >= MAX_PENDING_CALLS {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    pending.insert(key.clone(), cancellation.clone());
                }
                // A legacy HTTP disconnect is not cancellation. Keep the native
                // invocation until explicit cancellation, session close or timeout.
                let task = tokio::spawn(async move {
                    let result = call_tool(&state, tool, arguments, cancellation).await;
                    session.pending.lock().remove(&key);
                    rpc_response(&id, Ok(result), false)
                });
                task.await
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            } else {
                // Modern HTTP cancels by dropping its response stream. Keeping
                // the invocation in this stream also drops its native cleanup guard.
                sse(stream::once(async move {
                    let result = call_tool(
                        &state,
                        tool,
                        arguments,
                        state.transport.shutdown.child_token(),
                    )
                    .await;
                    let response = rpc_value(&id, Ok(result), true);
                    Ok::<_, Infallible>(
                        Event::default().event("message").data(response.to_string()),
                    )
                }))
            }
        }
        "subscriptions/listen" if modern => {
            let Some(filter) = params["notifications"].as_object() else {
                return rpc_response(
                    id,
                    Err(error(-32602, "notifications must be an object")),
                    true,
                );
            };
            if [
                "toolsListChanged",
                "promptsListChanged",
                "resourcesListChanged",
            ]
            .iter()
            .any(|key| filter.get(*key).is_some_and(|value| !value.is_boolean()))
                || filter.get("resourceSubscriptions").is_some_and(|value| {
                    !value
                        .as_array()
                        .is_some_and(|uris| uris.iter().all(Value::is_string))
                })
            {
                return rpc_response(id, Err(error(-32602, "invalid notification filter")), true);
            }
            let tools = filter.get("toolsListChanged") == Some(&Value::Bool(true));
            notifications(&state, None, Some(id.clone()), tools)
        }
        _ => rpc_response(
            id,
            Err(error(-32601, format!("unknown method: {method}"))),
            modern,
        ),
    }
}

fn validate_modern(headers: &HeaderMap, method: &str, params: &Value) -> Result<(), Value> {
    let meta = &params["_meta"];
    let version = meta["io.modelcontextprotocol/protocolVersion"]
        .as_str()
        .ok_or_else(|| error(-32602, "missing protocol version in _meta"))?;
    if !meta["io.modelcontextprotocol/clientCapabilities"].is_object() {
        return Err(error(-32602, "missing client capabilities in _meta"));
    }
    if header(headers, "mcp-protocol-version") != Some(version)
        || header(headers, "mcp-method") != Some(method)
    {
        return Err(error(
            -32020,
            "protocol version or method header does not match the body",
        ));
    }
    if method == "tools/call" {
        let name = decoded_header(headers, "mcp-name");
        if name.as_deref() != params["name"].as_str() || name.is_none() {
            return Err(error(-32020, "tool name header does not match the body"));
        }
    }
    if version != MODERN_VERSION {
        let mut err = error(-32022, "unsupported protocol version");
        err["data"] = json!({"supported":supported_versions(), "requested":version});
        return Err(err);
    }
    Ok(())
}

fn decoded_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = header(headers, name)?;
    if let Some(encoded) = value
        .strip_prefix("=?base64?")
        .and_then(|value| value.strip_suffix("?="))
    {
        String::from_utf8(STANDARD.decode(encoded).ok()?).ok()
    } else {
        Some(value.to_owned())
    }
}

fn validate_tool_headers(headers: &HeaderMap, schema: &Value, value: &Value) -> Result<(), Value> {
    if let Some(name) = schema["x-mcp-header"].as_str() {
        let expected = match value {
            Value::String(value) => Some(value.clone()),
            Value::Bool(_) | Value::Number(_) => Some(value.to_string()),
            _ => None,
        };
        if decoded_header(headers, &format!("mcp-param-{name}")) != expected {
            return Err(error(
                -32020,
                format!("tool parameter header does not match the body: {name}"),
            ));
        }
    }
    if let Some(properties) = schema["properties"].as_object() {
        for (name, property) in properties {
            validate_tool_headers(headers, property, &value[name])?;
        }
    }
    Ok(())
}

fn expire_sessions(sessions: &mut HashMap<String, Arc<Session>>) {
    sessions.retain(|_, session| {
        let keep = session.last_used.lock().elapsed() < SESSION_IDLE_TIMEOUT;
        if !keep {
            session.closed.cancel();
        }
        keep
    });
}

fn initialize(state: &HttpMcp, id: &Value, params: &Value, headers: &HeaderMap) -> Response {
    if headers.contains_key("mcp-session-id") {
        return bad_request(id, error(-32600, "initialize must start a new session"));
    }
    let Some(requested) = params["protocolVersion"].as_str() else {
        return bad_request(id, error(-32602, "missing protocolVersion"));
    };
    if !params["capabilities"].is_object()
        || !params["clientInfo"]["name"].is_string()
        || !params["clientInfo"]["version"].is_string()
    {
        return bad_request(id, error(-32602, "missing capabilities or clientInfo"));
    }
    let version = if LEGACY_VERSIONS.contains(&requested) {
        requested
    } else {
        LEGACY_VERSION
    };
    let mut random = [0; 32];
    if let Err(err) = getrandom::fill(&mut random) {
        return rpc_response(id, Err(error(-32603, err.to_string())), false);
    }
    let session_id = URL_SAFE_NO_PAD.encode(random);
    let mut sessions = state.sessions.lock();
    expire_sessions(&mut sessions);
    if sessions.len() >= MAX_SESSIONS {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    sessions.insert(
        session_id.clone(),
        Arc::new(Session {
            version: version.to_owned(),
            initialized: AtomicBool::new(false),
            last_used: Mutex::new(Instant::now()),
            closed: state.transport.shutdown.child_token(),
            pending: Mutex::default(),
            changes: Mutex::new(state.page.changes()),
            stream: Mutex::default(),
        }),
    );
    let mut response = rpc_response(
        id,
        Ok(json!({
            "protocolVersion":version, "capabilities":capabilities(),
            "serverInfo":server_info(), "instructions":INSTRUCTIONS
        })),
        false,
    );
    response
        .headers_mut()
        .insert("mcp-session-id", session_id.parse().unwrap());
    response
}

async fn call_tool(
    state: &HttpMcp,
    tool: SiteTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Value {
    let invocation = state.page.invoke(tool, arguments, state.config.auto_submit);
    let outcome = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(anyhow::anyhow!("WebMCP invocation canceled")),
        result = tokio::time::timeout(state.config.tool_timeout, invocation) => {
            result.context("WebMCP invocation timed out").and_then(|result| result)
        }
    };
    let (output, is_error) = match outcome {
        Ok(response) if response["status"] == "Completed" => (response["output"].clone(), false),
        Ok(response) => (json!(response_error(&response)), true),
        Err(err) => (json!(err.to_string()), true),
    };
    let text = output
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| output.to_string());
    let mut result = json!({"content":[{"type":"text", "text":text}], "isError":is_error});
    if !is_error && output.is_object() {
        result["structuredContent"] = output;
    }
    result
}

fn rpc_value(id: &Value, result: Result<Value, Value>, modern: bool) -> Value {
    match result {
        Ok(mut result) => {
            if modern {
                result["resultType"] = json!("complete");
                result["_meta"] = json!({"io.modelcontextprotocol/serverInfo":server_info()});
            }
            json!({"jsonrpc":"2.0", "id":id, "result":result})
        }
        Err(err) => json!({"jsonrpc":"2.0", "id":id, "error":err}),
    }
}

async fn get_notifications(State(state): State<Arc<HttpMcp>>, headers: HeaderMap) -> Response {
    if header(&headers, "mcp-protocol-version") == Some(MODERN_VERSION) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if !accepts(&headers, "text/event-stream") {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    match state.session(&headers) {
        Ok(session) if session.initialized.load(Ordering::Acquire) => {
            notifications(&state, Some(session), None, true)
        }
        Ok(_) => StatusCode::BAD_REQUEST.into_response(),
        Err(status) => status.into_response(),
    }
}

fn notifications(
    state: &HttpMcp,
    session: Option<Arc<Session>>,
    id: Option<Value>,
    tools: bool,
) -> Response {
    let closed = session.as_ref().map_or_else(
        || state.transport.shutdown.child_token(),
        |session| {
            let token = session.closed.child_token();
            if let Some(previous) = session.stream.lock().replace(token.clone()) {
                previous.cancel();
            }
            token
        },
    );
    let changes = session.as_ref().map_or_else(
        || state.page.changes(),
        |session| session.changes.lock().clone(),
    );
    let meta = id
        .as_ref()
        .map(|id| json!({"io.modelcontextprotocol/subscriptionId":id}));
    let first = meta.as_ref().map(|meta| json!({
        "jsonrpc":"2.0", "method":"notifications/subscriptions/acknowledged", "params":{
            "_meta":meta, "notifications":if tools { json!({"toolsListChanged":true}) } else { json!({}) }
        }
    }));
    let page = state.page.clone();
    sse(stream::unfold(
        (page, changes, closed, session, meta, first),
        move |(page, mut changes, closed, session, meta, first)| async move {
            let event = if let Some(first) = first {
                first
            } else {
                tokio::select! {
                    biased;
                    _ = closed.cancelled() => return None,
                    _ = page.closed().cancelled() => return None,
                    result = changes.changed(), if tools => { if result.is_err() { return None; } }
                }
                if let Some(session) = &session {
                    session.changes.lock().borrow_and_update();
                }
                let mut notification =
                    json!({"jsonrpc":"2.0", "method":"notifications/tools/list_changed"});
                if let Some(meta) = &meta {
                    notification["params"] = json!({"_meta":meta});
                }
                notification
            };
            Some((
                Ok::<_, Infallible>(Event::default().event("message").data(event.to_string())),
                (page, changes, closed, session, meta, None),
            ))
        },
    ))
}

fn sse(
    events: impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static,
) -> Response {
    let mut response = Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

async fn delete_session(State(state): State<Arc<HttpMcp>>, headers: HeaderMap) -> Response {
    if header(&headers, "mcp-protocol-version") == Some(MODERN_VERSION) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    match state.session(&headers) {
        Ok(session) => {
            session.closed.cancel();
            state
                .sessions
                .lock()
                .remove(header(&headers, "mcp-session-id").unwrap());
            StatusCode::OK.into_response()
        }
        Err(status) => status.into_response(),
    }
}
