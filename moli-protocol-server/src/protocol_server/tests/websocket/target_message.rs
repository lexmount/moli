use super::*;

fn nested_message(message: &serde_json::Value, session_id: &str) -> Option<serde_json::Value> {
    if message["method"] != "Target.receivedMessageFromTarget"
        || message["params"]["sessionId"] != session_id
    {
        return None;
    }
    serde_json::from_str(message["params"]["message"].as_str()?).ok()
}

fn nested_reply(
    message: &serde_json::Value,
    session_id: &str,
    id: u64,
) -> Option<serde_json::Value> {
    nested_message(message, session_id).filter(|nested| nested["id"] == id)
}

async fn send_nested_command(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    outer_id: u64,
    session_id: &str,
    inner_id: u64,
    method: &str,
    params: serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut output = send_cdp_command(socket, outer_id, "Target.sendMessageToTarget", None,
        json!({"sessionId":session_id,"message":json!({"id":inner_id,"method":method,"params":params}).to_string()})).await;
    if !output
        .iter()
        .any(|m| nested_reply(m, session_id, inner_id).is_some())
    {
        output.extend(
            recv_until_match(socket, |m| nested_reply(m, session_id, inner_id).is_some()).await,
        );
    }
    assert!(
        output.iter().all(|m| m.get("sessionId").is_none()),
        "non-flat output escaped: {output:?}"
    );
    output
}

