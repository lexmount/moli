use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::get,
};
use futures_util::StreamExt;
use moli_core::runtime::storage_partition::StoragePartitionState;
use parking_lot::Mutex;
use std::sync::Arc;
use tower::ServiceExt;

const PAGE: &str = r#"<!doctype html><body>
<form toolname="manual" tooldescription="Return a confirmed form">
<input name="message" required><button>Send</button></form>
<iframe src="/child"></iframe>
<script>
let count = 0, extra;
const ctx = document.modelContext;
ctx.registerTool({name:'echo',description:'Root echo',
 inputSchema:{type:'object',properties:{text:{type:'string','x-mcp-header':'Text'}},required:['text']},
 annotations:{readOnlyHint:true}, execute:({text})=>({text,count:++count})});
ctx.registerTool({name:'waiting',description:'Wait for cancellation',
 execute:(_,options)=>{globalThis.waitCount=(globalThis.waitCount||0)+1;
 return new Promise(resolve=>options.signal.addEventListener('abort',()=>{
 globalThis.cancelCount=(globalThis.cancelCount||0)+1;resolve('late');}));}});
ctx.registerTool({name:'fail',description:'Throw an error',execute:()=>{throw new Error('fixture failure')}});
ctx.registerTool({name:'add',description:'Add a tool',execute:async()=>{
 extra=new AbortController();await ctx.registerTool({name:'extra',description:'Extra',execute:()=>({extra:true})},
 {signal:extra.signal});return 'added';}});
ctx.registerTool({name:'drop',description:'Remove a tool',execute:()=>{extra.abort();return 'removed'}});
document.forms[0].addEventListener('submit',event=>{
 event.preventDefault();if(event.agentInvoked)event.respondWith({message:document.forms[0].elements.message.value});
});
</script>"#;
const CHILD: &str = r#"<!doctype html><script>
document.modelContext.registerTool({name:'echo',description:'Child echo',execute:({text})=>({text,child:true})});
</script>"#;

#[tokio::test]
async fn mcp_page_open_waits_for_the_requested_document() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (started, requested) = tokio::sync::oneshot::channel();
    let started = Arc::new(Mutex::new(Some(started)));
    let release = Arc::new(tokio::sync::Notify::new());
    let ready = release.clone();
    let fixture = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/",
                get(move || {
                    let started = started.clone();
                    let ready = ready.clone();
                    async move {
                        if let Some(started) = started.lock().take() {
                            let _ = started.send(());
                        }
                        ready.notified().await;
                        axum::response::Html(
                            "<!doctype html><main id=ready>requested document</main>",
                        )
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let state = AppState::new_with_storage_partition_and_runtime_config(
        "127.0.0.1:9222".parse().unwrap(),
        Arc::new(StoragePartitionState::open(None).unwrap()),
        Default::default(),
        1000,
    )
    .unwrap();
    let owner = state.cdp_owner_registry.shared_owner().unwrap();
    let mut opening = tokio::spawn(async move { WebMcpPage::open(owner, &url).await });
    tokio::time::timeout(Duration::from_secs(5), requested)
        .await
        .unwrap()
        .unwrap();
    // The server has received navigation but has not returned a Document.
    // Completing now would accept the old about:blank page as the result.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut opening)
            .await
            .is_err()
    );
    release.notify_one();
    let page = tokio::time::timeout(Duration::from_secs(5), opening)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let result = page
        .command(
            "Runtime.evaluate",
            json!({
                "expression":"document.querySelector('#ready').textContent", "returnByValue":true
            }),
        )
        .await
        .unwrap();
    assert_eq!(result["result"]["value"], "requested document");
    drop(page);
    state.cdp_owner_registry.shutdown().await;
    fixture.abort();
}

struct TestService {
    state: AppState,
    page: WebMcpPage,
    router: Router,
    session: String,
    cancellation: CancellationToken,
    fixture: tokio::task::JoinHandle<()>,
}

impl Drop for TestService {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.fixture.abort();
    }
}

