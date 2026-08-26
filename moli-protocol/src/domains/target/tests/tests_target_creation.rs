use super::*;

fn stored_cookie(name: &str, value: &str) -> moli_cookie_jar::StoredCookie {
    moli_cookie_jar::StoredCookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: "example.com".to_owned(),
        host_only: false,
        path: "/".to_owned(),
        secure: false,
        http_only: false,
        expires: None,
        same_site: moli_cookie_jar::StoredCookieSameSite::Unspecified,
        priority: None,
        partition_key: None,
        source_scheme: moli_cookie_jar::StoredCookieSourceScheme::NonSecure,
        source_port: -1,
        creation_index: 0,
        last_access_index: 0,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_clears_stale_crash_state() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({"id": 9, "method": "Target.createTarget",
                       "params": {"browserContextId": "BID-9", "url": "about:blank"}}))
        .await;
    ctx.expect_event("Target.targetCreated", None);
    let target_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_target_id_owned())
        .expect("target id after create");
    ctx.expect_result(9, json!({ "targetId": target_id }), None);

    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}

/// cdp.target: createTarget – no existing browser context, creates one
#[tokio::test(flavor = "multi_thread")]
async fn create_target_creates_browser_context_if_none() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                       "params": {"url": "about:blank"}}))
        .await;
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let tid = bc.active_target_id_owned().unwrap();
    let tab_target_id = tab_id_for_page(&ctx, &tid);
    assert_eq!(
        ctx.conn.tab_target_id_for_page_target_id(&tid),
        Some(tab_target_id.as_str())
    );
    assert_eq!(
        ctx.conn
            .primary_page_target_id_for_tab_target_id(&tab_target_id),
        Some(tid.as_str())
    );
    assert_eq!(ctx.conn.tab_target_count(), 1);
    ctx.expect_event("Target.targetCreated", None);
    ctx.expect_result(10, json!({ "targetId": tid }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_for_tab_returns_the_stable_tab_host() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 10000,
        "method": "Target.createTarget",
        "params": { "url": "about:blank", "forTab": true }
    }))
    .await;

    let tab_target_id = take_response_by_id(&mut ctx, 10000)["result"]["targetId"]
        .as_str()
        .expect("created tab target id")
        .to_owned();
    let page_target_id = ctx
        .conn
        .primary_page_target_id_for_tab_target_id(&tab_target_id)
        .expect("created tab must expose its primary page")
        .to_owned();
    assert_ne!(tab_target_id, page_target_id);

    ctx.process_async(json!({
        "id": 10001,
        "method": "Target.getTargetInfo",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;
    let tab_info = take_response_by_id(&mut ctx, 10001);
    assert_eq!(tab_info["result"]["targetInfo"]["type"], json!("tab"));

    ctx.process_async(json!({
        "id": 10002,
        "method": "Target.getTargetInfo",
        "params": { "targetId": page_target_id.clone() }
    }))
    .await;
    let page_info = take_response_by_id(&mut ctx, 10002);
    assert_eq!(page_info["result"]["targetInfo"]["type"], json!("page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_targets_default_filter_excludes_tab() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 10010,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 10010)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10011,
        "method": "Target.getTargets"
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 10011);
    let target_infos = response["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos");
    assert_eq!(target_infos.len(), 1);
    assert_eq!(target_infos[0]["targetId"], json!(target_id));
    assert_eq!(target_infos[0]["type"], json!("page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_targets_tab_filter_returns_stable_tab_target() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 10020,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 10020)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10021,
        "method": "Target.getTargets",
        "params": {
            "filter": [{ "type": "tab" }]
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 10021);
    let target_infos = response["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos");
    assert_eq!(target_infos.len(), 1);
    assert_eq!(target_infos[0]["targetId"], json!(tab_target_id));
    assert_eq!(target_infos[0]["type"], json!("tab"));
    assert_eq!(target_infos[0]["attached"], json!(false));
    assert!(target_infos[0]["browserContextId"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_targets_catchall_includes_tab_and_page_with_attached_state() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 10025,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 10025)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10026,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;
    take_response_by_id(&mut ctx, 10026);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10027,
        "method": "Target.getTargets",
        "params": { "filter": [{}] }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 10027);
    let target_infos = response["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos");
    let tab_info = target_infos
        .iter()
        .find(|info| info["targetId"] == json!(tab_target_id))
        .unwrap_or_else(|| panic!("missing tab targetInfo: {target_infos:?}"));
    assert_eq!(tab_info["type"], json!("tab"));
    assert_eq!(tab_info["attached"], json!(true));

    let page_info = target_infos
        .iter()
        .find(|info| info["targetId"] == json!(page_target_id))
        .unwrap_or_else(|| panic!("missing page targetInfo: {target_infos:?}"));
    assert_eq!(page_info["type"], json!("page"));
    assert_eq!(
        page_info["attached"],
        json!(false),
        "attaching the tab control-plane target must not attach the page execution target"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_returns_tab_target_info() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 10030,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 10030)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10031,
        "method": "Target.getTargetInfo",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 10031);
    assert_eq!(
        response["result"]["targetInfo"]["targetId"],
        json!(tab_target_id)
    );
    assert_eq!(response["result"]["targetInfo"]["type"], json!("tab"));
    assert_eq!(response["result"]["targetInfo"]["attached"], json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_discover_targets_catchall_reports_tab_and_page() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 10040,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 10040)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10041,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{}]
        }
    }))
    .await;

    ctx.expect_result(10041, json!({}), None);
    let tab_created = ctx.take_first_matching("tab targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
    });
    assert_eq!(tab_created["params"]["targetInfo"]["type"], json!("tab"));
    let page_created = ctx.take_first_matching("page targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    assert_eq!(page_created["params"]["targetInfo"]["type"], json!("page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_discover_targets_does_not_replay_reported_targets() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 10042,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    take_response_by_id(&mut ctx, 10042);
    ctx.sent.clear();

    for command_id in [10043, 10044] {
        ctx.process_async(json!({
            "id": command_id,
            "method": "Target.setDiscoverTargets",
            "params": {
                "discover": true,
                "filter": [{}]
            }
        }))
        .await;
        ctx.expect_result(command_id, json!({}), None);
        if command_id == 10043 {
            assert!(
                ctx.sent
                    .iter()
                    .any(|message| message["method"] == json!("Target.targetCreated")),
                "first discovery should report existing targets"
            );
            ctx.sent.clear();
        }
    }

    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "second discovery should not replay already reported targets: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_targets_without_filter_reuses_discovery_filter() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 10045,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 10045)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10046,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(10046, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10047,
        "method": "Target.getTargets",
        "params": {}
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 10047);
    let targets = response["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos");
    assert!(
        targets.iter().any(|target| {
            target["type"] == json!("tab") && target["targetId"] == json!(tab_target_id)
        }),
        "discovery catch-all filter should make getTargets include tab: {response:?}"
    );
    assert!(
        targets.iter().any(|target| {
            target["type"] == json!("page") && target["targetId"] == json!(page_target_id)
        }),
        "getTargets should still include page: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_discover_targets_false_rejects_non_empty_filter() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 10048,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": false,
            "filter": [{}]
        }
    }))
    .await;

    ctx.expect_error(
        10048,
        -32602,
        "Filter should not be present with `discover` is off",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn target_destroyed_uses_reported_hosts_after_discovery_filter_changes() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 10049,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 10049)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10050,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(10050, json!({}), None);
    ctx.take_first_matching("reported tab", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
    });
    ctx.take_first_matching("reported page", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10051,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{ "type": "service_worker" }]
        }
    }))
    .await;
    ctx.expect_result(10051, json!({}), None);
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "filter narrowing should not replay page/tab targets: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 10052,
        "method": "Target.closeTarget",
        "params": { "targetId": page_target_id }
    }))
    .await;
    ctx.expect_result(10052, json!({ "success": true }), None);
    ctx.take_first_matching("destroyed tab", |message| {
        message["method"] == json!("Target.targetDestroyed")
            && message["params"]["targetId"] == json!(tab_target_id)
    });
    ctx.take_first_matching("destroyed page", |message| {
        message["method"] == json!("Target.targetDestroyed")
            && message["params"]["targetId"] == json!(page_target_id)
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_target_session_discovery_receives_lifecycle_events_with_session_id() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 10053,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let attached = ctx.take_first_matching("browser attached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("browser target session id")
        .to_owned();
    ctx.expect_result(
        10053,
        json!({ "sessionId": browser_session_id.as_str() }),
        None,
    );

    ctx.process_async(json!({
        "id": 10054,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let first_page_target_id = take_response_by_id(&mut ctx, 10054)["result"]["targetId"]
        .as_str()
        .expect("first target id")
        .to_owned();
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "target creation before discovery should not emit targetCreated: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 10055,
        "sessionId": browser_session_id.as_str(),
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(10055, json!({}), Some(&browser_session_id));
    ctx.take_first_matching("session initial targetCreated", |message| {
        message["sessionId"] == json!(browser_session_id)
            && message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(first_page_target_id)
    });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10056,
        "method": "Target.createTarget",
        "params": { "url": "about:blank#session-discovery" }
    }))
    .await;
    let second_page_target_id = take_response_by_id(&mut ctx, 10056)["result"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.take_first_matching("session targetCreated after create", |message| {
        message["sessionId"] == json!(browser_session_id)
            && message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(second_page_target_id)
    });
    assert!(
        !ctx.sent.iter().any(|message| {
            message.get("sessionId").is_none()
                && message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] == json!(second_page_target_id)
        }),
        "session discovery event must not be emitted as a root Target event: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10057,
        "method": "Target.closeTarget",
        "params": { "targetId": second_page_target_id }
    }))
    .await;
    ctx.expect_result(10057, json!({ "success": true }), None);
    ctx.take_first_matching("session targetDestroyed after close", |message| {
        message["sessionId"] == json!(browser_session_id)
            && message["method"] == json!("Target.targetDestroyed")
            && message["params"]["targetId"] == json!(second_page_target_id)
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn detached_browser_target_session_stops_receiving_discovery_events() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 10058,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let attached = ctx.take_first_matching("browser attached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("browser target session id")
        .to_owned();
    ctx.expect_result(
        10058,
        json!({ "sessionId": browser_session_id.as_str() }),
        None,
    );

    ctx.process_async(json!({
        "id": 10059,
        "sessionId": browser_session_id.as_str(),
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(10059, json!({}), Some(&browser_session_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10060,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": browser_session_id.as_str() }
    }))
    .await;
    ctx.expect_result(10060, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10061,
        "method": "Target.createTarget",
        "params": { "url": "about:blank#after-browser-session-detach" }
    }))
    .await;
    let created_target_id = take_response_by_id(&mut ctx, 10061)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!(browser_session_id)
                && message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] == json!(created_target_id)
        }),
        "detached browser session must not receive discovery events: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_target_session_discovery_receives_target_info_changed_after_navigation() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 10062,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let attached = ctx.take_first_matching("browser attached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("browser target session id")
        .to_owned();
    ctx.expect_result(
        10062,
        json!({ "sessionId": browser_session_id.as_str() }),
        None,
    );

    ctx.process_async(json!({
        "id": 10063,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 10063)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10064,
        "sessionId": browser_session_id.as_str(),
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(10064, json!({}), Some(&browser_session_id));
    ctx.take_first_matching("session initial targetCreated", |message| {
        message["sessionId"] == json!(browser_session_id)
            && message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10065,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id }
    }))
    .await;
    let target_session_id = take_response_by_id(&mut ctx, 10065)["result"]["sessionId"]
        .as_str()
        .expect("attached target session id")
        .to_owned();
    ctx.take_first_matching("page attached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    });
    let attached_changed =
        ctx.take_first_matching("session targetInfoChanged after attach", |message| {
            message["sessionId"] == json!(browser_session_id)
                && message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        });
    assert_eq!(
        attached_changed["params"]["targetInfo"]["attached"],
        json!(true)
    );
    ctx.sent.clear();

    let url = "data:text/html,<title>Session Discovery Navigation</title><main>navigation</main>";
    ctx.process_async(json!({
        "id": 10066,
        "sessionId": target_session_id.as_str(),
        "method": "Page.navigate",
        "params": { "url": url }
    }))
    .await;

    let changed = ctx
        .wait_for_scheduler_message(
            "title-aware targetInfoChanged after navigation",
            |message| {
                message["sessionId"] == json!(browser_session_id)
                    && message["method"] == json!("Target.targetInfoChanged")
                    && message["params"]["targetInfo"]["targetId"] == json!(target_id)
                    && message["params"]["targetInfo"]["title"]
                        == json!("Session Discovery Navigation")
            },
        )
        .await;
    assert_eq!(changed["params"]["targetInfo"]["url"], json!(url));
    assert_eq!(
        changed["params"]["targetInfo"]["title"],
        json!("Session Discovery Navigation")
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message.get("sessionId").is_none()
                && message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        }),
        "session discovery targetInfoChanged must not be emitted as a root event: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_emits_popup_target_created_from_runtime_work() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup",
        "TID-opener",
        "<main>popup opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('https://example.com/popup?from=runtime', '_blank') !== null"
        }
    }))
    .await;

    ctx.expect_result(
        12,
        json!({
            "result": {
                "type": "boolean",
                "value": true
            }
        }),
        None,
    );
    let created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .expect("targetCreated event should be recorded");
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Page.windowOpen")),
        "Page.windowOpen must remain gated by Page.enable: {:?}",
        ctx.sent
    );
    assert!(
        created["params"]["targetInfo"]["moliPopupId"].is_null(),
        "internal popup id must not be exposed on the CDP wire: {created:?}"
    );
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "type": "page",
                "url": "https://example.com/popup?from=runtime",
                "browserContextId": "BID-popup",
                "attached": false,
                "canAccessOpener": true,
                "openerId": "TID-opener",
                "openerFrameId": "TID-opener"
            }
        })),
    );
    let (targets_result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetTargets(
            crate::devtools_runtime::DevToolsGetTargetsCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::Cdp,
                    session_id: None,
                    target_id: None,
                    browser_context_id: None,
                },
                root: None,
                max_depth: None,
                filter: None,
            },
        ))
        .await
        .into_parts();
    let crate::devtools_runtime::DevToolsCommandResult::GetTargets(targets) =
        targets_result.expect("GetTargets should return typed targets")
    else {
        panic!("GetTargets returned unexpected result");
    };
    let popup_info = targets
        .targets
        .iter()
        .find(|target| target.url == "https://example.com/popup?from=runtime")
        .expect("typed GetTargets should include popup target");
    assert_eq!(popup_info.moli_popup_id, Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_emits_page_event_before_creating_popup_target() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-page-window-open",
        "TID-page-window-opener",
        "<main>popup opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 120,
        "method": "Page.enable"
    }))
    .await;
    ctx.expect_result(120, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 121,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('https://example.com/page-window-open', '_blank') !== null",
            "userGesture": true
        }
    }))
    .await;

    let window_open_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.windowOpen"))
        .unwrap_or_else(|| panic!("missing Page.windowOpen event: {:?}", ctx.sent));
    let target_created_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Target.targetCreated"))
        .unwrap_or_else(|| panic!("missing Target.targetCreated event: {:?}", ctx.sent));
    let evaluate_response_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(121))
        .unwrap_or_else(|| panic!("missing Runtime.evaluate response: {:?}", ctx.sent));
    assert!(
        window_open_index < evaluate_response_index && window_open_index < target_created_index,
        "Page.windowOpen must precede the command response and popup target creation: {:?}",
        ctx.sent
    );
    ctx.expect_result(
        121,
        json!({
            "result": {
                "type": "boolean",
                "value": true
            }
        }),
        None,
    );
    assert_eq!(
        ctx.sent[window_open_index],
        json!({
            "method": "Page.windowOpen",
            "params": {
                "url": "https://example.com/page-window-open",
                "windowName": "_blank",
                "windowFeatures": [
                    "menubar",
                    "toolbar",
                    "status",
                    "scrollbars",
                    "resizable"
                ],
                "userGesture": true
            }
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn timer_window_open_emits_page_event_from_runtime_work() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-timer-window-open",
        "TID-timer-window-opener",
        "<main>timer popup opener</main>",
    )
    .await;
    ctx.process_async(json!({
        "id": 122,
        "method": "Page.enable"
    }))
    .await;
    ctx.expect_result(122, json!({}), None);
    ctx.sent.clear();

    ctx.process_and_wait_for_response_async(json!({
        "id": 123,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "setTimeout(() => window.open('about:blank#timer-window-open', '_blank'), 0); 'scheduled'"
        }
    }))
    .await;
    ctx.expect_result(
        123,
        json!({
            "result": {
                "type": "string",
                "value": "scheduled"
            }
        }),
        None,
    );

    let event = ctx
        .wait_for_scheduler_message("timer Page.windowOpen", |message| {
            message["method"] == json!("Page.windowOpen")
        })
        .await;
    assert_eq!(
        event["params"],
        json!({
            "url": "about:blank#timer-window-open",
            "windowName": "_blank",
            "windowFeatures": [
                "menubar",
                "toolbar",
                "status",
                "scrollbars",
                "resizable"
            ],
            "userGesture": false
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_hands_off_session_storage_snapshot_and_initial_storage_key() {
    async fn opener() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><main>session storage opener</main>",
        )
    }

    async fn blocked_cross_origin_popup(
        axum::extract::State((request_count, request_started, response_release, response_returned)): axum::extract::State<(
            Arc<std::sync::atomic::AtomicUsize>,
            Arc<tokio::sync::Semaphore>,
            Arc<tokio::sync::Semaphore>,
            Arc<tokio::sync::Semaphore>,
        )>,
    ) -> impl IntoResponse {
        // The opener's lightweight WindowProxy facade starts its mirrored
        // load before the concrete popup action can reach protocol. Let that
        // renderer-local request finish; gate the second request, which is the
        // real auxiliary target navigation whose lifetime this test covers.
        let request_index = request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if request_index == 0 {
            return opener().await;
        }
        assert_eq!(
            request_index, 1,
            "popup URL must have exactly two load owners"
        );
        request_started.add_permits(1);
        let permit = response_release
            .acquire()
            .await
            .expect("popup response gate must remain open");
        permit.forget();
        response_returned.add_permits(1);
        opener().await
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/opener", get(opener)))
            .await
            .unwrap();
    });
    let cross_origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cross_origin_addr = cross_origin_listener.local_addr().unwrap();
    let cross_origin_request_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cross_origin_request_started = Arc::new(tokio::sync::Semaphore::new(0));
    let cross_origin_response_release = Arc::new(tokio::sync::Semaphore::new(0));
    let cross_origin_response_returned = Arc::new(tokio::sync::Semaphore::new(0));
    let cross_origin_state = (
        cross_origin_request_count.clone(),
        cross_origin_request_started.clone(),
        cross_origin_response_release.clone(),
        cross_origin_response_returned.clone(),
    );
    let cross_origin_server = tokio::spawn(async move {
        axum::serve(
            cross_origin_listener,
            Router::new()
                .route("/popup-first", get(opener))
                .route("/popup", get(blocked_cross_origin_popup))
                .with_state(cross_origin_state),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    enable_root_target_discovery_for_test(&mut ctx);
    let mut browser_context = ctx.conn.new_browser_context("BID-popup-storage".to_owned());
    browser_context.set_active_target_id("TID-popup-storage-opener");
    ctx.conn.browser_context = Some(browser_context);
    let opener_url = format!("http://{addr}/opener");
    let page = ctx
        .conn
        .load_page_via_runtime_async(&opener_url)
        .await
        .expect("opener page should load");
    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.set_target_url(page.final_url().as_str().to_owned());
        let _ = browser_context
            .active_target
            .runtime_slot
            .replace_loaded_page(Some(page));
    }
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new().run_until(async {
    ctx.process_async(json!({
        "id": 1200,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-popup-storage-opener" }
    }))
    .await;
    let opener_session_id = take_response_by_id(&mut ctx, 1200)["result"]["sessionId"]
        .as_str()
        .expect("opener session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1201,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { localStorage.clear(); localStorage.setItem('shared', 'opener'); sessionStorage.clear(); sessionStorage.setItem('phase', 'before'); const popup = window.open('about:blank', '_blank'); sessionStorage.setItem('phase', 'after'); return `${popup.sessionStorage.getItem('phase')}|${sessionStorage.getItem('phase')}|${popup.localStorage.getItem('shared')}`; })()",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_result(
        1201,
        json!({
            "result": {
                "type": "string",
                "value": "before|after|opener"
            }
        }),
        None,
    );

    let created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .expect("window.open should create a popup target");
    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1202,
        "method": "Target.attachToTarget",
        "params": { "targetId": popup_target_id }
    }))
    .await;
    let popup_session_id = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(1202))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("popup session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1203,
        "sessionId": popup_session_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "`${String(sessionStorage.getItem('phase'))}|${String(localStorage.getItem('shared'))}`",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_result(
        1203,
        json!({
            "result": {
                "type": "string",
                "value": "before|opener"
            }
        }),
        Some(&popup_session_id),
    );

    ctx.sent.clear();
    let first_cross_origin_url = format!("http://{cross_origin_addr}/popup-first");
    ctx.process_and_wait_for_response_async(json!({
        "id": 12031,
        "sessionId": popup_session_id,
        "method": "Page.navigate",
        "params": { "url": first_cross_origin_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 12031);
    ctx.sent.clear();
    ctx.wait_until_scheduler_state(
        "attached popup cross-origin navigation commit",
        |conn| {
            conn.browser_context_by_id("BID-popup-storage")
                .and_then(|browser_context| {
                    loaded_page_for_target(browser_context, &popup_target_id)
                })
                .is_some_and(|page| page.final_url().as_str() == first_cross_origin_url)
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 12032,
        "sessionId": popup_session_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "`${String(sessionStorage.getItem('phase'))}|${String(localStorage.getItem('shared'))}`",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_result(
        12032,
        json!({
            "result": {
                "type": "string",
                "value": "null|null"
            }
        }),
        Some(&popup_session_id),
    );

    ctx.sent.clear();
    let cross_origin_url = format!("http://{cross_origin_addr}/popup");
    ctx.process_async(json!({
        "id": 1204,
        "method": "Runtime.evaluate",
        "sessionId": opener_session_id,
        "params": {
            "expression": format!(
                "(() => {{ sessionStorage.setItem('cross-origin-secret', 'opener-only'); return window.open({cross_origin_url:?}, '_blank') !== null; }})()"
            ),
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_result(
        1204,
        json!({
            "result": {
                "type": "boolean",
                "value": true
            }
        }),
        Some(&opener_session_id),
    );
    let cross_origin_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["url"] == json!(cross_origin_url)
        })
        .expect("cross-origin window.open should create a popup target");
    let cross_origin_target_id = cross_origin_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("cross-origin popup target id")
        .to_owned();
    ctx.sent.clear();

    ctx.wait_until_scheduler_state("cross-origin popup navigation starts", |conn| {
        conn.browser_context_by_id("BID-popup-storage")
            .is_some_and(|browser_context| {
                browser_context
                    .has_pending_document_navigation_for_target(Some(&cross_origin_target_id))
            })
    })
    .await;
    let request_started = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cross_origin_request_started.acquire(),
    )
    .await
    .expect("cross-origin popup request should start")
    .expect("cross-origin popup request gate must remain open");
    request_started.forget();

    // Attach while the target's requested URL is still loading. Session
    // attachment belongs to the browsing context and must not race the
    // replacement Document's commit.
    ctx.process_and_wait_for_response_async(json!({
        "id": 1205,
        "method": "Target.attachToTarget",
        "params": { "targetId": cross_origin_target_id }
    }))
    .await;
    let attach_response = take_response_by_id(&mut ctx, 1205);
    let cross_origin_session_id = attach_response["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("cross-origin popup session id: {attach_response:?}"))
        .to_owned();
    assert!(
        ctx.conn
            .browser_context_by_id("BID-popup-storage")
            .is_some_and(|browser_context| browser_context
                .has_pending_document_navigation_for_target(Some(&cross_origin_target_id))),
        "attaching a DevTools session must not replace the target-owned navigation"
    );
    ctx.sent.clear();
    cross_origin_response_release.add_permits(1);
    let response_returned = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cross_origin_response_returned.acquire(),
    )
    .await
    .expect("cross-origin popup response should leave the server gate")
    .expect("cross-origin popup response gate must remain open");
    response_returned.forget();

    ctx.wait_until_scheduler_state(
        "cross-origin popup navigation commit",
        |conn| {
            conn.browser_context_by_id("BID-popup-storage")
                .and_then(|browser_context| {
                    loaded_page_for_target(browser_context, &cross_origin_target_id)
                })
                .is_some_and(|page| page.final_url().as_str() == cross_origin_url)
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 1206,
        "sessionId": cross_origin_session_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "String(sessionStorage.getItem('cross-origin-secret'))",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_result(
        1206,
        json!({
            "result": {
                "type": "string",
                "value": "null"
            }
        }),
        Some(&cross_origin_session_id),
    );
    }).await;

    server.abort();
    cross_origin_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn puppeteer_window_open_uses_parent_context_and_reports_opener() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-puppeteer-popup-context",
        "TID-puppeteer-popup-opener",
        "<main>popup opener</main>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 125,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('data:text/html,<main>puppeteer popup</main>', '_blank') !== null",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["id"] == json!(125)
                && message["result"]["result"]["type"] == json!("boolean")
                && message["result"]["result"]["value"] == json!(true)
        }),
        "window.open should resolve to a popup window handle: {sent:?}"
    );
    let created = sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["url"]
                    == json!("data:text/html,<main>puppeteer popup</main>")
        })
        .unwrap_or_else(|| panic!("missing Puppeteer popup targetCreated: {sent:?}"));
    assert_eq!(created["params"]["targetInfo"]["type"], json!("page"));
    assert_eq!(
        created["params"]["targetInfo"]["browserContextId"],
        json!("BID-puppeteer-popup-context")
    );
    assert_eq!(
        created["params"]["targetInfo"]["openerId"],
        json!("TID-puppeteer-popup-opener")
    );
    assert_eq!(
        created["params"]["targetInfo"]["openerFrameId"],
        json!("TID-puppeteer-popup-opener")
    );
    assert_eq!(
        created["params"]["targetInfo"]["canAccessOpener"],
        json!(true)
    );

    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id");
    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    let popup_info = browser_context
        .devtools_target_info(popup_target_id)
        .expect("popup target should remain discoverable in parent context");
    assert_eq!(
        popup_info.browser_context_id.as_ref().map(|id| id.as_str()),
        Some("BID-puppeteer-popup-context")
    );
    assert_eq!(
        popup_info.opener_id.as_ref().map(|id| id.as_str()),
        Some("TID-puppeteer-popup-opener")
    );
    assert_eq!(
        popup_info.opener_frame_id.as_ref().map(|id| id.as_str()),
        Some("TID-puppeteer-popup-opener")
    );
    assert!(popup_info.can_access_opener);
}

#[tokio::test(flavor = "multi_thread")]
async fn resetting_opener_target_clears_live_opener_but_keeps_frame_attribution() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-reset",
        "TID-opener-reset",
        "<main>popup opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 120,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('https://example.com/popup-after-reset', '_blank') !== null"
        }
    }))
    .await;

    let sent = ctx.take_all();
    let created = sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .expect("window.open should create a popup target");
    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    assert_eq!(
        created["params"]["targetInfo"]["openerId"],
        json!("TID-opener-reset")
    );
    assert_eq!(
        created["params"]["targetInfo"]["openerFrameId"],
        json!("TID-opener-reset")
    );

    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .reset_active_target_slot_to_empty_async()
        .await;

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        browser_context.target_opener_ids.get(&popup_target_id),
        None,
        "popup target must not retain a stale openerId after the opener target slot is reset"
    );
    assert_eq!(
        browser_context
            .target_opener_frame_ids
            .get(&popup_target_id)
            .map(String::as_str),
        Some("TID-opener-reset"),
        "popup target must retain immutable DevTools opener-frame attribution"
    );
    assert!(
        !browser_context
            .target_can_access_opener
            .contains(&popup_target_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_initial_empty_document_record_captures_creator_identity() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-creator",
        "TID-opener-creator",
        "<main>popup opener</main>",
    )
    .await;
    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.set_target_security_origin("https://opener.example".to_owned());
        browser_context.set_target_secure_context_type("Secure".to_owned());
    }

    ctx.process_async(json!({
        "id": 121,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('about:blank#creator', '_blank') !== null"
        }
    }))
    .await;

    let sent = ctx.take_all();
    let created = sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .expect("window.open should create a popup target");
    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id");

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    let initial = browser_context
        .parked_target_owner_state(popup_target_id)
        .and_then(|owner_state| owner_state.initial_empty_document_state())
        .expect("popup target should record initial empty document");
    let creator = initial
        .creator()
        .expect("window.open initial empty document should record creator identity");
    assert_eq!(creator.target_id(), "TID-opener-creator");
    assert_eq!(creator.security_origin(), "https://opener.example");
    assert_eq!(creator.secure_context_type(), "Secure");
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_initial_empty_document_frame_tree_inherits_opener_origin() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-about-blank-origin",
        "TID-opener-about-blank-origin",
        "<main>popup opener</main>",
    )
    .await;
    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.set_target_security_origin("https://opener.example".to_owned());
        browser_context.set_target_secure_context_type("Secure".to_owned());
    }

    ctx.process_async(json!({
        "id": 122,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('about:blank', '_blank') !== null"
        }
    }))
    .await;
    let sent = ctx.take_all();
    let created = sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .expect("window.open should create a popup target");
    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();

    // Ported from Chromium's
    // URLLoaderFactoryInInitialEmptyDoc_NewPopupToAboutBlank coverage: a popup
    // opened to about:blank remains on the initial empty document and inherits
    // the opener's origin.
    ctx.process_async(json!({
        "id": 123,
        "method": "Target.attachToTarget",
        "params": { "targetId": popup_target_id }
    }))
    .await;
    let attached = ctx.take_all();
    let attach_response = attached
        .iter()
        .find(|message| message["id"] == json!(123))
        .expect("attachToTarget should respond");
    let popup_session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("popup session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 124,
        "method": "Page.getFrameTree",
        "sessionId": popup_session_id
    }))
    .await;
    let frame_tree_messages = ctx.take_all();
    let frame_tree_response = frame_tree_messages
        .iter()
        .find(|message| message["id"] == json!(124))
        .expect("Page.getFrameTree should respond");
    let frame = &frame_tree_response["result"]["frameTree"]["frame"];
    assert_eq!(frame["id"], json!(popup_target_id));
    assert_eq!(frame["url"], json!("about:blank"));
    assert_eq!(frame["securityOrigin"], json!("https://opener.example"));
    assert_eq!(frame["secureContextType"], json!("Secure"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    let initial = browser_context
        .parked_target_owner_state(&popup_target_id)
        .and_then(|owner_state| owner_state.initial_empty_document_state())
        .expect("popup target should still record initial empty document");
    assert!(initial.is_on_initial_empty_document());
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_self_navigates_current_target_without_popup_target() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-self",
        "TID-opener-self",
        "<main>popup opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('data:text/html,<main>self target</main>', '_self') !== null"
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| message["id"] == json!(13)
            && message["result"]["result"]["type"] == json!("boolean")
            && message["result"]["result"]["value"] == json!(true)),
        "Runtime.evaluate should return the existing WindowProxy for _self: {sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "_self must not create a popup target: {sent:?}"
    );
    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        browser_context.target_url(),
        "data:text/html,<main>self target</main>"
    );
    assert_eq!(browser_context.active_target_id(), Some("TID-opener-self"));
}

