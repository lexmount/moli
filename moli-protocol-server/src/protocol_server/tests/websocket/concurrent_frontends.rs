use super::*;

fn response_by_id(messages: &[serde_json::Value], id: u64) -> &serde_json::Value {
    messages
        .iter()
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("missing response {id}: {messages:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_browser_frontends_preserve_protocol_publication_order() {
    const FRONTENDS: usize = 25;
    const ROUNDS: u64 = 4;

    let (addr, server) = spawn_test_protocol_server().await;
    let endpoint = discover_browser_websocket_url(addr)
        .await
        .expect("discover browser endpoint");
    let barrier = tokio::sync::Barrier::new(FRONTENDS);
    let frontends = (0..FRONTENDS).map(|frontend| {
        let endpoint = &endpoint;
        let barrier = &barrier;
        async move {
            let (mut socket, _) = connect_async(endpoint)
                .await
                .expect("connect independent browser frontend");
            barrier.wait().await;
            for (id, method, params) in [
                (1, "Target.setDiscoverTargets", json!({ "discover": true })),
                (
                    2,
                    "Target.setAutoAttach",
                    json!({ "autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true }),
                ),
            ] {
                let messages = send_cdp_command(&mut socket, id, method, None, params).await;
                assert_eq!(response_by_id(&messages, id)["result"], json!({}));
            }
            for round in 0..ROUNDS {
                barrier.wait().await;
                // Deliberately reuse command IDs across independent clients.
                let id = 3 + round * 10;
                let created = send_cdp_command(
                    &mut socket,
                    id,
                    "Target.createTarget",
                    None,
                    json!({ "url": "data:text/html,<title>concurrent</title>target" }),
                )
                .await;
                let target = response_by_id(&created, id)["result"]["targetId"]
                    .as_str()
                    .expect("created target")
                    .to_owned();
                let attached = send_cdp_command(
                    &mut socket,
                    id + 1,
                    "Target.attachToTarget",
                    None,
                    json!({ "targetId": target, "flatten": true }),
                )
                .await;
                let session = response_by_id(&attached, id + 1)["result"]["sessionId"]
                    .as_str()
                    .expect("attached session")
                    .to_owned();
                for (offset, method) in [(2, "Page.enable"), (3, "Runtime.enable")] {
                    let messages = send_cdp_command(
                        &mut socket,
                        id + offset,
                        method,
                        Some(&session),
                        json!({}),
                    )
                    .await;
                    assert_eq!(response_by_id(&messages, id + offset)["result"], json!({}));
                }
                let navigated = send_cdp_command(
                    &mut socket,
                    id + 4,
                    "Page.navigate",
                    Some(&session),
                    json!({ "url": format!("data:text/html,<p>{frontend}-{round}</p>") }),
                )
                .await;
                assert!(response_by_id(&navigated, id + 4)["result"]["frameId"].is_string());
                let evaluated = send_cdp_command(
                    &mut socket,
                    id + 5,
                    "Runtime.evaluate",
                    Some(&session),
                    json!({ "expression": format!("{frontend} + {round}"), "returnByValue": true }),
                )
                .await;
                assert_eq!(
                    response_by_id(&evaluated, id + 5)["result"]["result"]["value"],
                    json!(frontend as u64 + round),
                );
                let detached = send_cdp_command(
                    &mut socket,
                    id + 6,
                    "Target.detachFromTarget",
                    None,
                    json!({ "sessionId": session }),
                )
                .await;
                assert_eq!(response_by_id(&detached, id + 6)["result"], json!({}));
                let closed = send_cdp_command(
                    &mut socket,
                    id + 7,
                    "Target.closeTarget",
                    None,
                    json!({ "targetId": target }),
                )
                .await;
                assert_eq!(response_by_id(&closed, id + 7)["result"]["success"], true);
            }
            socket.close(None).await.expect("close browser frontend");
        }
    });
    futures_util::future::join_all(frontends).await;
    abort_test_cdp_server(server).await;
}
