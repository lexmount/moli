use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn process_environment_conflicts_do_not_stage_navigation_policy_or_change_request_language() {
    async fn handler(
        State(seen): State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
        headers: HeaderMap,
        uri: Uri,
    ) -> impl IntoResponse {
        seen.lock().push((
            uri.path().to_owned(),
            headers
                .get(axum::http::header::ACCEPT_LANGUAGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LOCALE-TZ",
        "TID-000000000PL",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 10419480,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(10419480, json!({}), None);

    ctx.process_async(json!({
        "id": 10419481,
        "method": "Emulation.setLocaleOverride",
        "sessionId": "SID-active",
        "params": { "locale": "en-GB" }
    }))
    .await;
    ctx.expect_result(10419481, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419482,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": "SID-active",
        "params": { "timezoneId": "UTC" }
    }))
    .await;
    ctx.expect_result(10419482, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419483,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LOCALE-TZ", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(10419483, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10419484,
        "method": "Emulation.setLocaleOverride",
        "sessionId": second_session_id,
        "params": { "locale": "fr-FR" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 10419484)["error"]["message"],
        "Another locale override is already in effect"
    );

    ctx.process_async(json!({
        "id": 10419485,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": second_session_id,
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 10419485)["error"]["message"],
        "Timezone override is already in effect"
    );

    {
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(
                    active
                        .active_page_target()
                        .devtools_sessions
                        .effective_locale_override()
                        .as_deref(),
                    Some("en-GB")
                );
                assert_eq!(
                    active
                        .active_page_target()
                        .devtools_sessions
                        .effective_timezone_override()
                        .as_deref(),
                    Some("UTC")
                );

                active
                    .background_target(&second_target_id)
                    .expect("second target is present without a process environment claim")
            };
            assert_eq!(
                staged
                    .devtools_sessions
                    .effective_locale_override()
                    .as_deref(),
                None,
                "a rejected process claim must not be replayed by navigation"
            );
            assert_eq!(
                staged
                    .devtools_sessions
                    .effective_timezone_override()
                    .as_deref(),
                None
            );
        });
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 10419486,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419486);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 10419487,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ lang: navigator.language, locale: Intl.DateTimeFormat().resolvedOptions().locale, tz: Intl.DateTimeFormat().resolvedOptions().timeZone })"
            }
        }))
    .await;
    let active_eval = take_response_by_id(&mut ctx, 10419487);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["lang"], json!("en-US"));
    assert_eq!(active_payload["locale"], json!("en-GB"));
    assert_eq!(active_payload["tz"], json!("UTC"));

    ctx.process_async(json!({
        "id": 10419488,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PL"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419488);
    ctx.take_all();

    // The rejected values were not staged. After the owning target closes,
    // this session can explicitly acquire both claims for the first time.
    for (id, method, params) in [
        (
            104194880,
            "Emulation.setLocaleOverride",
            json!({"locale": "fr-FR"}),
        ),
        (
            104194881,
            "Emulation.setTimezoneOverride",
            json!({"timezoneId": "Asia/Shanghai"}),
        ),
    ] {
        ctx.process_async(json!({"id": id, "method": method,
            "sessionId": second_session_id, "params": params}))
            .await;
        ctx.expect_result(id, json!({}), Some(&second_session_id));
    }

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 10419489,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419489);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
            "id": 10419490,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ lang: navigator.language, locale: Intl.DateTimeFormat().resolvedOptions().locale, tz: Intl.DateTimeFormat().resolvedOptions().timeZone })"
            }
        }))
    .await;
    let activated_eval = take_response_by_id(&mut ctx, 10419490);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["lang"], json!("en-US"));
    assert_eq!(activated_payload["locale"], json!("fr-FR"));
    assert_eq!(activated_payload["tz"], json!("Asia/Shanghai"));

    let seen = seen.lock().clone();
    assert_eq!(
        seen,
        vec![
            ("/page-a".to_owned(), Some("en-US,en;q=0.9".to_owned())),
            ("/page-b".to_owned(), Some("en-US,en;q=0.9".to_owned()))
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_locale_before_activation() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let accept_language = headers
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        axum::response::Html(format!(
            "<!doctype html><html><body data-accept-language=\"{accept_language}\"><script>document.body.textContent = [navigator.language, document.body.dataset.acceptLanguage].join('|');</script></body></html>"
        ))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LOCALE-CLEAR",
        "TID-000000000PLC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 104194801,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194801, json!({}), None);

    ctx.process_async(json!({
        "id": 104194802,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LOCALE-CLEAR", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194802, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194803,
        "method": "Emulation.setLocaleOverride",
        "sessionId": second_session_id,
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(104194803, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194804,
        "method": "Emulation.setLocaleOverride",
        "sessionId": second_session_id,
        "params": {}
    }))
    .await;
    ctx.expect_result(104194804, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
let active = {
            conn.browser_context
                .as_ref()
                .expect("active browser context")
        };
        assert!(
            active
                .active_page_target()
                .devtools_sessions
                .effective_locale_override()
                .as_deref()
                .is_none(),
            "active target should keep its default locale override",
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "clearing staged locale back to default should fold away the background state entry",
        );
});
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 104194805,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194805);
    ctx.take_all();

    let active_html = loaded_page_html_for_test(&mut ctx).await;
    let active_surface = active_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("active payload should be embedded in body")
        .to_owned();
    assert!(
        active_surface.starts_with("en-US|en-US,en;q=0.9"),
        "active target should retain the default navigator and request languages: {active_surface}"
    );

    ctx.process_async(json!({
        "id": 104194806,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PLC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194806);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 104194807,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194807);
    ctx.take_all();

    let activated_html = loaded_page_html_for_test(&mut ctx).await;
    let activated_surface = activated_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("activated payload should be embedded in body")
        .to_owned();

    assert_eq!(
        activated_surface, active_surface,
        "activated target should observe default locale surface after clearing its staged override"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_timezone_before_activation() {
    async fn handler() -> impl IntoResponse {
        axum::response::Html(
            "<!doctype html><html><body><script>document.body.textContent = Intl.DateTimeFormat().resolvedOptions().timeZone;</script></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-TIMEZONE-CLEAR",
        "TID-000000000PTC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 104194808,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194808, json!({}), None);

    ctx.process_async(json!({
        "id": 104194809,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-TIMEZONE-CLEAR", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194809, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194810,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": second_session_id,
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    ctx.expect_result(104194810, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194811,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": second_session_id,
        "params": { "timezoneId": "" }
    }))
    .await;
    ctx.expect_result(104194811, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
let active = {
            conn.browser_context
                .as_ref()
                .expect("active browser context")
        };
        assert!(
            active
                .active_page_target()
                .devtools_sessions
                .effective_timezone_override()
                .as_deref()
                .is_none(),
            "active target should keep its default timezone override",
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "clearing staged timezone back to default should fold away the background state entry",
        );
});
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 104194812,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194812);
    ctx.take_all();

    let active_html = loaded_page_html_for_test(&mut ctx).await;
    let active_surface = active_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("active payload should be embedded in body")
        .to_owned();
    assert_ne!(active_surface, "Asia/Shanghai");

    ctx.process_async(json!({
        "id": 104194813,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PTC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194813);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 104194814,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194814);
    ctx.take_all();

    let activated_html = loaded_page_html_for_test(&mut ctx).await;
    let activated_surface = activated_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("activated payload should be embedded in body")
        .to_owned();

    assert_eq!(
        activated_surface, active_surface,
        "activated target should observe default timezone surface after clearing its staged override"
    );
    assert_ne!(
        activated_surface, "Asia/Shanghai",
        "activated target should not retain the staged timezone override"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_emulation_overrides_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-EMU",
        "TID-000000000PE",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 104194901,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194901, json!({}), None);

    ctx.process_async(json!({
        "id": 104194902,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": "SID-active",
        "params": {
            "width": 1280,
            "height": 720,
            "deviceScaleFactor": 2,
            "screenWidth": 1440,
            "screenHeight": 900,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(104194902, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194903,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": "SID-active",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194903, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194904,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": "SID-active",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194904, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194905,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-EMU", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194905, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194906,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": second_session_id,
        "params": {
            "width": 640,
            "height": 360,
            "deviceScaleFactor": 1,
            "screenWidth": 800,
            "screenHeight": 600,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(104194906, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194907,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(104194907, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194908,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(104194908, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(
                    active
                        .active_page_target()
                        .effective_emulation_state
                        .emulated_device_metrics
                        .as_ref()
                        .map(|metrics| (
                            metrics.width,
                            metrics.height,
                            metrics.device_scale_factor,
                            metrics.screen_width,
                            metrics.screen_height
                        )),
                    Some((1280, 720, 2.0, 1440, 900))
                );
                assert!(
                    active
                        .active_page_target()
                        .effective_emulation_state
                        .max_touch_points
                        != 0
                );
                assert!(
                    active
                        .active_page_target()
                        .effective_emulation_state
                        .focus_emulation_enabled
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert_eq!(
                staged
                    .effective_emulation_state
                    .emulated_device_metrics
                    .as_ref()
                    .map(|metrics| (
                        metrics.width,
                        metrics.height,
                        metrics.device_scale_factor,
                        metrics.screen_width,
                        metrics.screen_height
                    )),
                Some((640, 360, 1.0, 800, 600))
            );
            assert_eq!(staged.effective_emulation_state.max_touch_points, 0);
            assert!(!staged.effective_emulation_state.focus_emulation_enabled);
        });
    }

    ctx.process_async(json!({
        "id": 104194909,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194909);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194910,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height, maxTouchPoints: navigator.maxTouchPoints, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 104194910);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["innerWidth"], json!(1280));
    assert_eq!(active_payload["innerHeight"], json!(720));
    assert_eq!(active_payload["dpr"], json!(2));
    assert_eq!(active_payload["screenWidth"], json!(1440));
    assert_eq!(active_payload["screenHeight"], json!(900));
    assert_eq!(active_payload["maxTouchPoints"], json!(1));
    assert_eq!(active_payload["hasFocus"], json!(true));
    assert_eq!(active_payload["hidden"], json!(false));
    assert_eq!(active_payload["visibilityState"], json!("visible"));

    ctx.process_async(json!({
        "id": 104194911,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PE"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194911);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194912,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194912);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194913,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height, maxTouchPoints: navigator.maxTouchPoints, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let activated_eval = take_response_by_id(&mut ctx, 104194913);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["innerWidth"], json!(640));
    assert_eq!(activated_payload["innerHeight"], json!(360));
    assert_eq!(activated_payload["dpr"], json!(1));
    assert_eq!(activated_payload["screenWidth"], json!(800));
    assert_eq!(activated_payload["screenHeight"], json!(600));
    assert_eq!(activated_payload["maxTouchPoints"], json!(0));
    assert_eq!(activated_payload["hasFocus"], json!(true));
    assert_eq!(activated_payload["hidden"], json!(false));
    assert_eq!(activated_payload["visibilityState"], json!("visible"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_page_settings_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-PAGE",
        "TID-000000000PP",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 104194914,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194914, json!({}), None);

    ctx.process_async(json!({
        "id": 104194915,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-PAGE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194915, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194916,
        "method": "Page.setBypassCSP",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194916, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194917,
        "method": "Page.setFontFamilies",
        "sessionId": second_session_id,
        "params": {
            "standard": "Georgia",
            "fixed": "Fira Code"
        }
    }))
    .await;
    ctx.expect_result(104194917, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194918,
        "method": "Page.setInterceptFileChooserDialog",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194918, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("staged page settings for background target")
            };
            assert!(
                staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .page_bypass_csp_enabled
            );
            assert_eq!(
                staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .page_font_families
                    .get("standard"),
                Some(&json!("Georgia"))
            );
            assert_eq!(
                staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .page_font_families
                    .get("fixed"),
                Some(&json!("Fira Code"))
            );
            assert!(
                staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .page_intercept_file_chooser_dialog_enabled
            );
        });
    }

    ctx.process_async(json!({
        "id": 104194919,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104194919, json!({}), None);

    ctx.with_fixture(|conn| {
        let active = {
            conn.browser_context
                .as_ref()
                .expect("activated browser context")
        };
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_bypass_csp_enabled
        );
        assert_eq!(
            active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_font_families
                .get("standard"),
            Some(&json!("Georgia"))
        );
        assert_eq!(
            active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_font_families
                .get("fixed"),
            Some(&json!("Fira Code"))
        );
        assert!(
            active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_device_metrics_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-EMU-CLEAR",
        "TID-000000000PM",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 1041949131,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949131, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949132,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-EMU-CLEAR", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949132, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949133,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": second_session_id,
        "params": {
            "width": 640,
            "height": 360,
            "deviceScaleFactor": 1,
            "screenWidth": 800,
            "screenHeight": 600,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(1041949133, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949134,
        "method": "Emulation.clearDeviceMetricsOverride",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949134, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
let active = {
            conn.browser_context
                .as_ref()
                .expect("active browser context")
        };
        assert!(
            active
                .active_page_target()
                .effective_emulation_state
                .emulated_device_metrics
                .is_none(),
            "active target should keep its default device metrics",
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "clearing staged device metrics back to default should fold away the background state entry",
        );
});
    }

    ctx.process_async(json!({
        "id": 1041949135,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949135);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949136,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 1041949136);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");

    ctx.process_async(json!({
        "id": 1041949137,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PM"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949137);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949138,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949138);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949139,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height })"
            }
        })).await;
    let activated_eval = take_response_by_id(&mut ctx, 1041949139);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");

    assert_eq!(
        activated_payload, active_payload,
        "activated target should observe default metrics after clearing its staged override"
    );
    assert_ne!(activated_payload["innerWidth"], json!(640));
    assert_ne!(activated_payload["innerHeight"], json!(360));
    assert_ne!(activated_payload["screenWidth"], json!(800));
    assert_ne!(activated_payload["screenHeight"], json!(600));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_touch_and_focus_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-EMU-CLEAR-TF",
        "TID-000000000PT",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 1041949141,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949141, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949142,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-EMU-CLEAR-TF", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949142, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949143,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(1041949143, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949144,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(1041949144, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949145,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(1041949145, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949146,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(1041949146, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
let active = {
            conn.browser_context
                .as_ref()
                .expect("active browser context")
        };
        assert!(
            active
                .active_page_target()
                .effective_emulation_state
                .max_touch_points
                == 0,
            "active target should keep default touch emulation"
        );
        assert!(
            !active
                .active_page_target()
                .effective_emulation_state
                .focus_emulation_enabled,
            "active target should keep default focus emulation"
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "clearing staged touch/focus back to defaults should fold away the background state entry",
        );
});
    }

    ctx.process_async(json!({
        "id": 1041949147,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949147);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949148,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ maxTouchPoints: navigator.maxTouchPoints, hasFocusType: typeof document.hasFocus, hasFocusValue: typeof document.hasFocus === 'function' ? document.hasFocus() : null, hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 1041949148);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");

    ctx.process_async(json!({
        "id": 1041949149,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PT"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949149);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949150,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949150);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949151,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ maxTouchPoints: navigator.maxTouchPoints, hasFocusType: typeof document.hasFocus, hasFocusValue: typeof document.hasFocus === 'function' ? document.hasFocus() : null, hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let activated_eval = take_response_by_id(&mut ctx, 1041949151);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");

    assert_eq!(
        activated_payload, active_payload,
        "activated target should observe default touch/focus surfaces after clearing staged overrides"
    );
    assert_ne!(activated_payload["maxTouchPoints"], json!(1));
    assert_eq!(active_payload["hasFocusType"], json!("function"));
    assert_eq!(active_payload["hasFocusValue"], json!(true));
    assert_eq!(active_payload["hidden"], json!(false));
    assert_eq!(active_payload["visibilityState"], json!("visible"));
}
