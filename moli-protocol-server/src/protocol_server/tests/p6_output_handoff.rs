use super::*;

use tokio::sync::Notify;

/// P6-R0 contract witness: a background navigation must retain its own output
/// residence. Finishing the response body has to publish both the command
/// terminal and load without a later protocol command incidentally capturing the
/// connection.
#[tokio::test]
async fn background_navigation_load_is_published_without_followup_command() {
    let release_slow_body = Arc::new(Notify::new());
    let response_head_sent = Arc::new(Notify::new());
    // Use the raw fixture so the response head and first chunk are flushed
    // independently of EOF. An Axum body stream may coalesce them while idle,
    // which would test the fixture transport rather than Page.navigate.
    let (fixture_addr, fixture_server) =
        spawn_response_stage_streaming_document_fixture_server_with_head_signal(
            Arc::clone(&release_slow_body),
            Some(Arc::clone(&response_head_sent)),
        )
        .await;

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(
        discover_browser_websocket_url(cdp_addr)
            .await
            .expect("discover browser endpoint"),
    )
    .await
    .expect("connect websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let create_target = send_cdp_command(
        &mut socket,
        2,
        "Target.createTarget",
        None,
        json!({
            "browserContextId": browser_context_id,
            "url": "about:blank",
        }),
    )
    .await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();
    let attach = send_cdp_command(
        &mut socket,
        3,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id, "flatten": true }),
    )
    .await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();
    let _ = send_cdp_command(&mut socket, 4, "Page.enable", Some(&session_id), json!({})).await;

    let initial_url = "data:text/html,<main>initial</main>";
    let initial_messages =
        cdp_navigate_and_wait_for_load(&mut socket, 5, &session_id, initial_url).await;
    assert!(
        initial_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(initial_url)
        }),
        "the setup navigation must commit its exact frame before the background-navigation witness starts: {initial_messages:#?}"
    );

    let slow_url = format!("http://{fixture_addr}/slow");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": slow_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send streaming Page.navigate");
    timeout(Duration::from_secs(3), response_head_sent.notified())
        .await
        .expect("streaming fixture should flush response head before EOF");
    // No command is sent after this point. The server-side body terminal, the
    // renderer completion, and the protocol output scheduler alone must make
    // both the command response and lifecycle terminal visible.
    release_slow_body.notify_one();
    let mut messages = Vec::new();
    let mut saw_response = false;
    let mut saw_load = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !(saw_response && saw_load) {
        let message = tokio::time::timeout_at(deadline, recv_ws_json(&mut socket))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "background navigation did not publish its response and load without a \
                     follow-up command; messages after Page.navigate: {messages:#?}"
                )
            });
        saw_response |= message["id"] == json!(6_u64) && message.get("result").is_some();
        saw_load |= message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired");
        messages.push(message);
    }

    let response = messages
        .iter()
        .position(|message| message["id"] == json!(6_u64))
        .expect("Page.navigate should publish its command terminal");
    let navigated = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(slow_url)
        })
        .expect("the background navigation should publish its exact frame URL");
    let loaded = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        })
        .expect("the background navigation should publish load");
    assert!(
        response < loaded && navigated < loaded,
        "the command response and exact frame navigation must become visible before load: {messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server
        .await
        .expect("streaming navigation fixture should finish");
}
