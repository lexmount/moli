use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::get,
};
use futures_util::StreamExt;
use moli_core::runtime::storage_partition::StoragePartitionState;
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
 inputSchema:{type:'object',properties:{text:{type:'string'}},required:['text']},
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
        let transport = StreamableHttpServerConfig::default()
            .with_json_response(true)
            .enforce_origin_validation()
            .with_cancellation_token(cancellation.clone());
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
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"modern-test", "version":"1"},
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    for (id, method, params) in [
        (20, "tools/list", json!({"_meta":meta})),
        (
            21,
            "tools/call",
            json!({"name":"echo","arguments":{"text":"modern"},"_meta":meta}),
        ),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", "127.0.0.1:9222")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2026-07-28")
            .header("mcp-method", method);
        if let Some(name) = params["name"].as_str() {
            request = request.header("mcp-name", name);
        }
        let response = service
            .router
            .clone()
            .oneshot(
                request
                    .body(Body::from(
                        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert!(response.headers().get("mcp-session-id").is_none());
        let response = read_json(response).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert!(response.get("error").is_none(), "{response}");
        if method == "tools/list" {
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
