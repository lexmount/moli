use super::*;

#[derive(Clone, Copy, Debug)]
enum CreateOrigin {
    Cdp,
    ClassicSession,
    ClassicWindow,
    Bidi,
    CdpPopup,
    BidiPopup,
}

#[tokio::test]
async fn shared_page_creation_from_cdp() {
    assert_shared_page_creation(CreateOrigin::Cdp).await;
}

#[tokio::test]
async fn shared_page_creation_from_classic_session() {
    assert_shared_page_creation(CreateOrigin::ClassicSession).await;
}

#[tokio::test]
async fn shared_page_creation_from_classic_window() {
    assert_shared_page_creation(CreateOrigin::ClassicWindow).await;
}

#[tokio::test]
async fn shared_page_creation_from_bidi() {
    assert_shared_page_creation(CreateOrigin::Bidi).await;
}

#[tokio::test]
async fn shared_page_creation_from_cdp_popup() {
    assert_shared_page_creation(CreateOrigin::CdpPopup).await;
}

#[tokio::test]
async fn shared_page_creation_from_bidi_popup() {
    assert_shared_page_creation(CreateOrigin::BidiPopup).await;
}

async fn assert_shared_page_creation(origin: CreateOrigin) {
    let (addr, server) = spawn_test_protocol_server().await;
    let session = classic_new_session_on_server(addr).await;
    let base = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/window"),
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
    let info = send_cdp_command(
        &mut cdp,
        2,
        "Target.getTargetInfo",
        None,
        json!({"targetId":base}),
    )
    .await;
    let user_context = bidi_message_by_id(&info, 2)["result"]["targetInfo"]["browserContextId"]
        .as_str()
        .unwrap()
        .to_owned();
    let attached = send_cdp_command(
        &mut cdp,
        3,
        "Target.attachToTarget",
        None,
        json!({"targetId":base,"flatten":true}),
    )
    .await;
    let cdp_session = bidi_message_by_id(&attached, 3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut observers = Vec::new();
    for scoped in [false, false, true] {
        let mut bidi = if scoped {
            connect_classic_session_bidi_socket(addr, &session).await
        } else {
            let (mut bidi, _) = connect_async(format!("ws://{addr}/session")).await.unwrap();
            assert_eq!(
                send_bidi_command_response(&mut bidi, 1, "session.new", json!({})).await["type"],
                "success"
            );
            bidi
        };
        if scoped {
            let contexts =
                send_bidi_command_response(&mut bidi, 1, "browser.getUserContexts", json!({}))
                    .await;
            assert_eq!(
                contexts["result"]["userContexts"],
                json!([{"userContext":"default"}])
            );
        }
        assert_eq!(
            send_bidi_command_response(
                &mut bidi,
                2,
                "session.subscribe",
                json!({"events":["browsingContext.contextCreated","browsingContext.contextDestroyed"]})
            )
            .await["type"],
            "success"
        );
        observers.push((bidi, Vec::new()));
    }
    let mut cdp_messages = Vec::new();
    let mut extra_session = None;
    let mut target = None;
    let script = "window.open('about:blank','shared-created');true";
    match origin {
        CreateOrigin::Cdp => {
            cdp_messages = send_cdp_command(
                &mut cdp,
                4,
                "Target.createTarget",
                None,
                json!({"url":"about:blank","browserContextId":user_context}),
            )
            .await;
            target = Some(
                bidi_message_by_id(&cdp_messages, 4)["result"]["targetId"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        CreateOrigin::ClassicSession => {
            let created_session = classic_new_session_on_server(addr).await;
            target = Some(
                classic_request_on_server_with_body(
                    addr,
                    "GET",
                    &format!("/session/{created_session}/window"),
                    json!({}),
                )
                .await["value"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
            extra_session = Some(created_session);
        }
        CreateOrigin::ClassicWindow => {
            target = Some(
                classic_request_on_server_with_body(
                    addr,
                    "POST",
                    &format!("/session/{session}/window/new"),
                    json!({"type":"tab"}),
                )
                .await["value"]["handle"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        CreateOrigin::Bidi | CreateOrigin::BidiPopup => {
            let (socket, messages) = &mut observers[0];
            let (method, params) = if matches!(origin, CreateOrigin::Bidi) {
                (
                    "browsingContext.create",
                    json!({"type":"tab","userContext":user_context}),
                )
            } else {
                (
                    "script.evaluate",
                    json!({"expression":script,"target":{"context":base},"awaitPromise":false}),
                )
            };
            messages.extend(send_cdp_command(socket, 3, method, None, params).await);
            assert_eq!(
                bidi_message_by_id(messages, 3)["type"],
                "success",
                "{messages:?}"
            );
            if matches!(origin, CreateOrigin::Bidi) {
                target = Some(
                    bidi_message_by_id(messages, 3)["result"]["context"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        }
        CreateOrigin::CdpPopup => {
            cdp_messages = send_cdp_command(
                &mut cdp,
                4,
                "Runtime.evaluate",
                Some(&cdp_session),
                json!({"expression":script,"returnByValue":true}),
            )
            .await;
            assert_eq!(
                bidi_message_by_id(&cdp_messages, 4)["result"]["result"]["value"],
                true
            );
        }
    }
    let created = |message: &serde_json::Value| {
        message["method"] == "Target.targetCreated"
            && message["params"]["targetInfo"]["type"] == "page"
            && target.as_ref().map_or_else(
                || message["params"]["targetInfo"]["openerId"] == base,
                |target| message["params"]["targetInfo"]["targetId"] == *target,
            )
    };
    if !cdp_messages.iter().any(&created) {
        cdp_messages.extend(recv_until_match(&mut cdp, &created).await);
    }
    let event = cdp_messages
        .iter()
        .find(|message| created(message))
        .unwrap();
    let info = &event["params"]["targetInfo"];
    let target = info["targetId"].as_str().unwrap();
    // Actual CDP creation followed by each observer's owner-processed commands
    // bounds notification delivery without sleeps or a missing-event timeout.
    for (index, (socket, messages)) in observers.iter_mut().enumerate() {
        let scoped = index == 2;
        let visible = !scoped || info["browserContextId"] == user_context;
        let expected_user_context = if scoped {
            json!("default")
        } else {
            info["browserContextId"].clone()
        };
        messages
            .extend(send_cdp_command(socket, 4, "browsingContext.getTree", None, json!({})).await);
        messages.extend(send_cdp_command(socket, 5, "session.status", None, json!({})).await);
        let events = messages
            .iter()
            .filter(|message| {
                message["method"] == "browsingContext.contextCreated"
                    && message["params"]["context"] == target
            })
            .collect::<Vec<_>>();
        assert_eq!(
            events.len(),
            usize::from(visible),
            "{origin:?}, observer {index}: one creation only within the session's scope: {messages:?}"
        );
        if visible {
            assert_eq!(events[0]["params"]["url"], "about:blank");
            assert_eq!(events[0]["params"]["parent"], serde_json::Value::Null);
            assert_eq!(events[0]["params"]["userContext"], expected_user_context);
        }
        let tree = bidi_message_by_id(messages, 4)["result"]["contexts"]
            .as_array()
            .unwrap();
        assert_eq!(
            tree.iter().any(|context| context["context"] == target),
            visible
        );
        assert!(tree.iter().any(|context| context["context"] == base));
        for context in tree.iter().filter(|context| context["context"] == target) {
            assert_eq!(context["userContext"], expected_user_context);
        }
        let root =
            send_bidi_command_response(socket, 6, "browsingContext.getTree", json!({"root":base}))
                .await;
        assert_eq!(
            root["result"]["contexts"][0]["userContext"],
            if scoped { "default" } else { &user_context }
        );
        messages.clear();
    }
    let mut closed = send_cdp_command(
        &mut cdp,
        7,
        "Target.closeTarget",
        None,
        json!({"targetId":target}),
    )
    .await;
    let destroyed = |message: &serde_json::Value| {
        message["method"] == "Target.targetDestroyed" && message["params"]["targetId"] == target
    };
    if !closed.iter().any(&destroyed) {
        closed.extend(recv_until_match(&mut cdp, destroyed).await);
    }
    for (index, (socket, messages)) in observers.iter_mut().enumerate() {
        let scoped = index == 2;
        let visible = !scoped || info["browserContextId"] == user_context;
        messages
            .extend(send_cdp_command(socket, 7, "browsingContext.getTree", None, json!({})).await);
        messages.extend(send_cdp_command(socket, 8, "session.status", None, json!({})).await);
        let events = messages
            .iter()
            .filter(|message| {
                message["method"] == "browsingContext.contextDestroyed"
                    && message["params"]["context"] == target
            })
            .collect::<Vec<_>>();
        assert_eq!(
            events.len(),
            usize::from(visible),
            "{origin:?}, observer {index}: {messages:?}"
        );
        if visible {
            assert_eq!(
                events[0]["params"]["userContext"],
                if scoped {
                    json!("default")
                } else {
                    info["browserContextId"].clone()
                }
            );
        }
        let tree = bidi_message_by_id(messages, 7)["result"]["contexts"]
            .as_array()
            .unwrap();
        assert!(tree.iter().all(|context| context["context"] != target));
        assert!(tree.iter().any(|context| context["context"] == base));
    }
    for (mut socket, _) in observers {
        socket.close(None).await.unwrap();
    }
    cdp.close(None).await.unwrap();
    for session in std::iter::once(session).chain(extra_session) {
        classic_request_on_server_with_body(
            addr,
            "DELETE",
            &format!("/session/{session}"),
            json!({}),
        )
        .await;
    }
    abort_test_cdp_server(server).await;
}