impl TestService {
    async fn open(auto_submit: bool, tool_timeout: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let fixture = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/", get(|| async { axum::response::Html(PAGE) }))
                    .route("/child", get(|| async { axum::response::Html(CHILD) })),
            )
            .await
            .unwrap();
        });
        let state = AppState::new_with_storage_partition_and_runtime_config(
            "127.0.0.1:9222".parse().unwrap(),
            Arc::new(StoragePartitionState::open(None).unwrap()),
            Default::default(),
            1000,
        )
        .unwrap();
        let page = tokio::time::timeout(
            Duration::from_secs(10),
            WebMcpPage::open(state.cdp_owner_registry.shared_owner().unwrap(), &url),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut changes = page.changes();
            while page.tools().unwrap().len() < 7 {
                changes.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let cancellation = CancellationToken::new();
        let transport = transport_config(
            vec!["127.0.0.1".to_owned(), "localhost".to_owned()],
            9222,
            cancellation.clone(),
        );
        let router = build_router(state.clone()).merge(mcp_router(
            page.clone(),
            WebMcpConfig {
                auto_submit,
                tool_timeout,
                ..Default::default()
            },
            transport,
        ));
        let response = router
            .clone()
            .oneshot(mcp_request(
                None,
                json!({
                    "jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                        "protocolVersion":"2025-11-25", "capabilities":{},
                        "clientInfo":{"name":"test", "version":"1"}
                    }
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let session = response.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let initialized = read_json(response).await;
        assert_eq!(
            initialized["result"]["capabilities"]["tools"]["listChanged"],
            true
        );
        let notification = router
            .clone()
            .oneshot(mcp_request(
                Some(&session),
                json!({
                    "jsonrpc":"2.0", "method":"notifications/initialized"
                }),
            ))
            .await
            .unwrap();
        assert_eq!(notification.status(), StatusCode::ACCEPTED);
        Self {
            state,
            page,
            router,
            session,
            cancellation,
            fixture,
        }
    }

    async fn rpc(&self, id: u64, method: &str, params: Value) -> Value {
        let response = self
            .router
            .clone()
            .oneshot(mcp_request(
                Some(&self.session),
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            ))
            .await
            .unwrap();
        read_json(response).await
    }

    async fn call(&self, id: u64, name: &str, arguments: Value) -> Value {
        self.rpc(id, "tools/call", json!({"name":name,"arguments":arguments}))
            .await
    }

    async fn evaluate(&self, expression: &str) -> Value {
        self.page
            .command(
                "Runtime.evaluate",
                json!({"expression":expression,"returnByValue":true}),
            )
            .await
            .unwrap()["result"]["value"]
            .clone()
    }

    async fn wait_for(&self, expression: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.evaluate(expression).await != true {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn close(self) {
        self.state.cdp_owner_registry.shutdown().await;
    }
}

fn mcp_request(session: Option<&str>, value: Value) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "127.0.0.1:9222")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    request.body(Body::from(value.to_string())).unwrap()
}

fn modern_request(id: Value, method: &str, mut params: Value) -> Request<Body> {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"modern-test", "version":"1"},
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    let mut request = mcp_request(
        None,
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    );
    let headers = request.headers_mut();
    headers.insert("mcp-protocol-version", "2026-07-28".parse().unwrap());
    headers.insert("mcp-method", method.parse().unwrap());
    if let Some(name) = params["name"].as_str() {
        headers.insert("mcp-name", name.parse().unwrap());
    }
    if let Some(text) = params["arguments"]["text"].as_str() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        headers.insert(
            "mcp-param-text",
            format!("=?base64?{}?=", STANDARD.encode(text))
                .parse()
                .unwrap(),
        );
    }
    request
}

async fn next_sse_event<S>(stream: &mut S) -> Value
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut received = String::new();
        loop {
            received.push_str(&String::from_utf8_lossy(
                &stream.next().await.unwrap().unwrap(),
            ));
            for line in received.lines() {
                if let Some(data) = line.strip_prefix("data: ")
                    && let Ok(event) = serde_json::from_str(data)
                {
                    return event;
                }
            }
        }
    })
    .await
    .unwrap()
}