#[tokio::test(flavor = "multi_thread")]
async fn call_function_on_window_open_self_navigates_current_target_without_popup_target() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-call-popup-self",
        "TID-call-opener-self",
        "<main>popup opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 131,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ run() { return window.open('data:text/html,<main>call self target</main>', '_self') !== null; } })"
        }
    }))
    .await;
    let setup = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(131))
        .expect("Runtime.evaluate setup response");
    let object_id = setup["result"]["result"]["objectId"]
        .as_str()
        .expect("setup should return object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 132,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.run(); }",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| message["id"] == json!(132)
            && message["result"]["result"]["type"] == json!("boolean")
            && message["result"]["result"]["value"] == json!(true)),
        "Runtime.callFunctionOn should return the existing WindowProxy for _self: {sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "_self must not create a popup target from Runtime.callFunctionOn: {sent:?}"
    );
    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        browser_context.target_url(),
        "data:text/html,<main>call self target</main>"
    );
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-call-opener-self")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_named_target_reuses_existing_popup_target() {
    let mut ctx = TestContext::new();
    // Target creation/named-target selection is synchronous with
    // `window.open()`, but fetching the selected target's URL is not. Mirror
    // the socket scheduler so this ownership test never joins the Runtime
    // response to external network completion.
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-name",
                "TID-opener-name",
                "<main>popup opener</main>",
            )
            .await;
            ctx.process_async(json!({
                "id": 12,
                "method": "Target.attachToTarget",
                "params": { "targetId": "TID-opener-name" }
            }))
            .await;
            let opener_session_id = take_response_by_id(&mut ctx, 12)["result"]["sessionId"]
                .as_str()
                .expect("opener session id")
                .to_owned();
            ctx.sent.clear();
            ctx.process_async(json!({
                "id": 13,
                "method": "Page.enable",
                "sessionId": opener_session_id
            }))
            .await;
            ctx.expect_result(13, json!({}), Some(&opener_session_id));
            ctx.sent.clear();

            ctx.process_async(json!({
                "id": 14,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": "window.open('data:text/html,first-popup', 'reportWindow') !== null"
                }
            }))
            .await;

            let first_sent = ctx.take_all();
            let created = first_sent
                .iter()
                .find(|message| message["method"] == json!("Target.targetCreated"))
                .expect("first named window.open should create a target");
            let target_id = created["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("created target id")
                .to_owned();
            assert_eq!(
                created["params"]["targetInfo"]["url"],
                json!("data:text/html,first-popup")
            );
            assert!(
                first_sent
                    .iter()
                    .any(|message| message["method"] == json!("Page.windowOpen")),
                "first named window.open should emit Page.windowOpen: {first_sent:?}"
            );

            ctx.process_async(json!({
                "id": 15,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": "window.open('data:text/html,second-popup', 'reportWindow') !== null"
                }
            }))
            .await;

            let second_sent = ctx.take_all();
            assert!(
                !second_sent
                    .iter()
                    .any(|message| message["method"] == json!("Target.targetCreated")),
                "second window.open with the same named target must not create a new target: {second_sent:?}"
            );
            assert!(
                !second_sent
                    .iter()
                    .any(|message| message["method"] == json!("Page.windowOpen")),
                "reusing an existing named target must not emit Page.windowOpen: {second_sent:?}"
            );
            // Chromium returns from `window.open()` after selecting the named
            // target; its final URL becomes observable only when the
            // independently scheduled navigation commits.
            let changed = ctx
                .wait_for_scheduler_message(
                    "reused named popup final target URL",
                    |message| {
                        message["method"] == json!("Target.targetInfoChanged")
                            && message["params"]["targetInfo"]["targetId"]
                                == json!(target_id)
                            && message["params"]["targetInfo"]["url"]
                                == json!("data:text/html,second-popup")
                    },
                )
                .await;
            assert_eq!(
                changed["params"]["targetInfo"]["targetId"],
                json!(target_id)
            );
            assert_eq!(
                changed["params"]["targetInfo"]["url"],
                json!("data:text/html,second-popup")
            );
            ctx.wait_until_scheduler_state("reused named popup remains selected", |conn| {
                conn.browser_context_by_id("BID-popup-name")
                    .is_some_and(|browser_context| {
                        browser_context.active_target_id() == Some(target_id.as_str())
                            && loaded_page_for_target(browser_context, &target_id).is_some_and(
                                |page| {
                                    page.final_url().as_str() == "data:text/html,second-popup"
                                },
                            )
                    })
            })
            .await;
            let browser_context = ctx.conn.browser_context.as_ref().unwrap();
            assert_eq!(browser_context.active_target_id(), Some(target_id.as_str()));
            assert_eq!(
                browser_context.target_url(),
                "data:text/html,second-popup"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_named_target_reused_in_same_command_emits_one_page_event() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-name-same-command",
        "TID-opener-name-same-command",
        "<main>popup opener</main>",
    )
    .await;
    ctx.process_async(json!({
        "id": 16,
        "method": "Page.enable"
    }))
    .await;
    ctx.expect_result(16, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 17,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "
                window.open('https://example.com/first-popup', 'sameCommandWindow');
                window.open('https://example.com/second-popup', 'sameCommandWindow');
                true
            "
        }
    }))
    .await;

    let sent = ctx.take_all();
    let window_open_events = sent
        .iter()
        .filter(|message| message["method"] == json!("Page.windowOpen"))
        .collect::<Vec<_>>();
    assert_eq!(
        window_open_events.len(),
        1,
        "only creation of the named browsing context emits Page.windowOpen: {sent:?}"
    );
    assert_eq!(
        window_open_events[0]["params"]["url"],
        json!("https://example.com/first-popup")
    );
    assert_eq!(
        sent.iter()
            .filter(|message| message["method"] == json!("Target.targetCreated"))
            .count(),
        1,
        "same-command named reuse must create one target: {sent:?}"
    );
    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(browser_context.background_targets.len(), 1);
    assert_eq!(
        browser_context.background_targets[0].target_url(),
        "https://example.com/second-popup"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn named_popup_reuse_with_catchall_discovery_only_changes_the_page_target_info() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-info-change",
        "TID-popup-info-opener",
        "<main>popup opener</main>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 151,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(151, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 152,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('https://example.com/first-popup', 'reportWindow') !== null"
        }
    }))
    .await;
    let first_sent = ctx.take_all();
    let page_created = first_sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["type"] == json!("page")
                && message["params"]["targetInfo"]["url"]
                    == json!("https://example.com/first-popup")
        })
        .unwrap_or_else(|| panic!("missing first popup page targetCreated: {first_sent:?}"));
    let page_target_id = page_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup page target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    assert!(
        first_sent.iter().any(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
                && message["params"]["targetInfo"]["type"] == json!("tab")
        }),
        "catch-all discovery should report popup tab targetCreated: {first_sent:?}"
    );

    ctx.process_async(json!({
        "id": 153,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('https://example.com/second-popup', 'reportWindow') !== null"
        }
    }))
    .await;

    let second_sent = ctx.take_all();
    assert!(
        second_sent.iter().all(|message| {
            message["method"] != json!("Target.targetInfoChanged")
                || message["params"]["targetInfo"]["targetId"] != json!(tab_target_id)
        }),
        "document navigation must not mirror Page targetInfoChanged onto the stable Tab host: {second_sent:?}"
    );
    assert!(
        second_sent.iter().any(|message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
                && message["params"]["targetInfo"]["type"] == json!("page")
                && message["params"]["targetInfo"]["url"]
                    == json!("https://example.com/second-popup")
        }),
        "catch-all discovery should report page targetInfoChanged: {second_sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn anchor_blank_target_uses_implicit_noopener() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-anchor-popup",
        "TID-anchor-opener",
        "<main>anchor popup opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 16,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body.innerHTML = '<a id=\"p\" href=\"https://example.com/anchor-popup\" target=\"_blank\">popup</a>'; document.getElementById('p').click(); 'clicked'",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| message["id"] == json!(16)
            && message["result"]["result"]["value"] == json!("clicked")),
        "Runtime.evaluate should resolve after the anchor click: {sent:?}"
    );
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["url"]
                    == json!("https://example.com/anchor-popup")
                && message["params"]["targetInfo"]["canAccessOpener"] == json!(false)
                && message["params"]["targetInfo"]["openerId"] == json!("TID-anchor-opener")
                && message["params"]["targetInfo"]["openerFrameId"] == json!("TID-anchor-opener")
        }),
        "anchor target=_blank should retain its DevTools creator while denying DOM opener access: {sent:?}"
    );
}

