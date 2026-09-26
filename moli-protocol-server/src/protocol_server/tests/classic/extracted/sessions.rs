use super::*;

#[tokio::test]
async fn webdriver_classic_status_session_and_delete_routes_use_value_envelope() {
    let app = build_router(test_state());

    let (status_status, status_headers, status) =
        classic_request_status_headers_and_json(app.clone(), Method::GET, "/status").await;
    assert_eq!(status_status, StatusCode::OK);
    assert_classic_webdriver_json_headers(&status_headers);
    assert_eq!(
        status,
        json!({
            "value": {
                "ready": true,
                "message": ""
            }
        })
    );

    let (new_session_status, new_session_headers, session) =
        classic_request_status_headers_and_json(app.clone(), Method::POST, "/session").await;
    assert_eq!(new_session_status, StatusCode::OK);
    assert_classic_webdriver_json_headers(&new_session_headers);
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id")
        .to_owned();
    assert_eq!(session_id, "classic-session-1");
    assert_eq!(
        session["value"]["capabilities"]["browserName"],
        json!("moli")
    );
    assert_eq!(
        session["value"]["capabilities"]["pageLoadStrategy"],
        json!("normal")
    );
    assert_eq!(
        session["value"]["capabilities"]["webSocketUrl"],
        json!(format!("ws://127.0.0.1:9222/session/{session_id}"))
    );

    let running_status = classic_request_json(app.clone(), Method::GET, "/status").await;
    assert_eq!(
        running_status,
        json!({
            "value": {
                "ready": false,
                "message": ""
            }
        })
    );

    let (default_timeouts_status, default_timeouts_headers, default_timeouts) =
        classic_request_status_headers_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/timeouts"),
        )
        .await;
    assert_eq!(default_timeouts_status, StatusCode::OK);
    assert_classic_webdriver_json_headers(&default_timeouts_headers);
    assert_eq!(
        default_timeouts,
        json!({
            "value": {
                "script": 30000,
                "pageLoad": 300000,
                "implicit": 0
            }
        })
    );

    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({
            "script": 25,
            "implicit": 4
        }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));

    let updated_timeouts = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/timeouts"),
    )
    .await;
    assert_eq!(
        updated_timeouts,
        json!({
            "value": {
                "script": 25,
                "pageLoad": 300000,
                "implicit": 4
            }
        })
    );

    let (invalid_timeout_status, invalid_timeout_headers, invalid_timeout) =
        classic_request_status_headers_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/timeouts"),
            json!({ "script": -1 }),
        )
        .await;
    assert_eq!(invalid_timeout_status, StatusCode::BAD_REQUEST);
    assert_classic_webdriver_json_headers(&invalid_timeout_headers);
    assert_eq!(invalid_timeout["value"]["error"], json!("invalid argument"));

    let initial_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(initial_url, json!({ "value": "about:blank" }));

    let deleted = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    assert_eq!(deleted, json!({ "value": null }));

    let stopped_status = classic_request_json(app.clone(), Method::GET, "/status").await;
    assert_eq!(
        stopped_status,
        json!({
            "value": {
                "ready": true,
                "message": ""
            }
        })
    );

    let (missing_status, missing_headers, missing) = classic_request_status_headers_and_json(
        app,
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_classic_webdriver_json_headers(&missing_headers);
    assert_eq!(missing["value"]["error"], json!("invalid session id"));
}
#[tokio::test]
async fn webdriver_classic_new_session_rejects_unmatched_browser_name_before_allocation() {
    let app = build_router(test_state());

    let (status, response) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        "/session",
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "browserName": "moli-smoke-impossible-browser"
                }
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response["value"]["error"], json!("session not created"));
    assert_eq!(
        response["value"]["message"],
        json!("No matching capabilities found")
    );
    let status = classic_request_json(app, Method::GET, "/status").await;
    assert_eq!(status["value"]["ready"], json!(true));
}
#[tokio::test]
async fn webdriver_classic_timeouts_match_wpt_null_integer_and_unknown_field_semantics() {
    // Ported from Chromium/WPT webdriver/tests/classic/get_timeouts/get.py and
    // webdriver/tests/classic/set_timeouts/set.py.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let timeouts_path = format!("/session/{session_id}/timeouts");

    let default_timeouts = classic_request_json(app.clone(), Method::GET, &timeouts_path).await;
    assert_eq!(
        default_timeouts,
        json!({
            "value": {
                "script": 30000,
                "pageLoad": 300000,
                "implicit": 0
            }
        })
    );

    let unknown_fields =
        classic_request_json_with_body(app.clone(), Method::POST, &timeouts_path, json!({"a": 42}))
            .await;
    assert_eq!(unknown_fields, json!({ "value": null }));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &timeouts_path).await,
        default_timeouts
    );

    let (empty_status, empty_response) =
        classic_request_status_and_json(app.clone(), Method::POST, &timeouts_path).await;
    assert_eq!(empty_status, StatusCode::BAD_REQUEST);
    assert_eq!(empty_response["value"]["error"], json!("invalid argument"));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &timeouts_path).await,
        default_timeouts
    );

    let null_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &timeouts_path,
        json!({
            "script": null,
            "pageLoad": null,
            "implicit": null
        }),
    )
    .await;
    assert_eq!(null_timeouts, json!({ "value": null }));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &timeouts_path).await,
        json!({
            "value": {
                "script": null,
                "pageLoad": null,
                "implicit": null
            }
        })
    );

    let safe_integer = 9_007_199_254_740_991_u64;
    for key in ["script", "pageLoad", "implicit"] {
        let set = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &timeouts_path,
            json!({ key: safe_integer }),
        )
        .await;
        assert_eq!(set, json!({ "value": null }), "setting {key}");
        assert_eq!(
            classic_request_json(app.clone(), Method::GET, &timeouts_path).await["value"][key],
            json!(safe_integer),
            "getting {key}"
        );

        let set_integer_float = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &timeouts_path,
            json!({ key: 2.0 }),
        )
        .await;
        assert_eq!(
            set_integer_float,
            json!({ "value": null }),
            "setting integer-valued float for {key}"
        );
        assert_eq!(
            classic_request_json(app.clone(), Method::GET, &timeouts_path).await["value"][key],
            json!(2),
            "getting integer-valued float for {key}"
        );

        for invalid in [json!(-1), json!(2.5), json!(9_007_199_254_740_992_u64)] {
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &timeouts_path,
                json!({ key: invalid }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "invalid {key}");
            assert_eq!(response["value"]["error"], json!("invalid argument"));
        }

        for invalid in [json!([]), json!({}), json!(false), json!("10")] {
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &timeouts_path,
                json!({ key: invalid }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "invalid type for {key}");
            assert_eq!(response["value"]["error"], json!("invalid argument"));
        }
    }
}
#[tokio::test]
async fn webdriver_classic_find_element_honors_implicit_timeout() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "implicit": 500 }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));

    let url = "data:text/html,<body>implicit<script>setTimeout(function(){var node=document.createElement('main');node.className='late';node.textContent='late';document.body.appendChild(node);},100);</script></body>";
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let late = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": ".late"
        }),
    )
    .await;
    assert!(
        late["value"]["element-6066-11e4-a52e-4f735466cecf"]
            .as_str()
            .is_some(),
        "late element should be returned after implicit wait: {late:#?}"
    );

    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "implicit": 100 }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));
    let missing_started = std::time::Instant::now();
    let (missing_status, missing) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": ".never"
        }),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(missing["value"]["error"], json!("no such element"));
    assert!(
        missing_started.elapsed() >= Duration::from_millis(80),
        "missing single element should wait close to implicit timeout"
    );

    let missing_elements = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "css selector",
            "value": ".never"
        }),
    )
    .await;
    assert_eq!(missing_elements, json!({ "value": [] }));
}
#[tokio::test]
async fn webdriver_classic_navigation_honors_page_load_timeout() {
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) =
        spawn_classic_delayed_navigation_fixture_server(Duration::from_millis(250)).await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "pageLoad": 10 }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));

    let url = format!("http://{fixture_addr}/slow");
    let (timeout_status, timeout_response) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(timeout_status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(timeout_response["value"]["error"], json!("timeout"));
    assert_eq!(
        timeout_response["value"]["message"],
        json!("page load timed out")
    );

    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "pageLoad": 300_000 }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));

    let recovery_url = "data:text/html,<main>page-load-recovered</main>";
    let recovered = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": recovery_url }),
    )
    .await;
    assert_eq!(recovered, json!({ "value": null }));

    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": recovery_url }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}