async fn read_json(response: axum::response::Response) -> Value {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    if let Ok(value) = serde_json::from_slice(&bytes) {
        return value;
    }
    // Legacy session responses use SSE, including an empty priming event.
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .find_map(|data| serde_json::from_str(data).ok())
        .unwrap_or_else(|| panic!("MCP response {status} has no JSON message: {text}"))
}

#[tokio::test]
async fn mcp_catalog_and_calls_share_live_cdp_page() {
    let service = TestService::open(false, Duration::from_secs(5)).await;
    let listed = service.rpc(2, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 7);
    let root = tools
        .iter()
        .find(|tool| tool["description"] == "Root echo")
        .unwrap();
    let child = tools
        .iter()
        .find(|tool| tool["description"] == "Child echo")
        .unwrap();
    assert_ne!(root["name"], child["name"]);
    assert_eq!(root["inputSchema"]["required"], json!(["text"]));
    assert_eq!(root["annotations"]["readOnlyHint"], true);
    assert_eq!(child["_meta"]["moli/webmcp"]["name"], "echo");
    let native_name = root["name"].as_str().unwrap();
    let echoed = service.call(3, native_name, json!({"text":"hello"})).await;
    assert_eq!(
        echoed["result"]["structuredContent"],
        json!({"text":"hello", "count":1})
    );
    let echoed = service.call(4, native_name, json!({"text":"again"})).await;
    assert_eq!(echoed["result"]["structuredContent"]["count"], 2);
    let echoed = service
        .call(5, child["name"].as_str().unwrap(), json!({"text":"child"}))
        .await;
    assert_eq!(echoed["result"]["structuredContent"]["child"], true);
    let failed = service.call(6, "fail", json!({})).await;
    assert_eq!(failed["result"]["isError"], true);
    assert!(
        failed["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("fixture failure")
    );
    let unknown = service.call(7, "unknown", json!({})).await;
    assert_eq!(unknown["error"]["code"], -32602);
    let changes = service
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header("host", "127.0.0.1:9222")
                .header("accept", "text/event-stream")
                .header("mcp-session-id", &service.session)
                .header("mcp-protocol-version", "2025-11-25")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(changes.status(), StatusCode::OK);
    let mut stream = changes.into_body().into_data_stream();
    service.call(8, "add", json!({})).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut received = String::new();
        while !received.contains("notifications/tools/list_changed") {
            let chunk = stream.next().await.unwrap().unwrap();
            received.push_str(&String::from_utf8_lossy(&chunk));
        }
    })
    .await
    .unwrap();
    drop(stream);
    assert!(service.page.find("extra").is_some());
    service.call(9, "drop", json!({})).await;
    assert!(service.page.find("extra").is_none());
    let discovery = service
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/json/list")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let discovery = read_json(discovery).await;
    assert!(discovery.as_array().unwrap().iter().any(|target| {
        target["id"] == service.page.target_id()
            && target["url"]
                .as_str()
                .unwrap()
                .starts_with("http://127.0.0.1:")
    }));
    let page_url = discovery
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["id"] == service.page.target_id())
        .unwrap()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let rejected = service
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("host", "127.0.0.1:9222")
                .header("origin", "https://other.example")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    service
        .evaluate("document.querySelector('iframe').remove()")
        .await;
    service
        .wait_for("document.querySelector('iframe') === null")
        .await;
    let mut changes = service.page.changes();
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.page.tools().unwrap().len() != 6 {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    service
        .page
        .command("Page.navigate", json!({"url":"about:blank"}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !service.page.tools().unwrap().is_empty() {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    service
        .page
        .command("Page.navigate", json!({"url":page_url}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.page.tools().unwrap().len() != 7 {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let restarted = service
        .call(16, native_name, json!({"text":"new document"}))
        .await;
    assert_eq!(restarted["result"]["structuredContent"]["count"], 1);
    let deleted = service
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/mcp")
                .header("host", "127.0.0.1:9222")
                .header("mcp-session-id", &service.session)
                .header("mcp-protocol-version", "2025-11-25")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
    let terminated = service
        .router
        .clone()
        .oneshot(mcp_request(
            Some(&service.session),
            json!({"jsonrpc":"2.0","id":17,"method":"tools/list","params":{}}),
        ))
        .await
        .unwrap();
    assert_eq!(terminated.status(), StatusCode::NOT_FOUND);
    service.close().await;
}

#[tokio::test]
async fn mcp_modern_stateless_requests_use_the_same_catalog() {
    let service = TestService::open(false, Duration::from_secs(5)).await;
    for (id, method, params) in [
        (19, "server/discover", json!({})),
        (20, "tools/list", json!({})),
        (
            21,
            "tools/call",
            json!({"name":"echo","arguments":{"text":"modern"}}),
        ),
    ] {
        let response = service
            .router
            .clone()
            .oneshot(modern_request(json!(id), method, params))
            .await
            .unwrap();
        let status = response.status();
        assert!(response.headers().get("mcp-session-id").is_none());
        let response = read_json(response).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response["result"]["resultType"], "complete");
        assert_eq!(
            response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "moli-webmcp"
        );
        if method == "server/discover" {
            assert!(
                response["result"]["supportedVersions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("2026-07-28"))
            );
        } else if method == "tools/list" {
            assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 7);
        } else {
            assert_eq!(response["result"]["structuredContent"]["text"], "modern");
        }
    }
    service.close().await;
}

#[tokio::test]
async fn mcp_cancellation_and_form_confirmation_use_native_invocations() {
    let service = TestService::open(false, Duration::from_millis(100)).await;
    let waiting = service.call(10, "waiting", json!({})).await;
    assert_eq!(waiting["result"]["isError"], true);
    service.wait_for("globalThis.cancelCount === 1").await;
    let manual = service
        .call(14, "manual", json!({"message":"timeout"}))
        .await;
    assert_eq!(manual["result"]["isError"], true);
    service
        .wait_for("!document.forms[0].matches(':tool-form-active')")
        .await;
    service.close().await;

    let service = TestService::open(false, Duration::from_secs(5)).await;
    let router = service.router.clone();
    let session = service.session.clone();
    let pending = tokio::spawn(async move {
        router
            .oneshot(mcp_request(
                Some(&session),
                json!({"jsonrpc":"2.0","id":11,
            "method":"tools/call","params":{"name":"manual","arguments":{"message":"confirmed"}}}),
            ))
            .await
            .unwrap()
    });
    service
        .wait_for("document.forms[0].matches(':tool-form-active')")
        .await;
    assert_eq!(
        service
            .evaluate("document.forms[0].elements.message.value")
            .await,
        "confirmed"
    );
    service
        .evaluate("document.forms[0].querySelector('button').click()")
        .await;
    let completed = read_json(pending.await.unwrap()).await;
    assert_eq!(
        completed["result"]["structuredContent"],
        json!({"message":"confirmed"})
    );
    let router = service.router.clone();
    let session = service.session.clone();
    let pending = tokio::spawn(async move {
        router
            .oneshot(mcp_request(
                Some(&session),
                json!({"jsonrpc":"2.0","id":12,
            "method":"tools/call","params":{"name":"waiting","arguments":{}}}),
            ))
            .await
            .unwrap()
    });
    service.wait_for("globalThis.waitCount === 1").await;
    let canceled = service
        .router
        .clone()
        .oneshot(mcp_request(
            Some(&service.session),
            json!({
                "jsonrpc":"2.0","method":"notifications/cancelled", "params":{"requestId":12}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(canceled.status(), StatusCode::ACCEPTED);
    service.wait_for("globalThis.cancelCount === 1").await;
    pending.abort();
    service.close().await;

    let service = TestService::open(true, Duration::from_secs(5)).await;
    let completed = service
        .call(13, "manual", json!({"message":"automatic"}))
        .await;
    assert_eq!(
        completed["result"]["structuredContent"],
        json!({"message":"automatic"})
    );
    let invalid = service.call(15, "manual", json!({"unknown":"input"})).await;
    assert_eq!(invalid["result"]["isError"], true);
    assert!(
        invalid["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Invalid form tool input")
    );
    service.close().await;
}

#[tokio::test]
async fn mcp_validates_json_rpc_and_http_before_running_tools() {
    let service = TestService::open(false, Duration::from_secs(5)).await;
    for (message, code) in [
        (json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]), -32600),
        (json!({"jsonrpc":"1.0","id":1,"method":"ping"}), -32600),
        (json!({"jsonrpc":"2.0","id":null,"method":"ping"}), -32600),
        (json!({"jsonrpc":"2.0","id":1.5,"method":"ping"}), -32600),
        (json!({"jsonrpc":"2.0","id":1,"result":{}}), -32600),
        (
            json!({"jsonrpc":"2.0","id":1,"method":"ping","params":[]}),
            -32602,
        ),
    ] {
        let response = service
            .router
            .clone()
            .oneshot(mcp_request(Some(&service.session), message))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(read_json(response).await["error"]["code"], code);
    }
    for (body, status, code) in [
        ("{".to_owned(), StatusCode::BAD_REQUEST, Some(-32700)),
        (
            " ".repeat(2 * 1024 * 1024 + 1),
            StatusCode::PAYLOAD_TOO_LARGE,
            None,
        ),
    ] {
        let mut request = mcp_request(Some(&service.session), json!({}));
        *request.body_mut() = Body::from(body);
        let response = service.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), status);
        if let Some(code) = code {
            assert_eq!(read_json(response).await["error"]["code"], code);
        }
    }
    for (name, value, status) in [
        ("origin", "https://other.example", StatusCode::FORBIDDEN),
        ("origin", "http://127.0.0.1:9223", StatusCode::FORBIDDEN),
        (
            "origin",
            "http://127.0.0.1:9222/path",
            StatusCode::FORBIDDEN,
        ),
        ("origin", "null", StatusCode::FORBIDDEN),
        ("host", "other.example:9222", StatusCode::FORBIDDEN),
        ("accept", "application/json", StatusCode::NOT_ACCEPTABLE),
        (
            "content-type",
            "text/plain",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            "mcp-protocol-version",
            "1900-01-01",
            StatusCode::BAD_REQUEST,
        ),
        ("mcp-session-id", "unknown", StatusCode::NOT_FOUND),
    ] {
        let mut request = mcp_request(
            Some(&service.session),
            json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
        );
        request.headers_mut().insert(name, value.parse().unwrap());
        assert_eq!(
            service
                .router
                .clone()
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            status,
            "{name}: {value}"
        );
    }
    let mut request = mcp_request(
        Some(&service.session),
        json!({"jsonrpc":"2.0","id":"ping-id","method":"ping"}),
    );
    request
        .headers_mut()
        .insert("origin", "http://127.0.0.1:9222".parse().unwrap());
    let response = service.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(read_json(response).await["id"], "ping-id");
    for (method, params, code) in [
        ("unknown", json!({}), -32601),
        ("tools/list", json!({"cursor":"next"}), -32602),
        ("tools/call", json!({"name":"echo","arguments":[]}), -32602),
        ("tools/call", json!({"name":true}), -32602),
    ] {
        assert_eq!(service.rpc(2, method, params).await["error"]["code"], code);
    }

    // Legacy negotiation chooses a supported version and enforces it on later requests.
    for (requested, expected) in [("1900-01-01", "2025-11-25"), ("2025-03-26", "2025-03-26")] {
        let response = service.router.clone().oneshot(mcp_request(None, json!({
            "jsonrpc":"2.0","id":"initialize","method":"initialize","params":{
                "protocolVersion":requested,"clientInfo":{"name":"test","version":"1"},"capabilities":{}
            }
        }))).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let session = response.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            read_json(response).await["result"]["protocolVersion"],
            expected
        );
        let mut request = mcp_request(
            Some(&session),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
        );
        request
            .headers_mut()
            .insert("mcp-protocol-version", expected.parse().unwrap());
        assert_eq!(
            service
                .router
                .clone()
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let mut request = mcp_request(
            Some(&session),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        );
        request
            .headers_mut()
            .insert("mcp-protocol-version", expected.parse().unwrap());
        assert_eq!(
            service
                .router
                .clone()
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
        let mut request = mcp_request(
            Some(&session),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
        );
        request
            .headers_mut()
            .insert("mcp-protocol-version", expected.parse().unwrap());
        assert_eq!(read_json(service.router.clone().oneshot(request).await.unwrap()).await["result"]["tools"].as_array().unwrap().len(), 7);
    }
    service.close().await;
}

fn pending_call(
    service: &TestService,
    session: &str,
    id: Value,
) -> tokio::task::JoinHandle<axum::response::Response> {
    let router = service.router.clone();
    let request = mcp_request(
        Some(session),
        json!({
            "jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"waiting"}
        }),
    );
    tokio::spawn(async move { router.oneshot(request).await.unwrap() })
}

#[tokio::test]
async fn mcp_legacy_cancellation_is_scoped_to_the_session_and_request_id() {
    let service = TestService::open(false, Duration::from_secs(5)).await;
    let initialized = service.router.clone().oneshot(mcp_request(None, json!({
        "jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","clientInfo":{"name":"second-client","version":"1"},"capabilities":{}
        }
    }))).await.unwrap();
    let second_session = initialized.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_ne!(second_session, service.session);
    let initialized = service
        .router
        .clone()
        .oneshot(mcp_request(
            Some(&second_session),
            json!({
                "jsonrpc":"2.0","method":"notifications/initialized"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(initialized.status(), StatusCode::ACCEPTED);
    let first = pending_call(&service, &service.session, json!(42));
    let second = pending_call(&service, &second_session, json!(42));
    service.wait_for("globalThis.waitCount === 2").await;
    let duplicate = service
        .rpc(42, "tools/call", json!({"name":"waiting"}))
        .await;
    assert_eq!(duplicate["error"]["code"], -32600);
    for (session, request_id) in [
        (&service.session, json!("42")),
        (&second_session, json!(42)),
    ] {
        let response = service.router.clone().oneshot(mcp_request(Some(session), json!({
            "jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request_id}
        }))).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    assert_eq!(
        read_json(second.await.unwrap()).await["result"]["isError"],
        true
    );
    service.wait_for("globalThis.cancelCount === 1").await;
    assert!(
        !first.is_finished(),
        "another client's cancellation ended the first request"
    );

    // Dropping a legacy HTTP response future leaves its invocation cancellable.
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let duplicate = service
        .rpc(42, "tools/call", json!({"name":"waiting"}))
        .await;
    assert_eq!(duplicate["error"]["code"], -32600);
    let canceled = service
        .router
        .clone()
        .oneshot(mcp_request(
            Some(&service.session),
            json!({
                "jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":42}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(canceled.status(), StatusCode::ACCEPTED);
    service.wait_for("globalThis.cancelCount === 2").await;

    let pending = pending_call(&service, &service.session, json!("close-me"));
    service.wait_for("globalThis.waitCount === 3").await;
    let closed = service
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/mcp")
                .header("host", "127.0.0.1:9222")
                .header("mcp-session-id", &service.session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(closed.status(), StatusCode::OK);
    assert_eq!(
        read_json(pending.await.unwrap()).await["result"]["isError"],
        true
    );
    service.wait_for("globalThis.cancelCount === 3").await;
    let surviving = service
        .router
        .clone()
        .oneshot(mcp_request(
            Some(&second_session),
            json!({
                "jsonrpc":"2.0","id":"still-open","method":"tools/list"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        read_json(surviving).await["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    service.close().await;
}

#[tokio::test]
async fn mcp_modern_validates_metadata_and_streams_changes_and_cancellation() {
    let service = TestService::open(false, Duration::from_secs(5)).await;
    for (header, value) in [
        ("mcp-method", "tools/list"),
        ("mcp-name", "other"),
        ("mcp-param-text", "other"),
    ] {
        let mut request = modern_request(
            json!(1),
            "tools/call",
            json!({"name":"echo","arguments":{"text":"guarded"}}),
        );
        request.headers_mut().insert(header, value.parse().unwrap());
        let response = service.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(read_json(response).await["error"]["code"], -32020);
    }
    for (version, capabilities, code) in [
        ("1900-01-01", json!({}), -32022),
        ("2026-07-28", Value::Null, -32602),
    ] {
        let mut request = modern_request(json!(2), "tools/list", json!({}));
        let meta = json!({"io.modelcontextprotocol/protocolVersion":version,"io.modelcontextprotocol/clientCapabilities":capabilities});
        *request.body_mut() = Body::from(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":meta}})
                .to_string(),
        );
        request
            .headers_mut()
            .insert("mcp-protocol-version", version.parse().unwrap());
        let response = service.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = read_json(response).await;
        assert_eq!(error["error"]["code"], code);
        if code == -32022 {
            assert!(
                error["error"]["data"]["supported"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("2026-07-28"))
            );
        }
    }
    // Rejected calls must not execute author JavaScript.
    assert_eq!(
        service
            .call(3, "echo", json!({"text":"first real call"}))
            .await["result"]["structuredContent"]["count"],
        1
    );
    let mut request = modern_request(
        json!(4),
        "tools/call",
        json!({"name":"echo","arguments":{"text":"中文"}}),
    );
    request
        .headers_mut()
        .insert("mcp-name", "=?base64?ZWNobw==?=".parse().unwrap());
    assert_eq!(
        read_json(service.router.clone().oneshot(request).await.unwrap()).await["result"]["structuredContent"]
            ["text"],
        "中文"
    );

    let subscribed = service
        .router
        .clone()
        .oneshot(modern_request(
            json!("updates"),
            "subscriptions/listen",
            json!({
                "notifications":{"toolsListChanged":true,"promptsListChanged":true}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(subscribed.status(), StatusCode::OK);
    let mut stream = subscribed.into_body().into_data_stream();
    let acknowledged = next_sse_event(&mut stream).await;
    assert_eq!(
        acknowledged["method"],
        "notifications/subscriptions/acknowledged"
    );
    assert_eq!(
        acknowledged["params"]["notifications"],
        json!({"toolsListChanged":true})
    );
    assert_eq!(
        acknowledged["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        "updates"
    );
    service.call(5, "add", json!({})).await;
    let changed = next_sse_event(&mut stream).await;
    assert_eq!(changed["method"], "notifications/tools/list_changed");
    assert_eq!(
        changed["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        "updates"
    );
    drop(stream);

    // Only one standalone stream per legacy session receives notifications.
    let request = || {
        Request::builder()
            .uri("/mcp")
            .header("host", "127.0.0.1:9222")
            .header("accept", "text/event-stream")
            .header("mcp-session-id", &service.session)
            .body(Body::empty())
            .unwrap()
    };
    let old = service.router.clone().oneshot(request()).await.unwrap();
    let current = service.router.clone().oneshot(request()).await.unwrap();
    let mut old = old.into_body().into_data_stream();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), old.next())
            .await
            .unwrap()
            .is_none()
    );
    service.call(6, "drop", json!({})).await;
    let mut current = current.into_body().into_data_stream();
    assert_eq!(
        next_sse_event(&mut current).await["method"],
        "notifications/tools/list_changed"
    );
    drop(current);
    for method in ["GET", "DELETE"] {
        let request = Request::builder()
            .method(method)
            .uri("/mcp")
            .header("host", "127.0.0.1:9222")
            .header("mcp-protocol-version", "2026-07-28")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            service
                .router
                .clone()
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    let pending = service
        .router
        .clone()
        .oneshot(modern_request(
            json!("cancel-me"),
            "tools/call",
            json!({"name":"waiting"}),
        ))
        .await
        .unwrap();
    assert_eq!(pending.status(), StatusCode::OK);
    let reading = tokio::spawn(read_json(pending));
    service.wait_for("globalThis.waitCount === 1").await;
    reading.abort();
    assert!(reading.await.unwrap_err().is_cancelled());
    service.wait_for("globalThis.cancelCount === 1").await;
    service.close().await;
}