#[tokio::test]
async fn nonflat_target_wraps_initial_runtime_context_and_later_renderer_events() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_target_with_transport(&mut socket, 2, &context, false).await;
    let mut output = send_nested_command(
        &mut socket,
        70,
        &target.session_id,
        10,
        "Runtime.enable",
        json!({}),
    )
    .await;
    if !output.iter().any(|m| {
        nested_message(m, &target.session_id)
            .is_some_and(|m| m["method"] == "Runtime.executionContextCreated")
    }) {
        output.extend(
            recv_until_match(&mut socket, |m| {
                nested_message(m, &target.session_id)
                    .is_some_and(|m| m["method"] == "Runtime.executionContextCreated")
            })
            .await,
        );
    }
    let reply = output
        .iter()
        .find_map(|m| nested_reply(m, &target.session_id, 10))
        .unwrap();
    assert_eq!(reply["result"], json!({}));
    assert!(reply.get("sessionId").is_none());
    let replay = output
        .iter()
        .find_map(|m| {
            nested_message(m, &target.session_id)
                .filter(|m| m["method"] == "Runtime.executionContextCreated")
        })
        .unwrap();
    assert!(replay["params"]["context"]["id"].is_number());
    assert!(replay.get("sessionId").is_none());
    // The enable request no longer has a pending-command record. A separate
    // flat attachment now generates a renderer event on the original session.
    let peer = send_cdp_command(
        &mut socket,
        71,
        "Target.attachToTarget",
        None,
        json!({"targetId":target.target_id,"flatten":true}),
    )
    .await;
    let peer_session = peer.last().unwrap()["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    output.extend(
        send_cdp_command(
            &mut socket,
            72,
            "Runtime.evaluate",
            Some(&peer_session),
            json!({"expression":"console.log('after-enable');42","returnByValue":true}),
        )
        .await,
    );
    if !output.iter().any(|m| {
        nested_message(m, &target.session_id)
            .is_some_and(|m| m["method"] == "Runtime.consoleAPICalled")
    }) {
        output.extend(
            recv_until_match(&mut socket, |m| {
                nested_message(m, &target.session_id)
                    .is_some_and(|m| m["method"] == "Runtime.consoleAPICalled")
            })
            .await,
        );
    }
    assert!(
        output.iter().all(|m| m["sessionId"] != target.session_id),
        "non-flat event leaked: {output:?}"
    );
    assert!(
        output
            .iter()
            .filter_map(|m| nested_message(m, &target.session_id))
            .all(|m| m.get("sessionId").is_none())
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn legacy_target_transport_acks_while_runtime_is_held_and_keeps_normal_call_routing() {
    let (addr, server) = spawn_test_protocol_server().await;
    let url = format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}");
    let (mut socket, _) = connect_async(&url).await.unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_target_with_transport(&mut socket, 2, &context, false).await;
    let (mut peer, _) = connect_async(&url).await.unwrap();
    let attached = send_cdp_command(
        &mut peer,
        1,
        "Target.attachToTarget",
        None,
        json!({"targetId":target.target_id,"flatten":true}),
    )
    .await;
    let peer_session = attached.iter().find(|m| m["id"] == 1).unwrap()["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();

    let ack = send_cdp_command(&mut socket, 70, "Target.sendMessageToTarget", None, json!({
        "sessionId":target.session_id,"message":json!({"id":42,"method":"Runtime.evaluate","params":{
            "expression":"new Promise(resolve=>{globalThis.releaseLegacyTarget=resolve})","awaitPromise":true,"returnByValue":true,
        }}).to_string(),
    })).await;
    assert!(ack.iter().any(|m| *m == json!({"id":70,"result":{}})));
    assert!(
        !ack.iter()
            .any(|m| nested_reply(m, &target.session_id, 42).is_some()),
        "the original promise is still held: {ack:?}"
    );
    // Use the same client id on a different frontend while the original waits.
    let peer_output = send_cdp_command(
        &mut peer,
        42,
        "Runtime.evaluate",
        Some(&peer_session),
        json!({
            "expression":"typeof releaseLegacyTarget","returnByValue":true,
        }),
    )
    .await;
    assert!(
        peer_output
            .iter()
            .any(|m| m["id"] == 42 && m["result"]["result"]["value"] == "function"),
        "the nested renderer call must be admitted before its peer: {peer_output:?}"
    );
    let metadata = send_cdp_command(&mut socket, 71, "Browser.getVersion", None, json!({})).await;
    assert!(
        !metadata
            .iter()
            .any(|m| nested_reply(m, &target.session_id, 42).is_some())
    );
    send_cdp_command(
        &mut peer,
        43,
        "Runtime.evaluate",
        Some(&peer_session),
        json!({"expression":"releaseLegacyTarget(42)"}),
    )
    .await;
    let output = recv_until_match(&mut socket, |m| {
        nested_reply(m, &target.session_id, 42).is_some()
    })
    .await;
    assert!(
        !output.iter().any(|m| m.get("id").is_some()),
        "the inner reply cannot escape as a flat response: {output:?}"
    );
    let reply = nested_reply(output.last().unwrap(), &target.session_id, 42).unwrap();
    assert_eq!(reply["result"]["result"]["value"], 42);
    assert!(reply.get("sessionId").is_none());
    let tail = send_cdp_command(&mut socket, 72, "Browser.getVersion", None, json!({})).await;
    assert!(
        !tail
            .iter()
            .any(|m| nested_reply(m, &target.session_id, 42).is_some())
    );

    let _ = peer.close(None).await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn legacy_target_navigation_routes_deferred_reply_and_fetch_continuation_through_normal_tasks()
 {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_target_with_transport(&mut socket, 2, &context, false).await;
    send_nested_command(
        &mut socket,
        4,
        &target.session_id,
        40,
        "Fetch.enable",
        json!({"patterns":[{"urlPattern":"*legacy-held*"}]}),
    )
    .await;
    let mut output = send_cdp_command(&mut socket,70,"Target.sendMessageToTarget",None,json!({
        "sessionId":target.session_id,"message":json!({"id":42,"method":"Page.navigate","params":{"url":"http://fixture.invalid/legacy-held"}}).to_string(),
    })).await;
    if !output.iter().any(|m| {
        nested_message(m, &target.session_id).is_some_and(|m| m["method"] == "Fetch.requestPaused")
    }) {
        output.extend(
            recv_until_match(&mut socket, |m| {
                nested_message(m, &target.session_id)
                    .is_some_and(|m| m["method"] == "Fetch.requestPaused")
            })
            .await,
        );
    }
    assert!(
        !output
            .iter()
            .any(|m| nested_reply(m, &target.session_id, 42).is_some())
    );
    let request = output
        .iter()
        .find_map(|m| {
            nested_message(m, &target.session_id).filter(|m| m["method"] == "Fetch.requestPaused")
        })
        .unwrap()["params"]["requestId"]
        .clone();
    output.extend(send_cdp_command(&mut socket, 71, "Browser.getVersion", None, json!({})).await);
    assert!(
        !output
            .iter()
            .any(|m| nested_reply(m, &target.session_id, 42).is_some()),
        "the navigation remains intercepted"
    );
    output.extend(send_cdp_command(&mut socket,72,"Target.sendMessageToTarget",None,json!({
        "sessionId":target.session_id,"message":json!({"id":43,"method":"Fetch.fulfillRequest","params":{
            "requestId":request,"responseCode":200,"responseHeaders":[{"name":"Content-Type","value":"text/html"}],
            "body":"PGh0bWw+PHRpdGxlPmxlZ2FjeTwvdGl0bGU+PC9odG1sPg==",
        }}).to_string(),
    })).await);
    while ![42, 43].iter().all(|id| {
        output
            .iter()
            .any(|m| nested_reply(m, &target.session_id, *id).is_some())
    }) {
        output.extend(
            recv_until_match(&mut socket, |m| {
                nested_reply(m, &target.session_id, 42).is_some()
                    || nested_reply(m, &target.session_id, 43).is_some()
            })
            .await,
        );
    }
    for id in [42, 43] {
        let replies: Vec<_> = output
            .iter()
            .filter_map(|m| nested_reply(m, &target.session_id, id))
            .collect();
        assert_eq!(
            replies.len(),
            1,
            "one reply for each actual command: {output:?}"
        );
        assert!(
            replies[0].get("error").is_none(),
            "actual command must succeed: {replies:?}"
        );
        assert!(replies[0].get("sessionId").is_none());
        assert!(
            !output.iter().any(|m| m["id"] == id),
            "no flat reply or global id collision: {output:?}"
        );
    }
    assert!(
        nested_reply(
            output
                .iter()
                .find(|m| nested_reply(m, &target.session_id, 42).is_some())
                .unwrap(),
            &target.session_id,
            42
        )
        .unwrap()["result"]["loaderId"]
            .is_string()
    );
    let title = send_nested_command(
        &mut socket,
        73,
        &target.session_id,
        44,
        "Runtime.evaluate",
        json!({"expression":"document.title","returnByValue":true}),
    )
    .await;
    assert_eq!(
        nested_reply(title.last().unwrap(), &target.session_id, 44).unwrap()["result"]["result"]["value"],
        "legacy"
    );
    assert!(
        output.iter().all(|m| m.get("sessionId").is_none()),
        "non-flat events cannot escape: {output:?}"
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn legacy_target_held_runtime_does_not_block_detach_or_sibling_session() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_target_with_transport(&mut socket, 2, &context, false).await;
    let sibling = cdp_create_attached_target(&mut socket, 4, &context).await;
    send_cdp_command(&mut socket,70,"Target.sendMessageToTarget",None,json!({
        "sessionId":target.session_id,"message":json!({"id":42,"method":"Runtime.evaluate","params":{
            "expression":"new Promise(resolve=>{globalThis.legacyTargetHeld=true})","awaitPromise":true,
        }}).to_string(),
    })).await;
    let marker = send_nested_command(
        &mut socket,
        71,
        &target.session_id,
        43,
        "Runtime.evaluate",
        json!({"expression":"String(globalThis.legacyTargetHeld)","returnByValue":true}),
    )
    .await;
    assert_eq!(
        nested_reply(marker.last().unwrap(), &target.session_id, 43).unwrap()["result"]["result"]["value"],
        "true"
    );
    let detached = send_cdp_command(
        &mut socket,
        72,
        "Target.detachFromTarget",
        None,
        json!({"sessionId":target.session_id}),
    )
    .await;
    assert!(
        detached
            .iter()
            .any(|m| m["id"] == 72 && m.get("error").is_none())
    );
    assert_eq!(
        cdp_runtime_evaluate_string(&mut socket, &sibling.session_id, 73, "String(6*7)").await,
        "42"
    );
    let stale = send_cdp_command(&mut socket,74,"Target.sendMessageToTarget",None,json!({"sessionId":target.session_id,"message":"{\"id\":42,\"method\":\"Runtime.evaluate\"}"})).await;
    assert!(
        stale
            .iter()
            .any(|m| m["id"] == 74 && m["error"]["code"] == -32602)
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn nonflat_auto_attached_worker_keeps_its_parent_envelopes() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_target_with_transport(&mut socket, 2, &context, false).await;
    send_nested_command(&mut socket, 70, &target.session_id, 10, "Target.setAutoAttach",
        json!({"autoAttach":true,"waitForDebuggerOnStart":false,"flatten":false,"filter":[{"type":"worker"}]})).await;
    let mut output = send_nested_command(&mut socket, 71, &target.session_id, 11, "Runtime.evaluate",
        json!({"expression":"globalThis.transportWorker=new Worker(URL.createObjectURL(new Blob(['onmessage=()=>{}'],{type:'text/javascript'})));true","returnByValue":true})).await;
    let worker_attachment = |m: &serde_json::Value| {
        nested_message(m, &target.session_id).filter(|m| {
            m["method"] == "Target.attachedToTarget"
                && m["params"]["targetInfo"]["type"] == "worker"
        })
    };
    if !output.iter().any(|m| worker_attachment(m).is_some()) {
        output.extend(recv_until_match(&mut socket, |m| worker_attachment(m).is_some()).await);
    }
    let worker = output.iter().find_map(worker_attachment).unwrap()["params"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    output.extend(send_nested_command(&mut socket, 72, &target.session_id, 12, "Target.sendMessageToTarget",
        json!({"sessionId":worker,"message":json!({"id":13,"method":"Runtime.enable"}).to_string()})).await);
    let worker_message = |m: &serde_json::Value| {
        nested_message(m, &target.session_id).and_then(|m| nested_message(&m, &worker))
    };
    for method in [None, Some("Runtime.executionContextCreated")] {
        let matches = |m: &serde_json::Value| {
            worker_message(m).is_some_and(|m| match method {
                None => m["id"] == 13,
                Some(method) => m["method"] == method,
            })
        };
        if !output.iter().any(matches) {
            output.extend(recv_until_match(&mut socket, matches).await);
        }
    }
    assert!(
        output.iter().all(|m| m.get("sessionId").is_none()),
        "nested worker output escaped: {output:?}"
    );
    for m in output.iter().filter_map(worker_message) {
        assert!(
            m.get("sessionId").is_none(),
            "worker message must stay inside both envelopes: {m:?}"
        );
        assert!(
            m.get("error").is_none(),
            "worker enable must succeed: {m:?}"
        );
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
