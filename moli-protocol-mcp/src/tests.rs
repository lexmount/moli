use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::{HttpConfig, ServerInfo, ToolService, router};

struct Fixture {
    started: watch::Sender<usize>,
    dropped: AtomicUsize,
    changed: watch::Sender<u64>,
    closed: CancellationToken,
    shutdown: CancellationToken,
}

struct Tool(Value);

impl AsRef<Value> for Tool {
    fn as_ref(&self) -> &Value {
        &self.0
    }
}

struct Invocation(Arc<Fixture>);

impl Drop for Invocation {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl ToolService for Arc<Fixture> {
    type Tool = Tool;

    fn tools(&self) -> Result<Vec<Value>, String> {
        if self.closed.is_cancelled() {
            return Err("fixture is closed".to_owned());
        }
        Ok(["echo", "waiting", "fail"]
            .into_iter()
            .map(|name| {
                json!({
                    "name":name,"description":name,"inputSchema":{"type":"object","properties":{}}
                })
            })
            .collect())
    }

    fn tool(&self, name: &str) -> Option<Tool> {
        self.tools()
            .ok()?
            .into_iter()
            .find(|tool| tool["name"] == name)
            .map(Tool)
    }

    async fn call(&self, tool: Tool, arguments: Value) -> Result<Value, String> {
        match tool.0["name"].as_str().unwrap() {
            "echo" => Ok(arguments["output"].clone()),
            "fail" => Err("application failure".to_owned()),
            "waiting" => {
                let _invocation = Invocation(self.clone());
                self.started.send_modify(|started| *started += 1);
                pending().await
            }
            _ => unreachable!(),
        }
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn closed(&self) -> &CancellationToken {
        &self.closed
    }
}

fn fixture() -> (Router, Arc<Fixture>) {
    let fixture = Arc::new(Fixture {
        started: watch::channel(0).0,
        dropped: AtomicUsize::new(0),
        changed: watch::channel(0).0,
        closed: CancellationToken::new(),
        shutdown: CancellationToken::new(),
    });
    let app = router(
        fixture.clone(),
        HttpConfig {
            allowed_hosts: vec!["127.0.0.1".to_owned()],
            port: 9222,
            server_info: ServerInfo {
                name: "fixture-server".to_owned(),
                version: "1".to_owned(),
                instructions: None,
            },
            shutdown: fixture.shutdown.clone(),
        },
    );
    (app, fixture)
}

fn request(session: Option<&str>, message: Value) -> Request<Body> {
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
    request.body(Body::from(message.to_string())).unwrap()
}

fn modern_request(id: Value, method: &str, mut params: Value) -> Request<Body> {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    let mut request = request(
        None,
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    );
    request
        .headers_mut()
        .insert("mcp-protocol-version", "2026-07-28".parse().unwrap());
    request
        .headers_mut()
        .insert("mcp-method", method.parse().unwrap());
    if let Some(name) = params["name"].as_str() {
        request
            .headers_mut()
            .insert("mcp-name", name.parse().unwrap());
    }
    request
}

async fn read_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    if let Ok(value) = serde_json::from_slice(&bytes) {
        return value;
    }
    let text = String::from_utf8_lossy(&bytes);
    serde_json::from_str(
        text.lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap()
}

async fn initialize(app: &Router) -> String {
    let response = app.clone().oneshot(request(None, json!({
        "jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}
        }
    }))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let result = read_json(response).await;
    assert_eq!(result["result"]["serverInfo"]["name"], "fixture-server");
    assert!(result["result"].get("instructions").is_none());
    let initialized = app
        .clone()
        .oneshot(request(
            Some(&session),
            json!({
                "jsonrpc":"2.0","method":"notifications/initialized"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(initialized.status(), StatusCode::ACCEPTED);
    session
}

async fn wait_for_started(fixture: &Fixture, count: usize) {
    let mut started = fixture.started.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while *started.borrow_and_update() < count {
            started.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

async fn next_sse_event<S>(stream: &mut S) -> Value
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let bytes = stream
                .next()
                .await
                .expect("stream ended before event")
                .unwrap();
            if let Some(data) = String::from_utf8_lossy(&bytes)
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
            {
                return serde_json::from_str(data).unwrap();
            }
        }
    })
    .await
    .unwrap()
}

async fn assert_stream_ended<S>(stream: &mut S)
where
    S: futures_util::Stream + Unpin,
{
    assert!(
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .is_none(),
        "stream emitted an event after completion"
    );
}

#[tokio::test]
async fn ping_is_only_available_to_legacy_clients() {
    let (app, _) = fixture();
    let session = initialize(&app).await;
    let ping = app
        .clone()
        .oneshot(request(
            Some(&session),
            json!({"jsonrpc":"2.0","id":"legacy-ping","method":"ping"}),
        ))
        .await
        .unwrap();
    assert_eq!(ping.status(), StatusCode::OK);
    assert_eq!(
        read_json(ping).await,
        json!({"jsonrpc":"2.0","id":"legacy-ping","result":{}})
    );
    let ping = app
        .oneshot(modern_request(json!("modern-ping"), "ping", json!({})))
        .await
        .unwrap();
    assert_eq!(ping.status(), StatusCode::OK);
    let response = read_json(ping).await;
    assert_eq!(response["id"], "modern-ping");
    assert_eq!(response["error"]["code"], -32601);
    assert!(response.get("result").is_none());
}

#[tokio::test]
async fn modern_subscriptions_finish_once_after_acknowledgment_and_changes() {
    for id in [json!(7), json!("updates")] {
        for filter in [json!({"toolsListChanged":true}), json!({})] {
            for shutdown in [false, true] {
                let (app, fixture) = fixture();
                let response = app
                    .oneshot(modern_request(
                        id.clone(),
                        "subscriptions/listen",
                        json!({"notifications":filter}),
                    ))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let mut stream = response.into_body().into_data_stream();
                assert_eq!(
                    next_sse_event(&mut stream).await,
                    json!({
                        "jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged",
                        "params":{"_meta":{"io.modelcontextprotocol/subscriptionId":id},
                                  "notifications":filter}
                    })
                );
                fixture.changed.send_modify(|version| *version += 1);
                if filter["toolsListChanged"] == true {
                    assert_eq!(
                        next_sse_event(&mut stream).await,
                        json!({
                            "jsonrpc":"2.0","method":"notifications/tools/list_changed",
                            "params":{"_meta":{"io.modelcontextprotocol/subscriptionId":id}}
                        })
                    );
                }
                // A queued catalog change must not outrun graceful completion.
                fixture.changed.send_modify(|version| *version += 1);
                if shutdown {
                    fixture.shutdown.cancel();
                } else {
                    fixture.closed.cancel();
                }
                assert_eq!(
                    next_sse_event(&mut stream).await,
                    json!({
                        "jsonrpc":"2.0","id":id,"result":{
                            "resultType":"complete","_meta":{
                                "io.modelcontextprotocol/subscriptionId":id,
                                "io.modelcontextprotocol/serverInfo":{"name":"fixture-server","version":"1"}
                            }
                        }
                    })
                );
                assert_stream_ended(&mut stream).await;
                assert_eq!(fixture.changed.receiver_count(), 0);
            }
        }
    }
}

#[tokio::test]
async fn disconnected_modern_subscription_releases_its_listener() {
    let (app, fixture) = fixture();
    let response = app
        .clone()
        .oneshot(modern_request(
            json!("disconnected"),
            "subscriptions/listen",
            json!({"notifications":{"toolsListChanged":true}}),
        ))
        .await
        .unwrap();
    let mut stream = response.into_body().into_data_stream();
    let acknowledged = next_sse_event(&mut stream).await;
    assert_eq!(
        acknowledged["method"],
        "notifications/subscriptions/acknowledged"
    );
    assert_eq!(fixture.changed.receiver_count(), 1);
    drop(stream);
    assert_eq!(fixture.changed.receiver_count(), 0);
    assert!(!fixture.closed.is_cancelled());
    assert!(!fixture.shutdown.is_cancelled());
    assert_eq!(
        read_json(
            app.oneshot(modern_request(json!(2), "tools/list", json!({})))
                .await
                .unwrap()
        )
        .await["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn legacy_notification_streams_end_without_a_final_response() {
    for shutdown in [false, true] {
        let (app, fixture) = fixture();
        let session = initialize(&app).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .header("host", "127.0.0.1:9222")
                    .header("accept", "text/event-stream")
                    .header("mcp-session-id", session)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        fixture.changed.send_modify(|version| *version += 1);
        assert_eq!(
            next_sse_event(&mut stream).await,
            json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"})
        );
        if shutdown {
            fixture.shutdown.cancel();
        } else {
            fixture.closed.cancel();
        }
        assert_stream_ended(&mut stream).await;
    }
}

#[tokio::test]
async fn application_tools_and_identity_work_without_browser_dependencies() {
    let (app, _) = fixture();
    let session = initialize(&app).await;
    let listed = app
        .clone()
        .oneshot(request(
            Some(&session),
            json!({
                "jsonrpc":"2.0","id":2,"method":"tools/list"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        read_json(listed).await["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for output in [
        json!({"nested":true}),
        json!("hello"),
        json!([true, 42, null]),
    ] {
        let called = app
            .clone()
            .oneshot(modern_request(
                json!(3),
                "tools/call",
                json!({
                    "name":"echo","arguments":{"output":output}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(called.status(), StatusCode::OK);
        let result = read_json(called).await;
        assert_eq!(
            result["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "fixture-server"
        );
        assert_eq!(result["result"]["resultType"], "complete");
        assert_eq!(result["result"]["isError"], false);
        if output.is_object() {
            assert_eq!(result["result"]["structuredContent"], output);
        } else {
            assert_eq!(
                result["result"]["content"][0]["text"],
                output
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| output.to_string())
            );
        }
    }
    let failed = app
        .clone()
        .oneshot(modern_request(
            json!(4),
            "tools/call",
            json!({"name":"fail"}),
        ))
        .await
        .unwrap();
    let result = read_json(failed).await;
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(
        result["result"]["content"][0]["text"],
        "application failure"
    );
}

#[tokio::test]
async fn legacy_cancellation_drops_only_the_matching_application_future() {
    let (app, fixture) = fixture();
    let first = initialize(&app).await;
    let second = initialize(&app).await;
    let invoke = |session: &str| {
        let app = app.clone();
        let request = request(
            Some(session),
            json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"waiting"}}),
        );
        tokio::spawn(async move { app.oneshot(request).await.unwrap() })
    };
    let first_call = invoke(&first);
    let second_call = invoke(&second);
    wait_for_started(&fixture, 2).await;
    let canceled = app
        .clone()
        .oneshot(request(
            Some(&first),
            json!({
                "jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(canceled.status(), StatusCode::ACCEPTED);
    assert_eq!(
        read_json(first_call.await.unwrap()).await["result"]["isError"],
        true
    );
    assert_eq!(fixture.dropped.load(Ordering::SeqCst), 1);
    assert!(!second_call.is_finished());
    let closed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/mcp")
                .header("host", "127.0.0.1:9222")
                .header("mcp-session-id", second)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(closed.status(), StatusCode::OK);
    assert_eq!(
        read_json(second_call.await.unwrap()).await["result"]["isError"],
        true
    );
    assert_eq!(fixture.dropped.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn disconnect_and_service_closure_drop_application_futures() {
    let (app, fixture) = fixture();
    let response = app
        .clone()
        .oneshot(modern_request(
            json!(1),
            "tools/call",
            json!({"name":"waiting"}),
        ))
        .await
        .unwrap();
    let reader = tokio::spawn(read_json(response));
    wait_for_started(&fixture, 1).await;
    reader.abort();
    assert!(reader.await.unwrap_err().is_cancelled());
    assert_eq!(fixture.dropped.load(Ordering::SeqCst), 1);
    let session = initialize(&app).await;
    let response = tokio::spawn(async move {
        app.oneshot(request(
            Some(&session),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"waiting"}}),
        ))
        .await
        .unwrap()
    });
    wait_for_started(&fixture, 2).await;
    fixture.closed.cancel();
    assert_eq!(
        read_json(response.await.unwrap()).await["result"]["isError"],
        true
    );
    assert_eq!(fixture.dropped.load(Ordering::SeqCst), 2);
}