async fn dispatch_anchor_left_click(ctx: &mut TestContext, command_id: u64, modifiers: u8) {
    for (offset, event_type, buttons) in [(0, "mousePressed", 1), (1, "mouseReleased", 0)] {
        ctx.process_async(json!({
            "id": command_id + offset,
            "method": "Input.dispatchMouseEvent",
            "params": {
                "type": event_type,
                "x": 20,
                "y": 20,
                "button": "left",
                "buttons": buttons,
                "clickCount": 1,
                "modifiers": modifiers
            }
        }))
        .await;
        ctx.expect_result(command_id + offset, json!({}), None);
    }
}

fn popup_target_id_for_url(messages: &[serde_json::Value], url: &str) -> String {
    messages
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["type"] == json!("page")
                && message["params"]["targetInfo"]["url"] == json!(url)
        })
        .and_then(|message| message["params"]["targetInfo"]["targetId"].as_str())
        .unwrap_or_else(|| panic!("anchor click should create an exact page target: {messages:#?}"))
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn anchor_left_click_promotes_blank_target_to_foreground() {
    const POPUP_HREF: &str = "data:text/html,%3Cmain%3Eforeground-popup%3C/main%3E";
    const POPUP_URL: &str = "data:text/html,<main>foreground-popup</main>";
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-anchor-foreground",
                "TID-anchor-foreground-opener",
                &format!(
                    "<a href='{POPUP_HREF}' target='_blank' style='position:absolute;left:0;top:0;width:100px;height:100px;display:block'>popup</a>"
                ),
            )
            .await;
            let _ = ctx.take_all();

            dispatch_anchor_left_click(&mut ctx, 16_100, 0).await;

            let sent = ctx.take_all();
            let popup_target_id = popup_target_id_for_url(&sent, POPUP_URL);
            let browser_context = ctx
                .conn
                .browser_context_by_id("BID-anchor-foreground")
                .expect("anchor browser context should remain available");
            assert_eq!(
                browser_context.active_target_id(),
                Some(popup_target_id.as_str())
            );
            assert!(
                browser_context
                    .background_target("TID-anchor-foreground-opener")
                    .is_some(),
                "foreground popup should demote its opener"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn anchor_left_click_activates_popup_while_initial_navigation_waits_for_debugger() {
    const POPUP_HREF: &str =
        "data:text/html,%3Cmain%3Edebugger-waiting-foreground-popup%3C/main%3E";
    const POPUP_URL: &str = "data:text/html,<main>debugger-waiting-foreground-popup</main>";
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-anchor-debugger-wait",
                "TID-anchor-debugger-wait-opener",
                &format!(
                    "<a href='{POPUP_HREF}' target='_blank' style='position:absolute;left:0;top:0;width:100px;height:100px;display:block'>popup</a>"
                ),
            )
            .await;
            let _ = ctx.take_all();
            ctx.process_async(json!({
                "id": 16_150,
                "method": "Target.setAutoAttach",
                "params": {
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true
                }
            }))
            .await;
            ctx.expect_result(16_150, json!({}), None);
            let _ = ctx.take_all();

            dispatch_anchor_left_click(&mut ctx, 16_151, 0).await;

            let sent = ctx.take_all();
            let popup_target_id = popup_target_id_for_url(&sent, POPUP_URL);
            let popup_session_id = sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Target.attachedToTarget")
                        && message["params"]["targetInfo"]["type"] == json!("page")
                        && message["params"]["targetInfo"]["targetId"]
                            == json!(popup_target_id)
                })
                .and_then(|message| message["params"]["sessionId"].as_str())
                .unwrap_or_else(|| {
                    panic!("foreground popup should be auto-attached: {sent:#?}")
                })
                .to_owned();
            let browser_context = ctx
                .conn
                .browser_context_by_id("BID-anchor-debugger-wait")
                .expect("anchor browser context should remain available");
            assert_eq!(
                browser_context.active_target_id(),
                Some(popup_target_id.as_str()),
                "foreground selection must not wait for initial navigation"
            );
            assert!(
                browser_context
                    .active_target
                    .runtime_slot
                    .loaded_page()
                    .is_some_and(|page| moli_url::is_about_blank(page.final_url())),
                "waitForDebuggerOnStart should retain the active popup's initial about:blank document"
            );

            ctx.process_async(json!({
                "id": 16_153,
                "method": "Runtime.runIfWaitingForDebugger",
                "sessionId": popup_session_id
            }))
            .await;
            ctx.expect_result(16_153, json!({}), Some(&popup_session_id));
            ctx.wait_until_scheduler_state("resumed foreground popup navigation", |conn| {
                conn.browser_context_by_id("BID-anchor-debugger-wait")
                    .is_some_and(|browser_context| {
                        browser_context.active_target_id() == Some(popup_target_id.as_str())
                            && browser_context
                                .active_target
                                .runtime_slot
                                .loaded_page()
                                .is_some_and(|page| page.final_url().as_str() == POPUP_URL)
                    })
            })
            .await;
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn anchor_platform_new_tab_click_keeps_blank_target_in_background() {
    const POPUP_HREF: &str = "data:text/html,%3Cmain%3Ebackground-popup%3C/main%3E";
    const POPUP_URL: &str = "data:text/html,<main>background-popup</main>";
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-anchor-background",
                "TID-anchor-background-opener",
                &format!(
                    "<a href='{POPUP_HREF}' target='_blank' style='position:absolute;left:0;top:0;width:100px;height:100px;display:block'>popup</a>"
                ),
            )
            .await;
            let _ = ctx.take_all();

            #[cfg(target_os = "macos")]
            let platform_new_tab_modifier = 4;
            #[cfg(not(target_os = "macos"))]
            let platform_new_tab_modifier = 2;
            dispatch_anchor_left_click(&mut ctx, 16_200, platform_new_tab_modifier).await;

            let sent = ctx.take_all();
            let popup_target_id = popup_target_id_for_url(&sent, POPUP_URL);
            let browser_context = ctx
                .conn
                .browser_context_by_id("BID-anchor-background")
                .expect("anchor browser context should remain available");
            assert_eq!(
                browser_context.active_target_id(),
                Some("TID-anchor-background-opener")
            );
            assert!(
                browser_context
                    .background_target(&popup_target_id)
                    .is_some(),
                "platform-new-tab-clicked popup should remain in the background"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn anchor_blank_target_with_rel_opener_preserves_exact_opener() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-anchor-popup-opener",
        "TID-anchor-popup-opener",
        "<main>anchor popup opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 17,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body.innerHTML = '<a id=\"p\" href=\"https://example.com/anchor-popup-opener\" target=\"_blank\" rel=\"opener\">popup</a>'; document.getElementById('p').click(); 'clicked'",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| message["id"] == json!(17)
            && message["result"]["result"]["value"] == json!("clicked")),
        "Runtime.evaluate should resolve after the anchor click: {sent:?}"
    );
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["url"]
                    == json!("https://example.com/anchor-popup-opener")
                && message["params"]["targetInfo"]["canAccessOpener"] == json!(true)
                && message["params"]["targetInfo"]["openerId"] == json!("TID-anchor-popup-opener")
                && message["params"]["targetInfo"]["openerFrameId"]
                    == json!("TID-anchor-popup-opener")
        }),
        "rel=opener should preserve the exact opener target and frame: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_seeds_browser_context_from_connection_cookie_profile() {
    let mut ctx = TestContext::from_conn(CdpConnection::new_with_initial_cookies(vec![
        stored_cookie("sid", "seeded"),
    ]));
    ctx.conn.set_root_target_discovery_enabled(true);
    ctx.process_async(json!({"id": 11, "method": "Target.createTarget",
                       "params": {"url": "about:blank"}}))
        .await;
    ctx.expect_event("Target.targetCreated", None);
    let tid = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_target_id_owned()
        .unwrap();
    ctx.expect_result(11, json!({ "targetId": tid }), None);

    let cookies = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .snapshot_cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "sid");
    assert_eq!(cookies[0].value, "seeded");
}