#[tokio::test]
async fn webdriver_classic_execute_sync_honors_script_timeout() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "script": 25 }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "data:text/html,<main>sync-timeout</main>" }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (timeout_status, timeout_response) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return new Promise(resolve => setTimeout(() => resolve('late'), 1000));",
            "args": []
        }),
    )
    .await;
    assert_eq!(timeout_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(timeout_response["value"]["error"], json!("script timeout"));

    let reset_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "script": 1000 }),
    )
    .await;
    assert_eq!(reset_timeouts, json!({ "value": null }));

    let recovered = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1]('async recovered');",
            "args": []
        }),
    )
    .await;
    assert_eq!(recovered, json!({ "value": "async recovered" }));

    let navigated_after_timeout = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<main>timeout-recovered-navigation</main>") }),
    )
    .await;
    assert_eq!(navigated_after_timeout, json!({ "value": null }));

    let recovered = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('main').textContent;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        recovered,
        json!({ "value": "timeout-recovered-navigation" })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
#[tokio::test]
async fn webdriver_classic_execute_sync_timeout_interrupts_non_yielding_script() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "script": 100 }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));

    let (timeout_status, timeout_response) = tokio::time::timeout(
        Duration::from_secs(10),
        classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "for (;;) {}",
                "args": []
            }),
        ),
    )
    .await
    .expect("non-yielding script timeout must interrupt V8 and return");
    assert_eq!(timeout_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(timeout_response["value"]["error"], json!("script timeout"));

    let reset_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "script": 1000 }),
    )
    .await;
    assert_eq!(reset_timeouts, json!({ "value": null }));

    let recovered = tokio::time::timeout(
        Duration::from_secs(5),
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return 42;",
                "args": []
            }),
        ),
    )
    .await
    .expect("renderer must accept another script after timeout termination");
    assert_eq!(recovered, json!({ "value": 42 }));

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}
#[tokio::test]
async fn webdriver_classic_execute_async_honors_script_timeout() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let set_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "script": 25 }),
    )
    .await;
    assert_eq!(set_timeouts, json!({ "value": null }));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "data:text/html,<main>timeout</main>" }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (timeout_status, timeout_response) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "setTimeout(() => arguments[arguments.length - 1]('late'), 1000);",
            "args": []
        }),
    )
    .await;
    assert_eq!(timeout_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(timeout_response["value"]["error"], json!("script timeout"));

    let reset_timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "script": 1000 }),
    )
    .await;
    assert_eq!(reset_timeouts, json!({ "value": null }));

    let recovered_async = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1]('async recovered');",
            "args": []
        }),
    )
    .await;
    assert_eq!(recovered_async, json!({ "value": "async recovered" }));

    let recovered = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('main').textContent;",
            "args": []
        }),
    )
    .await;
    assert_eq!(recovered, json!({ "value": "timeout" }));

    let recovery_url = classic_data_url("<main>timeout-recovered-navigation</main>");
    let navigated_after_timeout = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": recovery_url }),
    )
    .await;
    assert_eq!(navigated_after_timeout, json!({ "value": null }));

    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": recovery_url }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}
