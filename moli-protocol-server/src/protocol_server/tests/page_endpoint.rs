use super::*;

#[tokio::test]
async fn page_and_tab_ids_are_random_per_instance_and_shared_with_cdp() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (other_addr, other_server) = spawn_test_protocol_server().await;
    let (page_id, tab_id) = discovered_startup_target_ids(addr).await;
    let (other_page_id, other_tab_id) = discovered_startup_target_ids(other_addr).await;
    let mut ids = std::collections::HashSet::new();
    for id in [&page_id, &tab_id, &other_page_id, &other_tab_id] {
        assert_chromium_target_id(id);
        assert!(
            ids.insert(id.to_owned()),
            "target identities must not be shared"
        );
    }
    assert_eq!(
        discovered_startup_target_ids(addr).await,
        (page_id.clone(), tab_id.clone())
    );

    let (mut browser, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    for (id, kind) in [(&page_id, "page"), (&tab_id, "tab")] {
        let messages = send_cdp_command(
            &mut browser,
            1,
            "Target.getTargetInfo",
            None,
            json!({"targetId": id}),
        )
        .await;
        let response = messages.iter().find(|message| message["id"] == 1).unwrap();
        assert_eq!(response["result"]["targetInfo"]["targetId"], id.as_str());
        assert_eq!(response["result"]["targetInfo"]["type"], kind);
        let (mut target, _) = connect_async(format!("ws://{addr}/devtools/page/{id}"))
            .await
            .unwrap();
        let messages =
            send_cdp_command(&mut target, 1, "Target.getTargetInfo", None, json!({})).await;
        let response = messages.iter().find(|message| message["id"] == 1).unwrap();
        assert_eq!(response["result"]["targetInfo"]["targetId"], id.as_str());
        target.close(None).await.unwrap();
    }
    for _ in 0..3 {
        let (status, body) = fetch_server_response(addr, "PUT", "/json/new?about:blank").await;
        assert_eq!(status, 200);
        let target: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = target["id"].as_str().unwrap();
        assert_chromium_target_id(id);
        assert!(ids.insert(id.to_owned()));
        assert_eq!(
            target["webSocketDebuggerUrl"],
            format!("ws://{addr}/devtools/page/{id}")
        );
        let messages = send_cdp_command(
            &mut browser,
            2,
            "Target.getTargetInfo",
            None,
            json!({"targetId": id}),
        )
        .await;
        assert_eq!(
            messages.iter().find(|message| message["id"] == 2).unwrap()["result"]["targetInfo"]["targetId"],
            id
        );
    }
    browser.close(None).await.unwrap();
    server.abort();
    other_server.abort();
}

#[tokio::test]
async fn page_websocket_rejects_legacy_guessed_and_foreign_target_ids() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (other_addr, other_server) = spawn_test_protocol_server().await;
    let (page_id, tab_id) = discovered_startup_target_ids(addr).await;
    let (other_page_id, other_tab_id) = discovered_startup_target_ids(other_addr).await;
    for id in [
        "moli-default",
        "moli-default-tab",
        "TID-1",
        "TAB-1",
        "00000000000000000000000000000001",
        &other_page_id,
        &other_tab_id,
    ] {
        assert_ne!(id, page_id);
        assert_ne!(id, tab_id);
        assert_eq!(
            rejected_websocket_status(format!("ws://{addr}/devtools/page/{id}")).await,
            404,
            "{id}"
        );
        for action in ["activate", "close"] {
            let (status, _) =
                fetch_server_response(addr, "GET", &format!("/json/{action}/{id}")).await;
            assert_eq!(status, 404, "{action} {id}");
        }
    }
    server.abort();
    other_server.abort();
}

#[tokio::test]
async fn page_target_ids_change_between_instances_at_the_same_address() {
    let first = build_router(test_state());
    let second = build_router(test_state());
    let before = request_json_with_router(first, Method::GET, "/json/list?for_tab").await;
    let after = request_json_with_router(second.clone(), Method::GET, "/json/list?for_tab").await;
    assert_eq!(
        after,
        request_json_with_router(second.clone(), Method::GET, "/json/list?for_tab").await
    );
    for old in before.as_array().unwrap() {
        let old_id = old["id"].as_str().unwrap();
        assert_chromium_target_id(old_id);
        assert!(
            after
                .as_array()
                .unwrap()
                .iter()
                .all(|target| target["id"] != old_id)
        );
        let (status, _) = request_status_and_text_with_router(
            second.clone(),
            Method::GET,
            &format!("/json/activate/{old_id}"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