/// cdp.target: createTarget with auto-attach emits attachedToTarget event
#[tokio::test(flavor = "multi_thread")]
async fn create_target_with_auto_attach() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 9, "method": "Target.setAutoAttach",
                       "params": {"autoAttach": true, "waitForDebuggerOnStart": false}}))
        .await;
    ctx.take_all();
    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                       "params": {"url": "about:blank"}}))
        .await;
    ctx.expect_event("Target.targetCreated", None);
    ctx.expect_event("Target.attachedToTarget", None);
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_attach_only_owner_receives_target_info_changed_without_target_created() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 9050,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(9050, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9051,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 9051)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "auto-attach-only owner must not receive targetCreated without discovery: {:?}",
        ctx.sent
    );
    let attached = ctx.take_first_matching("auto-attached page", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    });
    let page_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("auto-attached page session id")
        .to_owned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        }),
        "auto-attach-only owner must not receive initial targetInfoChanged before the auto-attached index is committed: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    let url = "data:text/html,<title>Auto Attach InfoChanged</title><main>navigation</main>";
    ctx.process_async(json!({
        "id": 9052,
        "sessionId": page_session_id.as_str(),
        "method": "Page.navigate",
        "params": { "url": url }
    }))
    .await;

    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "auto-attach-only parsed title targetInfoChanged",
        |message| {
            message.get("sessionId").is_none()
                && message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
                && message["params"]["targetInfo"]["title"] == json!("Auto Attach InfoChanged")
        },
    )
    .await;
    let changed = ctx.take_first_matching("auto-attach-only targetInfoChanged", |message| {
        message.get("sessionId").is_none()
            && message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
            && message["params"]["targetInfo"]["title"] == json!("Auto Attach InfoChanged")
    });
    assert_eq!(changed["params"]["targetInfo"]["url"], json!(url));
    assert_eq!(
        changed["params"]["targetInfo"]["title"],
        json!("Auto Attach InfoChanged")
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "navigation must not synthesize targetCreated for auto-attach-only owner: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_with_puppeteer_tab_filter_auto_attaches_tab() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9020,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }
    }))
    .await;
    ctx.expect_result(9020, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9021,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 9021)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 9022,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;

    let target_id = take_response_by_id(&mut ctx, 9022)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &target_id);
    let created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        })
        .unwrap_or_else(|| panic!("missing targetCreated: {:?}", ctx.sent));
    assert_eq!(created["params"]["targetInfo"]["type"], json!("page"));
    assert_eq!(created["params"]["targetInfo"]["attached"], json!(false));
    let attached = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
        })
        .unwrap_or_else(|| panic!("missing attachedToTarget: {:?}", ctx.sent));
    assert_eq!(attached["params"]["targetInfo"]["type"], json!("tab"));
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));
    assert_eq!(attached["params"]["waitingForDebugger"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_puppeteer_tab_auto_attach_clears_service_worker_pause_owner() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9030,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }
    }))
    .await;
    ctx.expect_result(9030, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9031,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 9031)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &target_id);
    let tab_session_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
        })
        .and_then(|message| message["params"]["sessionId"].as_str())
        .expect("auto-attached tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9032,
        "sessionId": tab_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(9032, json!({}), None);
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 2);

    ctx.process_async(json!({
        "id": 9033,
        "method": "Target.closeTarget",
        "params": { "targetId": target_id }
    }))
    .await;
    ctx.expect_result(9033, json!({ "success": true }), None);

    ctx.process_async(json!({
        "id": 9034,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(9034, json!({}), None);

    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 0);
    assert!(ctx.conn.browser_contexts().all(|browser_context| {
        !browser_context
            .renderer_runtime()
            .service_worker_pause_on_start_for_devtools()
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_with_puppeteer_tab_filter_auto_attaches_popup_tab() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-tab",
        "TID-popup-opener",
        "<main>popup opener</main>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9023,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }
    }))
    .await;
    ctx.expect_result(9023, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9024,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('data:text/html,<main>popup tab</main>', 'popupTab') !== null",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    let created = sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["url"]
                    == json!("data:text/html,<main>popup tab</main>")
                && message["params"]["targetInfo"]["type"] == json!("page")
        })
        .unwrap_or_else(|| panic!("missing popup page targetCreated: {sent:?}"));
    let page_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup page target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    let attached = sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
        })
        .unwrap_or_else(|| panic!("missing popup tab attachedToTarget: {sent:?}"));
    assert_eq!(attached["params"]["targetInfo"]["type"], json!("tab"));
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));
    assert_eq!(attached["params"]["waitingForDebugger"], json!(true));
    let tab_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup tab session id");
    assert!(matches!(
        ctx.conn.session_route(Some(tab_session_id)),
        Some(crate::conn::CdpSessionRoute::TabTarget {
            tab_target_id: route_tab_target_id,
            ..
        }) if route_tab_target_id == tab_target_id
    ));
    assert!(
        !sent.iter().any(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
        }),
        "Puppeteer tab filter must not auto-attach the popup page directly: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_session_auto_attach_routes_popup_tab_attached_event_to_owner_session() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-browser-owner",
        "TID-popup-browser-owner-opener",
        "<main>popup opener</main>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9025,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let browser_attached = ctx.take_first_matching("browser attached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_session_id = browser_attached["params"]["sessionId"]
        .as_str()
        .expect("browser target session id")
        .to_owned();
    ctx.expect_result(
        9025,
        json!({ "sessionId": browser_session_id.as_str() }),
        None,
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9026,
        "sessionId": browser_session_id.as_str(),
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }
    }))
    .await;
    ctx.expect_result(9026, json!({}), Some(&browser_session_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9027,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('data:text/html,<main>popup owner tab</main>', 'popupOwnerTab') !== null",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    let created = sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["url"]
                    == json!("data:text/html,<main>popup owner tab</main>")
                && message["params"]["targetInfo"]["type"] == json!("page")
        })
        .unwrap_or_else(|| panic!("missing popup page targetCreated: {sent:?}"));
    let page_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup page target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    let attached = sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!(browser_session_id)
                && message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
        })
        .unwrap_or_else(|| panic!("missing owner-routed popup tab attachedToTarget: {sent:?}"));
    assert_eq!(attached["params"]["targetInfo"]["type"], json!("tab"));
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));
    assert_eq!(attached["params"]["waitingForDebugger"], json!(true));
    let tab_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup tab session id")
        .to_owned();
    assert!(matches!(
        ctx.conn.session_route(Some(&tab_session_id)),
        Some(crate::conn::CdpSessionRoute::TabTarget {
            tab_target_id: route_tab_target_id,
            ..
        }) if route_tab_target_id == tab_target_id
    ));
    assert!(
        ctx.conn
            .auto_attached_sessions_for_owner(Some(&browser_session_id))
            .contains(&tab_session_id),
        "popup tab session must be committed under the browser owner"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tab_session_auto_attach_catchall_attaches_child_page() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 9030,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }
    }))
    .await;
    ctx.expect_result(9030, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9031,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 9031)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 9032,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 9032)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    let tab_attached = ctx.take_first_matching("tab attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
    });
    let tab_session_id = tab_attached["params"]["sessionId"]
        .as_str()
        .expect("tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9033,
        "sessionId": tab_session_id.clone(),
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{}]
        }
    }))
    .await;

    ctx.expect_result(9033, json!({}), Some(&tab_session_id));
    let page_attached = ctx.take_first_matching("child page attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["sessionId"] == json!(tab_session_id)
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    assert_eq!(page_attached["params"]["targetInfo"]["type"], json!("page"));
    assert_eq!(
        page_attached["params"]["targetInfo"]["attached"],
        json!(true)
    );
    assert_eq!(page_attached["params"]["waitingForDebugger"], json!(false));
    let page_session_id = page_attached["params"]["sessionId"]
        .as_str()
        .expect("page session id");
    assert!(matches!(
        ctx.conn.session_route(Some(page_session_id)),
        Some(crate::conn::CdpSessionRoute::ActiveTarget { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn root_auto_attach_disable_detaches_auto_attached_tab_child_page_cascade() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 9035,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }
    }))
    .await;
    ctx.expect_result(9035, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9036,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 9036)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    let tab_attached = ctx.take_first_matching("tab attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
    });
    let tab_session_id = tab_attached["params"]["sessionId"]
        .as_str()
        .expect("tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9037,
        "sessionId": tab_session_id.clone(),
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(9037, json!({}), Some(&tab_session_id));
    let page_attached = ctx.take_first_matching("child page attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["sessionId"] == json!(tab_session_id)
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    let page_session_id = page_attached["params"]["sessionId"]
        .as_str()
        .expect("page session id")
        .to_owned();
    assert!(ctx.conn.session_route(Some(&tab_session_id)).is_some());
    assert!(ctx.conn.session_route(Some(&page_session_id)).is_some());
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9038,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(9038, json!({}), None);
    ctx.take_first_matching("child page detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!(page_target_id)
            && message["params"]["sessionId"] == json!(page_session_id)
    });
    ctx.take_first_matching("tab detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!(tab_target_id)
            && message["params"]["sessionId"] == json!(tab_session_id)
    });
    assert_eq!(ctx.conn.session_route(Some(&page_session_id)), None);
    assert_eq!(ctx.conn.session_route(Some(&tab_session_id)), None);
    assert!(ctx.conn.auto_attached_sessions_for_owner(None).is_empty());
    assert!(
        ctx.conn
            .auto_attached_sessions_for_owner(Some(&tab_session_id))
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tab_session_auto_attach_only_attaches_its_own_child_page() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 9040,
        "method": "Target.createTarget",
        "params": { "url": "about:blank#first" }
    }))
    .await;
    let first_page_target_id = take_response_by_id(&mut ctx, 9040)["result"]["targetId"]
        .as_str()
        .expect("first target id")
        .to_owned();
    let first_tab_target_id = tab_id_for_page(&ctx, &first_page_target_id);

    ctx.process_async(json!({
        "id": 9041,
        "method": "Target.createTarget",
        "params": { "url": "about:blank#second" }
    }))
    .await;
    let second_page_target_id = take_response_by_id(&mut ctx, 9041)["result"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let second_tab_target_id = tab_id_for_page(&ctx, &second_page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9042,
        "method": "Target.attachToTarget",
        "params": { "targetId": first_tab_target_id.clone() }
    }))
    .await;
    let first_tab_session_id = take_response_by_id(&mut ctx, 9042)["result"]["sessionId"]
        .as_str()
        .expect("first tab session")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9043,
        "sessionId": first_tab_session_id.clone(),
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{}]
        }
    }))
    .await;

    ctx.expect_result(9043, json!({}), Some(&first_tab_session_id));
    let first_page_attached =
        ctx.take_first_matching("first child page attachedToTarget", |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["sessionId"] == json!(first_tab_session_id)
                && message["params"]["targetInfo"]["targetId"] == json!(first_page_target_id)
        });
    assert_eq!(
        first_page_attached["params"]["targetInfo"]["type"],
        json!("page")
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && (message["params"]["targetInfo"]["targetId"] == json!(second_page_target_id)
                    || message["params"]["targetInfo"]["targetId"] == json!(second_tab_target_id))
        }),
        "tab-session auto-attach must not scan sibling tab/page targets: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn incomplete_popup_rollback_clears_tab_page_sessions_and_target_graph() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-popup-rollback", "TID-popup-opener");

    let page_target_id = "TID-popup-rollback";
    let tab_target_id = ctx.conn.register_top_level_page_target(page_target_id);
    {
        let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
        browser_context.stage_background_target(
            page_target_id.to_owned(),
            Some("SID-popup-page-primary".to_owned()),
            "about:blank".to_owned(),
            Some("about:blank".to_owned()),
            None,
        );
        assert!(browser_context.assign_auto_attached_session_to_target(
            page_target_id,
            "SID-popup-page-aux".to_owned()
        ));
        browser_context.remember_target_window_name("popupName", page_target_id);
        browser_context.remember_target_popup_id(Some(42), page_target_id);
    }
    assert!(ctx.conn.assign_session_to_tab_target(
        &tab_target_id,
        "SID-popup-tab".to_owned(),
        false
    ));
    ctx.conn
        .register_auto_attached_session("SID-popup-tab".to_owned(), None);
    ctx.conn
        .register_auto_attached_session("SID-popup-page-primary".to_owned(), None);
    ctx.conn
        .register_auto_attached_session("SID-popup-page-aux".to_owned(), None);

    assert!(ctx.conn.session_route(Some("SID-popup-tab")).is_some());
    assert!(
        ctx.conn
            .session_route(Some("SID-popup-page-primary"))
            .is_some()
    );
    assert!(ctx.conn.session_route(Some("SID-popup-page-aux")).is_some());
    assert_eq!(ctx.conn.tab_target_count(), 1);

    lifecycle::rollback_incomplete_popup_target_async(
        &mut ctx.conn,
        Some("BID-popup-rollback"),
        page_target_id,
    )
    .await;

    assert_eq!(ctx.conn.session_route(Some("SID-popup-tab")), None);
    assert_eq!(ctx.conn.session_route(Some("SID-popup-page-primary")), None);
    assert_eq!(ctx.conn.session_route(Some("SID-popup-page-aux")), None);
    assert!(ctx.conn.auto_attached_sessions_for_owner(None).is_empty());
    assert_eq!(ctx.conn.tab_target_count(), 0);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .background_target(page_target_id)
            .is_none()
    );
    assert!(
        ctx.conn
            .primary_page_target_id_for_tab_target_id(&tab_target_id)
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn incomplete_active_popup_rollback_clears_active_slot_sessions_and_target_graph() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc(&mut ctx, "BID-active-popup-rollback");

    let page_target_id = "TID-active-popup-rollback";
    let tab_target_id = ctx.conn.register_top_level_page_target(page_target_id);
    {
        let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
        browser_context.set_active_target_id(page_target_id);
        browser_context.set_target_url("about:blank".to_owned());
        browser_context.begin_active_target_initial_empty_document("about:blank".to_owned());
        browser_context.attach_active_session("SID-active-popup-page-primary");
        assert!(browser_context.assign_auto_attached_session_to_target(
            page_target_id,
            "SID-active-popup-page-aux".to_owned()
        ));
    }
    assert!(ctx.conn.assign_session_to_tab_target(
        &tab_target_id,
        "SID-active-popup-tab".to_owned(),
        false
    ));
    ctx.conn
        .register_auto_attached_session("SID-active-popup-tab".to_owned(), None);
    ctx.conn
        .register_auto_attached_session("SID-active-popup-page-primary".to_owned(), None);
    ctx.conn
        .register_auto_attached_session("SID-active-popup-page-aux".to_owned(), None);

    assert!(
        ctx.conn
            .session_route(Some("SID-active-popup-tab"))
            .is_some()
    );
    assert!(
        ctx.conn
            .session_route(Some("SID-active-popup-page-primary"))
            .is_some()
    );
    assert!(
        ctx.conn
            .session_route(Some("SID-active-popup-page-aux"))
            .is_some()
    );
    assert_eq!(ctx.conn.tab_target_count(), 1);

    lifecycle::rollback_incomplete_popup_target_async(
        &mut ctx.conn,
        Some("BID-active-popup-rollback"),
        page_target_id,
    )
    .await;

    assert_eq!(ctx.conn.session_route(Some("SID-active-popup-tab")), None);
    assert_eq!(
        ctx.conn
            .session_route(Some("SID-active-popup-page-primary")),
        None
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-active-popup-page-aux")),
        None
    );
    assert!(ctx.conn.auto_attached_sessions_for_owner(None).is_empty());
    assert_eq!(ctx.conn.tab_target_count(), 0);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        None
    );
    assert!(
        ctx.conn
            .primary_page_target_id_for_tab_target_id(&tab_target_id)
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unannounced_page_session_cleanup_clears_route_and_auto_attached_owner_index() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-page-session-cleanup", "TID-page-cleanup");
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_auto_attached_session_to_target(
                "TID-page-cleanup",
                "SID-page-cleanup".to_owned()
            )
    );
    ctx.conn
        .register_auto_attached_session("SID-page-cleanup".to_owned(), Some("SID-tab-owner"));
    assert!(ctx.conn.session_route(Some("SID-page-cleanup")).is_some());
    assert_eq!(
        ctx.conn
            .auto_attached_sessions_for_owner(Some("SID-tab-owner")),
        vec!["SID-page-cleanup".to_owned()]
    );

    let prepared = ctx.conn.prepare_auto_attach_session_commit(
        "SID-page-cleanup",
        Some("SID-tab-owner".to_owned()),
        false,
    );
    let rollback_plan = ctx
        .conn
        .rollback_prepared_attach_session_without_event_async(&prepared)
        .await;
    assert_eq!(
        rollback_plan.rolled_back_session_ids(),
        &["SID-page-cleanup".to_owned()]
    );
    assert!(rollback_plan.into_background_events().is_empty());

    assert_eq!(ctx.conn.session_route(Some("SID-page-cleanup")), None);
    assert!(
        ctx.conn
            .auto_attached_sessions_for_owner(Some("SID-tab-owner"))
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_without_debugger_wait_starts_requested_url_navigation() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9008,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 9009,
        "method": "Target.createTarget",
        "params": {
            "url": "data:text/html,<title>created-target-ready</title>"
        }
    }))
    .await;

    let messages = ctx.take_all();
    let attached = messages
        .iter()
        .find(|message| message["method"] == "Target.attachedToTarget")
        .expect("created target should be auto-attached");
    assert_eq!(attached["params"]["waitingForDebugger"], json!(false));
    let target_id = attached["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("created target id");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("created target session id");
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(9009) && message["result"]["targetId"] == json!(target_id)
        }),
        "Target.createTarget response should retain the created target id: {messages:?}"
    );

    ctx.process_async(json!({
        "id": 9010,
        "sessionId": session_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "JSON.stringify({url: document.URL, title: document.title})",
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 9010);
    assert_eq!(
        evaluation["result"]["result"]["value"],
        json!(
            "{\"url\":\"data:text/html,<title>created-target-ready</title>\",\"title\":\"created-target-ready\"}"
        )
    );

    ctx.wait_for_scheduler_message("created target title metadata", |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
            && message["params"]["targetInfo"]["title"] == json!("created-target-ready")
    })
    .await;

    ctx.process_async(json!({
        "id": 9011,
        "sessionId": session_id,
        "method": "Page.getNavigationHistory"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 9011);
    assert_eq!(history["result"]["currentIndex"], json!(0));
    assert_eq!(
        history["result"]["entries"],
        json!([{
            "id": 1,
            "url": "data:text/html,<title>created-target-ready</title>",
            "userTypedURL": "data:text/html,<title>created-target-ready</title>",
            "title": "created-target-ready",
            "transitionType": "auto_toplevel"
        }])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_with_wait_for_debugger_auto_attach_marks_attached_event() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9010,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true
        }
    }))
    .await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 9011,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;

    ctx.expect_event("Target.targetCreated", None);
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(attached["params"]["waitingForDebugger"], json!(true));
    let target_id = attached["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    ctx.expect_result(9011, json!({ "targetId": target_id }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_waiting_for_debugger_does_not_replay_replaced_initial_document_load() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9012,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(9012, json!({}), None);

    let target_url = "data:text/html,<title>replacement-ready</title>";
    ctx.process_async(json!({
        "id": 9013,
        "method": "Target.createTarget",
        "params": { "url": target_url }
    }))
    .await;
    let attached = ctx.take_first_matching("waiting target attachment", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["url"] == json!(target_url)
    });
    assert_eq!(attached["params"]["waitingForDebugger"], json!(true));
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("waiting target session id")
        .to_owned();
    let target_id = attached["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("waiting target id")
        .to_owned();
    let create_response = take_response_by_id(&mut ctx, 9013);
    assert_eq!(create_response["result"]["targetId"], json!(target_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9014,
        "sessionId": session_id,
        "method": "Page.enable"
    }))
    .await;
    ctx.expect_result(9014, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 9015,
        "sessionId": session_id,
        "method": "Page.setLifecycleEventsEnabled",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(9015, json!({}), Some(&session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Page.lifecycleEvent")),
        "the internal initial about:blank lifecycle must not satisfy waits for the requested target URL: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .runtime_session_owner_initial_empty_document_has_replacement_url(Some(&session_id))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_with_auto_attach_emits_attached_event_for_each_owner() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9001,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(9001, json!({}), None);

    ctx.process_async(json!({ "id": 9002, "method": "Target.attachToBrowserTarget" }))
        .await;
    let browser_attached = ctx.take_one();
    let browser_session_id = browser_attached["params"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    ctx.expect_result(
        9002,
        json!({ "sessionId": browser_session_id.as_str() }),
        None,
    );

    ctx.process_async(json!({
        "id": 9003,
        "sessionId": browser_session_id.as_str(),
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": true }
    }))
    .await;
    ctx.expect_result(9003, json!({}), Some(&browser_session_id));

    ctx.process_async(json!({
        "id": 9004,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;

    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let first_attached = ctx.take_one();
    let second_attached = ctx.take_one();
    assert_eq!(first_attached["method"], "Target.attachedToTarget");
    assert_eq!(second_attached["method"], "Target.attachedToTarget");
    assert_eq!(
        first_attached["params"]["targetInfo"]["targetId"],
        target_id
    );
    assert_eq!(
        second_attached["params"]["targetInfo"]["targetId"],
        target_id
    );
    assert_ne!(
        first_attached["params"]["sessionId"],
        second_attached["params"]["sessionId"]
    );
    let mut waiting_flags = [
        first_attached["params"]["waitingForDebugger"].as_bool(),
        second_attached["params"]["waitingForDebugger"].as_bool(),
    ];
    waiting_flags.sort();
    assert_eq!(waiting_flags, [Some(false), Some(true)]);
    ctx.expect_result(9004, json!({ "targetId": target_id }), None);
}

/// cdp.target: createTarget – unknown browserContextId error
#[tokio::test(flavor = "multi_thread")]
async fn create_target_wrong_bc_id_error() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                       "params": {"browserContextId": "BID-8"}}))
        .await;
    ctx.expect_error(10, -31998, "UnknownBrowserContextId");
}

/// cdp.target: createTarget with matching browserContextId
#[tokio::test(flavor = "multi_thread")]
async fn create_target_with_bc_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                       "params": {"browserContextId": "BID-9"}}))
        .await;
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.has_active_target());
    let tid = bc.active_target_id().unwrap().to_owned();
    ctx.expect_event("Target.targetCreated", None);
    ctx.expect_result(10, json!({ "targetId": tid }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_stages_second_target_in_background_slot() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");

    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                       "params": {"browserContextId": "BID-9", "url": "about:blank"}}))
        .await;
    let created = take_created_target_id(&mut ctx, 10);
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.background_targets.len(), 1);
    assert_eq!(bc.background_targets[0].target_id(), created);
}
