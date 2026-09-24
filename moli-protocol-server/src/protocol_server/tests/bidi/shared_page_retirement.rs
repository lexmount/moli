use super::*;

#[derive(Clone, Copy, Debug)]
enum CloseOrigin {
    CdpTarget,
    CdpPage,
    ClassicWindow,
    BidiContext,
    CdpUserContext,
    BidiUserContext,
    ClassicSession,
}

#[tokio::test]
async fn shared_page_retirement_from_cdp_target() {
    assert_shared_page_retirement(CloseOrigin::CdpTarget).await;
}

#[tokio::test]
async fn shared_page_retirement_from_cdp_page() {
    assert_shared_page_retirement(CloseOrigin::CdpPage).await;
}

#[tokio::test]
async fn shared_page_retirement_from_classic_window() {
    assert_shared_page_retirement(CloseOrigin::ClassicWindow).await;
}

#[tokio::test]
async fn shared_page_retirement_from_bidi_context() {
    assert_shared_page_retirement(CloseOrigin::BidiContext).await;
}

#[tokio::test]
async fn shared_page_retirement_from_cdp_user_context() {
    assert_shared_page_retirement(CloseOrigin::CdpUserContext).await;
}

#[tokio::test]
async fn shared_page_retirement_from_bidi_user_context() {
    assert_shared_page_retirement(CloseOrigin::BidiUserContext).await;
}

#[tokio::test]
async fn shared_page_retirement_from_classic_session() {
    assert_shared_page_retirement(CloseOrigin::ClassicSession).await;
}

async fn assert_shared_page_retirement(origin: CloseOrigin) {
    let (addr, server) = spawn_test_protocol_server().await;
    let session = classic_new_session_on_server(addr).await;
    let peer = classic_new_session_on_server(addr).await;
    let target = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/window"),
        json!({}),
    )
    .await["value"]
        .as_str()
        .unwrap()
        .to_owned();
    let peer_target = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{peer}/window"),
        json!({}),
    )
    .await["value"]
        .as_str()
        .unwrap()
        .to_owned();
    let (mut cdp, _) = connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
        .await
        .unwrap();
    send_cdp_command(
        &mut cdp,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({"discover":true}),
    )
    .await;
    let targets = send_cdp_command(&mut cdp, 2, "Target.getTargets", None, json!({})).await;
    let info = bidi_message_by_id(&targets, 2)["result"]["targetInfos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|info| info["targetId"] == target)
        .unwrap();
    let user_context = info["browserContextId"].as_str().unwrap().to_owned();
    let attached = send_cdp_command(
        &mut cdp,
        3,
        "Target.attachToTarget",
        None,
        json!({"targetId":target,"flatten":true}),
    )
    .await;
    let cdp_session = bidi_message_by_id(&attached, 3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut observers = Vec::new();
    for scoped in [true, false] {
        let (mut bidi, _) = connect_async(format!("ws://{addr}/session")).await.unwrap();
        assert_eq!(
            send_bidi_command_response(&mut bidi, 1, "session.new", json!({})).await["type"],
            "success"
        );
        let mut params = json!({"events":["browsingContext.contextDestroyed"]});
        if scoped {
            params["contexts"] = json!([target]);
        }
        assert_eq!(
            send_bidi_command_response(&mut bidi, 2, "session.subscribe", params).await["type"],
            "success"
        );
        observers.push((bidi, Vec::new()));
    }

    let cdp_messages = match origin {
        CloseOrigin::CdpTarget => {
            send_cdp_command(
                &mut cdp,
                4,
                "Target.closeTarget",
                None,
                json!({"targetId":target}),
            )
            .await
        }
        CloseOrigin::CdpPage => {
            send_cdp_command(&mut cdp, 4, "Page.close", Some(&cdp_session), json!({})).await
        }
        CloseOrigin::CdpUserContext => {
            send_cdp_command(
                &mut cdp,
                4,
                "Target.disposeBrowserContext",
                None,
                json!({"browserContextId":user_context}),
            )
            .await
        }
        CloseOrigin::BidiContext | CloseOrigin::BidiUserContext => {
            let (socket, messages) = &mut observers[0];
            let (method, params) = if matches!(origin, CloseOrigin::BidiContext) {
                ("browsingContext.close", json!({"context":target}))
            } else {
                (
                    "browser.removeUserContext",
                    json!({"userContext":user_context}),
                )
            };
            messages.extend(send_cdp_command(socket, 3, method, None, params).await);
            assert_eq!(
                bidi_message_by_id(messages, 3)["type"],
                "success",
                "{messages:?}"
            );
            Vec::new()
        }
        CloseOrigin::ClassicWindow | CloseOrigin::ClassicSession => {
            let suffix = if matches!(origin, CloseOrigin::ClassicWindow) {
                "/window"
            } else {
                ""
            };
            classic_request_on_server_with_body(
                addr,
                "DELETE",
                &format!("/session/{session}{suffix}"),
                json!({}),
            )
            .await;
            Vec::new()
        }
    };
    if !cdp_messages.is_empty() {
        assert!(
            bidi_message_by_id(&cdp_messages, 4).get("error").is_none(),
            "{cdp_messages:?}"
        );
    }
    let destroyed = |message: &serde_json::Value| {
        message["method"] == "Target.targetDestroyed" && message["params"]["targetId"] == target
    };
    if !cdp_messages.iter().any(destroyed) {
        recv_until_match(&mut cdp, destroyed).await;
    }
    // CDP destruction proves physical close has crossed the shared owner. Each
    // subsequent BiDi command drains the preceding notification batch without
    // timing sleeps or waiting for an event that the old path never publishes.
    for (socket, messages) in &mut observers {
        messages
            .extend(send_cdp_command(socket, 4, "browsingContext.getTree", None, json!({})).await);
        messages.extend(send_cdp_command(socket, 5, "session.status", None, json!({})).await);
        let events = messages
            .iter()
            .filter(|event| {
                event["method"] == "browsingContext.contextDestroyed"
                    && event["params"]["context"] == target
            })
            .collect::<Vec<_>>();
        assert_eq!(
            events.len(),
            1,
            "{origin:?}: each global/scoped observer must receive one actual retirement: {messages:?}"
        );
        assert_eq!(events[0]["params"]["url"], "about:blank");
        assert_eq!(events[0]["params"]["children"], json!([]));
        let contexts = bidi_message_by_id(messages, 4)["result"]["contexts"]
            .as_array()
            .unwrap();
        assert!(contexts.iter().all(|context| context["context"] != target));
        assert!(
            contexts
                .iter()
                .any(|context| context["context"] == peer_target)
        );
    }
    let handles = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{peer}/window/handles"),
        json!({}),
    )
    .await;
    assert_eq!(handles["value"], json!([peer_target]));
    let result = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{peer}/execute/sync"),
        json!({"script":"return 6*7","args":[]}),
    )
    .await;
    assert_eq!(result["value"], 42);
    for (mut socket, _) in observers {
        socket.close(None).await.unwrap();
    }
    cdp.close(None).await.unwrap();
    // Closing Classic's last window may already have ended its session.
    let (status, _) = classic_request_status_on_server_with_body(
        addr,
        "DELETE",
        &format!("/session/{session}"),
        json!({}),
    )
    .await;
    assert!(matches!(status, 200 | 404));
    classic_request_on_server_with_body(addr, "DELETE", &format!("/session/{peer}"), json!({}))
        .await;
    abort_test_cdp_server(server).await;
}
