use super::*;

#[tokio::test]
async fn element_handlers_use_native_owners_across_independent_browser_windows() {
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
    let probe = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-renderer-v8/tests/fixtures/element-handler-native-owners.js"
    ))
    .trim_end()
    .trim_end_matches(';');
    let expression = format!(
        r#"(async () => {{ await ({probe}); return {{total:__uiEventResults.total, failures:__uiEventResults.checks.filter(row => !row.passed), independentRealms:__uiEventResults.separateFunctionRealm && __uiEventResults.separateMediaInterface}}; }})()"#
    );
    let messages = send_cdp_command(
        &mut socket,
        10,
        "Runtime.evaluate",
        Some(&session_id),
        json!({"expression":expression, "awaitPromise":true, "returnByValue":true}),
    )
    .await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .expect("probe response");
    assert!(
        response["error"].is_null() && response["result"]["exceptionDetails"].is_null(),
        "probe must complete without exceptions: {response:#}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        json!({"total":5,"failures":[],"independentRealms":true})
    );
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
