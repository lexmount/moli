use super::*;

/// Network.getCookies returns an empty list.
#[tokio::test(flavor = "multi_thread")]
async fn get_cookies_returns_empty() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 2, "method": "Network.getCookies"}))
        .await;
    ctx.expect_error(2, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn get_all_cookies_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 2_1, "method": "Network.getAllCookies"}))
        .await;
    ctx.expect_error(2_1, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn network_get_all_cookies_returns_unfiltered_browser_context_cookies() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-GAC".into()));

    ctx.process_async(json!({
        "id": 2_2,
        "method": "Network.setCookies",
        "params": {
            "cookies": [
                { "name": "alpha", "value": "1", "url": "https://example.com/app" },
                { "name": "beta", "value": "2", "url": "https://other.example/app" }
            ]
        }
    }))
    .await;
    ctx.expect_result(2_2, json!({}), None);

    ctx.process_async(json!({
        "id": 2_3,
        "method": "Network.getCookies",
        "params": { "urls": ["https://example.com/app"] }
    }))
    .await;
    let filtered = ctx.take_response_by_id(2_3);
    let filtered_cookies = filtered["result"]["cookies"]
        .as_array()
        .expect("filtered cookies array");
    assert_eq!(filtered_cookies.len(), 1);
    assert_eq!(filtered_cookies[0]["name"], json!("alpha"));

    ctx.process_async(json!({
        "id": 2_4,
        "method": "Network.getAllCookies"
    }))
    .await;
    let all = ctx.take_response_by_id(2_4);
    let mut names = all["result"]["cookies"]
        .as_array()
        .expect("all cookies array")
        .iter()
        .map(|cookie| cookie["name"].as_str().expect("cookie name").to_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, vec!["alpha", "beta"]);
}
#[tokio::test(flavor = "multi_thread")]
async fn set_extra_http_headers_replaces_previous_headers() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("NID-A", "TID-A"));

    ctx.process_async(json!({"id": 2, "method": "Network.enable"}))
        .await;
    ctx.expect_result(2, json!({}), None);

    ctx.process_async(json!({"id": 3, "method": "Network.setExtraHTTPHeaders", "params": { "headers": { "foo": "bar" }}}))
        .await;
    ctx.expect_result(3, json!({}), None);

    ctx.process_async(json!({"id": 4, "method": "Network.setExtraHTTPHeaders", "params": { "headers": { "food": "bars" }}}))
        .await;
    ctx.expect_result(4, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_page_target()
            .effective_policy()
            .extra_headers()
            .len(),
        1
    );
    assert_eq!(
        bc.active_page_target().effective_policy().extra_headers()[0],
        ("food".to_owned(), "bars".to_owned())
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_applies_to_subsequent_navigation_requests() {
    async fn handler(
        State(seen): State<Arc<Mutex<Option<String>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = user_agent;
        "<!doctype html><html><body>ok</body></html>"
    }

    let seen = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 30,
        "method": "Network.setUserAgentOverride",
        "params": { "userAgent": "moli-network-ua" }
    }))
    .await;
    ctx.expect_result(30, json!({}), None);

    ctx.process_async(json!({
        "id": 31,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let _ = ctx.take_all();
    assert_eq!(seen.lock().as_deref(), Some("moli-network-ua"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn background_target_user_agent_override_reaches_replacement_document() {
    async fn handler(
        State(seen): State<Arc<Mutex<Vec<String>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        seen.lock().push(user_agent);
        "<!doctype html><html><body>ok</body></html>"
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-default".to_owned());
    browser_context.set_active_target_id("TID-initial");
    let renderer_runtime = browser_context.renderer_runtime_owner_access();
    let engine = NavigationEngine::new_with_fetch_config_and_browser_context_access(
        ctx.conn.fetch_config().clone(),
        renderer_runtime,
        OptionalResourceFetchMask::NONE,
        true,
    )
    .expect("new BrowserContext owner should be live");
    ctx.conn.replace_standalone_navigation_engine(engine);
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);

    ctx.process_async(json!({
        "id": 35,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let target_id = ctx.take_response_by_id(35)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();

    ctx.process_async(json!({
        "id": 36,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id, "flatten": true }
    }))
    .await;
    let session_id = ctx.take_response_by_id(36)["result"]["sessionId"]
        .as_str()
        .expect("attached session id")
        .to_owned();
    ctx.take_first_matching("background target attached event", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(session_id)
    });

    ctx.process_async(json!({
        "id": 37,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    let first_navigation = ctx.take_response_by_id(37);
    assert_eq!(first_navigation["result"]["frameId"], json!(target_id));
    let first_loader_id = first_navigation["result"]["loaderId"]
        .as_str()
        .expect("first navigation loader id")
        .to_owned();
    wait_until_renderer_document_load(&mut ctx, Some(&session_id), &target_id, &first_loader_id)
        .await;

    ctx.process_async(json!({
        "id": 38,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "document.readyState", "returnByValue": true }
    }))
    .await;
    assert_eq!(
        ctx.take_response_by_id(38)["result"]["result"]["value"],
        json!("complete")
    );

    ctx.process_async(json!({
        "id": 39,
        "method": "Network.setUserAgentOverride",
        "sessionId": session_id,
        "params": { "userAgent": "Moli/Background-Target" }
    }))
    .await;
    ctx.expect_result(39, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 40,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    let second_navigation = ctx.take_response_by_id(40);
    assert_eq!(second_navigation["result"]["frameId"], json!(target_id));
    let second_loader_id = second_navigation["result"]["loaderId"]
        .as_str()
        .expect("second navigation loader id")
        .to_owned();
    wait_until_renderer_document_load(&mut ctx, Some(&session_id), &target_id, &second_loader_id)
        .await;

    ctx.process_async(json!({
        "id": 41,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "navigator.userAgent", "returnByValue": true }
    }))
    .await;
    assert_eq!(
        seen.lock().last().map(String::as_str),
        Some("Moli/Background-Target")
    );
    assert_eq!(
        ctx.take_response_by_id(41)["result"]["result"]["value"],
        json!("Moli/Background-Target")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_applies_to_current_page_fetch_requests() {
    async fn handler(
        State(seen): State<Arc<Mutex<Option<String>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = user_agent;
        ([(ACCESS_CONTROL_ALLOW_ORIGIN.as_str(), "*")], "ok")
    }

    let seen = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/api", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>ok</body>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 33,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 34,
        "method": "Network.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": { "userAgent": "moli-network-live-ua" }
    }))
    .await;
    ctx.expect_result(34, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 35,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!("fetch('http://{addr}/api').then(r => r.text())")
        }
    }))
    .await;

    flush_until_subresource_finished(
        &mut ctx,
        "Fetch",
        1,
        "runtime fetch after live user agent override",
    )
    .await;

    assert_eq!(seen.lock().as_deref(), Some("moli-network-live-ua"));

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_rejects_invalid_params() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 32,
        "method": "Network.setUserAgentOverride",
        "params": {}
    }))
    .await;
    ctx.expect_error(32, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn network_get_cookies_omits_same_site_for_unspecified_cookie() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N-U".into()));

    ctx.process_async(json!({
        "id": 101,
        "method": "Network.setCookie",
        "params": {
            "name": "plain",
            "value": "1",
            "url": "https://example.com/app"
        }
    }))
    .await;
    ctx.expect_result(101, json!({}), None);

    ctx.process_async(json!({
        "id": 102,
        "method": "Network.getCookies",
        "params": { "urls": ["https://example.com/app"] }
    }))
    .await;
    let result = ctx.take_one();
    let cookies = result["result"]["cookies"]
        .as_array()
        .expect("network cookies array");
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].get("sameSite").is_none());
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_returns_success_and_cookie_report() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N1".into()));

    ctx.process_async(json!({
        "id": 40,
        "method": "Network.setCookie",
        "params": {
            "name": "strict",
            "value": "1",
            "url": "https://example.com/app",
            "secure": true,
            "sameSite": "Strict"
        }
    }))
    .await;
    ctx.expect_result(
        40,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "effectiveSameSite": "Strict",
                "warningReasons": []
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_reports_secure_access_warning_for_localhost_http() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N1W".into()));

    ctx.process_async(json!({
        "id": 42,
        "method": "Network.setCookie",
        "params": {
            "name": "sid",
            "value": "1",
            "url": "http://localhost/app",
            "secure": true
        }
    }))
    .await;
    ctx.expect_result(
        42,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "effectiveSameSite": "NoRestriction",
                "warningReasons": ["SecureAccessGrantedNonCryptographic"]
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_returns_rejected_cookie_report_without_protocol_error() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N2".into()));

    ctx.process_async(json!({
        "id": 41,
        "method": "Network.setCookie",
        "params": {
            "name": "cross",
            "value": "1",
            "url": "https://example.com/app",
            "secure": false,
            "sameSite": "None"
        }
    }))
    .await;
    ctx.expect_result(
        41,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "SameSiteNoneRequiresSecure"
                },
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_reports_structured_facade_rejections_without_protocol_error() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N3".into()));

    ctx.process_async(json!({
        "id": 43,
        "method": "Network.setCookie",
        "params": {
            "name": "sid",
            "value": "1",
            "url": "https://example.com/app",
            "domain": ".example.com",
            "path": "app"
        }
    }))
    .await;
    ctx.expect_result(
        43,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "PathMustStartWithSlash"
                },
                "rejectionReasons": ["PathMustStartWithSlash"],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_accepts_leading_dot_domain() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N3-DOT".into()));

    ctx.process_async(json!({
        "id": 431,
        "method": "Network.setCookie",
        "params": {
            "name": "sid",
            "value": "1",
            "url": "https://example.com/app",
            "domain": ".example.com",
            "path": "/"
        }
    }))
    .await;
    ctx.expect_result(
        431,
        json!({
            "success": true,
            "cookieReports": [{
                "status": {
                    "kind": "Accepted",
                    "storeAction": "Inserted"
                },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_reports_invalid_url_as_cookie_rejection() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N4".into()));

    ctx.process_async(json!({
        "id": 44,
        "method": "Network.setCookie",
        "params": {
            "name": "sid",
            "value": "1",
            "url": "https://example.com:bad/app"
        }
    }))
    .await;
    ctx.expect_result(
        44,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "InvalidUrl"
                },
                "rejectionReasons": ["InvalidUrl", "UnspecifiedDomain"],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_reports_structured_name_value_rejections_without_protocol_error() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N5".into()));

    ctx.process_async(json!({
        "id": 45,
        "method": "Network.setCookie",
        "params": {
            "name": "",
            "value": "a=b",
            "url": "https://example.com/app"
        }
    }))
    .await;
    ctx.expect_result(
        45,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "EmptyNameValueContainsEquals"
                },
                "rejectionReasons": ["EmptyNameValueContainsEquals"],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_uses_browser_context_default_cookie_url_when_missing() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-N6", "TID-N6");
    bc.set_target_url("https://example.com/path".into());
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 46,
        "method": "Network.setCookie",
        "params": {
            "name": "sid",
            "value": "1"
        }
    }))
    .await;
    ctx.expect_result(
        46,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 47,
        "method": "Network.getCookies",
        "params": { "urls": ["https://example.com/path"] }
    }))
    .await;
    ctx.expect_result(
        47,
        json!({
            "cookies": [{
                "name": "sid",
                "value": "1",
                "domain": "example.com",
                "path": "/",
                "size": 4,
                "secure": true
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_reports_missing_cookie_url_when_no_default_scope_exists() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N7".into()));

    ctx.process_async(json!({
        "id": 48,
        "method": "Network.setCookie",
        "params": {
            "name": "sid",
            "value": "1"
        }
    }))
    .await;
    ctx.expect_result(
        48,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "MissingCookieUrl"
                },
                "rejectionReasons": ["MissingCookieUrl"],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_set_cookie_keeps_cookie_store_available_after_lock_holder_panic() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-N-store-panic".into()));
    let cookie_store = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .cookie_store_for_test()
        .clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = cookie_store.lock();
        panic!("panic while holding cookie store lock");
    }));

    ctx.process_async(json!({
        "id": 480,
        "method": "Network.setCookie",
        "params": {
            "name": "sid",
            "value": "1",
            "url": "https://example.com/"
        }
    }))
    .await;
    ctx.expect_result(
        480,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 481,
        "method": "Network.getCookies",
        "params": { "urls": ["https://example.com/"] }
    }))
    .await;
    ctx.expect_result(
        481,
        json!({
            "cookies": [{
                "name": "sid",
                "value": "1",
                "domain": "example.com",
                "path": "/",
                "size": 4,
                "secure": true
            }]
        }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_get_cookies_lists_same_site_none_secure_cookie_for_loopback_http_url() {
    // Regression: a SameSite=None; Secure cookie stored for a loopback origin
    // was dropped from the Network.getCookies listing when queried with an
    // http:// loopback URL, even though the cookie read path (document.cookie)
    // treats loopback as a secure context and shows it. Chromium lists it too.
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-NONEC".into()));

    ctx.process_async(json!({
        "id": 50,
        "method": "Network.setCookie",
        "params": {
            "name": "nonec",
            "value": "1",
            "url": "http://127.0.0.1:8765/",
            "secure": true,
            "sameSite": "None"
        }
    }))
    .await;
    let set = ctx.take_response_by_id(50);
    assert_eq!(
        set["result"]["success"],
        json!(true),
        "SameSite=None; Secure cookie should be accepted for a loopback origin; got {set}"
    );

    ctx.process_async(json!({
        "id": 51,
        "method": "Network.getCookies",
        "params": { "urls": ["http://127.0.0.1:8765/"] }
    }))
    .await;
    let listed = ctx.take_response_by_id(51);
    let cookies = listed["result"]["cookies"]
        .as_array()
        .expect("getCookies cookies array");
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie["name"] == json!("nonec")),
        "SameSite=None; Secure cookie must appear for an http loopback URL; got {cookies:?}"
    );
}
