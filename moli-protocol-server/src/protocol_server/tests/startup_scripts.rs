use super::*;

type TestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const PROBE: &str = r#"JSON.stringify([window, frames[0]].map(w => [
    w.__startupRuns, w.__pageStartupRuns, w.__scriptOrder
]))"#;

fn startup_config() -> NavigationRuntimeConfig {
    protocol_server_test_runtime_config(
        protocol_server_test_fetch_config(FetchConfig::default()),
        OptionalResourceFetchMask::NONE,
    )
    .with_document_start_scripts(vec![
        "globalThis.__startupRuns = (globalThis.__startupRuns || 0) + 1; globalThis.__scriptOrder = ['startup-a'];".to_owned(),
        "globalThis.__scriptOrder.push('startup-b');".to_owned(),
    ])
}

fn fixture() -> (String, DedicatedFixtureServer) {
    let app = Router::new()
        .route(
            "/page",
            get(|| async {
                axum::response::Html(concat!(
                    "<!doctype html><script>",
                    "globalThis.__pageStartupRuns = globalThis.__startupRuns;",
                    "(globalThis.__scriptOrder ||= []).push('page');",
                    "</script><iframe src='/child'></iframe>"
                ))
            }),
        )
        .route(
            "/child",
            get(|| async {
                axum::response::Html(concat!(
                    "<!doctype html><script>",
                    "globalThis.__pageStartupRuns = globalThis.__startupRuns;",
                    "(globalThis.__scriptOrder ||= []).push('page');",
                    "</script>child"
                ))
            }),
        );
    let (addr, server) = spawn_dedicated_fixture_server(app, "startup-scripts");
    (format!("http://{addr}/page"), server)
}

fn expected_probe(dynamic: Option<&str>) -> serde_json::Value {
    let mut order = vec!["startup-a", "startup-b"];
    order.extend(dynamic);
    order.push("page");
    json!([[1, 1, order], [1, 1, order]])
}

