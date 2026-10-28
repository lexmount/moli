use super::*;

fn nested_reply(
    message: &serde_json::Value,
    session_id: &str,
    id: u64,
) -> Option<serde_json::Value> {
    if message["method"] != "Target.receivedMessageFromTarget"
        || message["params"]["sessionId"] != session_id
    {
        return None;
    }
    let nested: serde_json::Value =
        serde_json::from_str(message["params"]["message"].as_str()?).ok()?;
    (nested["id"] == id).then_some(nested)
}

#[tokio::test]
async fn legacy_target_transport_acks_while_runtime_is_held_and_keeps_normal_call_routing() {
    let (addr, server) = spawn_test_protocol_server().await;
    let url = format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}");
    let (mut socket, _) = connect_async(&url).await.unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &context).await;
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
    let target = cdp_create_attached_target(&mut socket, 2, &context).await;
    send_cdp_command(
        &mut socket,
        4,
        "Fetch.enable",
        Some(&target.session_id),
        json!({"patterns":[{"urlPattern":"*legacy-held*"}]}),
    )
    .await;
    let mut output = send_cdp_command(&mut socket,70,"Target.sendMessageToTarget",None,json!({
        "sessionId":target.session_id,"message":json!({"id":42,"method":"Page.navigate","params":{"url":"http://fixture.invalid/legacy-held"}}).to_string(),
    })).await;
    if !output.iter().any(|m| m["method"] == "Fetch.requestPaused") {
        output
            .extend(recv_until_match(&mut socket, |m| m["method"] == "Fetch.requestPaused").await);
    }
    assert!(
        !output
            .iter()
            .any(|m| nested_reply(m, &target.session_id, 42).is_some())
    );
    let request = output
        .iter()
        .find(|m| m["method"] == "Fetch.requestPaused")
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
    assert_eq!(
        cdp_runtime_evaluate_string(&mut socket, &target.session_id, 73, "document.title").await,
        "legacy"
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
    let target = cdp_create_attached_target(&mut socket, 2, &context).await;
    let sibling = cdp_create_attached_target(&mut socket, 4, &context).await;
    send_cdp_command(&mut socket,70,"Target.sendMessageToTarget",None,json!({
        "sessionId":target.session_id,"message":json!({"id":42,"method":"Runtime.evaluate","params":{
            "expression":"new Promise(resolve=>{globalThis.legacyTargetHeld=true})","awaitPromise":true,
        }}).to_string(),
    })).await;
    let marker = cdp_runtime_evaluate_string(
        &mut socket,
        &target.session_id,
        71,
        "String(globalThis.legacyTargetHeld)",
    )
    .await;
    assert_eq!(marker, "true");
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
