use super::*;
use moli_core::browser::{BrowserEvent, BrowserHandle};

pub(super) async fn server_with_browser() -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
    BrowserHandle,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = protocol_server_test_state(
        addr,
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
    );
    let browser = state.browser_service.handle();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    (addr, server, browser)
}

#[tokio::test]
async fn websocket_native_context_disposal_retires_pending_calls_and_exact_sessions() {
    let (addr, server, browser) = server_with_browser().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    send_cdp_command(
        &mut socket,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({"discover": true}),
    )
    .await;
    let (_, mut events) = browser.subscribe().unwrap();
    let context_id = cdp_create_browser_context(&mut socket, 2).await;
    let BrowserEvent::ContextCreated(context) = events.try_recv().unwrap().event else {
        panic!("expected the new physical Context");
    };
    let target = cdp_create_attached_target(&mut socket, 3, &context_id).await;
    send_cdp_command(
        &mut socket,
        5,
        "Runtime.enable",
        Some(&target.session_id),
        json!({}),
    )
    .await;
    send_cdp_command_without_wait(
        &mut socket, 6, "Runtime.evaluate", Some(&target.session_id),
        json!({"expression": "console.log('native-close-ready'); new Promise(() => {})", "awaitPromise": true}),
    ).await;
    recv_until_match(&mut socket, |message| {
        message["method"] == "Runtime.consoleAPICalled"
            && message["sessionId"] == target.session_id
            && message["params"]["args"][0]["value"] == "native-close-ready"
    })
    .await;

    assert!(browser.remove_context(context).unwrap());
    let (mut failed, mut detached, mut destroyed) = (false, false, false);
    let mut observed = recv_until_match(&mut socket, |message| {
        failed |= message["id"] == 6 && message.get("error").is_some();
        detached |= message["method"] == "Target.detachedFromTarget"
            && message["params"]["sessionId"] == target.session_id;
        destroyed |= message["method"] == "Target.targetDestroyed"
            && message["params"]["targetId"] == target.target_id;
        failed && detached && destroyed
    })
    .await;
    let targets = send_cdp_command(&mut socket, 7, "Target.getTargets", None, json!({})).await;
    let reply = targets.iter().find(|message| message["id"] == 7).unwrap();
    let live = reply["result"]["targetInfos"].as_array().unwrap();
    assert!(
        live.iter()
            .any(|info| info["targetId"] == DEFAULT_TARGET_ID)
    );
    assert!(live.iter().all(|info| info["targetId"] != target.target_id));
    observed.extend(targets);
    assert_eq!(
        observed.iter().filter(|message| message["id"] == 6).count(),
        1
    );
    assert_eq!(
        observed
            .iter()
            .filter(|message| {
                message["method"] == "Target.targetDestroyed"
                    && message["params"]["targetId"] == target.target_id
            })
            .count(),
        1
    );
    let stale = send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        Some(&target.session_id),
        json!({"expression": "1"}),
    )
    .await;
    assert!(
        stale
            .iter()
            .any(|message| message["id"] == 8 && message.get("error").is_some())
    );
    // A protocol-initiated disposal has already retired its projection. Its
    // later Browser event must not duplicate Target/session destruction.
    let protocol_context = cdp_create_browser_context(&mut socket, 9).await;
    let protocol_target = cdp_create_attached_target(&mut socket, 10, &protocol_context).await;
    let mut closed = send_cdp_command(
        &mut socket,
        12,
        "Target.disposeBrowserContext",
        None,
        json!({"browserContextId": protocol_context}),
    )
    .await;
    closed.extend(send_cdp_command(&mut socket, 13, "Target.getTargets", None, json!({})).await);
    assert_eq!(
        closed
            .iter()
            .filter(|message| {
                message["method"] == "Target.targetDestroyed"
                    && message["params"]["targetId"] == protocol_target.target_id
            })
            .count(),
        1
    );
    assert_eq!(
        closed
            .iter()
            .filter(|message| {
                message["method"] == "Target.detachedFromTarget"
                    && message["params"]["sessionId"] == protocol_target.session_id
            })
            .count(),
        1
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_bidi_observes_native_context_disposal_without_another_command() {
    let (addr, server, browser) = server_with_browser().await;
    let (_, mut events) = browser.subscribe().unwrap();
    let (mut socket, _) = connect_async(format!("ws://{addr}/session")).await.unwrap();
    let session = send_cdp_command(&mut socket, 1, "session.new", None, json!({})).await;
    assert!(
        session
            .iter()
            .any(|message| message["id"] == 1 && message["type"] == "success")
    );
    let tree = send_cdp_command(&mut socket, 2, "browsingContext.getTree", None, json!({})).await;
    let reply = tree.iter().find(|message| message["id"] == 2).unwrap();
    let target = reply["result"]["contexts"][0]["context"]
        .as_str()
        .unwrap()
        .to_owned();
    let navigated = send_cdp_command(
        &mut socket,
        6,
        "browsingContext.navigate",
        None,
        json!({
            "context": target, "url": "about:blank", "wait": "complete",
        }),
    )
    .await;
    assert!(
        navigated
            .iter()
            .any(|message| message["id"] == 6 && message["type"] == "success"),
        "{navigated:?}"
    );
    let evaluated = send_cdp_command(
        &mut socket,
        5,
        "script.evaluate",
        None,
        json!({
            "expression": "73", "target": {"context": target}, "awaitPromise": false,
        }),
    )
    .await;
    assert!(
        evaluated
            .iter()
            .any(|message| message["id"] == 5 && message["result"]["result"]["value"] == 73),
        "unexpected script result: {evaluated:#?}"
    );
    let BrowserEvent::ContextCreated(context) = events.try_recv().unwrap().event else {
        panic!("expected the BiDi physical Context");
    };
    send_cdp_command(
        &mut socket,
        3,
        "session.subscribe",
        None,
        json!({"events": ["browsingContext.contextDestroyed"]}),
    )
    .await;
    assert!(browser.remove_context(context).unwrap());
    recv_until_match(&mut socket, |message| {
        message["method"] == "browsingContext.contextDestroyed"
            && message["params"]["context"] == target
    })
    .await;
    let tree = send_cdp_command(&mut socket, 4, "browsingContext.getTree", None, json!({})).await;
    let reply = tree.iter().find(|message| message["id"] == 4).unwrap();
    assert_eq!(reply["result"]["contexts"], json!([]));
    assert!(
        !tree
            .iter()
            .any(|message| message["method"] == "browsingContext.contextDestroyed")
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
