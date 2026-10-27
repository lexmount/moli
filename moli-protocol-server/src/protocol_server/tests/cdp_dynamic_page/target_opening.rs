use super::*;

#[tokio::test]
async fn websocket_attachment_wait_allows_peer_metadata_before_renderer_release() {
    use std::sync::Arc;
    use tokio::sync::Notify;

    struct ReleaseOnDrop(Arc<Notify>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let witness = Arc::clone(&entered);
    let held = Arc::clone(&release);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        axum::Router::new()
            .route(
                "/",
                axum::routing::get(|| async { axum::response::Html("<title>Attach wait</title>") }),
            )
            .route(
                "/held",
                axum::routing::get(move || {
                    let witness = Arc::clone(&witness);
                    let held = Arc::clone(&held);
                    async move {
                        witness.notify_one();
                        held.notified().await;
                        "released"
                    }
                }),
            ),
        "target-attachment-wait",
    );
    let release = ReleaseOnDrop(release);
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;
    send_cdp_command(&mut page, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut page, 2, &format!("http://{fixture_addr}/")).await;
    send_cdp_command_without_wait(&mut page, 3, "Runtime.evaluate", None, json!({
        "expression": "const held = new XMLHttpRequest(); held.open('GET', '/held', false); held.send(); 'released'",
        "returnByValue": true,
    })).await;
    timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("the original renderer is held");
    send_cdp_command_without_wait(
        &mut browser,
        10,
        "Target.attachToTarget",
        None,
        json!({
            "targetId": target_id, "flatten": true,
        }),
    )
    .await;
    let peer = timeout(
        Duration::from_secs(5),
        send_cdp_command(&mut browser, 11, "Browser.getVersion", None, json!({})),
    )
    .await;
    // This explicit release follows the peer observation, including its error
    // path. The HTTP fixture has no timeout that could make progress by itself.
    drop(release);
    let peer = peer.expect("metadata must complete while attachment still waits for the renderer");
    assert!(response_by_id(&peer, 11)["result"].is_object(), "{peer:?}");
    assert!(
        !peer.iter().any(|message| message["id"] == 10),
        "attachment must still wait for its original renderer: {peer:?}"
    );
    let attached = recv_until_id(&mut browser, 10).await;
    let response = attached
        .iter()
        .position(|message| message["id"] == 10)
        .unwrap();
    let session = attached[response]["result"]["sessionId"].as_str().unwrap();
    let event = attached
        .iter()
        .position(|message| {
            message["method"] == "Target.attachedToTarget"
                && message["params"]["sessionId"] == session
        })
        .unwrap();
    assert!(event < response, "{attached:?}");
    let evaluated = recv_until_id(&mut page, 3).await;
    assert_eq!(
        response_by_id(&evaluated, 3)["result"]["result"]["value"],
        json!("released")
    );
    page.close(None).await.unwrap();
    browser.close(None).await.unwrap();
    server.abort();
}
