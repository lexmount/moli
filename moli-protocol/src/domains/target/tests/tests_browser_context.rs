use super::*;

/// cdp.target: getBrowserContexts
#[tokio::test(flavor = "multi_thread")]
async fn get_browser_contexts_returns_ids() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-X");
    ctx.conn
        .insert_browser_context(BrowserContext::new("BID-Y".into()));
    ctx.process_async(json!({"id": 5, "method": "Target.getBrowserContexts"}))
        .await;
    ctx.expect_result(5, json!({ "browserContextIds": ["BID-X", "BID-Y"] }), None);
}

/// cdp.target: createBrowserContext – success
#[tokio::test(flavor = "multi_thread")]
async fn create_browser_context_success() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 4, "method": "Target.createBrowserContext"}))
        .await;
    let bc_id = ctx.conn.browser_context.as_ref().unwrap().id.clone();
    ctx.expect_result(4, json!({ "browserContextId": bc_id }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_browser_context_rejects_persistent_partition_id() {
    for (index, partition_id, expected_message) in [
        (4, "tenant-a", "PersistentBrowserContextNotSupported"),
        (5, "default", "DefaultPersistentBrowserContextNotAllowed"),
        (6, "", "InvalidPersistentBrowserContextId"),
        (7, ".", "InvalidPersistentBrowserContextId"),
        (8, "..", "InvalidPersistentBrowserContextId"),
        (9, "tenant/a", "InvalidPersistentBrowserContextId"),
        (10, "tenant\\a", "InvalidPersistentBrowserContextId"),
    ] {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({
            "id": index,
            "method": "Target.createBrowserContext",
            "params": {
                "persistentPartitionId": partition_id
            }
        }))
        .await;
        ctx.expect_error(index, -32602, expected_message);
        assert!(
            ctx.conn.browser_contexts().next().is_none(),
            "{partition_id:?} must not create an ephemeral fallback context"
        );
    }
}

/// cdp.target: createBrowserContext – can create an inactive context when one already exists
#[tokio::test(flavor = "multi_thread")]
async fn create_browser_context_multiple_contexts() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 4, "method": "Target.createBrowserContext"}))
        .await;
    let first_id = ctx.conn.browser_context.as_ref().unwrap().id.clone();
    ctx.take_all();
    ctx.process_async(json!({"id": 5, "method": "Target.createBrowserContext"}))
        .await;
    let second_id = ctx.conn.inactive_browser_contexts[0].id.clone();
    assert_ne!(first_id, second_id);
    ctx.expect_result(5, json!({ "browserContextId": second_id }), None);
}

/// cdp.target: disposeBrowserContext – missing param
#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_missing_params() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 7, "method": "Target.disposeBrowserContext"}))
        .await;
    ctx.expect_error(7, -32602, "InvalidParams");
}

/// cdp.target: disposeBrowserContext – wrong id
#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_wrong_id() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 8, "method": "Target.disposeBrowserContext",
                       "params": {"browserContextId": "BID-10"}}))
        .await;
    ctx.expect_error(8, -32000, "Failed to find context with id BID-10");
}

