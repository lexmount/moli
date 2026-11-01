use super::*;

#[tokio::test]
async fn media_source_closed_frontend_validates_receivers_realms_and_conversion_order() {
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
    let expression = format!(
        "{} .then(() => ({{total:__uiEventResults.total, failures:__uiEventResults.checks.filter(row=>!row.passed)}}))",
        include_str!("media_source_closed.js"),
    );
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
        .expect("fixture response");
    assert!(
        response["error"].is_null() && response["result"]["exceptionDetails"].is_null(),
        "fixture must complete without exceptions: {response:#}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        json!({"total":477,"failures":[]})
    );
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
