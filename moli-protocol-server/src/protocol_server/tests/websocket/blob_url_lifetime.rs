use super::*;

#[tokio::test]
async fn websocket_cdp_blob_url_loads_survive_revocation_after_start() {
    let app = Router::new()
        .route(
            "/page",
            get(|| async {
                axum::response::Html(
                    "<!doctype html><meta charset=\"utf-8\"><body><script src=\"/probe.js\"></script>",
                )
            }),
        )
        .route(
            "/blank",
            get(|| async { axum::response::Html("<!doctype html><body>blank") }),
        )
        .route(
            "/probe.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/javascript; charset=utf-8")],
                    include_str!("blob_url_lifetime.js"),
                )
            }),
        );
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(app, "blob-url-lifetime");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let mut results = Vec::new();
    for (index, (kind, revoke)) in [
        "script",
        "module",
        "iframe",
        "iframe-navigation",
        "popup",
        "popup-noopener",
        "anchor",
    ]
    .into_iter()
    .flat_map(|kind| [false, true].map(|revoke| (kind, revoke)))
    .enumerate()
    {
        let id = 100 * index as u64 + 1;
        let context = cdp_create_browser_context(&mut socket, id).await;
        let session = cdp_create_attached_target(&mut socket, id + 1, &context).await;
        send_cdp_command(
            &mut socket,
            id + 4,
            "Page.enable",
            Some(&session.session_id),
            json!({}),
        )
        .await;
        cdp_navigate_and_wait_for_load(
            &mut socket,
            id + 5,
            &session.session_id,
            &format!("http://{fixture_addr}/page"),
        )
        .await;
        let expression = format!(
            "probeBlobLifetime({}, {revoke})",
            serde_json::to_string(kind).unwrap(),
        );
        let messages = send_cdp_command(
            &mut socket,
            id + 6,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({"expression": expression, "awaitPromise": true, "returnByValue": true}),
        )
        .await;
        let response = messages
            .iter()
            .find(|message| message["id"] == id + 6)
            .expect("Blob lifetime evaluation response");
        assert!(response["error"].is_null(), "{kind}/{revoke}: {response}");
        assert!(
            response["result"]["exceptionDetails"].is_null(),
            "{kind}/{revoke}: {response}"
        );
        results.push(response["result"]["result"]["value"].clone());
        send_cdp_command(
            &mut socket,
            id + 7,
            "Target.disposeBrowserContext",
            None,
            json!({"browserContextId": context}),
        )
        .await;
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
    let failures: Vec<_> = results
        .iter()
        .filter(|result| result["outcome"] != "loaded")
        .collect();
    assert_eq!(results.len(), 14);
    assert!(
        failures.is_empty(),
        "Blob URL lifetime failures: {failures:?}"
    );
}