/// cdp.target: disposeBrowserContext – success
#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_success() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-20");
    ctx.conn.download_behavior.set_browser_context(
        "BID-20".into(),
        "allow".into(),
        Some("/tmp/downloads".into()),
        true,
    );
    ctx.process_async(json!({"id": 9, "method": "Target.disposeBrowserContext",
                       "params": {"browserContextId": "BID-20"}}))
        .await;
    ctx.expect_result(9, json!({}), None);
    assert!(ctx.conn.browser_context.is_none());
    assert_eq!(
        ctx.conn.download_behavior,
        crate::conn::BrowserDownloadBehavior::default()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_emits_detached_events_for_attached_target() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-20".into());
    bc.set_active_target_id("TID-000000000A");
    bc.attach_active_session("SID-000000000A");
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({"id": 10, "method": "Target.disposeBrowserContext",
                       "params": {"browserContextId": "BID-20"}}))
        .await;
    ctx.expect_result(10, json!({}), None);
    ctx.expect_event(
        "Inspector.detached",
        Some(&json!({ "reason": "Render process gone." })),
    );
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000A",
            "sessionId": "SID-000000000A"
        })),
    );
    assert!(ctx.conn.browser_context.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_detaches_tab_session_and_clears_target_graph() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 101,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 101)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 102,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 102)["result"]["targetId"]
        .as_str()
        .expect("page target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    assert_eq!(ctx.conn.tab_target_count(), 1);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 103,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": tab_target_id,
            "flatten": true
        }
    }))
    .await;
    let tab_session_id = take_response_by_id(&mut ctx, 103)["result"]["sessionId"]
        .as_str()
        .expect("tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 104,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;

    ctx.expect_result(104, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": tab_target_id,
            "sessionId": tab_session_id
        })),
    );
    assert_eq!(ctx.conn.tab_target_count(), 0);
    assert!(
        ctx.conn
            .primary_page_target_id_for_tab_target_id(&tab_target_id)
            .is_none()
    );
    assert!(ctx.conn.session_route(Some(&tab_session_id)).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_detaches_page_session_and_clears_session_registry() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 105,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 105)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 106,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 106)["result"]["targetId"]
        .as_str()
        .expect("page target id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 107,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": page_target_id.clone(),
            "flatten": true
        }
    }))
    .await;
    let page_session_id = take_response_by_id(&mut ctx, 107)["result"]["sessionId"]
        .as_str()
        .expect("page session id")
        .to_owned();
    assert_eq!(
        ctx.conn.attached_sessions_for_target(&page_target_id),
        vec![page_session_id.clone()]
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 108,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;

    ctx.expect_result(108, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": page_target_id,
            "sessionId": page_session_id
        })),
    );
    assert_eq!(ctx.conn.session_route(Some(&page_session_id)), None);
    assert!(
        ctx.conn
            .attached_sessions_for_target(&page_target_id)
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_fans_out_page_session_teardown_and_target_destruction() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 109,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(109, json!({}), None);

    ctx.process_async(json!({
        "id": 110,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 110)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    let mut targets = Vec::new();
    for (offset, fragment) in [(0_u64, "first"), (10_u64, "second")] {
        let create_id = 111 + offset;
        ctx.process_async(json!({
            "id": create_id,
            "method": "Target.createTarget",
            "params": {
                "browserContextId": browser_context_id.clone(),
                "url": format!("about:blank#{fragment}")
            }
        }))
        .await;
        let target_id = take_response_by_id(&mut ctx, create_id)["result"]["targetId"]
            .as_str()
            .expect("page target id")
            .to_owned();

        let mut session_ids = Vec::new();
        for attach_index in 0_u64..2 {
            let attach_id = create_id + 1 + attach_index;
            ctx.process_async(json!({
                "id": attach_id,
                "method": "Target.attachToTarget",
                "params": {
                    "targetId": target_id.clone(),
                    "flatten": true
                }
            }))
            .await;
            let session_id = take_response_by_id(&mut ctx, attach_id)["result"]["sessionId"]
                .as_str()
                .expect("page session id")
                .to_owned();
            session_ids.push(session_id);
        }
        targets.push((target_id, session_ids));
    }

    for (id, session_id) in [
        (140_u64, targets[0].1[0].as_str()),
        (141_u64, targets[1].1[1].as_str()),
    ] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "Inspector.enable"
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 142,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let messages = ctx.take_all();
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(142) && message["result"] == json!({})),
        "dispose response missing: {messages:?}"
    );

    let mut expected_session_ids = targets
        .iter()
        .flat_map(|(_, session_ids)| session_ids.iter().cloned())
        .collect::<Vec<_>>();
    expected_session_ids.sort();

    let mut inspector_session_ids = messages
        .iter()
        .filter(|message| message["method"] == json!("Inspector.detached"))
        .map(|message| {
            assert_eq!(message["params"]["reason"], json!("Render process gone."));
            message["sessionId"]
                .as_str()
                .expect("Inspector.detached session id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    inspector_session_ids.sort();
    assert_eq!(inspector_session_ids, expected_session_ids);

    let mut detached_sessions = messages
        .iter()
        .filter(|message| message["method"] == json!("Target.detachedFromTarget"))
        .filter_map(|message| {
            let target_id = message["params"]["targetId"].as_str()?;
            targets
                .iter()
                .any(|(expected_target_id, _)| expected_target_id == target_id)
                .then(|| {
                    (
                        target_id.to_owned(),
                        message["params"]["sessionId"]
                            .as_str()
                            .expect("detached session id")
                            .to_owned(),
                    )
                })
        })
        .collect::<Vec<_>>();
    detached_sessions.sort();
    let mut expected_detached_sessions = targets
        .iter()
        .flat_map(|(target_id, session_ids)| {
            session_ids
                .iter()
                .map(|session_id| (target_id.clone(), session_id.clone()))
        })
        .collect::<Vec<_>>();
    expected_detached_sessions.sort();
    assert_eq!(detached_sessions, expected_detached_sessions);

    let mut destroyed_target_ids = messages
        .iter()
        .filter(|message| message["method"] == json!("Target.targetDestroyed"))
        .filter_map(|message| {
            message["params"]["targetId"]
                .as_str()
                .map(str::to_owned)
                .filter(|target_id| {
                    targets
                        .iter()
                        .any(|(expected_target_id, _)| expected_target_id == target_id)
                })
        })
        .collect::<Vec<_>>();
    destroyed_target_ids.sort();
    let mut expected_target_ids = targets
        .iter()
        .map(|(target_id, _)| target_id.clone())
        .collect::<Vec<_>>();
    expected_target_ids.sort();
    assert_eq!(destroyed_target_ids, expected_target_ids);

    let last_inspector_index = messages
        .iter()
        .rposition(|message| message["method"] == json!("Inspector.detached"))
        .expect("Inspector.detached events");
    let first_target_detach_index = messages
        .iter()
        .position(|message| message["method"] == json!("Target.detachedFromTarget"))
        .expect("Target.detachedFromTarget events");
    assert!(
        last_inspector_index < first_target_detach_index,
        "Chromium emits all context Inspector.detached events before target teardown: {messages:?}"
    );

    assert!(
        ctx.conn
            .browser_context_by_id(&browser_context_id)
            .is_none()
    );
    for (target_id, session_ids) in targets {
        assert!(ctx.conn.attached_sessions_for_target(&target_id).is_empty());
        for session_id in session_ids {
            assert!(ctx.conn.session_route(Some(&session_id)).is_none());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_tears_down_worker_sessions_and_targets() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc(&mut ctx, "BID-worker-dispose");
    push_shared_worker_target(
        &mut ctx,
        moli_shared_worker::SharedWorkerInstanceId::from_u64(71),
        "TID-shared-worker-dispose",
        "https://example.test/shared-worker.js",
        "shared-worker-dispose",
        None,
    );
    push_service_worker_target(
        &mut ctx,
        72,
        "TID-service-worker-dispose",
        "https://example.test/service-worker.js",
        "https://example.test/",
        None,
    );

    ctx.process_async(json!({
        "id": 143,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(143, json!({}), None);
    ctx.sent.clear();

    let mut target_sessions = Vec::new();
    for (id, target_id) in [
        (144_u64, "TID-shared-worker-dispose"),
        (145_u64, "TID-service-worker-dispose"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Target.attachToTarget",
            "params": {
                "targetId": target_id,
                "flatten": true
            }
        }))
        .await;
        let session_id = take_response_by_id(&mut ctx, id)["result"]["sessionId"]
            .as_str()
            .expect("worker session id")
            .to_owned();
        target_sessions.push((target_id.to_owned(), session_id));
    }
    let shared_worker_session_id = target_sessions[0].1.clone();
    ctx.conn
        .shared_worker_target_for_session_mut(Some(&shared_worker_session_id))
        .expect("shared worker target session")
        .register_pending_inspector_await(
            &shared_worker_session_id,
            9_001,
            Some(&shared_worker_session_id),
            None,
        );
    let service_worker_session_id = target_sessions[1].1.clone();
    ctx.conn
        .service_worker_target_for_session_mut(Some(&service_worker_session_id))
        .expect("service worker target session")
        .register_pending_inspector_await(
            &service_worker_session_id,
            9_002,
            Some(&service_worker_session_id),
            None,
        );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 146,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-worker-dispose" }
    }))
    .await;
    let messages = ctx.take_all();
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(146) && message["result"] == json!({})),
        "dispose response missing: {messages:?}"
    );

    let mut expected_session_ids = target_sessions
        .iter()
        .map(|(_, session_id)| session_id.clone())
        .collect::<Vec<_>>();
    expected_session_ids.sort();
    let mut inspector_session_ids = messages
        .iter()
        .filter(|message| message["method"] == json!("Inspector.detached"))
        .map(|message| {
            assert_eq!(message["params"]["reason"], json!("Render process gone."));
            message["sessionId"]
                .as_str()
                .expect("Inspector.detached worker session")
                .to_owned()
        })
        .collect::<Vec<_>>();
    inspector_session_ids.sort();
    assert_eq!(inspector_session_ids, expected_session_ids);

    let mut pending_failure_indices = Vec::new();
    for (request_id, session_id) in [
        (9_001_u64, shared_worker_session_id),
        (9_002_u64, service_worker_session_id),
    ] {
        let index = messages
            .iter()
            .position(|message| message["id"] == json!(request_id))
            .expect("pending worker Inspector request failure");
        assert_eq!(messages[index]["sessionId"], json!(session_id));
        assert_eq!(
            messages[index]["error"]["message"],
            json!("Browser context disposed")
        );
        pending_failure_indices.push(index);
    }

    let mut detached_target_sessions = messages
        .iter()
        .filter(|message| message["method"] == json!("Target.detachedFromTarget"))
        .filter_map(|message| {
            Some((
                message["params"]["targetId"].as_str()?.to_owned(),
                message["params"]["sessionId"].as_str()?.to_owned(),
            ))
        })
        .collect::<Vec<_>>();
    detached_target_sessions.sort();
    let mut expected_target_sessions = target_sessions.clone();
    expected_target_sessions.sort();
    assert_eq!(detached_target_sessions, expected_target_sessions);

    let mut destroyed_target_ids = messages
        .iter()
        .filter(|message| message["method"] == json!("Target.targetDestroyed"))
        .filter_map(|message| message["params"]["targetId"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    destroyed_target_ids.sort();
    let mut expected_target_ids = target_sessions
        .iter()
        .map(|(target_id, _)| target_id.clone())
        .collect::<Vec<_>>();
    expected_target_ids.sort();
    assert_eq!(destroyed_target_ids, expected_target_ids);

    let first_inspector_index = messages
        .iter()
        .position(|message| message["method"] == json!("Inspector.detached"))
        .expect("worker Inspector.detached events");
    let last_inspector_index = messages
        .iter()
        .rposition(|message| message["method"] == json!("Inspector.detached"))
        .expect("worker Inspector.detached events");
    let first_target_detach_index = messages
        .iter()
        .position(|message| message["method"] == json!("Target.detachedFromTarget"))
        .expect("worker Target.detachedFromTarget events");
    assert!(
        pending_failure_indices
            .into_iter()
            .all(|index| index < first_inspector_index)
    );
    assert!(last_inspector_index < first_target_detach_index);

    assert!(
        ctx.conn
            .browser_context_by_id("BID-worker-dispose")
            .is_none()
    );
    for (target_id, session_id) in target_sessions {
        assert!(ctx.conn.session_route(Some(&session_id)).is_none());
        assert_eq!(ctx.conn.target_registry_host_kind(&target_id), None);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn puppeteer_dispose_browser_context_closes_only_belonging_targets() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 130,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(130, json!({}), None);

    ctx.process_async(json!({
        "id": 131,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let first_browser_context_id = take_response_by_id(&mut ctx, 131)["result"]["browserContextId"]
        .as_str()
        .expect("first browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 132,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": first_browser_context_id,
            "url": "about:blank#first"
        }
    }))
    .await;
    let first_page_target_id = take_response_by_id(&mut ctx, 132)["result"]["targetId"]
        .as_str()
        .expect("first page target id")
        .to_owned();
    ctx.take_first_matching("first context page targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(first_page_target_id)
    });

    ctx.process_async(json!({
        "id": 133,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let second_browser_context_id =
        take_response_by_id(&mut ctx, 133)["result"]["browserContextId"]
            .as_str()
            .expect("second browser context id")
            .to_owned();

    ctx.process_async(json!({
        "id": 134,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": second_browser_context_id,
            "url": "about:blank#second"
        }
    }))
    .await;
    let second_page_target_id = take_response_by_id(&mut ctx, 134)["result"]["targetId"]
        .as_str()
        .expect("second page target id")
        .to_owned();
    ctx.take_first_matching("second context page targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(second_page_target_id)
    });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 135,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": first_browser_context_id }
    }))
    .await;

    ctx.expect_result(135, json!({}), None);
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(second_page_target_id)
        }),
        "disposing one Puppeteer browser context must not destroy sibling context targets: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .browser_context_by_id(&first_browser_context_id)
            .is_none()
    );
    assert!(
        ctx.conn
            .browser_context_by_id(&second_browser_context_id)
            .is_some()
    );

    ctx.process_async(json!({
        "id": 136,
        "method": "Target.getTargets"
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 136);
    let target_infos = targets["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos");
    assert!(
        target_infos
            .iter()
            .all(|info| info["targetId"] != json!(first_page_target_id)),
        "disposed context target must not remain discoverable: {targets:?}"
    );
    assert!(
        target_infos
            .iter()
            .any(|info| info["targetId"] == json!(second_page_target_id)
                && info["browserContextId"] == json!(second_browser_context_id)),
        "sibling context target should remain discoverable: {targets:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_aborts_paused_request_stage_navigation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>dispose-bc</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-9".into());
    bc.set_active_target_id("TID-000000000A");
    bc.attach_active_session("SID-1");
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 20,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(20, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let network_id = paused["params"]["networkId"].clone();

    ctx.process_async(json!({
        "id": 22,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-9" }
    }))
    .await;
    ctx.expect_result(22, json!({}), None);

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Browser context disposed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 21);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Browser context disposed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.detached");

    let target = ctx.take_one();
    assert_eq!(target["method"], "Target.detachedFromTarget");
    assert_eq!(target["params"]["targetId"], "TID-000000000A");

    assert!(ctx.conn.browser_context.is_none());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_aborts_root_session_navigation_without_target_session() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>dispose-root-session</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-root-dispose".into());
    bc.set_active_target_id("TID-root-dispose");
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 23,
        "method": "Fetch.enable"
    }))
    .await;
    ctx.expect_result(23, json!({}), None);

    ctx.process_async(json!({
        "id": 24,
        "method": "Page.navigate",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let paused = take_main_document_request_pause(&mut ctx);
    let network_id = paused["params"]["networkId"].clone();
    ctx.conn.register_pending_inspector_await(9_003, None);

    ctx.process_async(json!({
        "id": 25,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-root-dispose" }
    }))
    .await;
    ctx.expect_result(25, json!({}), None);

    let pending_error_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(9_003))
        .expect("root pending Inspector request failure");
    let pending_error = ctx.sent.remove(pending_error_index);
    assert!(pending_error.get("sessionId").is_none());
    assert_eq!(
        pending_error["error"]["message"],
        json!("Browser context disposed")
    );

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert!(failed.get("sessionId").is_none());
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Browser context disposed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 24);
    assert!(error.get("sessionId").is_none());
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Browser context disposed");
    assert!(ctx.sent.is_empty());
    assert!(ctx.conn.browser_context.is_none());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_aborts_paused_runtime_fetch_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn data() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "payload")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/data", any(data)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let data_url = format!("http://{addr}/data");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-9".into());
    bc.set_active_target_id("TID-000000000A");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("Fetch fixture target")
        .enable_primary_network_events();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(23, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 24,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(24, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 25,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
  fetch("{}").catch(() => {{}});
  return "scheduled";
}})()"#, data_url)
        }
    }))
    .await;
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(25))
        .expect("runtime evaluate response");
    ctx.sent.remove(pos);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(data_url)
        })
        .cloned()
        .expect("subresource fetch requestPaused event");
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 26,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-9" }
    }))
    .await;
    ctx.expect_result(26, json!({}), None);

    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        }),
        "Chromium does not synthesize a subresource loadingFailed event while disposing its BrowserContext"
    );
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.detached") && message["sessionId"] == json!("SID-1")
    }));

    assert!(ctx.conn.browser_context.is_none());

    server.abort();
}