async fn http_json(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    timeout(Duration::from_secs(15), async {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let body = body.to_string();
        stream.write_all(format!(
            "{method} {path} HTTP/1.0\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()
        ).as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        let response = String::from_utf8(bytes).unwrap();
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        serde_json::from_str(body).unwrap_or_else(|error| panic!("{method} {path}: {error}; {response}"))
    }).await.expect("protocol HTTP response")
}

async fn bidi_command(
    socket: &mut TestSocket,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let messages = send_cdp_command(socket, id, method, None, params).await;
    let response = messages
        .into_iter()
        .find(|message| message["id"] == id)
        .unwrap();
    assert_eq!(response["type"], "success", "{method}: {response}");
    response["result"].clone()
}

#[tokio::test]
async fn startup_scripts_cdp_navigations_and_late_browser_contexts() {
    let (url, _fixture) = fixture();
    let (addr, server) = spawn_test_protocol_server_with_runtime_config(startup_config()).await;
    let version = http_json(addr, "GET", "/json/version", json!({})).await;
    let (mut socket, _) = connect_async(version["webSocketDebuggerUrl"].as_str().unwrap())
        .await
        .unwrap();
    let session = cdp_create_default_session_and_navigate(&mut socket, &url).await;
    let probe = cdp_runtime_evaluate_string(&mut socket, &session, 10, PROBE).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&probe).unwrap(),
        expected_probe(None)
    );

    // Creating these contexts happens after the standalone engine has materialized.
    for base in [20, 40] {
        let context = cdp_create_browser_context(&mut socket, base).await;
        let target = cdp_create_attached_target(&mut socket, base + 1, &context).await;
        let registered = send_cdp_command(
            &mut socket,
            base + 5,
            "Page.addScriptToEvaluateOnNewDocument",
            Some(&target.session_id),
            json!({"source": "globalThis.__scriptOrder.push('target');"}),
        )
        .await;
        let identifier = registered
            .iter()
            .find(|message| message["id"] == base + 5)
            .unwrap()["result"]["identifier"]
            .as_str()
            .unwrap()
            .to_owned();
        for navigation in 0..2 {
            if navigation == 1 {
                let removed = send_cdp_command(
                    &mut socket,
                    base + 6,
                    "Page.removeScriptToEvaluateOnNewDocument",
                    Some(&target.session_id),
                    json!({"identifier": identifier}),
                )
                .await;
                assert!(
                    removed
                        .iter()
                        .any(|message| message["id"] == base + 6 && message["result"] == json!({}))
                );
            }
            cdp_navigate_and_wait_for_load(
                &mut socket,
                base + 3,
                &target.session_id,
                &format!("{url}?n={navigation}"),
            )
            .await;
            let probe =
                cdp_runtime_evaluate_string(&mut socket, &target.session_id, base + 4, PROBE).await;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&probe).unwrap(),
                expected_probe((navigation == 0).then_some("target"))
            );
        }
    }
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn startup_scripts_bidi_navigations_and_preload_removal_are_isolated() {
    let (url, _fixture) = fixture();
    let (addr, server) = spawn_test_protocol_server_with_runtime_config(startup_config()).await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/session")).await.unwrap();
    bidi_command(&mut socket, 1, "session.new", json!({})).await;
    let tree = bidi_command(&mut socket, 2, "browsingContext.getTree", json!({})).await;
    let first = tree["contexts"][0]["context"].as_str().unwrap().to_owned();

    let messages = send_cdp_command(
        &mut socket,
        3,
        "script.removePreloadScript",
        None,
        json!({"script": "1"}),
    )
    .await;
    let response = messages.iter().find(|message| message["id"] == 3).unwrap();
    assert_eq!(
        response["type"], "error",
        "CLI scripts have no protocol ID: {response}"
    );
    assert_eq!(response["error"], "no such script");
    let user = bidi_command(&mut socket, 4, "browser.createUserContext", json!({})).await;
    let second = bidi_command(
        &mut socket,
        5,
        "browsingContext.create",
        json!({
            "type": "tab", "userContext": user["userContext"]
        }),
    )
    .await;
    let contexts = [first, second["context"].as_str().unwrap().to_owned()];
    let added = bidi_command(
        &mut socket,
        6,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "() => { globalThis.__scriptOrder.push('dynamic'); }",
            "userContexts": ["default", user["userContext"]]
        }),
    )
    .await;

    for dynamic in [Some("dynamic"), None] {
        if dynamic.is_none() {
            bidi_command(
                &mut socket,
                7,
                "script.removePreloadScript",
                json!({"script": added["script"]}),
            )
            .await;
        }
        for context in &contexts {
            for navigation in 0..2 {
                bidi_command(&mut socket, 8, "browsingContext.navigate", json!({
                    "context": context, "url": format!("{url}?n={navigation}"), "wait": "complete"
                })).await;
                let result = bidi_command(
                    &mut socket,
                    9,
                    "script.evaluate",
                    json!({
                        "target": {"context": context}, "expression": PROBE, "awaitPromise": false
                    }),
                )
                .await;
                assert_eq!(result["type"], "success", "{result}");
                let probe: serde_json::Value =
                    serde_json::from_str(result["result"]["value"].as_str().unwrap()).unwrap();
                assert_eq!(
                    probe,
                    expected_probe(dynamic),
                    "context={context}, dynamic={dynamic:?}"
                );
            }
        }
    }
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn startup_scripts_classic_navigations_and_new_windows() {
    let (url, _fixture) = fixture();
    let (addr, server) = spawn_test_protocol_server_with_runtime_config(startup_config()).await;
    let session = http_json(addr, "POST", "/session", json!({})).await;
    let session = session["value"]["sessionId"].as_str().unwrap();
    for window in 0..2 {
        if window == 1 {
            let new = http_json(
                addr,
                "POST",
                &format!("/session/{session}/window/new"),
                json!({"type": "tab"}),
            )
            .await;
            let switched = http_json(
                addr,
                "POST",
                &format!("/session/{session}/window"),
                json!({"handle": new["value"]["handle"]}),
            )
            .await;
            assert_eq!(switched, json!({"value": null}));
        }
        for navigation in 0..2 {
            let navigated = http_json(
                addr,
                "POST",
                &format!("/session/{session}/url"),
                json!({"url": format!("{url}?n={navigation}")}),
            )
            .await;
            assert_eq!(navigated, json!({"value": null}));
            let result = http_json(
                addr,
                "POST",
                &format!("/session/{session}/execute/sync"),
                json!({"script": format!("return {PROBE};"), "args": []}),
            )
            .await;
            let probe: serde_json::Value =
                serde_json::from_str(result["value"].as_str().unwrap()).unwrap();
            assert_eq!(probe, expected_probe(None));
        }
    }
    let deleted = http_json(addr, "DELETE", &format!("/session/{session}"), json!({})).await;
    assert_eq!(deleted, json!({"value": null}));
    abort_test_cdp_server(server).await;
}
