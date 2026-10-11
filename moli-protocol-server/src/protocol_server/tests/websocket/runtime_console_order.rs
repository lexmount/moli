use super::*;

async fn assert_timer_console_precedes_response(child: bool, call_function: bool) {
    let (cdp_addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to CDP");
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let created = send_cdp_command(
        &mut socket,
        2,
        "Target.createTarget",
        None,
        json!({"url":"about:blank", "browserContextId":context}),
    )
    .await;
    let target = created.iter().find(|message| message["id"] == 2).unwrap()["result"]["targetId"]
        .as_str()
        .unwrap();
    let attached = send_cdp_command(
        &mut socket,
        3,
        "Target.attachToTarget",
        None,
        json!({"targetId":target, "flatten":true}),
    )
    .await;
    let session =
        attached.iter().find(|message| message["id"] == 3).unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
    send_cdp_command(&mut socket, 4, "Page.enable", Some(&session), json!({})).await;
    cdp_navigate_and_wait_for_load(
        &mut socket,
        5,
        &session,
        "data:text/html,<title>root</title><iframe srcdoc='<title>child</title>'></iframe>",
    )
    .await;
    let tree = send_cdp_command(
        &mut socket,
        6,
        "Page.getFrameTree",
        Some(&session),
        json!({}),
    )
    .await;
    let tree = &tree.iter().find(|message| message["id"] == 6).unwrap()["result"]["frameTree"];
    let frame = if child { &tree["childFrames"][0] } else { tree };
    let frame_id = frame["frame"]["id"].as_str().unwrap();
    if child {
        send_cdp_command(
            &mut socket,
            7,
            "Page.createIsolatedWorld",
            Some(&session),
            json!({"frameId":frame_id, "worldName":"console-order-materialization"}),
        )
        .await;
    }
    let enabled =
        send_cdp_command(&mut socket, 8, "Runtime.enable", Some(&session), json!({})).await;
    let execution_context = enabled
        .iter()
        .find(|message| {
            message["method"] == "Runtime.executionContextCreated"
                && message["params"]["context"]["auxData"]["isDefault"] == true
                && message["params"]["context"]["auxData"]["frameId"] == frame_id
        })
        .expect("the actual default context must be reported")["params"]["context"]["id"]
        .as_i64()
        .unwrap();
    let expression = "new Promise(resolve => setTimeout(() => { console.log('timer-console-prefix'); resolve({title:document.title, child:window!==top}); }, 20))";
    let (method, params) = if call_function {
        (
            "Runtime.callFunctionOn",
            json!({"executionContextId":execution_context,
                   "functionDeclaration":format!("function () {{ return {expression}; }}"),
                   "awaitPromise":true, "returnByValue":true}),
        )
    } else {
        (
            "Runtime.evaluate",
            json!({"contextId":execution_context, "expression":expression,
                   "awaitPromise":true, "returnByValue":true}),
        )
    };
    // The only input after dispatch is the real WebSocket. A second command
    // must not be needed to publish the timer's console prefix or response.
    let observed = send_cdp_command(&mut socket, 9, method, Some(&session), params).await;
    socket.close(None).await.expect("close CDP socket");
    abort_test_cdp_server(server).await;

    let console = observed
        .iter()
        .position(|message| {
            message["sessionId"] == session
                && message["method"] == "Runtime.consoleAPICalled"
                && message["params"]["executionContextId"] == execution_context
                && message["params"]["args"][0]["value"] == "timer-console-prefix"
        })
        .unwrap_or_else(|| panic!("timer console must precede {method} reply: {observed:#?}"));
    let terminal = observed
        .iter()
        .position(|message| message["id"] == 9)
        .unwrap();
    assert!(console < terminal, "{observed:#?}");
    assert_eq!(observed[terminal]["sessionId"], session);
    assert_eq!(
        observed[terminal]["result"]["result"]["value"],
        json!({"title":if child {"child"} else {"root"}, "child":child})
    );
}

#[tokio::test]
async fn websocket_cdp_timer_console_precedes_root_evaluate_response() {
    assert_timer_console_precedes_response(false, false).await;
}

#[tokio::test]
async fn websocket_cdp_timer_console_precedes_child_evaluate_response() {
    assert_timer_console_precedes_response(true, false).await;
}

#[tokio::test]
async fn websocket_cdp_timer_console_precedes_root_call_function_response() {
    assert_timer_console_precedes_response(false, true).await;
}

#[tokio::test]
async fn websocket_cdp_timer_console_precedes_child_call_function_response() {
    assert_timer_console_precedes_response(true, true).await;
}
