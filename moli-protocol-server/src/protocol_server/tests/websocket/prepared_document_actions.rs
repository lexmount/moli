use super::*;
use serde_json::Value;

type TestCdpSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone, Copy, Debug)]
enum Action {
    Continue,
    Fulfill,
    Fail,
}

#[tokio::test]
async fn websocket_cdp_prepared_document_request_actions_settle_before_adoption() {
    for crashed in [false, true] {
        for action in [Action::Continue, Action::Fulfill, Action::Fail] {
            verify_fetch(crashed, false, action).await;
        }
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_response_actions_settle_before_adoption() {
    for crashed in [false, true] {
        for action in [Action::Continue, Action::Fulfill, Action::Fail] {
            verify_fetch(crashed, true, action).await;
        }
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_error_commit_follows_network_failure() {
    for crashed in [false, true] {
        // Close every connection without HTTP headers, including network
        // retries, so the navigation creates a browser-owned error Document.
        let reset = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let failed_url = format!("http://{}/reset", reset.local_addr().unwrap());
        let reset_server = tokio::spawn(async move {
            while let Ok((stream, _)) = reset.accept().await {
                drop(stream);
            }
        });
        let (addr, server) = spawn_test_protocol_server().await;
        let (mut socket, _) =
            connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                .await
                .unwrap();
        let context = cdp_create_browser_context(&mut socket, 1).await;
        let session = cdp_create_attached_target(&mut socket, 2, &context).await;
        configure(&mut socket, &session.session_id, crashed, false).await;
        send_cdp_command_without_wait(
            &mut socket,
            20,
            "Page.navigate",
            Some(&session.session_id),
            json!({"url": failed_url}),
        )
        .await;
        let mut observed = recv_until_match(&mut socket, |message| {
            message["method"] == "Page.loadEventFired" && message["sessionId"] == session.session_id
        })
        .await;
        if !observed.iter().any(|message| message["id"] == 20) {
            observed.extend(recv_until_match(&mut socket, |message| message["id"] == 20).await);
        }
        let methods = [
            "Network.requestWillBeSent",
            "Network.loadingFailed",
            "Page.frameNavigated",
            "Network.loadingFinished",
            "Page.domContentEventFired",
            "Page.loadEventFired",
        ];
        let positions = methods.map(|method| {
            let matching = observed
                .iter()
                .enumerate()
                .filter(|(_, message)| {
                    message["method"] == method && message["sessionId"] == session.session_id
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "{crashed}/{method}: {observed:?}");
            matching[0]
        });
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{crashed}: {observed:?}"
        );
        let request = &observed[positions[0]]["params"]["requestId"];
        assert_eq!(&observed[positions[1]]["params"]["requestId"], request);
        assert_eq!(&observed[positions[3]]["params"]["requestId"], request);
        let navigation = &observed.iter().find(|message| message["id"] == 20).unwrap()["result"];
        assert_eq!(
            navigation["errorText"],
            observed[positions[1]]["params"]["errorText"]
        );
        assert_eq!(
            observed[positions[2]]["params"]["frame"]["unreachableUrl"],
            failed_url
        );
        assert_eq!(
            observed[positions[2]]["params"]["frame"]["loaderId"],
            navigation["loaderId"]
        );
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
        reset_server.abort();
        let _ = reset_server.await;
    }
}

struct Fixture {
    url: String,
    requested: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    replacement_requested: Arc<tokio::sync::Notify>,
    arm_requested: Arc<tokio::sync::Notify>,
    arm_release: Arc<tokio::sync::Notify>,
    api_requests: Arc<std::sync::atomic::AtomicUsize>,
    _server: DedicatedFixtureServer,
}

impl Fixture {
    fn new() -> Self {
        let requested = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let request = Arc::clone(&requested);
        let gate = Arc::clone(&release);
        let replacement_requested = Arc::new(tokio::sync::Notify::new());
        let replacement = Arc::clone(&replacement_requested);
        let arm_requested = Arc::new(tokio::sync::Notify::new());
        let arm_release = Arc::new(tokio::sync::Notify::new());
        let arm_request = Arc::clone(&arm_requested);
        let arm_gate = Arc::clone(&arm_release);
        let api_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let api_count = Arc::clone(&api_requests);
        let router = Router::new()
            .route(
                "/page",
                get(|| async { "document is fulfilled by the client" }),
            )
            .route(
                "/api",
                get(move || {
                    let count = Arc::clone(&api_count);
                    async move {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        "network-api"
                    }
                }),
            )
            .route(
                "/arm",
                get(move || {
                    let requested = Arc::clone(&arm_request);
                    let release = Arc::clone(&arm_gate);
                    async move {
                        requested.notify_one();
                        // Hold response headers, so fetch().then() cannot run.
                        release.notified().await;
                        "armed"
                    }
                }),
            )
            .route(
                "/missing-worker.js",
                get(|| async { axum::http::StatusCode::NOT_FOUND }),
            )
            .route(
                "/replacement",
                get(move || {
                    let replacement = Arc::clone(&replacement);
                    async move {
                        replacement.notify_one();
                        "replacement"
                    }
                }),
            )
            .route("/popup", get(|| async { "popup" }))
            .route(
                "/slow.js",
                get(move || {
                    let request = Arc::clone(&request);
                    let gate = Arc::clone(&gate);
                    async move {
                        request.notify_one();
                        gate.notified().await;
                        (
                            [("Content-Type", "text/javascript")],
                            "globalThis.slowDone = true;",
                        )
                    }
                }),
            );
        let (addr, server) = spawn_dedicated_fixture_server(router, "prepared-document-actions");
        Self {
            url: format!("http://{addr}/page"),
            requested,
            release,
            replacement_requested,
            arm_requested,
            arm_release,
            api_requests,
            _server: server,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.release.notify_one();
        self.arm_release.notify_one();
    }
}

async fn start_document(
    socket: &mut TestCdpSocket,
    session_id: &str,
    fixture: &Fixture,
    inline: &str,
) -> Vec<Value> {
    let observed = begin_document(socket, session_id, fixture, inline).await;
    tokio::time::timeout(Duration::from_secs(10), fixture.requested.notified())
        .await
        .unwrap();
    assert!(
        !observed
            .iter()
            .any(|message| message["method"] == "Page.domContentEventFired")
    );
    observed
}

async fn begin_document(
    socket: &mut TestCdpSocket,
    session_id: &str,
    fixture: &Fixture,
    inline: &str,
) -> Vec<Value> {
    let html = format!("<!doctype html><script>{inline}</script><script src=/slow.js></script>");
    begin_document_html(socket, session_id, fixture, &html).await
}

async fn begin_document_html(
    socket: &mut TestCdpSocket,
    session_id: &str,
    fixture: &Fixture,
    html: &str,
) -> Vec<Value> {
    send_cdp_command_without_wait(
        socket,
        20,
        "Page.navigate",
        Some(session_id),
        json!({"url": fixture.url}),
    )
    .await;
    let observed = recv_until_match(socket, |message| {
        message["method"] == "Fetch.requestPaused"
            && message["params"]["resourceType"] == "Document"
    })
    .await;
    let request = observed.last().unwrap()["params"]["requestId"].clone();
    use base64::Engine as _;
    send_cdp_command_without_wait(
        socket,
        21,
        "Fetch.fulfillRequest",
        Some(session_id),
        json!({
            "requestId": request, "responseCode": 200,
            "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
            "body": base64::engine::general_purpose::STANDARD.encode(html),
        }),
    )
    .await;
    observed
}

async fn configure(socket: &mut TestCdpSocket, session_id: &str, crashed: bool, response: bool) {
    for (id, method) in [
        (5, "Runtime.enable"),
        (6, "Page.enable"),
        (7, "Network.enable"),
    ] {
        send_cdp_command(socket, id, method, Some(session_id), json!({})).await;
    }
    if crashed {
        let result = send_cdp_command(socket, 8, "Page.crash", Some(session_id), json!({})).await;
        assert!(
            result
                .iter()
                .any(|message| message["id"] == 8 && message.get("result").is_some()),
            "{result:?}"
        );
    }
    send_cdp_command(socket, 9, "Fetch.enable", Some(session_id), json!({"patterns": [
        {"urlPattern": "*page", "resourceType": "Document", "requestStage": "Request"},
        {"urlPattern": "*api", "resourceType": "Fetch", "requestStage": if response { "Response" } else { "Request" }},
    ]})).await;
}

async fn verify_fetch(crashed: bool, response: bool, action: Action) {
    eprintln!("prepared Fetch {crashed}/{response}/{action:?}");
    let fixture = Fixture::new();
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    configure(&mut socket, &session.session_id, crashed, response).await;
    let mut observed = start_document(&mut socket, &session.session_id, &fixture,
        "fetch('/api').then(r=>r.text()).then(v=>console.log('api-settled:'+v),()=>console.log('api-settled:failed'));").await;
    observed.extend(
        recv_until_match(&mut socket, |message| {
            message["method"] == "Fetch.requestPaused"
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/api"))
        })
        .await,
    );
    let pause = observed.last().unwrap();
    assert_eq!(pause["params"]["resourceType"], "XHR");
    assert_eq!(
        pause["params"].get("responseStatusCode").is_some(),
        response
    );
    let request = pause["params"]["requestId"].clone();
    let method = match (action, response) {
        (Action::Continue, false) => "Fetch.continueRequest",
        (Action::Continue, true) => "Fetch.continueResponse",
        (Action::Fulfill, _) => "Fetch.fulfillRequest",
        (Action::Fail, _) => "Fetch.failRequest",
    };
    let mut params = json!({"requestId": request});
    match action {
        Action::Fulfill => {
            params["responseCode"] = json!(200);
            params["body"] = json!("ZnVsZmlsbGVkLWFwaQ==");
        }
        Action::Fail => params["errorReason"] = json!("Failed"),
        Action::Continue => {}
    }
    let reply = send_cdp_command(&mut socket, 22, method, Some(&session.session_id), params).await;
    assert!(
        reply
            .iter()
            .any(|message| message["id"] == 22 && message.get("result").is_some()),
        "{crashed}/{response}/{action:?}: {reply:?}"
    );
    observed.extend(reply);
    let expected = match action {
        Action::Continue => "api-settled:network-api",
        Action::Fulfill => "api-settled:fulfilled-api",
        Action::Fail => "api-settled:failed",
    };
    let settled = |message: &Value| {
        message["method"] == "Runtime.consoleAPICalled"
            && message["params"]["args"][0]["value"] == expected
    };
    if !observed.iter().any(settled) {
        observed.extend(recv_until_match(&mut socket, settled).await);
    }
    assert!(
        !observed
            .iter()
            .any(|message| message["method"] == "Page.domContentEventFired"),
        "{observed:?}"
    );
    fixture.release.notify_one();
    if !observed
        .iter()
        .any(|message| message["method"] == "Page.loadEventFired")
    {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == "Page.loadEventFired"
            })
            .await,
        );
    }
    let duplicate = send_cdp_command(
        &mut socket,
        23,
        method,
        Some(&session.session_id),
        json!({"requestId": request, "responseCode": 200, "errorReason": "Failed"}),
    )
    .await;
    assert!(
        duplicate
            .iter()
            .any(|message| message["id"] == 23 && message["error"]["message"] == "RequestNotFound"),
        "{duplicate:?}"
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_prepared_document_worker_is_discovered_attached_and_retired() {
    verify_worker(false).await;
}

#[tokio::test]
async fn websocket_cdp_prepared_document_worker_is_retired_when_navigation_is_replaced() {
    verify_worker(true).await;
}

async fn verify_worker(replace_before_adoption: bool) {
    let fixture = Fixture::new();
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    configure(&mut socket, &session.session_id, false, false).await;
    send_cdp_command(
        &mut socket,
        10,
        "Target.setDiscoverTargets",
        None,
        json!({"discover": true}),
    )
    .await;
    let policy = send_cdp_command(&mut socket, 11, "Target.setAutoAttach", Some(&session.session_id), json!({
        "autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true, "filter": [{"type": "worker"}],
    })).await;
    assert!(
        policy
            .iter()
            .any(|message| message["id"] == 11 && message.get("result").is_some()),
        "{policy:?}"
    );
    let crashed = send_cdp_command(
        &mut socket,
        12,
        "Page.crash",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    assert!(
        crashed
            .iter()
            .any(|message| message["id"] == 12 && message.get("result").is_some()),
        "{crashed:?}"
    );
    let mut observed = start_document(
        &mut socket,
        &session.session_id,
        &fixture,
        "window.w=new Worker(URL.createObjectURL(new Blob(['setInterval(()=>{},1000)'])));",
    )
    .await;
    let attached = |message: &Value| {
        message["method"] == "Target.attachedToTarget"
            && message["params"]["targetInfo"]["type"] == "worker"
    };
    if !observed.iter().any(attached) {
        observed.extend(recv_until_match(&mut socket, attached).await);
    }
    let worker = &observed.iter().find(|message| attached(message)).unwrap()["params"];
    let worker_id = worker["targetInfo"]["targetId"].clone();
    let worker_session = worker["sessionId"].as_str().unwrap().to_owned();
    assert!(
        observed
            .iter()
            .any(|message| message["method"] == "Target.targetCreated"
                && message["params"]["targetInfo"]["targetId"] == worker_id),
        "{observed:?}"
    );
    assert!(
        !observed
            .iter()
            .any(|message| message["method"] == "Page.domContentEventFired")
    );
    let targets = send_cdp_command(&mut socket, 22, "Target.getTargets", None, json!({})).await;
    assert!(
        targets.last().unwrap()["result"]["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["targetId"] == worker_id),
        "{targets:?}"
    );
    let enabled = send_cdp_command(
        &mut socket,
        23,
        "Runtime.enable",
        Some(&worker_session),
        json!({}),
    )
    .await;
    assert!(
        enabled
            .iter()
            .any(|message| message["id"] == 23 && message.get("result").is_some()),
        "{enabled:?}"
    );
    if !replace_before_adoption {
        fixture.release.notify_one();
        recv_until_match(&mut socket, |message| {
            message["method"] == "Page.loadEventFired"
        })
        .await;
    }
    let replacement = fixture.url.replace("/page", "/replacement");
    let mut navigation = send_cdp_command(
        &mut socket,
        24,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": replacement}),
    )
    .await;
    let result = &navigation.last().unwrap()["result"];
    assert!(
        result.get("frameId").is_some() && result.get("errorText").is_none(),
        "{navigation:?}"
    );
    let destroyed = |message: &Value| {
        message["method"] == "Target.targetDestroyed" && message["params"]["targetId"] == worker_id
    };
    let loaded = |message: &Value| {
        message["method"] == "Page.loadEventFired" && message["sessionId"] == session.session_id
    };
    // The held request's native timeout is thirty seconds. Retirement must
    // finish independently of that timeout and before the fixture is released.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !navigation.iter().any(destroyed) || !navigation.iter().any(loaded) {
            navigation.extend(
                recv_until_match(&mut socket, |message| destroyed(message) || loaded(message))
                    .await,
            );
        }
    })
    .await
    .expect("replacement must load and retire the original Worker while its parser is held");
    let targets = send_cdp_command(&mut socket, 25, "Target.getTargets", None, json!({})).await;
    assert!(
        !targets.last().unwrap()["result"]["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["targetId"] == worker_id)
    );
    let native = send_cdp_command(
        &mut socket,
        26,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({"expression": "location.href", "returnByValue": true}),
    )
    .await;
    assert_eq!(
        native.last().unwrap()["result"]["result"]["value"],
        replacement
    );
    // Retirement was observed while the old parser was held; release its
    // network fixture before waiting for whole-server teardown.
    fixture.release.notify_one();
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_prepared_document_dialog_can_be_answered_before_adoption() {
    let fixture = Fixture::new();
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    configure(&mut socket, &session.session_id, true, false).await;
    send_cdp_command_without_wait(
        &mut socket,
        20,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": fixture.url}),
    )
    .await;
    let paused = recv_until_match(&mut socket, |message| {
        message["method"] == "Fetch.requestPaused"
            && message["params"]["resourceType"] == "Document"
    })
    .await;
    use base64::Engine as _;
    send_cdp_command_without_wait(&mut socket, 21, "Fetch.fulfillRequest", Some(&session.session_id), json!({
        "requestId": paused.last().unwrap()["params"]["requestId"], "responseCode": 200,
        "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
        "body": base64::engine::general_purpose::STANDARD.encode("<script>alert('prepared-dialog')</script><script src=/slow.js></script>"),
    })).await;
    let opened = recv_until_match(&mut socket, |message| {
        message["method"] == "Page.javascriptDialogOpening"
    })
    .await;
    assert_eq!(
        opened.last().unwrap()["params"]["message"],
        "prepared-dialog"
    );
    let answered = send_cdp_command(
        &mut socket,
        22,
        "Page.handleJavaScriptDialog",
        Some(&session.session_id),
        json!({"accept": true}),
    )
    .await;
    assert!(
        answered
            .iter()
            .any(|message| message["id"] == 22 && message.get("result").is_some()),
        "{answered:?}"
    );
    tokio::time::timeout(Duration::from_secs(10), fixture.requested.notified())
        .await
        .unwrap();
    fixture.release.notify_one();
    recv_until_match(&mut socket, |message| {
        message["method"] == "Page.loadEventFired"
    })
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_prepared_document_popup_keeps_accepted_target_creation() {
    let fixture = Fixture::new();
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    configure(&mut socket, &session.session_id, true, false).await;
    send_cdp_command(
        &mut socket,
        10,
        "Target.setDiscoverTargets",
        None,
        json!({"discover": true}),
    )
    .await;
    let mut observed = start_document(
        &mut socket,
        &session.session_id,
        &fixture,
        "window.w=window.open('/popup');",
    )
    .await;
    let created = |message: &Value| {
        message["method"] == "Target.targetCreated"
            && message["params"]["targetInfo"]["openerId"] == session.target_id
    };
    if !observed.iter().any(created) {
        observed.extend(recv_until_match(&mut socket, created).await);
    }
    let popup_id = observed.iter().find(|message| created(message)).unwrap()["params"]["targetInfo"]["targetId"].clone();
    assert!(
        !observed
            .iter()
            .any(|message| message["method"] == "Page.domContentEventFired")
    );
    fixture.release.notify_one();
    recv_until_match(&mut socket, |message| {
        message["method"] == "Page.loadEventFired" && message["sessionId"] == session.session_id
    })
    .await;
    let targets = send_cdp_command(&mut socket, 22, "Target.getTargets", None, json!({})).await;
    assert!(
        targets.last().unwrap()["result"]["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["targetId"] == popup_id),
        "{targets:?}"
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_prepared_document_fetch_is_revoked_by_replacement_disable_and_detach() {
    for crashed in [false, true] {
        for cancellation in ["replace", "disable", "detach"] {
            let fixture = Fixture::new();
            let (addr, server) = spawn_test_protocol_server().await;
            let (mut socket, _) =
                connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                    .await
                    .unwrap();
            let context = cdp_create_browser_context(&mut socket, 1).await;
            let session = cdp_create_attached_target(&mut socket, 2, &context).await;
            configure(&mut socket, &session.session_id, crashed, false).await;
            let mut observed = start_document(
                &mut socket,
                &session.session_id,
                &fixture,
                "fetch('/api').then(r=>r.text()).catch(()=>{});",
            )
            .await;
            observed.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Fetch.requestPaused"
                        && message["params"]["request"]["url"]
                            .as_str()
                            .is_some_and(|url| url.ends_with("/api"))
                })
                .await,
            );
            let request = observed.last().unwrap()["params"]["requestId"].clone();
            let (method, session_id, params) = match cancellation {
                "replace" => (
                    "Page.navigate",
                    Some(session.session_id.as_str()),
                    json!({"url": fixture.url.replace("/page", "/replacement")}),
                ),
                "disable" => (
                    "Fetch.disable",
                    Some(session.session_id.as_str()),
                    json!({}),
                ),
                "detach" => (
                    "Target.detachFromTarget",
                    None,
                    json!({"sessionId": session.session_id}),
                ),
                _ => unreachable!(),
            };
            let canceled = send_cdp_command(&mut socket, 22, method, session_id, params).await;
            assert!(
                canceled
                    .iter()
                    .any(|message| message["id"] == 22 && message.get("result").is_some()),
                "{crashed}/{cancellation}: {canceled:?}"
            );
            let stale = send_cdp_command(
                &mut socket,
                23,
                "Fetch.fulfillRequest",
                Some(&session.session_id),
                json!({"requestId": request, "responseCode": 200}),
            )
            .await;
            assert!(
                stale
                    .iter()
                    .any(|message| message["id"] == 23 && message.get("error").is_some()),
                "{crashed}/{cancellation}: {stale:?}"
            );
            fixture.release.notify_one();
            let alive =
                send_cdp_command(&mut socket, 24, "Target.getTargets", None, json!({})).await;
            assert!(
                alive
                    .iter()
                    .any(|message| message["id"] == 24 && message.get("result").is_some())
            );
            let live_session = if cancellation == "detach" {
                let attached = send_cdp_command(
                    &mut socket,
                    25,
                    "Target.attachToTarget",
                    None,
                    json!({"targetId": session.target_id, "flatten": true}),
                )
                .await;
                attached.last().unwrap()["result"]["sessionId"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            } else {
                session.session_id.clone()
            };
            for (id, method, params) in [
                (28, "Page.enable", json!({})),
                (
                    29,
                    "Page.setLifecycleEventsEnabled",
                    json!({"enabled": true}),
                ),
            ] {
                let enabled =
                    send_cdp_command(&mut socket, id, method, Some(&live_session), params).await;
                assert!(
                    enabled.last().unwrap().get("result").is_some(),
                    "{enabled:?}"
                );
            }
            let replacement = fixture.url.replace("/page", "/replacement");
            let recovered = send_cdp_command(
                &mut socket,
                26,
                "Page.navigate",
                Some(&live_session),
                json!({"url": replacement}),
            )
            .await;
            let result = &recovered.last().unwrap()["result"];
            assert!(
                result.get("frameId").is_some() && result.get("errorText").is_none(),
                "{crashed}/{cancellation}: {recovered:?}"
            );
            // An ACK may precede Page adoption. A detached session also loses
            // its Page subscription, so subscribe the replacement binding and
            // wait for this navigation's exact load before querying its realm.
            let loaded = |message: &Value| {
                message["method"] == "Page.lifecycleEvent"
                    && message["sessionId"] == live_session
                    && message["params"]["name"] == "load"
                    && message["params"]["frameId"] == result["frameId"]
                    && message["params"]["loaderId"] == result["loaderId"]
            };
            if !recovered.iter().any(loaded) {
                recv_until_match(&mut socket, loaded).await;
            }
            let native = send_cdp_command(
                &mut socket,
                27,
                "Runtime.evaluate",
                Some(&live_session),
                json!({"expression": "location.href", "returnByValue": true}),
            )
            .await;
            assert_eq!(
                native.last().unwrap()["result"]["result"]["value"],
                replacement,
                "{crashed}/{cancellation}: {native:?}"
            );
            let _ = socket.close(None).await;
            abort_test_cdp_server(server).await;
        }
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_history_updates_survive_adoption_without_duplicate_entries()
 {
    for crashed in [false, true] {
        for action in ["replaceState", "pushState"] {
            let fixture = Fixture::new();
            let (addr, server) = spawn_test_protocol_server().await;
            let (mut socket, _) =
                connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                    .await
                    .unwrap();
            let context = cdp_create_browser_context(&mut socket, 1).await;
            let session = cdp_create_attached_target(&mut socket, 2, &context).await;
            configure(&mut socket, &session.session_id, false, false).await;
            let previous_url = "data:text/html,<title>previous title</title>";
            let initial = send_cdp_command(
                &mut socket,
                13,
                "Page.navigate",
                Some(&session.session_id),
                json!({"url": previous_url}),
            )
            .await;
            if !initial
                .iter()
                .any(|message| message["method"] == "Page.loadEventFired")
            {
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Page.loadEventFired"
                })
                .await;
            }
            if crashed {
                let result = send_cdp_command(
                    &mut socket,
                    14,
                    "Page.crash",
                    Some(&session.session_id),
                    json!({}),
                )
                .await;
                assert!(result.last().unwrap().get("result").is_some(), "{result:?}");
            }
            let changed = fixture.url.replace("/page", "/changed");
            let mut observed = start_document(
                &mut socket,
                &session.session_id,
                &fixture,
                &format!("history.{action}(null,'','/changed');"),
            )
            .await;
            let changed_event = |message: &Value| {
                message["method"] == "Page.navigatedWithinDocument"
                    && message["params"]["url"] == changed
            };
            if !observed.iter().any(changed_event) {
                observed.extend(recv_until_match(&mut socket, changed_event).await);
            }
            assert!(
                !observed
                    .iter()
                    .any(|message| message["method"] == "Page.domContentEventFired"),
                "{observed:?}"
            );
            let targets =
                send_cdp_command(&mut socket, 22, "Target.getTargets", None, json!({})).await;
            assert!(
                targets.last().unwrap()["result"]["targetInfos"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|target| target["targetId"] == session.target_id
                        && target["url"] == changed),
                "{targets:?}"
            );
            let before = send_cdp_command(
                &mut socket,
                23,
                "Page.getNavigationHistory",
                Some(&session.session_id),
                json!({}),
            )
            .await;
            let before = &before.last().unwrap()["result"];
            let entries = before["entries"].as_array().unwrap();
            assert_eq!(entries.last().unwrap()["url"], changed, "{before:?}");
            assert_eq!(entries.last().unwrap()["title"], "", "{before:?}");
            if !crashed {
                assert_eq!(
                    entries
                        .iter()
                        .find(|entry| entry["url"] == previous_url)
                        .expect("ordinary navigation retains the preceding entry")["title"],
                    "previous title",
                    "{before:?}"
                );
            }
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry["url"] == fixture.url)
                    .count(),
                usize::from(action == "pushState"),
                "{before:?}"
            );
            fixture.release.notify_one();
            recv_until_match(&mut socket, |message| {
                message["method"] == "Page.loadEventFired"
            })
            .await;
            let after = send_cdp_command(
                &mut socket,
                24,
                "Page.getNavigationHistory",
                Some(&session.session_id),
                json!({}),
            )
            .await;
            assert_eq!(
                &after.last().unwrap()["result"],
                before,
                "{crashed}/{action}"
            );
            let native = send_cdp_command(
                &mut socket,
                25,
                "Runtime.evaluate",
                Some(&session.session_id),
                json!({"expression": "location.href", "returnByValue": true}),
            )
            .await;
            assert_eq!(native.last().unwrap()["result"]["result"]["value"], changed);
            let _ = socket.close(None).await;
            abort_test_cdp_server(server).await;
        }
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_script_navigation_replaces_pending_adoption() {
    for crashed in [false, true] {
        for action in ["assign", "replace"] {
            let fixture = Fixture::new();
            let (addr, server) = spawn_test_protocol_server().await;
            let (mut socket, _) =
                connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                    .await
                    .unwrap();
            let context = cdp_create_browser_context(&mut socket, 1).await;
            let session = cdp_create_attached_target(&mut socket, 2, &context).await;
            configure(&mut socket, &session.session_id, crashed, false).await;
            let mut observed = begin_document(
                &mut socket,
                &session.session_id,
                &fixture,
                &format!("location.{action}('/replacement');"),
            )
            .await;
            // The replacement must start while the original parser is still blocked.
            tokio::time::timeout(
                Duration::from_secs(10),
                fixture.replacement_requested.notified(),
            )
            .await
            .unwrap();
            let replacement = fixture.url.replace("/page", "/replacement");
            let loaded = |message: &Value| {
                message["method"] == "Page.loadEventFired"
                    && message["sessionId"] == session.session_id
            };
            if !observed.iter().any(loaded) {
                observed.extend(recv_until_match(&mut socket, loaded).await);
            }
            let history = send_cdp_command(
                &mut socket,
                22,
                "Page.getNavigationHistory",
                Some(&session.session_id),
                json!({}),
            )
            .await;
            observed.extend(history.clone());
            let entries = history.last().unwrap()["result"]["entries"]
                .as_array()
                .unwrap();
            assert_eq!(
                entries.last().unwrap()["url"],
                replacement,
                "{crashed}/{action}: {history:?}"
            );
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry["url"] == fixture.url)
                    .count(),
                usize::from(action == "assign"),
                "{history:?}"
            );
            for id in [20, 21] {
                if !observed.iter().any(|message| message["id"] == id) {
                    observed
                        .extend(recv_until_match(&mut socket, |message| message["id"] == id).await);
                }
                assert_eq!(
                    observed
                        .iter()
                        .filter(|message| message["id"] == id)
                        .count(),
                    1,
                    "{observed:?}"
                );
            }
            fixture.release.notify_one();
            let native = send_cdp_command(
                &mut socket,
                23,
                "Runtime.evaluate",
                Some(&session.session_id),
                json!({"expression": "location.href", "returnByValue": true}),
            )
            .await;
            assert_eq!(
                native.last().unwrap()["result"]["result"]["value"],
                replacement
            );
            let _ = socket.close(None).await;
            abort_test_cdp_server(server).await;
        }
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_file_chooser_preserves_source_node() {
    for crashed in [false, true] {
        let fixture = Fixture::new();
        let (addr, server) = spawn_test_protocol_server().await;
        let (mut socket, _) =
            connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                .await
                .unwrap();
        let context = cdp_create_browser_context(&mut socket, 1).await;
        let session = cdp_create_attached_target(&mut socket, 2, &context).await;
        configure(&mut socket, &session.session_id, false, false).await;
        let enabled = send_cdp_command(
            &mut socket,
            10,
            "Page.setInterceptFileChooserDialog",
            Some(&session.session_id),
            json!({"enabled": true}),
        )
        .await;
        assert!(
            enabled.last().unwrap().get("result").is_some(),
            "{enabled:?}"
        );
        if crashed {
            send_cdp_command(
                &mut socket,
                11,
                "Page.crash",
                Some(&session.session_id),
                json!({}),
            )
            .await;
        }
        let mut observed = start_document(&mut socket, &session.session_id, &fixture,
            "var f=document.createElement('input');f.type='file';document.documentElement.appendChild(f);f.click();").await;
        let opened = |message: &Value| message["method"] == "Page.fileChooserOpened";
        if !observed.iter().any(opened) {
            observed.extend(recv_until_match(&mut socket, opened).await);
        }
        assert!(
            !observed
                .iter()
                .any(|message| message["method"] == "Page.domContentEventFired"),
            "{observed:?}"
        );
        let chooser = observed.iter().find(|message| opened(message)).unwrap();
        let backend = chooser["params"]["backendNodeId"].clone();
        assert!(backend.as_u64().is_some_and(|id| id > 0), "{chooser:?}");
        fixture.release.notify_one();
        recv_until_match(&mut socket, |message| {
            message["method"] == "Page.loadEventFired"
        })
        .await;
        let node = send_cdp_command(
            &mut socket,
            22,
            "DOM.describeNode",
            Some(&session.session_id),
            json!({"backendNodeId": backend}),
        )
        .await;
        assert_eq!(
            node.last().unwrap()["result"]["node"]["nodeName"],
            "INPUT",
            "{node:?}"
        );
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_popup_navigation_and_close_keep_their_source() {
    for close in [false, true] {
        let fixture = Fixture::new();
        let (addr, server) = spawn_test_protocol_server().await;
        let (mut socket, _) =
            connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                .await
                .unwrap();
        let context = cdp_create_browser_context(&mut socket, 1).await;
        let session = cdp_create_attached_target(&mut socket, 2, &context).await;
        configure(&mut socket, &session.session_id, true, false).await;
        send_cdp_command(
            &mut socket,
            10,
            "Target.setDiscoverTargets",
            None,
            json!({"discover": true}),
        )
        .await;
        let mut observed = start_document(
            &mut socket,
            &session.session_id,
            &fixture,
        if close {
                "var w=window.open('/popup');fetch('/api').then(()=>w.close());"
            } else {
                "var w=window.open('/popup');fetch('/api').then(()=>{w.location.href='/replacement'});"
            },
        )
        .await;
        let created = |message: &Value| {
            message["method"] == "Target.targetCreated"
                && message["params"]["targetInfo"]["openerId"] == session.target_id
        };
        if !observed.iter().any(created) {
            observed.extend(recv_until_match(&mut socket, created).await);
        }
        let popup = observed.iter().find(|message| created(message)).unwrap()["params"]["targetInfo"]["targetId"].clone();
        let paused = |message: &Value| {
            message["method"] == "Fetch.requestPaused"
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/api"))
        };
        if !observed.iter().any(paused) {
            observed.extend(recv_until_match(&mut socket, paused).await);
        }
        let request =
            observed.iter().find(|message| paused(message)).unwrap()["params"]["requestId"].clone();
        observed.extend(
            send_cdp_command(
                &mut socket,
                23,
                "Fetch.fulfillRequest",
                Some(&session.session_id),
                json!({"requestId": request, "responseCode": 200}),
            )
            .await,
        );
        if close {
            let destroyed = |message: &Value| {
                message["method"] == "Target.targetDestroyed"
                    && message["params"]["targetId"] == popup
            };
            if !observed.iter().any(destroyed) {
                observed.extend(recv_until_match(&mut socket, destroyed).await);
            }
        } else {
            tokio::time::timeout(
                Duration::from_secs(10),
                fixture.replacement_requested.notified(),
            )
            .await
            .unwrap();
            let replacement = fixture.url.replace("/page", "/replacement");
            let navigated = |message: &Value| {
                message["method"] == "Target.targetInfoChanged"
                    && message["params"]["targetInfo"]["targetId"] == popup
                    && message["params"]["targetInfo"]["url"] == replacement
            };
            if !observed.iter().any(navigated) {
                observed.extend(recv_until_match(&mut socket, navigated).await);
            }
        }
        assert!(
            !observed
                .iter()
                .any(|message| message["method"] == "Page.domContentEventFired"
                    && message["sessionId"] == session.session_id),
            "{observed:?}"
        );
        fixture.release.notify_one();
        recv_until_match(&mut socket, |message| {
            message["method"] == "Page.loadEventFired" && message["sessionId"] == session.session_id
        })
        .await;
        let targets = send_cdp_command(&mut socket, 22, "Target.getTargets", None, json!({})).await;
        assert_eq!(
            targets.last().unwrap()["result"]["targetInfos"]
                .as_array()
                .unwrap()
                .iter()
                .any(|target| target["targetId"] == popup),
            !close,
            "{targets:?}"
        );
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_late_fetch_enable_applies_before_its_ack() {
    for crashed in [false, true] {
        verify_late_network_policy(crashed, true).await;
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_late_blocked_urls_apply_before_its_ack() {
    for crashed in [false, true] {
        verify_late_network_policy(crashed, false).await;
    }
}

async fn verify_late_network_policy(crashed: bool, fetch: bool) {
    let fixture = Fixture::new();
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    configure(&mut socket, &session.session_id, crashed, false).await;
    send_cdp_command(
        &mut socket,
        10,
        "Fetch.enable",
        Some(&session.session_id),
        json!({"patterns": [
            {"urlPattern": "*/page", "resourceType": "Document"}
        ]}),
    )
    .await;
    let mut observed = start_document(&mut socket, &session.session_id, &fixture,
        "fetch('/arm').then(()=>fetch('/api')).then(r=>r.text()).then(v=>console.log('policy:'+v),()=>console.log('policy:blocked'));"
    ).await;
    tokio::time::timeout(Duration::from_secs(10), fixture.arm_requested.notified())
        .await
        .unwrap();
    let (method, params, expected) = if fetch {
        (
            "Fetch.enable",
            json!({"patterns": [
                {"urlPattern": "*/page", "resourceType": "Document"},
                {"urlPattern": "*/api", "resourceType": "Fetch"}
            ]}),
            "policy:intercepted",
        )
    } else {
        (
            "Network.setBlockedURLs",
            json!({"urls": ["*/api"]}),
            "policy:blocked",
        )
    };
    let policy = send_cdp_command(&mut socket, 22, method, Some(&session.session_id), params).await;
    assert!(
        policy.last().unwrap().get("result").is_some(),
        "{crashed}/{method}: {policy:?}"
    );
    observed.extend(policy);
    fixture.arm_release.notify_one();
    if fetch {
        let paused = recv_until_match(&mut socket, |message| {
            message["method"] == "Fetch.requestPaused"
                && message["sessionId"] == session.session_id
                && message["params"]["request"]["url"] == fixture.url.replace("/page", "/api")
        })
        .await;
        let request = paused.last().unwrap()["params"]["requestId"].clone();
        observed.extend(paused);
        use base64::Engine as _;
        observed.extend(
            send_cdp_command(
                &mut socket,
                23,
                "Fetch.fulfillRequest",
                Some(&session.session_id),
                json!({
                    "requestId": request, "responseCode": 200,
                    "body": base64::engine::general_purpose::STANDARD.encode("intercepted")
                }),
            )
            .await,
        );
    }
    let settled = |message: &Value| {
        message["method"] == "Runtime.consoleAPICalled"
            && message["sessionId"] == session.session_id
            && message["params"]["args"][0]["value"] == expected
    };
    if !observed.iter().any(settled) {
        observed.extend(recv_until_match(&mut socket, settled).await);
    }
    assert_eq!(
        fixture
            .api_requests
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "{crashed}/{method}: policy ACK must precede an effective native update"
    );
    assert!(
        !observed
            .iter()
            .any(|message| message["method"] == "Page.domContentEventFired"
                && message["sessionId"] == session.session_id),
        "the policy must work before Page adoption: {observed:?}"
    );
    fixture.release.notify_one();
    recv_until_match(&mut socket, |message| {
        message["method"] == "Page.loadEventFired" && message["sessionId"] == session.session_id
    })
    .await;
    let native = send_cdp_command(
        &mut socket,
        24,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({"expression": "location.href", "returnByValue": true}),
    )
    .await;
    assert_eq!(
        native.last().unwrap()["result"]["result"]["value"],
        fixture.url
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_prepared_document_open_does_not_append_history_on_adoption() {
    for crashed in [false, true] {
        let fixture = Fixture::new();
        let (addr, server) = spawn_test_protocol_server().await;
        let (mut socket, _) =
            connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                .await
                .unwrap();
        let context = cdp_create_browser_context(&mut socket, 1).await;
        let session = cdp_create_attached_target(&mut socket, 2, &context).await;
        configure(&mut socket, &session.session_id, crashed, false).await;
        send_cdp_command(
            &mut socket,
            11,
            "Fetch.enable",
            Some(&session.session_id),
            json!({
                "patterns": [{"urlPattern": "*/page", "resourceType": "Document"}]
            }),
        )
        .await;
        send_cdp_command(
            &mut socket,
            10,
            "Target.setDiscoverTargets",
            None,
            json!({"discover": true}),
        )
        .await;
        // The asynchronous callback runs during a defer wait, outside parser
        // script execution where document.open() would otherwise be ignored.
        let html = "<!doctype html><title>before-open</title><script>fetch('/arm').then(()=>{document.open();document.write('<title>after-open</title><p>replacement</p>');document.close();});</script><script defer src=/slow.js></script>";
        let mut observed =
            begin_document_html(&mut socket, &session.session_id, &fixture, html).await;
        tokio::time::timeout(Duration::from_secs(10), fixture.requested.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), fixture.arm_requested.notified())
            .await
            .unwrap();
        let before_title = |message: &Value| {
            message["method"] == "Target.targetInfoChanged"
                && message["params"]["targetInfo"]["targetId"] == session.target_id
                && message["params"]["targetInfo"]["title"] == "before-open"
        };
        if !observed.iter().any(before_title) {
            observed.extend(recv_until_match(&mut socket, before_title).await);
        }
        observed.extend(
            send_cdp_command(
                &mut socket,
                22,
                "Page.getNavigationHistory",
                Some(&session.session_id),
                json!({}),
            )
            .await,
        );
        let before = observed.iter().find(|message| message["id"] == 22).unwrap()["result"].clone();
        let index = before["currentIndex"].as_u64().unwrap() as usize;
        assert_eq!(before["entries"][index]["title"], "before-open");
        assert_eq!(before["entries"][index]["url"], fixture.url);
        assert!(
            !observed
                .iter()
                .any(|message| message["method"] == "Page.domContentEventFired"
                    && message["sessionId"] == session.session_id),
            "the native history must already be visible before adoption"
        );
        fixture.arm_release.notify_one();
        let loaded = |message: &Value| {
            message["method"] == "Page.loadEventFired" && message["sessionId"] == session.session_id
        };
        if !observed.iter().any(loaded) {
            observed.extend(recv_until_match(&mut socket, loaded).await);
        }
        for id in [20, 21] {
            if !observed.iter().any(|message| message["id"] == id) {
                observed.extend(recv_until_match(&mut socket, |message| message["id"] == id).await);
            }
            let reply = observed.iter().find(|message| message["id"] == id).unwrap();
            assert!(
                reply.get("result").is_some() && reply["result"].get("errorText").is_none(),
                "{reply:?}"
            );
        }
        let adopted = send_cdp_command(
            &mut socket,
            23,
            "Page.getNavigationHistory",
            Some(&session.session_id),
            json!({}),
        )
        .await;
        let adopted = &adopted.last().unwrap()["result"];
        assert_eq!(
            adopted["entries"].as_array().unwrap().len(),
            before["entries"].as_array().unwrap().len(),
            "{crashed}: adoption must not append history: {adopted:?}"
        );
        assert_eq!(adopted["currentIndex"], before["currentIndex"]);
        assert_eq!(adopted["entries"][index]["title"], "after-open");
        let native = send_cdp_command(&mut socket, 24, "Runtime.evaluate", Some(&session.session_id), json!({"expression": "[document.title,document.body.textContent]", "returnByValue": true})).await;
        assert_eq!(
            native.last().unwrap()["result"]["result"]["value"],
            json!(["after-open", "replacement"])
        );
        fixture.release.notify_one();
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_failed_debugger_worker_retires_when_source_is_revoked() {
    for crashed in [false, true] {
        let fixture = Fixture::new();
        let (addr, server) = spawn_test_protocol_server().await;
        let (mut socket, _) =
            connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                .await
                .unwrap();
        let context = cdp_create_browser_context(&mut socket, 1).await;
        let session = cdp_create_attached_target(&mut socket, 2, &context).await;
        configure(&mut socket, &session.session_id, crashed, false).await;
        send_cdp_command(
            &mut socket,
            10,
            "Target.setDiscoverTargets",
            None,
            json!({"discover": true}),
        )
        .await;
        send_cdp_command(&mut socket, 11, "Target.setAutoAttach", Some(&session.session_id), json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true, "filter": [{"type": "worker"}]})).await;
        let mut observed = start_document(&mut socket, &session.session_id, &fixture,
            "window.w=new Worker('/missing-worker.js');window.w.onerror=()=>console.log('worker-failed');"
        ).await;
        let attached = |message: &Value| {
            message["method"] == "Target.attachedToTarget"
                && message["params"]["targetInfo"]["type"] == "worker"
        };
        if !observed.iter().any(attached) {
            observed.extend(recv_until_match(&mut socket, attached).await);
        }
        let worker = &observed.iter().find(|message| attached(message)).unwrap()["params"];
        assert_eq!(worker["waitingForDebugger"], true);
        let worker_id = worker["targetInfo"]["targetId"].clone();
        let worker_session = worker["sessionId"].as_str().unwrap().to_owned();
        observed.extend(
            send_cdp_command(
                &mut socket,
                22,
                "Runtime.enable",
                Some(&worker_session),
                json!({}),
            )
            .await,
        );
        let failed = |message: &Value| {
            message["method"] == "Runtime.consoleAPICalled"
                && message["sessionId"] == session.session_id
                && message["params"]["args"][0]["value"] == "worker-failed"
        };
        if !observed.iter().any(failed) {
            observed.extend(recv_until_match(&mut socket, failed).await);
        }
        let targets = send_cdp_command(&mut socket, 23, "Target.getTargets", None, json!({})).await;
        assert!(
            targets.last().unwrap()["result"]["targetInfos"]
                .as_array()
                .unwrap()
                .iter()
                .any(|target| target["targetId"] == worker_id)
        );
        observed.extend(targets);
        assert!(
            !observed
                .iter()
                .any(|message| message["method"] == "Page.domContentEventFired"
                    && message["sessionId"] == session.session_id)
        );
        // Keep the replacement's headers held too: revoking the original
        // prepared source must retire the failed Worker without a new commit.
        send_cdp_command_without_wait(
            &mut socket,
            24,
            "Page.navigate",
            Some(&session.session_id),
            json!({"url": fixture.url.replace("/page", "/arm")}),
        )
        .await;
        tokio::time::timeout(Duration::from_secs(10), fixture.arm_requested.notified())
            .await
            .unwrap();
        let destroyed = |message: &Value| {
            message["method"] == "Target.targetDestroyed"
                && message["params"]["targetId"] == worker_id
        };
        let detached = |message: &Value| {
            message["method"] == "Target.detachedFromTarget"
                && message["params"]["sessionId"] == worker_session
        };
        let mut retired = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !retired.iter().any(destroyed) || !retired.iter().any(detached) {
                retired.extend(
                    recv_until_match(&mut socket, |message| {
                        destroyed(message) || detached(message)
                    })
                    .await,
                );
            }
        })
        .await
        .expect("an unavailable Worker still has protocol retirement responsibility");
        let targets = send_cdp_command(&mut socket, 25, "Target.getTargets", None, json!({})).await;
        assert!(
            !targets.last().unwrap()["result"]["targetInfos"]
                .as_array()
                .unwrap()
                .iter()
                .any(|target| target["targetId"] == worker_id)
        );
        let closed = send_cdp_command(
            &mut socket,
            26,
            "Runtime.evaluate",
            Some(&worker_session),
            json!({"expression": "1"}),
        )
        .await;
        assert!(closed.last().unwrap().get("error").is_some(), "{closed:?}");
        fixture.arm_release.notify_one();
        retired.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == "Page.loadEventFired"
                    && message["sessionId"] == session.session_id
            })
            .await,
        );
        fixture.release.notify_one();
        retired
            .extend(send_cdp_command(&mut socket, 27, "Browser.getVersion", None, json!({})).await);
        assert_eq!(
            retired.iter().filter(|message| destroyed(message)).count(),
            1,
            "{retired:?}"
        );
        assert_eq!(
            retired.iter().filter(|message| detached(message)).count(),
            1,
            "{retired:?}"
        );
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
    }
}

#[tokio::test]
async fn websocket_cdp_prepared_document_title_precedes_later_history_push_and_adoption() {
    for crashed in [false, true] {
        let fixture = Fixture::new();
        let (addr, server) = spawn_test_protocol_server().await;
        let (mut socket, _) =
            connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                .await
                .unwrap();
        let context = cdp_create_browser_context(&mut socket, 1).await;
        let session = cdp_create_attached_target(&mut socket, 2, &context).await;
        configure(&mut socket, &session.session_id, crashed, false).await;
        send_cdp_command(
            &mut socket,
            10,
            "Target.setDiscoverTargets",
            None,
            json!({"discover": true}),
        )
        .await;
        let mut observed = start_document(&mut socket, &session.session_id, &fixture,
            "document.title='prepared title';console.log('title-ready');fetch('/arm').then(()=>{history.pushState(null,'','/changed');console.log('history-ready');});"
        ).await;
        tokio::time::timeout(Duration::from_secs(10), fixture.arm_requested.notified())
            .await
            .unwrap();
        let title_ready = |message: &Value| {
            message["method"] == "Target.targetInfoChanged"
                && message["params"]["targetInfo"]["targetId"] == session.target_id
                && message["params"]["targetInfo"]["title"] == "prepared title"
        };
        if !observed.iter().any(title_ready) {
            observed.extend(recv_until_match(&mut socket, title_ready).await);
        }
        let before_push = send_cdp_command(
            &mut socket,
            22,
            "Page.getNavigationHistory",
            Some(&session.session_id),
            json!({}),
        )
        .await;
        let before_push = &before_push.last().unwrap()["result"];
        let initial = before_push["entries"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(initial["url"], fixture.url, "{before_push:?}");
        assert_eq!(initial["title"], "prepared title", "{before_push:?}");
        // A separate native turn can only perform pushState after the first
        // title publication was processed and the initial entry was checked.
        fixture.arm_release.notify_one();
        let pushed = |message: &Value| {
            message["method"] == "Page.navigatedWithinDocument"
                && message["sessionId"] == session.session_id
                && message["params"]["url"] == fixture.url.replace("/page", "/changed")
        };
        observed.extend(recv_until_match(&mut socket, pushed).await);
        let after_push = send_cdp_command(
            &mut socket,
            23,
            "Page.getNavigationHistory",
            Some(&session.session_id),
            json!({}),
        )
        .await;
        let after_push = &after_push.last().unwrap()["result"];
        let entries = after_push["entries"].as_array().unwrap();
        assert_eq!(entries[entries.len() - 2], initial);
        assert_eq!(
            entries.last().unwrap()["url"],
            fixture.url.replace("/page", "/changed")
        );
        assert_eq!(entries.last().unwrap()["title"], "prepared title");
        assert!(
            !observed
                .iter()
                .any(|message| message["method"] == "Page.domContentEventFired"
                    && message["sessionId"] == session.session_id)
        );
        fixture.release.notify_one();
        recv_until_match(&mut socket, |message| {
            message["method"] == "Page.loadEventFired" && message["sessionId"] == session.session_id
        })
        .await;
        let adopted = send_cdp_command(
            &mut socket,
            24,
            "Page.getNavigationHistory",
            Some(&session.session_id),
            json!({}),
        )
        .await;
        assert_eq!(
            &adopted.last().unwrap()["result"],
            after_push,
            "{crashed}: adopting the Page must preserve every published entry"
        );
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
    }
}
