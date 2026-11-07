use super::*;

#[tokio::test]
async fn nested_workers_release_database_connections_when_their_ancestor_global_retires() {
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to browser websocket");
    let session_id = cdp_create_default_session_and_navigate(
        &mut socket,
        &format!("http://{fixture_addr}/page"),
    )
    .await;
    let expression = include_str!("nested-worker-retirement.js");
    let messages = send_cdp_command(
        &mut socket,
        10,
        "Runtime.evaluate",
        Some(&session_id),
        json!({"expression":expression,"awaitPromise":true,"returnByValue":true}),
    )
    .await;
    let response = messages
        .iter()
        .find(|message| message["id"] == 10)
        .expect("worker retirement fixture response");
    assert!(
        response["error"].is_null() && response["result"]["exceptionDetails"].is_null(),
        "worker retirement fixture must complete without exceptions: {response:#}"
    );
    let mut cases = Vec::new();
    for script_type in ["classic", "module"] {
        for (depth, mode) in [
            (0, "terminate"),
            (1, "terminate"),
            (2, "terminate"),
            (1, "close"),
            (1, "terminate-child"),
        ] {
            cases.push(json!({"scriptType":script_type, "depth":depth, "mode":mode,
                "heldBefore":true, "released":true, "controlRetained":true}));
        }
    }
    assert_eq!(
        response["result"]["result"]["value"],
        json!({"cases":cases,"controlReleased":true,"remainingNames":[]}),
        "retiring an ancestor must release descendant connections and retain unrelated live workers"
    );
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
