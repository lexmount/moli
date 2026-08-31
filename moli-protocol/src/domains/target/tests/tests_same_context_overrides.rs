use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_page_session_overrides_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104158,
        "method": "Network.enable",
        "sessionId": "SID-active"
    }))
    .await;
    ctx.expect_result(104158, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104159,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-active",
        "params": {
            "headers": {
                "X-Target": "A"
            }
        }
    }))
    .await;
    ctx.expect_result(104159, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104160,
        "method": "Emulation.setLocaleOverride",
        "sessionId": "SID-active",
        "params": { "locale": "en-GB" }
    }))
    .await;
    ctx.expect_result(104160, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104161,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104161, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104162,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104162)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1041621,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041621, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104163,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104163);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104164,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": second_session_id,
        "params": {
            "headers": {
                "X-Target": "B"
            }
        }
    }))
    .await;
    ctx.expect_result(104164, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104165,
        "method": "Emulation.setLocaleOverride",
        "sessionId": second_session_id,
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(104165, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104166,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(104166, json!({}), None);

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
        assert_eq!(
            bc.active_page_state().network_policy.extra_headers(),
            vec![("X-Target".into(), "A".into())]
        );
        assert_eq!(
            bc.active_page_state().locale_override.as_deref(),
            Some("en-GB")
        );
    }

    ctx.process_async(json!({
        "id": 104167,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>first-restored</title><div id='ok'>first restored</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let first_navigation = take_response_by_id(&mut ctx, 104167);
    assert_eq!(
        first_navigation["result"]["frameId"],
        json!("TID-000000000A")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041671,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "expression": "JSON.stringify({ title: document.title, lang: navigator.language, locale: Intl.DateTimeFormat().resolvedOptions().locale })"
        }
    }))
    .await;
    let first_eval = take_response_by_id(&mut ctx, 1041671);
    let first_payload = first_eval["result"]["result"]["value"]
        .as_str()
        .expect("first payload should be string");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first payload should be valid json");
    assert_eq!(first_payload["title"], json!("first-restored"));
    assert_eq!(first_payload["lang"], json!("en-US"));
    assert_eq!(first_payload["locale"], json!("en-GB"));

    ctx.process_async(json!({
        "id": 104168,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104168, json!({}), None);

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert_eq!(
            bc.active_page_state().network_policy.extra_headers(),
            vec![("X-Target".into(), "B".into())]
        );
        assert_eq!(
            bc.active_page_state().locale_override.as_deref(),
            Some("fr-FR")
        );
    }

    ctx.process_async(json!({
        "id": 104169,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second-restored</title><div id='ok'>second restored</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104169);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041691,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ title: document.title, lang: navigator.language, locale: Intl.DateTimeFormat().resolvedOptions().locale })"
        }
    }))
    .await;
    let second_eval = take_response_by_id(&mut ctx, 1041691);
    let second_payload = second_eval["result"]["result"]["value"]
        .as_str()
        .expect("second payload should be string");
    let second_payload: serde_json::Value =
        serde_json::from_str(second_payload).expect("second payload should be valid json");
    assert_eq!(second_payload["title"], json!("second-restored"));
    assert_eq!(second_payload["lang"], json!("en-US"));
    assert_eq!(second_payload["locale"], json!("fr-FR"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_network_conditions_after_session_scoped_owner_activity()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-SESSION-NET",
        "TID-000000000NA",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104169267,
        "method": "Network.emulateNetworkConditions",
        "sessionId": "SID-active",
        "params": {
            "offline": false,
            "latency": 150,
            "downloadThroughput": 1024,
            "uploadThroughput": 512,
            "connectionType": "cellular3g"
        }
    }))
    .await;
    ctx.expect_result(104169267, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104169268,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9-SESSION-NET", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104169268, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104169269,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104169269)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104169270,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104169270);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104169271,
        "method": "Network.emulateNetworkConditions",
        "sessionId": second_session_id,
        "params": {
            "offline": true,
            "latency": 25,
            "downloadThroughput": 2048,
            "uploadThroughput": 256,
            "connectionType": "wifi"
        }
    }))
    .await;
    ctx.expect_result(104169271, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104169272,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000NA" }
    }))
    .await;
    ctx.expect_result(104169272, json!({}), None);

    ctx.process_async(json!({
        "id": 104169273,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": "http://example.test/offline" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_error = take_response_by_id(&mut ctx, 104169273);
    assert_eq!(
        second_error["error"]["message"],
        json!("Network emulation offline")
    );

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context after direct background navigation");
        assert_eq!(active.active_target_id(), Some("TID-000000000NA"));
        let parked = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should keep parked network state");
        assert!(parked.network_policy.network_offline());
        assert_eq!(parked.network_policy.emulated_network_latency(), 25.0);
        assert_eq!(parked.network_policy.emulated_download_throughput(), 2048.0);
        assert_eq!(parked.network_policy.emulated_upload_throughput(), 256.0);
        assert_eq!(
            parked.network_policy.emulated_connection_type(),
            Some("wifi")
        );
    }

    ctx.process_async(json!({
        "id": 104169274,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": { "expression": "document.title" }
    }))
    .await;
    let first_eval = take_response_by_id(&mut ctx, 104169274);
    assert_eq!(first_eval["result"]["result"]["value"], json!("first"));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context after restoring first target");
        assert_eq!(active.active_target_id(), Some("TID-000000000NA"));
        assert!(!active.active_page_state().network_policy.network_offline());
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .emulated_network_latency(),
            150.0
        );
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .emulated_download_throughput(),
            1024.0
        );
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .emulated_upload_throughput(),
            512.0
        );
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .emulated_connection_type(),
            Some("cellular3g")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_cookie_manager_surface_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-COOKIE",
        "TID-000000000CKA",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .set_cookie_manager_policy_cookies_enabled_override_async(false)
        .await;

    ctx.process_async(json!({
        "id": 1041692,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": { "expression": "navigator.cookieEnabled" }
    }))
    .await;
    let first_before_switch = take_response_by_id(&mut ctx, 1041692);
    assert_eq!(
        first_before_switch["result"]["result"]["value"],
        json!(false)
    );

    ctx.process_async(json!({
        "id": 1041693,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9-COOKIE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(1041693, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041694,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1041694)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1041695,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041695);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10416951,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10416951, json!({}), None);

    ctx.conn
        .browser_context
        .as_mut()
        .expect("second target should be active after attach")
        .set_cookie_manager_policy_cookies_enabled_override_async(true)
        .await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("second target should still be active after cookies enabled override")
        .set_cookie_manager_policy_browser_context_overrides_async(
            &moli_cookie_jar::BrowserCookieFacadeContextOverrides::default()
                .with_site_for_cookies_url(
                    &url::Url::parse("https://target-b.example/root")
                        .expect("target-b site-for-cookies url"),
                ),
        )
        .await;
    let second_surface = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("active browser context after second navigation")
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        second_surface
            .capability_surface
            .manager_surface
            .policy
            .cookies_enabled_override,
        Some(true)
    );
    assert_eq!(
        second_surface
            .capability_surface
            .manager_surface
            .policy
            .browser_context_overrides
            .site_for_cookies_url
            .as_ref()
            .map(url::Url::as_str),
        Some("https://target-b.example/root")
    );

    ctx.process_async(json!({
        "id": 1041697,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000CKA" }
    }))
    .await;
    ctx.expect_result(1041697, json!({}), None);

    ctx.process_async(json!({
        "id": 1041698,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": { "expression": "navigator.cookieEnabled" }
    }))
    .await;
    let first_after_restore = take_response_by_id(&mut ctx, 1041698);
    assert_eq!(
        first_after_restore["result"]["result"]["value"],
        json!(false)
    );
    let first_surface = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("restored first target")
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        first_surface
            .capability_surface
            .manager_surface
            .policy
            .cookies_enabled_override,
        Some(false)
    );
    assert_eq!(
        first_surface
            .capability_surface
            .manager_surface
            .policy
            .browser_context_overrides,
        moli_cookie_jar::BrowserCookieFacadeContextOverrides::default()
    );

    ctx.process_async(json!({
        "id": 1041699,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(1041699, json!({}), None);

    let second_surface = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("restored second target")
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        second_surface
            .capability_surface
            .manager_surface
            .policy
            .cookies_enabled_override,
        Some(true)
    );
    assert_eq!(
        second_surface
            .capability_surface
            .manager_surface
            .policy
            .browser_context_overrides
            .site_for_cookies_url
            .as_ref()
            .map(url::Url::as_str),
        Some("https://target-b.example/root")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_cookie_manager_surface_after_close_target_promotion()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-COOKIE-CLOSE",
        "TID-000000000CKC",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .set_cookie_manager_policy_cookies_enabled_override_async(false)
        .await;

    ctx.process_async(json!({
        "id": 10416997,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9-COOKIE-CLOSE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(10416997, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10416998,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 10416998)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 10416999,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 10416999);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104169990,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104169990, json!({}), None);

    ctx.conn
        .browser_context
        .as_mut()
        .expect("second target should be active after attach")
        .set_cookie_manager_policy_cookies_enabled_override_async(true)
        .await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("second target should still be active after cookies enabled override")
        .set_cookie_manager_policy_browser_context_overrides_async(
            &moli_cookie_jar::BrowserCookieFacadeContextOverrides::default()
                .with_site_for_cookies_url(
                    &url::Url::parse("https://target-b-close.example/root")
                        .expect("target-b close site-for-cookies url"),
                ),
        )
        .await;

    let second_surface = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("active browser context after second target navigation")
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        second_surface
            .capability_surface
            .manager_surface
            .policy
            .cookies_enabled_override,
        Some(true)
    );
    assert_eq!(
        second_surface
            .capability_surface
            .manager_surface
            .policy
            .browser_context_overrides
            .site_for_cookies_url
            .as_ref()
            .map(url::Url::as_str),
        Some("https://target-b-close.example/root")
    );

    ctx.process_async(json!({
        "id": 10417000,
        "method": "Target.closeTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let close_response = take_response_by_id(&mut ctx, 10417000);
    assert_eq!(close_response["result"]["success"], json!(true));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context after close promotion");
        assert_eq!(active.active_target_id(), Some("TID-000000000CKC"));
    }

    let first_surface = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("restored first target")
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        first_surface
            .capability_surface
            .manager_surface
            .policy
            .cookies_enabled_override,
        Some(false)
    );
    assert_eq!(
        first_surface
            .capability_surface
            .manager_surface
            .policy
            .browser_context_overrides,
        moli_cookie_jar::BrowserCookieFacadeContextOverrides::default()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_loader_overrides_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-UA",
        "TID-000000000UA",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104170,
        "method": "Network.setUserAgentOverride",
        "sessionId": "SID-active",
        "params": { "userAgent": "Moli/Target-A" }
    }))
    .await;
    ctx.expect_result(104170, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104171,
        "method": "Security.setIgnoreCertificateErrors",
        "sessionId": "SID-active",
        "params": { "ignore": true }
    }))
    .await;
    ctx.expect_result(104171, json!({}), Some("SID-active"));

    ctx.conn
        .ensure_resource_request_client()
        .expect("loader for target A");
    assert_eq!(ctx.conn.user_agent(), "Moli/Target-A");
    assert!(!ctx.conn.tls_verify_host());

    ctx.process_async(json!({
        "id": 104172,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9-UA", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104172, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104173,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104173)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104174,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104174);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041740,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000UA" }
    }))
    .await;
    ctx.expect_result(1041740, json!({}), None);

    ctx.process_async(json!({
        "id": 104175,
        "method": "Network.setUserAgentOverride",
        "sessionId": second_session_id,
        "params": { "userAgent": "Moli/Target-B" }
    }))
    .await;
    ctx.expect_result(104175, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104176,
        "method": "Security.setIgnoreCertificateErrors",
        "sessionId": second_session_id,
        "params": { "ignore": false }
    }))
    .await;
    ctx.expect_result(104176, json!({}), Some(&second_session_id));

    {
        let browser_context = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_ne!(
            browser_context.active_target_id(),
            Some(second_target_id.as_str()),
            "the second target must still be in the background"
        );
        let background = browser_context
            .page_target(&second_target_id)
            .expect("background target");
        assert_eq!(
            background.network_policy.user_agent_override(),
            Some("Moli/Target-B")
        );
        assert_eq!(background.tls_verify_host_override, Some(true));
        assert!(
            background
                .navigation_engine()
                .expect("background navigation engine")
                .fetch_config()
                .tls_verify_host(),
            "the exact background engine must be rebuilt before activation"
        );
    }

    ctx.process_async(json!({
        "id": 1041741,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(1041741, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .user_agent_override(),
            Some("Moli/Target-B")
        );
        assert_eq!(
            active.active_page_state().tls_verify_host_override,
            Some(true)
        );
    }
    ctx.conn
        .ensure_resource_request_client()
        .expect("loader for target B");
    assert_eq!(ctx.conn.user_agent(), "Moli/Target-B");
    assert!(ctx.conn.tls_verify_host());

    ctx.process_async(json!({
        "id": 104177,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000UA" }
    }))
    .await;
    ctx.expect_result(104177, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000UA"));
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .user_agent_override(),
            Some("Moli/Target-A")
        );
        assert_eq!(
            active.active_page_state().tls_verify_host_override,
            Some(false)
        );
    }
    ctx.conn
        .ensure_resource_request_client()
        .expect("restored loader for target A");
    assert_eq!(ctx.conn.user_agent(), "Moli/Target-A");
    assert!(!ctx.conn.tls_verify_host());
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_proxy_loader_overrides_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PROXY",
        "TID-000000000PX",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    {
        let active = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("active browser context");
        active.active_page_state_mut().http_proxy_override =
            Some("http://proxy-a.test:8080".into());
        active.active_page_state_mut().http_no_proxy_override = Some("localhost,127.0.0.1".into());
    }
    ctx.conn.invalidate_resource_runtime_async().await;
    ctx.conn
        .ensure_resource_request_client()
        .expect("loader for target A");
    assert_eq!(ctx.conn.http_proxy(), Some("http://proxy-a.test:8080"));
    assert_eq!(ctx.conn.http_no_proxy(), Some("localhost,127.0.0.1"));

    ctx.process_async(json!({
        "id": 1041770,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9-PROXY", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(1041770, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041771,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1041771)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1041772,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041772);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10417721,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10417721, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        active.active_page_state_mut().http_proxy_override =
            Some("http://proxy-b.test:8080".into());
        active.active_page_state_mut().http_no_proxy_override = Some("::1,.example.com".into());
    }
    ctx.conn.invalidate_resource_runtime_async().await;
    ctx.conn
        .ensure_resource_request_client()
        .expect("loader for target B");
    assert_eq!(ctx.conn.http_proxy(), Some("http://proxy-b.test:8080"));
    assert_eq!(ctx.conn.http_no_proxy(), Some("::1,.example.com"));

    ctx.process_async(json!({
        "id": 1041773,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000PX" }
    }))
    .await;
    ctx.expect_result(1041773, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PX"));
        assert_eq!(
            active.active_page_state().http_proxy_override.as_deref(),
            Some("http://proxy-a.test:8080")
        );
        assert_eq!(
            active.active_page_state().http_no_proxy_override.as_deref(),
            Some("localhost,127.0.0.1")
        );
    }
    ctx.conn
        .ensure_resource_request_client()
        .expect("restored loader for target A");
    assert_eq!(ctx.conn.http_proxy(), Some("http://proxy-a.test:8080"));
    assert_eq!(ctx.conn.http_no_proxy(), Some("localhost,127.0.0.1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_loader_overrides_after_close_target_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-UA-CLOSE",
        "TID-000000000UC",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104178,
        "method": "Network.setUserAgentOverride",
        "sessionId": "SID-active",
        "params": { "userAgent": "Moli/Close-A" }
    }))
    .await;
    ctx.expect_result(104178, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104179,
        "method": "Security.setIgnoreCertificateErrors",
        "sessionId": "SID-active",
        "params": { "ignore": true }
    }))
    .await;
    ctx.expect_result(104179, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104180,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9-UA-CLOSE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104180, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104181,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104181)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104182,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104182);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104183,
        "method": "Network.setUserAgentOverride",
        "sessionId": second_session_id,
        "params": { "userAgent": "Moli/Close-B" }
    }))
    .await;
    ctx.expect_result(104183, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104184,
        "method": "Security.setIgnoreCertificateErrors",
        "sessionId": second_session_id,
        "params": { "ignore": false }
    }))
    .await;
    ctx.expect_result(104184, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104185,
        "method": "Target.closeTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let close_response = take_response_by_id(&mut ctx, 104185);
    assert_eq!(close_response["result"]["success"], json!(true));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000UC"));
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .user_agent_override(),
            Some("Moli/Close-A")
        );
        assert_eq!(
            active.active_page_state().tls_verify_host_override,
            Some(false)
        );
    }
    ctx.conn
        .ensure_resource_request_client()
        .expect("restored loader after close promotion");
    assert_eq!(ctx.conn.user_agent(), "Moli/Close-A");
    assert!(!ctx.conn.tls_verify_host());
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_proxy_loader_overrides_after_close_target_promotion()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PROXY-CLOSE",
        "TID-000000000PC",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    {
        let active = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("active browser context");
        active.active_page_state_mut().http_proxy_override =
            Some("http://proxy-close-a.test:8080".into());
        active.active_page_state_mut().http_no_proxy_override = Some("localhost,127.0.0.1".into());
    }
    ctx.conn.invalidate_resource_runtime_async().await;
    ctx.conn
        .ensure_resource_request_client()
        .expect("loader for target A");
    assert_eq!(
        ctx.conn.http_proxy(),
        Some("http://proxy-close-a.test:8080")
    );
    assert_eq!(ctx.conn.http_no_proxy(), Some("localhost,127.0.0.1"));

    ctx.process_async(json!({
        "id": 1041860,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9-PROXY-CLOSE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(1041860, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041861,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1041861)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1041862,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041862);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10418620,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10418620, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("active browser context");
        active.active_page_state_mut().http_proxy_override =
            Some("http://proxy-close-b.test:8080".into());
        active.active_page_state_mut().http_no_proxy_override = Some("::1,.example.com".into());
    }
    ctx.conn.invalidate_resource_runtime_async().await;
    ctx.conn
        .ensure_resource_request_client()
        .expect("loader for target B");
    assert_eq!(
        ctx.conn.http_proxy(),
        Some("http://proxy-close-b.test:8080")
    );
    assert_eq!(ctx.conn.http_no_proxy(), Some("::1,.example.com"));

    ctx.process_async(json!({
        "id": 1041863,
        "method": "Target.closeTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let close_response = take_response_by_id(&mut ctx, 1041863);
    assert_eq!(close_response["result"]["success"], json!(true));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PC"));
        assert_eq!(
            active.active_page_state().http_proxy_override.as_deref(),
            Some("http://proxy-close-a.test:8080")
        );
        assert_eq!(
            active.active_page_state().http_no_proxy_override.as_deref(),
            Some("localhost,127.0.0.1")
        );
    }
    ctx.conn
        .ensure_resource_request_client()
        .expect("restored loader after close promotion");
    assert_eq!(
        ctx.conn.http_proxy(),
        Some("http://proxy-close-a.test:8080")
    );
    assert_eq!(ctx.conn.http_no_proxy(), Some("localhost,127.0.0.1"));
}
