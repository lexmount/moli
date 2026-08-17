use super::*;
use axum::http::{HeaderMap, header};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Map, Value};

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
async fn webdriver_classic_response_headers_are_scoped_to_classic_http_routes() {
    let app = build_router(test_state());

    let cdp_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/json/version")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    assert!(
        cdp_response.headers().get(header::CACHE_CONTROL).is_none(),
        "CDP routes must not receive Classic WebDriver cache-control"
    );

    let websocket_upgrade_response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/session")
                .header(header::UPGRADE, "websocket")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    assert!(
        websocket_upgrade_response
            .headers()
            .get(header::CACHE_CONTROL)
            .is_none(),
        "BiDi WebSocket upgrade routes must not receive Classic WebDriver cache-control"
    );
}

#[tokio::test]
async fn webdriver_classic_response_headers_do_not_relabel_axum_route_errors() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let (missing_status, missing_headers, missing_body) = classic_request_status_headers_and_text(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/unknown"),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_classic_json_content_type_absent(&missing_headers);
    assert!(
        !missing_body.trim_start().starts_with('{'),
        "axum 404 should stay a non-Classic response body: {missing_body:?}"
    );

    let (method_status, method_headers, method_body) = classic_request_status_headers_and_text(
        app.clone(),
        Method::PUT,
        &format!("/session/{session_id}/window"),
    )
    .await;
    assert_eq!(method_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_classic_json_content_type_absent(&method_headers);
    assert!(
        !method_body.trim_start().starts_with('{'),
        "axum 405 should stay a non-Classic response body: {method_body:?}"
    );

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}

#[tokio::test]
async fn webdriver_classic_upload_file_matches_selenium_remote_zip_endpoint() {
    // Matches Chromium chromedriver's UploadFile unit fixture and Selenium
    // Python's current /se/file route: a single ZIP entry named "moo" with
    // contents "COW\n", base64 encoded with line breaks.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let upload = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/se/file"),
        json!({
            "file": "UEsDBBQAAAAAAMROi0K/wAzGBAAAAAQAAAADAAAAbW9vQ09XClBLAQIUAxQAAAAAAMROi0K/\nwAzGBAAAAAQAAAADAAAAAAAAAAAAAACggQAAAABtb29QSwUGAAAAAAEAAQAxAAAAJQAAAAAA\n"
        }),
    )
    .await;
    let uploaded_path = upload["value"]
        .as_str()
        .unwrap_or_else(|| panic!("upload file response should contain a path: {upload:?}"))
        .to_owned();
    assert_eq!(
        fs::read_to_string(&uploaded_path).expect("uploaded Selenium file should exist"),
        "COW\n"
    );

    let deleted =
        classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
    assert_eq!(deleted, json!({ "value": null }));
    assert!(
        !std::path::Path::new(&uploaded_path).exists(),
        "uploaded Selenium file should be removed with the Classic session"
    );
}

#[tokio::test]
async fn webdriver_classic_download_files_match_selenium_remote_extension() {
    let (fixture_addr, fixture_server) =
        spawn_delayed_download_fixture_server("Hello, World!", Duration::from_millis(20)).await;
    let app = build_router(test_state());
    let session = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        "/session",
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "se:downloadsEnabled": true
                }
            }
        }),
    )
    .await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    assert_eq!(
        session["value"]["capabilities"]["se:downloadsEnabled"],
        json!(true)
    );

    let page_url = format!("http://{fixture_addr}/page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated["value"], Value::Null);
    let link_id = classic_find_css_element_id(app.clone(), session_id, "#dl").await;
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{link_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let files_path = format!("/session/{session_id}/se/files");
    let mut names = Vec::new();
    for _ in 0..50 {
        let downloadable = classic_request_json(app.clone(), Method::GET, &files_path).await;
        names = downloadable["value"]["names"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if names.iter().any(|name| name == "saved.txt") {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(names, vec![json!("saved.txt")]);

    let downloaded = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &files_path,
        json!({ "name": "saved.txt" }),
    )
    .await;
    let contents = downloaded["value"]["contents"]
        .as_str()
        .expect("download response should include base64 ZIP");
    let zip = base64::Engine::decode(&BASE64_STANDARD, contents)
        .expect("download response should be base64");
    assert!(
        zip.windows("Hello, World!".len())
            .any(|window| window == b"Hello, World!"),
        "download ZIP should contain artifact bytes"
    );

    assert_eq!(
        classic_request_json(app.clone(), Method::DELETE, &files_path).await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &files_path).await,
        json!({ "value": { "names": [] } })
    );

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
    fixture_server.abort();
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
async fn webdriver_classic_page_load_strategy_maps_navigation_wait_policy() {
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) =
        spawn_classic_page_load_strategy_fixture_server(Duration::from_millis(250)).await;
    let url = format!("http://{fixture_addr}/page");

    let eager_session = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        "/session",
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "pageLoadStrategy": "eager"
                }
            }
        }),
    )
    .await;
    assert_eq!(
        eager_session["value"]["capabilities"]["pageLoadStrategy"],
        json!("eager")
    );
    let eager_session_id = eager_session["value"]["sessionId"]
        .as_str()
        .expect("classic eager session id");
    let eager_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{eager_session_id}/url"),
        json!({ "url": url.clone() }),
    )
    .await;
    assert_eq!(eager_navigated, json!({ "value": null }));
    let eager_lifecycle = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{eager_session_id}/execute/sync"),
        json!({
            "script": "return document.readyState + ':' + window.__classicLifecycle.join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        eager_lifecycle,
        json!({ "value": "interactive:dcl:interactive" })
    );

    let normal_session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let normal_session_id = normal_session["value"]["sessionId"]
        .as_str()
        .expect("classic normal session id");
    let normal_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{normal_session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(normal_navigated, json!({ "value": null }));
    let normal_lifecycle = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{normal_session_id}/execute/sync"),
        json!({
            "script": "return document.readyState + ':' + window.__classicLifecycle.join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        normal_lifecycle,
        json!({ "value": "complete:dcl:interactive|external:interactive|load:complete" })
    );

    let (invalid_status, invalid) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        "/session",
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "pageLoadStrategy": "fast"
                }
            }
        }),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid["value"]["error"], json!("invalid argument"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{eager_session_id}"),
    )
    .await;
    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{normal_session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_page_load_strategy_none_completes_in_background() {
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) =
        spawn_classic_delayed_navigation_fixture_server(Duration::from_millis(300)).await;
    let url = format!("http://{fixture_addr}/slow");

    let session = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        "/session",
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "pageLoadStrategy": "none"
                }
            }
        }),
    )
    .await;
    assert_eq!(
        session["value"]["capabilities"]["pageLoadStrategy"],
        json!("none")
    );
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic none session id");

    let started = std::time::Instant::now();
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "pageLoadStrategy=none should return before the delayed body response"
    );

    let completed_source = timeout(Duration::from_secs(2), async {
        loop {
            let (status, source) = classic_request_status_and_json(
                app.clone(),
                Method::GET,
                &format!("/session/{session_id}/source"),
            )
            .await;
            if status == StatusCode::OK
                && source["value"]
                    .as_str()
                    .is_some_and(|html| html.contains("slow navigation"))
            {
                return source;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("background none navigation should complete");
    assert!(
        completed_source["value"]
            .as_str()
            .is_some_and(|html| html.contains("slow navigation")),
        "background navigation should eventually publish the loaded page source"
    );

    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_get_title_waits_for_script_triggered_form_navigation() {
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) =
        spawn_classic_form_navigation_fixture_server(Duration::from_millis(250)).await;
    let form_url = format!("http://{fixture_addr}/form");

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/timeouts"),
            json!({ "pageLoad": 2000 }),
        )
        .await,
        json!({ "value": null })
    );

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": form_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let submitted = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('form').submit(); return 'submitted';",
            "args": []
        }),
    )
    .await;
    assert_eq!(submitted, json!({ "value": "submitted" }));

    let (title_status, title) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/title"),
    )
    .await;
    assert_eq!(
        title_status,
        StatusCode::OK,
        "title should wait for form navigation: {title:?}"
    );
    assert_eq!(title, json!({ "value": "Submitted Target" }));

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
    fixture_server.abort();
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
async fn webdriver_classic_locator_strategy_cases_ported_from_chromium() {
    // Ported from Chromium chrome/test/chromedriver/test/run_py_tests.py:
    // testFindElement, testNoSuchElementExceptionMessage, testFindElements,
    // testFindWithInvalidSelector and testFindWithEmptySelector, plus
    // Selenium common driver_element_finding_tests.py XPath basics.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": "data:text/html,<body><h1 class='header'>Heading</h1><div class='one'>a<input name='inside'></div><div class='two'>b</div><script>window.__classicLocatorFixture=1</script></body>"
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let div = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "tag name",
            "value": "div"
        }),
    )
    .await;
    assert!(
        div["value"]["element-6066-11e4-a52e-4f735466cecf"]
            .as_str()
            .is_some(),
        "tag name find element should return an element: {div:?}"
    );

    let divs = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "tag name",
            "value": "div"
        }),
    )
    .await;
    assert_eq!(divs["value"].as_array().expect("elements array").len(), 2);

    let wildcard_tags = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "tag name",
            "value": "*"
        }),
    )
    .await;
    assert!(
        wildcard_tags["value"]
            .as_array()
            .expect("wildcard tag name elements array")
            .len()
            >= 5,
        "tag name wildcard should follow getElementsByTagName semantics: {wildcard_tags:?}"
    );

    let xpath = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "xpath",
            "value": "//h1[@class='header']"
        }),
    )
    .await;
    let xpath_id = xpath["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("xpath locator should return an element: {xpath:?}"));
    let xpath_text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{xpath_id}/text"),
    )
    .await;
    assert_eq!(xpath_text, json!({ "value": "Heading" }));

    let xpath_divs = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "xpath",
            "value": "//div"
        }),
    )
    .await;
    assert_eq!(
        xpath_divs["value"]
            .as_array()
            .expect("xpath elements array")
            .len(),
        2
    );

    let (missing_status, missing) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "tag name",
            "value": "divine"
        }),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(missing["value"]["error"], json!("no such element"));

    let missing_elements = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "tag name",
            "value": "divine"
        }),
    )
    .await;
    assert_eq!(missing_elements, json!({ "value": [] }));

    for selector_like_tag in ["div, h1", "div > input", "input, script"] {
        let matched = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/elements"),
            json!({
                "using": "tag name",
                "value": selector_like_tag
            }),
        )
        .await;
        assert_eq!(
            matched,
            json!({ "value": [] }),
            "tag name must use getElementsByTagName semantics, not CSS selector semantics for {selector_like_tag:?}"
        );
    }

    let missing_xpath = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "xpath",
            "value": "//span[@id='missing']"
        }),
    )
    .await;
    assert_eq!(missing_xpath, json!({ "value": [] }));

    for endpoint in ["element", "elements"] {
        for invalid_selector in ["", ">-?!.#&<@*"] {
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/{endpoint}"),
                json!({
                    "using": "css selector",
                    "value": invalid_selector
                }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{endpoint} {invalid_selector:?}"
            );
            assert_eq!(
                response["value"]["error"],
                json!("invalid selector"),
                "{endpoint} {invalid_selector:?}"
            );
        }
    }

    for endpoint in ["element", "elements"] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/{endpoint}"),
            json!({
                "using": "xpath",
                "value": "this][isnot][valid"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{endpoint}: {response:?}");
        assert_eq!(response["value"]["error"], json!("invalid selector"));
    }

    let (compound_status, compound) = classic_request_status_and_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "class name",
            "value": "one two"
        }),
    )
    .await;
    assert_eq!(compound_status, StatusCode::BAD_REQUEST);
    assert_eq!(compound["value"]["error"], json!("invalid selector"));
}

#[tokio::test]
async fn webdriver_classic_element_equality_cases_ported_from_selenium() {
    // Ported from Selenium's Python element_equality_tests.py:
    // same element found through different locator strategies should compare equal,
    // while different elements should not.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": "data:text/html,<body><div id='one'>one</div></body>"
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let body = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "tag name",
            "value": "body"
        }),
    )
    .await;
    let body_id = body["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .expect("body element id");

    let xpath_body = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "xpath",
            "value": "//body"
        }),
    )
    .await;
    let xpath_body_id = xpath_body["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .expect("xpath body element id");
    assert_eq!(
        xpath_body_id, body_id,
        "the session node reference store should reuse an element id across locator strategies"
    );

    let div = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "tag name",
            "value": "div"
        }),
    )
    .await;
    let div_id = div["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .expect("div element id");

    let same = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/equals/{xpath_body_id}"),
    )
    .await;
    assert_eq!(same, json!({ "value": true }));

    let same_trailing_slash = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/equals/{xpath_body_id}/"),
    )
    .await;
    assert_eq!(same_trailing_slash, json!({ "value": true }));

    let different = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/equals/{div_id}"),
    )
    .await;
    assert_eq!(different, json!({ "value": false }));

    let (invalid_status, invalid) = classic_request_status_and_json(
        app,
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/equals/not-a-moli-node"),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::NOT_FOUND);
    assert_eq!(invalid["value"]["error"], json!("no such element"));
}

#[tokio::test]
async fn webdriver_classic_element_reference_owner_rejects_forged_and_stales_after_navigation() {
    // Mirrors Selenium's stale element behavior: an unknown element reference is
    // not known to the session, while a previously returned element becomes
    // stale after the active document changes.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let second_url = classic_data_url(
        "<body><a id='navigate' href='#'>navigate</a><main id='target'>second</main></body>",
    );
    let first_url = classic_data_url(&format!(
        "<body><a id='navigate' href='{second_url}'>navigate</a><main id='target'>first</main></body>"
    ));
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": first_url }),
    )
    .await;

    let element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    assert!(
        element_id.starts_with("moli-node-") && element_id.contains("-element-"),
        "element id should be owner-shaped: {element_id}"
    );
    let node_id = element_id
        .strip_prefix("moli-node-")
        .and_then(|value| value.split_once("-element-").map(|(node_id, _)| node_id))
        .expect("owner-shaped element id contains node id");
    let forged_legacy_id = format!("moli-node-{node_id}");
    let (forged_status, forged) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{forged_legacy_id}/text"),
    )
    .await;
    assert_eq!(forged_status, StatusCode::NOT_FOUND);
    assert_eq!(forged["value"]["error"], json!("no such element"));

    let forged_owner_id = format!("moli-node-{node_id}-element-999999");
    let (forged_owner_status, forged_owner) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{forged_owner_id}/text"),
    )
    .await;
    assert_eq!(forged_owner_status, StatusCode::NOT_FOUND);
    assert_eq!(forged_owner["value"]["error"], json!("no such element"));

    let navigate_link_id = classic_find_css_element_id(app.clone(), session_id, "#navigate").await;
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{navigate_link_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));
    let (stale_status, stale) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/text"),
    )
    .await;
    assert_eq!(stale_status, StatusCode::NOT_FOUND);
    assert_eq!(stale["value"]["error"], json!("stale element reference"));

    let fresh_element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    let fresh_text = classic_request_json(
        app,
        Method::GET,
        &format!("/session/{session_id}/element/{fresh_element_id}/text"),
    )
    .await;
    assert_eq!(fresh_text["value"], json!("second"));
}

#[tokio::test]
async fn webdriver_classic_execute_script_resolves_webelement_arguments() {
    // Mirrors Selenium JavascriptExecutor usage where WebElement arguments are
    // exposed to user script as DOM elements, including nested argument shapes.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let first_url =
        classic_data_url("<body><main id='target' data-kind='primary'>Text</main></body>");
    let second_url = classic_data_url("<body><main id='target'>Replacement</main></body>");
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": first_url }),
    )
    .await;

    let element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    let element_ref = json!({
        CLASSIC_ELEMENT_REFERENCE_KEY: element_id.clone(),
    });
    let sync = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].getAttribute('data-kind') + ':' + arguments[0].textContent;",
            "args": [element_ref.clone()]
        }),
    )
    .await;
    assert_eq!(sync, json!({ "value": "primary:Text" }));

    let nested = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].node.id + ':' + arguments[1][0].textContent;",
            "args": [
                { "node": element_ref.clone() },
                [element_ref.clone()]
            ]
        }),
    )
    .await;
    assert_eq!(nested, json!({ "value": "target:Text" }));

    let async_result = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1](arguments[0].id);",
            "args": [element_ref.clone()]
        }),
    )
    .await;
    assert_eq!(async_result, json!({ "value": "target" }));

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": second_url }),
    )
    .await;
    let (stale_status, stale) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].id;",
            "args": [element_ref]
        }),
    )
    .await;
    assert_eq!(stale_status, StatusCode::NOT_FOUND);
    assert_eq!(stale["value"]["error"], json!("stale element reference"));

    let node_id = element_id
        .strip_prefix("moli-node-")
        .and_then(|value| value.split_once("-element-").map(|(node_id, _)| node_id))
        .expect("owner-shaped element id contains node id");
    let forged_legacy_ref = json!({
        CLASSIC_ELEMENT_REFERENCE_KEY: format!("moli-node-{node_id}"),
    });
    let (forged_status, forged) = classic_request_status_and_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].id;",
            "args": [forged_legacy_ref]
        }),
    )
    .await;
    assert_eq!(forged_status, StatusCode::NOT_FOUND);
    assert_eq!(forged["value"]["error"], json!("no such element"));
}

#[tokio::test]
async fn webdriver_classic_execute_script_basic_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/execute.py null body, primitive serialization, ending
    // comment, and override-listener cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let execute_path = format!("/session/{session_id}/execute/sync");

    let (null_body_status, null_body) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!(null),
    )
    .await;
    assert_eq!(null_body_status, StatusCode::BAD_REQUEST);
    assert_eq!(null_body["value"]["error"], json!("invalid argument"));

    for (label, script, expected) in [
        ("null", "return null;", json!(null)),
        ("undefined", "return undefined;", json!(null)),
        ("true", "return true;", json!(true)),
        ("false", "return false;", json!(false)),
        ("number", "return 23;", json!(23)),
        ("string", "return 'foo';", json!("foo")),
        ("nul", "return String.fromCharCode(0);", json!("\u{0000}")),
    ] {
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{label}");
    }

    let ending_comment = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return 1; // foo",
            "args": []
        }),
    )
    .await;
    assert_eq!(ending_comment, json!({ "value": 1 }));

    let listener_page = classic_data_url(
        "<script>window.called=[];window.addEventListener=()=>called.push('Internal addEventListener');window.removeEventListener=()=>called.push('Internal removeEventListener');</script>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": listener_page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let unload = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return !window.onunload;",
            "args": []
        }),
    )
    .await;
    assert_eq!(unload, json!({ "value": true }));
    let called = classic_request_json_with_body(
        app,
        Method::POST,
        &execute_path,
        json!({
            "script": "return window.called;",
            "args": []
        }),
    )
    .await;
    assert_eq!(called, json!({ "value": [] }));
}

#[tokio::test]
async fn webdriver_classic_execute_async_script_basic_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_async_script/execute_async.py null body and primitive
    // serialization cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let execute_path = format!("/session/{session_id}/execute/async");

    let (null_body_status, null_body) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!(null),
    )
    .await;
    assert_eq!(null_body_status, StatusCode::BAD_REQUEST);
    assert_eq!(null_body["value"]["error"], json!("invalid argument"));

    for (label, expression, expected) in [
        ("null", "null", json!(null)),
        ("undefined", "undefined", json!(null)),
        ("true", "true", json!(true)),
        ("false", "false", json!(false)),
        ("number", "23", json!(23)),
        ("string", "'foo'", json!("foo")),
        ("nul", "String.fromCharCode(0)", json!("\u{0000}")),
    ] {
        let script = format!("arguments[arguments.length - 1]({expression});");
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{label}");
    }
}

#[tokio::test]
async fn webdriver_classic_execute_script_argument_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/arguments.py null, primitives, collection, and object
    // cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let null_response = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [arguments[0] === null, arguments[0]];",
            "args": [null]
        }),
    )
    .await;
    assert_eq!(null_response, json!({ "value": [true, null] }));

    for (label, value, expected_type) in [
        ("boolean", json!(true), "boolean"),
        ("number", json!(42), "number"),
        ("string", json!("foo"), "string"),
        ("string quote", json!("foo\"bar"), "string"),
        ("string injection", json!("\"); alert(1); //"), "string"),
        (
            "special key object",
            json!({ "foo-bar": "bar-foo" }),
            "object",
        ),
    ] {
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return [typeof arguments[0], arguments[0]];",
                "args": [value.clone()]
            }),
        )
        .await;
        assert_eq!(
            response,
            json!({ "value": [expected_type, value] }),
            "{label}"
        );
    }

    let collection = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [Array.isArray(arguments[0]), arguments[0]];",
            "args": [[1, 2, 3]]
        }),
    )
    .await;
    assert_eq!(collection, json!({ "value": [true, [1, 2, 3]] }));

    let object = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [typeof arguments[0], arguments[0]];",
            "args": [{ "foo": "bar", "cheese": 23 }]
        }),
    )
    .await;
    assert_eq!(
        object,
        json!({ "value": ["object", { "foo": "bar", "cheese": 23 }] })
    );

    for key in [
        CLASSIC_ELEMENT_REFERENCE_KEY,
        CLASSIC_SHADOW_ROOT_REFERENCE_KEY,
        CLASSIC_FRAME_REFERENCE_KEY,
        CLASSIC_WINDOW_REFERENCE_KEY,
    ] {
        for value in [json!(null), json!(false), json!(42), json!([]), json!({})] {
            let mut reference = Map::new();
            reference.insert(key.to_owned(), value);
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/execute/sync"),
                json!({
                    "script": "return true;",
                    "args": [Value::Object(reference)]
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{key}: {response:?}");
            assert_eq!(
                response["value"]["error"],
                json!("invalid argument"),
                "{key}: {response:?}"
            );
        }
    }
}

#[tokio::test]
async fn webdriver_classic_execute_async_script_argument_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_async_script/arguments.py null, primitives, collection, and
    // object cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let null_response = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1]([arguments[0] === null, arguments[0]]);",
            "args": [null]
        }),
    )
    .await;
    assert_eq!(null_response, json!({ "value": [true, null] }));

    for (label, value, expected_type) in [
        ("boolean", json!(true), "boolean"),
        ("number", json!(42), "number"),
        ("string", json!("foo"), "string"),
        ("string quote", json!("foo\"bar"), "string"),
        ("string injection", json!("\"); alert(1); //"), "string"),
        (
            "special key object",
            json!({ "foo-bar": "bar-foo" }),
            "object",
        ),
    ] {
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": "arguments[arguments.length - 1]([typeof arguments[0], arguments[0]]);",
                "args": [value.clone()]
            }),
        )
        .await;
        assert_eq!(
            response,
            json!({ "value": [expected_type, value] }),
            "{label}"
        );
    }

    let collection = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1]([Array.isArray(arguments[0]), arguments[0]]);",
            "args": [[1, 2, 3]]
        }),
    )
    .await;
    assert_eq!(collection, json!({ "value": [true, [1, 2, 3]] }));

    let object = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1]([typeof arguments[0], arguments[0]]);",
            "args": [{ "foo": "bar", "cheese": 23 }]
        }),
    )
    .await;
    assert_eq!(
        object,
        json!({ "value": ["object", { "foo": "bar", "cheese": 23 }] })
    );

    for key in [
        CLASSIC_ELEMENT_REFERENCE_KEY,
        CLASSIC_SHADOW_ROOT_REFERENCE_KEY,
        CLASSIC_FRAME_REFERENCE_KEY,
        CLASSIC_WINDOW_REFERENCE_KEY,
    ] {
        for value in [json!(null), json!(false), json!(42), json!([]), json!({})] {
            let mut reference = Map::new();
            reference.insert(key.to_owned(), value);
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/execute/async"),
                json!({
                    "script": "arguments[arguments.length - 1](true);",
                    "args": [Value::Object(reference)]
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{key}: {response:?}");
            assert_eq!(
                response["value"]["error"],
                json!("invalid argument"),
                "{key}: {response:?}"
            );
        }
    }
}

#[tokio::test]
async fn webdriver_classic_execute_script_dom_token_list_case_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/collections.py test_dom_token_list.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page_url = classic_data_url(r#"<div class="no cheese">foo</div>"#);
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;

    let element_id = classic_find_css_element_id(app.clone(), session_id, "div").await;
    let response = classic_request_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].classList;",
            "args": [{
                CLASSIC_ELEMENT_REFERENCE_KEY: element_id,
            }]
        }),
    )
    .await;
    assert_eq!(response, json!({ "value": ["no", "cheese"] }));
}

#[tokio::test]
async fn webdriver_classic_execute_script_collection_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/collections.py arguments, array, array_in_array,
    // FileList, HTMLAllCollection, HTMLCollection, HTMLFormControlsCollection,
    // HTMLOptionsCollection, and NodeList cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let arguments = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "function func() { return arguments; } return func('foo', 'bar');",
            "args": []
        }),
    )
    .await;
    assert_eq!(arguments, json!({ "value": ["foo", "bar"] }));

    let array = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [1, 2];",
            "args": []
        }),
    )
    .await;
    assert_eq!(array, json!({ "value": [1, 2] }));

    let array_in_array = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const arr = [1]; return [arr, arr];",
            "args": []
        }),
    )
    .await;
    assert_eq!(array_in_array, json!({ "value": [[1], [1]] }));

    let first_file = TempPath::new("classic-file-list-foo");
    let second_file = TempPath::new("classic-file-list-bar");
    fs::write(&first_file.path, b"morn morn").expect("write first FileList upload file");
    fs::write(&second_file.path, b"morn morn").expect("write second FileList upload file");
    let expected_file_names = [
        classic_temp_file_basename(&first_file),
        classic_temp_file_basename(&second_file),
    ];
    let file_page = classic_data_url("<input id='upload' type='file' multiple>");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": file_page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let upload_id = classic_find_css_element_id(app.clone(), session_id, "#upload").await;
    let uploaded = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{upload_id}/value"),
        json!({
            "text": format!(
                "{}\n{}",
                first_file.path.to_string_lossy(),
                second_file.path.to_string_lossy()
            )
        }),
    )
    .await;
    assert_eq!(uploaded, json!({ "value": null }));
    let file_list = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('input').files;",
            "args": []
        }),
    )
    .await;
    classic_assert_serialized_file_list_names("FileList", &file_list, &expected_file_names);

    let collections_page = classic_data_url(
        "<!doctype html><html><head><title>collections</title></head><body>\
         <p id='p-1'>foo</p><p id='p-2'>bar</p>\
         <form id='form'><input id='input-1'><input id='input-2'></form>\
         <select id='select'><option id='option-1'>one</option><option id='option-2'>two</option></select>\
         </body></html>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": collections_page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let p_ids = [
        classic_find_css_element_id(app.clone(), session_id, "#p-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#p-2").await,
    ];
    let html_collection = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.getElementsByTagName('p');",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app.clone(),
        session_id,
        "HTMLCollection",
        &html_collection,
        &p_ids,
    )
    .await;

    let node_list = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelectorAll('p');",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(app.clone(), session_id, "NodeList", &node_list, &p_ids)
        .await;

    let input_ids = [
        classic_find_css_element_id(app.clone(), session_id, "#input-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#input-2").await,
    ];
    let form_controls = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.forms[0].elements;",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app.clone(),
        session_id,
        "HTMLFormControlsCollection",
        &form_controls,
        &input_ids,
    )
    .await;

    let option_ids = [
        classic_find_css_element_id(app.clone(), session_id, "#option-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#option-2").await,
    ];
    let options = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('select').options;",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app.clone(),
        session_id,
        "HTMLOptionsCollection",
        &options,
        &option_ids,
    )
    .await;

    let all_page = classic_data_url(
        "<!doctype html><html><head><meta id='meta'></head><body>\
         <p id='all-p-1'>foo</p><p id='all-p-2'>bar</p>\
         </body></html>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": all_page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let document_all_ids = [
        classic_find_css_element_id(app.clone(), session_id, "html").await,
        classic_find_css_element_id(app.clone(), session_id, "head").await,
        classic_find_css_element_id(app.clone(), session_id, "#meta").await,
        classic_find_css_element_id(app.clone(), session_id, "body").await,
        classic_find_css_element_id(app.clone(), session_id, "#all-p-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#all-p-2").await,
    ];
    let document_all = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.all;",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app,
        session_id,
        "HTMLAllCollection",
        &document_all,
        &document_all_ids,
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_execute_async_script_collection_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_async_script/collections.py arguments, array, array_in_array,
    // FileList, HTMLAllCollection, HTMLCollection, HTMLFormControlsCollection,
    // HTMLOptionsCollection, and NodeList cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let arguments = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "const resolve = arguments[0]; function func() { return arguments; } resolve(func('foo', 'bar'));",
            "args": []
        }),
    )
    .await;
    assert_eq!(arguments, json!({ "value": ["foo", "bar"] }));

    let array = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0]([1, 2]);",
            "args": []
        }),
    )
    .await;
    assert_eq!(array, json!({ "value": [1, 2] }));

    let array_in_array = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "const arr = [1]; arguments[0]([arr, arr]);",
            "args": []
        }),
    )
    .await;
    assert_eq!(array_in_array, json!({ "value": [[1], [1]] }));

    let first_file = TempPath::new("classic-async-file-list-foo");
    let second_file = TempPath::new("classic-async-file-list-bar");
    fs::write(&first_file.path, b"morn morn").expect("write first async FileList upload file");
    fs::write(&second_file.path, b"morn morn").expect("write second async FileList upload file");
    let expected_file_names = [
        classic_temp_file_basename(&first_file),
        classic_temp_file_basename(&second_file),
    ];
    let file_page = classic_data_url("<input id='upload' type='file' multiple>");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": file_page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let upload_id = classic_find_css_element_id(app.clone(), session_id, "#upload").await;
    let uploaded = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{upload_id}/value"),
        json!({
            "text": format!(
                "{}\n{}",
                first_file.path.to_string_lossy(),
                second_file.path.to_string_lossy()
            )
        }),
    )
    .await;
    assert_eq!(uploaded, json!({ "value": null }));
    let file_list = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('input').files);",
            "args": []
        }),
    )
    .await;
    classic_assert_serialized_file_list_names("async FileList", &file_list, &expected_file_names);

    let collections_page = classic_data_url(
        "<!doctype html><html><head><title>collections</title></head><body>\
         <p id='p-1'>foo</p><p id='p-2'>bar</p>\
         <form id='form'><input id='input-1'><input id='input-2'></form>\
         <select id='select'><option id='option-1'>one</option><option id='option-2'>two</option></select>\
         </body></html>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": collections_page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let p_ids = [
        classic_find_css_element_id(app.clone(), session_id, "#p-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#p-2").await,
    ];
    let html_collection = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.getElementsByTagName('p'));",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app.clone(),
        session_id,
        "async HTMLCollection",
        &html_collection,
        &p_ids,
    )
    .await;

    let node_list = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelectorAll('p'));",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app.clone(),
        session_id,
        "async NodeList",
        &node_list,
        &p_ids,
    )
    .await;

    let input_ids = [
        classic_find_css_element_id(app.clone(), session_id, "#input-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#input-2").await,
    ];
    let form_controls = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.forms[0].elements);",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app.clone(),
        session_id,
        "async HTMLFormControlsCollection",
        &form_controls,
        &input_ids,
    )
    .await;

    let option_ids = [
        classic_find_css_element_id(app.clone(), session_id, "#option-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#option-2").await,
    ];
    let options = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('select').options);",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app.clone(),
        session_id,
        "async HTMLOptionsCollection",
        &options,
        &option_ids,
    )
    .await;

    let all_page = classic_data_url(
        "<!doctype html><html><head><meta id='meta'></head><body>\
         <p id='all-p-1'>foo</p><p id='all-p-2'>bar</p>\
         </body></html>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": all_page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let document_all_ids = [
        classic_find_css_element_id(app.clone(), session_id, "html").await,
        classic_find_css_element_id(app.clone(), session_id, "head").await,
        classic_find_css_element_id(app.clone(), session_id, "#meta").await,
        classic_find_css_element_id(app.clone(), session_id, "body").await,
        classic_find_css_element_id(app.clone(), session_id, "#all-p-1").await,
        classic_find_css_element_id(app.clone(), session_id, "#all-p-2").await,
    ];
    let document_all = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.all);",
            "args": []
        }),
    )
    .await;
    classic_assert_web_element_array_eq(
        app,
        session_id,
        "async HTMLAllCollection",
        &document_all,
        &document_all_ids,
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_execute_script_promise_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/promise.py resolve/reject cases. Timeout cases are covered
    // by webdriver_classic_execute_sync_honors_script_timeout.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    for (label, script, expected) in [
        (
            "promise resolve",
            "return Promise.resolve('foobar');",
            json!("foobar"),
        ),
        (
            "promise resolve delayed",
            "return new Promise(resolve => setTimeout(() => resolve('foobar'), 10));",
            json!("foobar"),
        ),
        (
            "promise all resolve",
            "return Promise.all([Promise.resolve(1), Promise.resolve(2)]);",
            json!([1, 2]),
        ),
        (
            "await promise resolve",
            "let res = await Promise.resolve('foobar'); return res;",
            json!("foobar"),
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{label}: {response:?}");
        assert_eq!(response, json!({ "value": expected }), "{label}");
    }

    for (label, script) in [
        (
            "promise reject",
            "return Promise.reject(new Error('my error'));",
        ),
        (
            "promise reject delayed",
            "return new Promise((resolve, reject) => setTimeout(() => reject(new Error('my error')), 10));",
        ),
        (
            "promise all reject",
            "return Promise.all([Promise.resolve(1), Promise.reject(new Error('error'))]);",
        ),
        (
            "await promise reject",
            "await Promise.reject(new Error('my error')); return 'foo';",
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{label}");
        assert_eq!(
            response["value"]["error"],
            json!("javascript error"),
            "{label}: {response:?}"
        );
    }
}

#[tokio::test]
async fn webdriver_classic_execute_async_script_promise_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_async_script/promise.py resolve/reject cases. Timeout cases are
    // covered by webdriver_classic_execute_async_honors_script_timeout.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    for (label, script, expected) in [
        (
            "promise resolve",
            "let resolve = arguments[0]; resolve(Promise.resolve('foobar'));",
            json!("foobar"),
        ),
        (
            "promise resolve delayed",
            "let resolve = arguments[0]; let promise = new Promise(resolve => setTimeout(() => resolve('foobar'), 10)); resolve(promise);",
            json!("foobar"),
        ),
        (
            "promise all resolve",
            "let resolve = arguments[0]; let promise = Promise.all([Promise.resolve(1), Promise.resolve(2)]); resolve(promise);",
            json!([1, 2]),
        ),
        (
            "await promise resolve",
            "let resolve = arguments[0]; let res = await Promise.resolve('foobar'); resolve(res);",
            json!("foobar"),
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{label}: {response:?}");
        assert_eq!(response, json!({ "value": expected }), "{label}");
    }

    for (label, script) in [
        (
            "promise reject",
            "let resolve = arguments[0]; resolve(Promise.reject(new Error('my error')));",
        ),
        (
            "promise reject delayed",
            "let resolve = arguments[0]; let promise = new Promise((resolve, reject) => setTimeout(() => reject(new Error('my error')), 10)); resolve(promise);",
        ),
        (
            "promise all reject",
            "let resolve = arguments[0]; let promise = Promise.all([Promise.resolve(1), Promise.reject(new Error('error'))]); resolve(promise);",
        ),
        (
            "await promise reject",
            "let resolve = arguments[0]; await Promise.reject(new Error('my error')); resolve('foo');",
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{label}");
        assert_eq!(
            response["value"]["error"],
            json!("javascript error"),
            "{label}: {response:?}"
        );
    }
}

#[tokio::test]
async fn webdriver_classic_execute_script_property_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/properties.py.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<input value=foobar>") }),
    )
    .await;
    let content_attribute = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('input').value;",
            "args": []
        }),
    )
    .await;
    assert_eq!(content_attribute, json!({ "value": "foobar" }));

    let idl_page = classic_data_url(
        "<input><script>document.querySelector('input').value = 'foobar';</script>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": idl_page }),
    )
    .await;
    let idl_attribute = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('input').value;",
            "args": []
        }),
    )
    .await;
    assert_eq!(idl_attribute, json!({ "value": "foobar" }));

    let element_property_page = classic_data_url(
        "<p id='foo'>foo</p><p id='bar'>bar</p>\
         <script>document.querySelector('#foo').bar = document.querySelector('#bar');</script>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": element_property_page }),
    )
    .await;
    let bar_id = classic_find_css_element_id(app.clone(), session_id, "#bar").await;
    let element_property = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('#foo').bar;",
            "args": []
        }),
    )
    .await;
    let returned_bar_id = element_property["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("expected WebElement result: {element_property:?}"));
    assert_eq!(
        returned_bar_id, bar_id,
        "script-returned element property should reuse the find-element WebElement id"
    );
    let same = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{returned_bar_id}/equals/{bar_id}"),
    )
    .await;
    assert_eq!(same, json!({ "value": true }));

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<input>") }),
    )
    .await;
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('input').foobar = 'foobar';",
            "args": []
        }),
    )
    .await;
    let script_property = classic_request_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('input').foobar;",
            "args": []
        }),
    )
    .await;
    assert_eq!(script_property, json!({ "value": "foobar" }));
}

#[tokio::test]
async fn webdriver_classic_execute_async_script_property_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_async_script/properties.py.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<input value=foobar>") }),
    )
    .await;
    let content_attribute = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('input').value);",
            "args": []
        }),
    )
    .await;
    assert_eq!(content_attribute, json!({ "value": "foobar" }));

    let idl_page = classic_data_url(
        "<input><script>document.querySelector('input').value = 'foobar';</script>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": idl_page }),
    )
    .await;
    let idl_attribute = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('input').value);",
            "args": []
        }),
    )
    .await;
    assert_eq!(idl_attribute, json!({ "value": "foobar" }));

    let element_property_page = classic_data_url(
        "<p id='foo'>foo</p><p id='bar'>bar</p>\
         <script>document.querySelector('#foo').bar = document.querySelector('#bar');</script>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": element_property_page }),
    )
    .await;
    let bar_id = classic_find_css_element_id(app.clone(), session_id, "#bar").await;
    let element_property = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('#foo').bar);",
            "args": []
        }),
    )
    .await;
    let returned_bar_id = element_property["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("expected WebElement result: {element_property:?}"));
    let same = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{returned_bar_id}/equals/{bar_id}"),
    )
    .await;
    assert_eq!(same, json!({ "value": true }));

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<input>") }),
    )
    .await;
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('input').foobar = 'foobar';",
            "args": []
        }),
    )
    .await;
    let script_property = classic_request_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('input').foobar);",
            "args": []
        }),
    )
    .await;
    assert_eq!(script_property, json!({ "value": "foobar" }));
}

#[tokio::test]
async fn webdriver_classic_execute_script_node_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/node.py top-context node type, web reference, stale
    // element, and detached shadow root cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let node_page = classic_data_url(
        "<!doctype html><div id='attr' data-kind='v'></div>\
         <div id='text-node'><p></p>Lorem</div>\
         <div id='comment'><!-- Comment --></div>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": node_page }),
    )
    .await;

    for (label, expression, expected_type, expected_result) in [
        (
            "attribute",
            "document.querySelector('#attr').attributes[0]",
            2,
            Some(json!({})),
        ),
        (
            "text",
            "document.querySelector('#text-node').childNodes[1]",
            3,
            Some(json!({})),
        ),
        (
            "comment",
            "document.querySelector('#comment').childNodes[0]",
            8,
            Some(json!({})),
        ),
        ("document", "document", 9, None),
        ("doctype", "document.doctype", 10, Some(json!({}))),
    ] {
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": format!("const result = {expression}; return {{ result, type: result.nodeType }};"),
                "args": []
            }),
        )
        .await;
        assert_eq!(
            response["value"]["type"],
            json!(expected_type),
            "{label}: {response:?}"
        );
        if let Some(expected_result) = expected_result {
            assert_eq!(
                response["value"]["result"], expected_result,
                "{label}: {response:?}"
            );
        } else {
            assert!(
                response["value"]["result"].get("location").is_some(),
                "{label}: expected serialized document to expose location: {response:?}"
            );
        }
    }

    let reference_page = classic_data_url(
        "<div id='target'></div><div id='host'></div>\
         <script>document.querySelector('#host').attachShadow({ mode: 'open' }).innerHTML = '<span>inside</span>';</script>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": reference_page }),
    )
    .await;
    let target_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    let element_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('#target');",
            "args": []
        }),
    )
    .await;
    let returned_target_id = element_reference["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("expected WebElement reference: {element_reference:?}"));
    assert_eq!(
        returned_target_id, target_id,
        "script-returned element should reuse the find-element WebElement id"
    );
    let same = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{returned_target_id}/equals/{target_id}"),
    )
    .await;
    assert_eq!(same, json!({ "value": true }));

    let shadow_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('#host').shadowRoot;",
            "args": []
        }),
    )
    .await;
    let shadow_id = shadow_reference["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("expected ShadowRoot reference: {shadow_reference:?}"))
        .to_owned();

    let (detached_shadow_status, detached_shadow) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const [host, shadowRoot] = arguments; host.remove(); return shadowRoot;",
            "args": [
                { CLASSIC_ELEMENT_REFERENCE_KEY: classic_find_css_element_id(app.clone(), session_id, "#host").await },
                { CLASSIC_SHADOW_ROOT_REFERENCE_KEY: shadow_id }
            ]
        }),
    )
    .await;
    assert_eq!(detached_shadow_status, StatusCode::NOT_FOUND);
    assert_eq!(
        detached_shadow["value"]["error"],
        json!("detached shadow root")
    );

    let stale_page = classic_data_url("<div id='stale'></div>");
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": stale_page }),
    )
    .await;
    let stale_id = classic_find_css_element_id(app.clone(), session_id, "#stale").await;
    let (stale_status, stale) = classic_request_status_and_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const elem = arguments[0]; elem.remove(); return elem;",
            "args": [{ CLASSIC_ELEMENT_REFERENCE_KEY: stale_id }]
        }),
    )
    .await;
    assert_eq!(stale_status, StatusCode::NOT_FOUND);
    assert_eq!(stale["value"]["error"], json!("stale element reference"));
}

#[tokio::test]
async fn webdriver_classic_execute_async_script_node_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_async_script/node.py top-context node type, web reference, stale
    // element, and detached shadow root cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let node_page = classic_data_url(
        "<!doctype html><div id='attr' data-kind='v'></div>\
         <div id='text-node'><p></p>Lorem</div>\
         <div id='comment'><!-- Comment --></div>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": node_page }),
    )
    .await;

    for (label, expression, expected_type, expected_result) in [
        (
            "attribute",
            "document.querySelector('#attr').attributes[0]",
            2,
            Some(json!({})),
        ),
        (
            "text",
            "document.querySelector('#text-node').childNodes[1]",
            3,
            Some(json!({})),
        ),
        (
            "comment",
            "document.querySelector('#comment').childNodes[0]",
            8,
            Some(json!({})),
        ),
        ("document", "document", 9, None),
        ("doctype", "document.doctype", 10, Some(json!({}))),
    ] {
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": format!("const resolve = arguments[0]; const result = {expression}; resolve({{ result, type: result.nodeType }});"),
                "args": []
            }),
        )
        .await;
        assert_eq!(
            response["value"]["type"],
            json!(expected_type),
            "{label}: {response:?}"
        );
        if let Some(expected_result) = expected_result {
            assert_eq!(
                response["value"]["result"], expected_result,
                "{label}: {response:?}"
            );
        } else {
            assert!(
                response["value"]["result"].get("location").is_some(),
                "{label}: expected serialized document to expose location: {response:?}"
            );
        }
    }

    let reference_page = classic_data_url(
        "<div id='target'></div><div id='host'></div>\
         <script>document.querySelector('#host').attachShadow({ mode: 'open' }).innerHTML = '<span>inside</span>';</script>",
    );
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": reference_page }),
    )
    .await;
    let target_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    let element_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('#target'));",
            "args": []
        }),
    )
    .await;
    let returned_target_id = element_reference["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("expected WebElement reference: {element_reference:?}"));
    let same = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{returned_target_id}/equals/{target_id}"),
    )
    .await;
    assert_eq!(same, json!({ "value": true }));

    let shadow_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[0](document.querySelector('#host').shadowRoot);",
            "args": []
        }),
    )
    .await;
    let shadow_id = shadow_reference["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("expected ShadowRoot reference: {shadow_reference:?}"))
        .to_owned();

    let (detached_shadow_status, detached_shadow) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "const [host, shadowRoot, resolve] = arguments; host.remove(); resolve(shadowRoot);",
            "args": [
                { CLASSIC_ELEMENT_REFERENCE_KEY: classic_find_css_element_id(app.clone(), session_id, "#host").await },
                { CLASSIC_SHADOW_ROOT_REFERENCE_KEY: shadow_id }
            ]
        }),
    )
    .await;
    assert_eq!(detached_shadow_status, StatusCode::NOT_FOUND);
    assert_eq!(
        detached_shadow["value"]["error"],
        json!("detached shadow root")
    );

    let stale_page = classic_data_url("<div id='stale'></div>");
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": stale_page }),
    )
    .await;
    let stale_id = classic_find_css_element_id(app.clone(), session_id, "#stale").await;
    let (stale_status, stale) = classic_request_status_and_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "const [elem, resolve] = arguments; elem.remove(); resolve(elem);",
            "args": [{ CLASSIC_ELEMENT_REFERENCE_KEY: stale_id }]
        }),
    )
    .await;
    assert_eq!(stale_status, StatusCode::NOT_FOUND);
    assert_eq!(stale["value"]["error"], json!("stale element reference"));
}

#[tokio::test]
async fn webdriver_classic_execute_script_object_and_cyclic_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/{objects.py,cyclic.py}.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    for (label, script, expected) in [
        (
            "object",
            "return { foo: 23, bar: true };",
            json!({ "foo": 23, "bar": true }),
        ),
        (
            "nested object",
            "return { foo: { cheese: 23 }, bar: true };",
            json!({ "foo": { "cheese": 23 }, "bar": true }),
        ),
        (
            "inherited enumerable object property",
            "const proto = { inherited: 2 }; const value = Object.create(proto); value.own = 1; return value;",
            json!({ "own": 1, "inherited": 2 }),
        ),
        (
            "object toJSON",
            "return { toJSON() { return ['foo', 'bar']; } };",
            json!(["foo", "bar"]),
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{label}: {response:?}");
        assert_eq!(response, json!({ "value": expected }), "{label}");
    }

    for (label, script) in [
        (
            "object toJSON exception",
            "return { toJSON() { throw Error('fail'); } };",
        ),
        (
            "collection self reference",
            "let arr = []; arr.push(arr); return arr;",
        ),
        (
            "object self reference",
            "let obj = {}; obj.reference = obj; return obj;",
        ),
        (
            "collection self reference in object",
            "let arr = []; arr.push(arr); return { value: arr };",
        ),
        (
            "object self reference in collection",
            "let obj = {}; obj.reference = obj; return [obj];",
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{label}: {response:?}"
        );
        assert_eq!(
            response["value"]["error"],
            json!("javascript error"),
            "{label}: {response:?}"
        );
    }

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "data:text/html,<div></div>" }),
    )
    .await;
    let div_id = classic_find_css_element_id(app.clone(), session_id, "div").await;

    for (label, script) in [
        (
            "element self reference",
            "let div = document.querySelector('div'); div.reference = div; return div;",
        ),
        (
            "element self reference in collection",
            "let div = document.querySelector('div'); div.reference = div; return [div];",
        ),
        (
            "element self reference in object",
            "let div = document.querySelector('div'); div.reference = div; return { foo: div };",
        ),
    ] {
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        let value = match label {
            "element self reference in collection" => response["value"][0].clone(),
            "element self reference in object" => response["value"]["foo"].clone(),
            _ => response["value"].clone(),
        };
        let returned_id = value[CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: expected WebElement response: {response:?}"));
        let same = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{returned_id}/equals/{div_id}"),
        )
        .await;
        assert_eq!(same, json!({ "value": true }), "{label}");
    }
}

#[tokio::test]
async fn webdriver_classic_execute_async_script_object_and_cyclic_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_async_script/{objects.py,cyclic.py}.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    for (label, script, expected) in [
        (
            "object",
            "arguments[0]({ foo: 23, bar: true });",
            json!({ "foo": 23, "bar": true }),
        ),
        (
            "nested object",
            "arguments[0]({ foo: { cheese: 23 }, bar: true });",
            json!({ "foo": { "cheese": 23 }, "bar": true }),
        ),
        (
            "inherited enumerable object property",
            "const proto = { inherited: 2 }; const value = Object.create(proto); value.own = 1; arguments[0](value);",
            json!({ "own": 1, "inherited": 2 }),
        ),
        (
            "object toJSON",
            "arguments[0]({ toJSON() { return ['foo', 'bar']; } });",
            json!(["foo", "bar"]),
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{label}: {response:?}");
        assert_eq!(response, json!({ "value": expected }), "{label}");
    }

    for (label, script) in [
        (
            "object toJSON exception",
            "arguments[0]({ toJSON() { throw Error('fail'); } });",
        ),
        (
            "collection self reference",
            "let arr = []; arr.push(arr); arguments[0](arr);",
        ),
        (
            "object self reference",
            "let obj = {}; obj.reference = obj; arguments[0](obj);",
        ),
        (
            "collection self reference in object",
            "let arr = []; arr.push(arr); arguments[0]({ value: arr });",
        ),
        (
            "object self reference in collection",
            "let obj = {}; obj.reference = obj; arguments[0]([obj]);",
        ),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{label}: {response:?}"
        );
        assert_eq!(
            response["value"]["error"],
            json!("javascript error"),
            "{label}: {response:?}"
        );
    }

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "data:text/html,<div></div>" }),
    )
    .await;
    let div_id = classic_find_css_element_id(app.clone(), session_id, "div").await;

    for (label, script) in [
        (
            "element self reference",
            "let div = document.querySelector('div'); div.reference = div; arguments[0](div);",
        ),
        (
            "element self reference in collection",
            "let div = document.querySelector('div'); div.reference = div; arguments[0]([div]);",
        ),
        (
            "element self reference in object",
            "let div = document.querySelector('div'); div.reference = div; arguments[0]({ foo: div });",
        ),
    ] {
        let response = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        let value = match label {
            "element self reference in collection" => response["value"][0].clone(),
            "element self reference in object" => response["value"]["foo"].clone(),
            _ => response["value"].clone(),
        };
        let returned_id = value[CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: expected WebElement response: {response:?}"));
        let same = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{returned_id}/equals/{div_id}"),
        )
        .await;
        assert_eq!(same, json!({ "value": true }), "{label}");
    }
}

#[tokio::test]
async fn webdriver_classic_frame_local_element_reference_errors_as_no_such_element_outside_frame() {
    // Ported from Chromium/WPT webdriver/tests/classic/execute_script/arguments.py
    // and find_element_from_element/find.py: a WebElement from a different
    // current frame is not addressable from the active browsing context.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page_url = format!("http://{fixture_addr}/page");
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;

    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id.clone(),
            }
        }),
    )
    .await;

    let child_element_id =
        classic_find_css_element_id(app.clone(), session_id, "#inside-frame").await;
    let child_element_ref = json!({
        CLASSIC_ELEMENT_REFERENCE_KEY: child_element_id.clone(),
    });

    let _ = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame/parent"),
    )
    .await;

    let (text_status, text) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{child_element_id}/text"),
    )
    .await;
    assert_eq!(text_status, StatusCode::NOT_FOUND);
    assert_eq!(text["value"]["error"], json!("no such element"));

    let (find_status, find) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{child_element_id}/element"),
        json!({
            "using": "css selector",
            "value": "main"
        }),
    )
    .await;
    assert_eq!(find_status, StatusCode::NOT_FOUND);
    assert_eq!(find["value"]["error"], json!("no such element"));

    let (sync_status, sync) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return true;",
            "args": [child_element_ref.clone()]
        }),
    )
    .await;
    assert_eq!(sync_status, StatusCode::NOT_FOUND);
    assert_eq!(sync["value"]["error"], json!("no such element"));

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "arguments[0].remove();",
            "args": [{
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id
            }]
        }),
    )
    .await;

    let (async_status, async_result) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1](true);",
            "args": [child_element_ref]
        }),
    )
    .await;
    assert_eq!(async_status, StatusCode::NOT_FOUND);
    assert_eq!(async_result["value"]["error"], json!("no such element"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_execute_script_round_trips_shadow_root_references() {
    // Ported from the shadow-root cases in Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/execute_script/
    // arguments.py and node.py.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r##"<!doctype html>
        <custom-element id="host"></custom-element>
        <script>
          const host = document.querySelector("#host");
          const root = host.attachShadow({ mode: "open" });
          root.innerHTML = `<span id="inside">shadow text</span>`;
        </script>"##;
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;

    let returned_shadow = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('#host').shadowRoot;",
            "args": []
        }),
    )
    .await;
    let returned_shadow_id = returned_shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("execute returned shadow root reference: {returned_shadow:?}"))
        .to_owned();
    let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
    let element_shadow = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    assert_eq!(
        element_shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY],
        json!(returned_shadow_id.clone()),
        "same ShadowRoot must keep the same WebDriver reference id"
    );
    let returned_shadow_again = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('#host').shadowRoot;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        returned_shadow_again["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY],
        json!(returned_shadow_id.clone()),
        "repeated execute_script should reuse the same ShadowRoot id"
    );

    let text_from_shadow_arg = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].querySelector('#inside').textContent;",
            "args": [{
                CLASSIC_SHADOW_ROOT_REFERENCE_KEY: returned_shadow_id.clone()
            }]
        }),
    )
    .await;
    assert_eq!(text_from_shadow_arg, json!({ "value": "shadow text" }));

    let nested_shadow_arg = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].root.querySelector('#inside').id;",
            "args": [{
                "root": {
                    CLASSIC_SHADOW_ROOT_REFERENCE_KEY: returned_shadow_id.clone()
                }
            }]
        }),
    )
    .await;
    assert_eq!(nested_shadow_arg, json!({ "value": "inside" }));

    let returned_element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].querySelector('#inside');",
            "args": [{
                CLASSIC_SHADOW_ROOT_REFERENCE_KEY: returned_shadow_id.clone()
            }]
        }),
    )
    .await;
    let returned_element_id = returned_element["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("execute returned element reference: {returned_element:?}"));
    let returned_element_text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{returned_element_id}/text"),
    )
    .await;
    assert_eq!(returned_element_text, json!({ "value": "shadow text" }));

    let async_shadow_arg = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1](arguments[0].querySelector('#inside').id);",
            "args": [{
                CLASSIC_SHADOW_ROOT_REFERENCE_KEY: returned_shadow_id.clone()
            }]
        }),
    )
    .await;
    assert_eq!(async_shadow_arg, json!({ "value": "inside" }));

    let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
    let host_ref = json!({
        CLASSIC_ELEMENT_REFERENCE_KEY: host_id,
    });
    let shadow_ref = json!({
        CLASSIC_SHADOW_ROOT_REFERENCE_KEY: returned_shadow_id.clone(),
    });
    let (detached_return_status, detached_return) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "arguments[0].remove(); return arguments[1];",
            "args": [host_ref, shadow_ref.clone()]
        }),
    )
    .await;
    assert_eq!(
        detached_return_status,
        StatusCode::NOT_FOUND,
        "detached shadow root return response: {detached_return:?}"
    );
    assert_eq!(
        detached_return["value"]["error"],
        json!("detached shadow root")
    );

    let (detached_arg_status, detached_arg) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return true;",
            "args": [shadow_ref]
        }),
    )
    .await;
    assert_eq!(detached_arg_status, StatusCode::NOT_FOUND);
    assert_eq!(
        detached_arg["value"]["error"],
        json!("detached shadow root")
    );

    let (invalid_status, invalid) = classic_request_status_and_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return true;",
            "args": [{
                CLASSIC_SHADOW_ROOT_REFERENCE_KEY: 42
            }]
        }),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid["value"]["error"], json!("invalid argument"));
}

#[tokio::test]
async fn webdriver_classic_execute_script_round_trips_window_and_frame_references() {
    // Ported from Chromium/WPT webdriver/tests/classic/execute_script/window.py
    // and execute_script/arguments.py WebWindow/WebFrame cases.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page_url = format!("http://{fixture_addr}/page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let current_window = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    let current_window_id = current_window["value"]
        .as_str()
        .expect("current window handle")
        .to_owned();

    let references = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [window, window.frames[0], { ref: window.frames[0] }];",
            "args": []
        }),
    )
    .await;
    let window_id = references["value"][0][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("execute returned WebWindow reference: {references:?}"))
        .to_owned();
    let frame_id = references["value"][1][CLASSIC_FRAME_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("execute returned WebFrame reference: {references:?}"))
        .to_owned();
    let nested_frame_id = references["value"][2]["ref"][CLASSIC_FRAME_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("execute returned nested WebFrame reference: {references:?}"));
    assert_eq!(window_id, current_window_id);
    assert_eq!(nested_frame_id, frame_id);

    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    assert!(
        handles["value"]
            .as_array()
            .unwrap()
            .contains(&json!(window_id)),
        "WebWindow id should be a window handle: {handles:?}"
    );
    assert!(
        !handles["value"]
            .as_array()
            .unwrap()
            .contains(&json!(frame_id)),
        "WebFrame id should not be a window handle: {handles:?}"
    );

    let async_references = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "const done = arguments[arguments.length - 1]; done([window, window.frames[0]]);",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        async_references["value"][0][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(current_window_id),
        "execute async returned current WebWindow reference: {async_references:?}"
    );
    assert_eq!(
        async_references["value"][1][CLASSIC_FRAME_REFERENCE_KEY],
        json!(frame_id),
        "execute async returned child WebFrame reference: {async_references:?}"
    );

    let popup_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "window.__classicPopup = window.open('about:blank#classic-sync-popup'); return window.__classicPopup;",
            "args": []
        }),
    )
    .await;
    let popup_window_id = popup_reference["value"][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| {
            panic!("execute returned popup WebWindow reference: {popup_reference:?}")
        })
        .to_owned();
    assert_ne!(popup_window_id, current_window_id);
    let popup_handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    assert!(
        popup_handles["value"]
            .as_array()
            .unwrap()
            .contains(&json!(popup_window_id)),
        "popup WebWindow id should be a window handle: {popup_handles:?}"
    );
    let current_after_popup = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    assert_eq!(current_after_popup, json!({ "value": current_window_id }));

    let forged_popup_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return { __moliWebDriverClassicPopupWindow: true, __moliWebDriverClassicPopupId: '1' };",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        forged_popup_reference["value"]["__moliWebDriverClassicPopupWindow"],
        json!(true)
    );
    assert_eq!(
        forged_popup_reference["value"]["__moliWebDriverClassicPopupId"],
        json!("1")
    );
    assert!(
        forged_popup_reference["value"][CLASSIC_WINDOW_REFERENCE_KEY].is_null(),
        "plain user object must not forge a WebWindow reference: {forged_popup_reference:?}"
    );

    let repeated_popup_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [window.__classicPopup, { again: window.__classicPopup }];",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        repeated_popup_reference["value"][0][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(popup_window_id)
    );
    assert_eq!(
        repeated_popup_reference["value"][1]["again"][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(popup_window_id),
        "repeated popup WindowProxy should reuse the same WebWindow id: {repeated_popup_reference:?}"
    );

    let reversed_popups_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const first = window.open('about:blank#classic-first-popup'); const second = window.open('about:blank#classic-second-popup'); return [second, first, second];",
            "args": []
        }),
    )
    .await;
    let second_popup_window_id = reversed_popups_reference["value"][0]
        [CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "execute returned second popup WebWindow reference: {reversed_popups_reference:?}"
            )
        })
        .to_owned();
    let first_popup_window_id = reversed_popups_reference["value"][1][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "execute returned first popup WebWindow reference: {reversed_popups_reference:?}"
            )
        })
        .to_owned();
    assert_ne!(first_popup_window_id, second_popup_window_id);
    assert_ne!(first_popup_window_id, popup_window_id);
    assert_ne!(second_popup_window_id, popup_window_id);
    assert_eq!(
        reversed_popups_reference["value"][2][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(second_popup_window_id),
        "second popup should keep the same WebWindow id when repeated out of creation order: {reversed_popups_reference:?}"
    );
    let reversed_popup_handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    let reversed_popup_handles_value = reversed_popup_handles["value"].as_array().unwrap();
    assert!(reversed_popup_handles_value.contains(&json!(first_popup_window_id)));
    assert!(reversed_popup_handles_value.contains(&json!(second_popup_window_id)));

    let async_popup_reference = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "const done = arguments[arguments.length - 1]; window.__classicAsyncPopup = window.open('about:blank#classic-async-popup'); done(window.__classicAsyncPopup);",
            "args": []
        }),
    )
    .await;
    let async_popup_window_id = async_popup_reference["value"][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| {
            panic!("execute async returned popup WebWindow reference: {async_popup_reference:?}")
        })
        .to_owned();
    assert_ne!(async_popup_window_id, current_window_id);
    assert_ne!(async_popup_window_id, popup_window_id);
    let async_popup_handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    assert!(
        async_popup_handles["value"]
            .as_array()
            .unwrap()
            .contains(&json!(async_popup_window_id)),
        "async popup WebWindow id should be a window handle: {async_popup_handles:?}"
    );

    let window_round_trip = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0] === window;",
            "args": [{
                CLASSIC_WINDOW_REFERENCE_KEY: window_id.clone()
            }]
        }),
    )
    .await;
    assert_eq!(window_round_trip, json!({ "value": true }));

    let frame_round_trip = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0] === window.frames[0];",
            "args": [{
                CLASSIC_FRAME_REFERENCE_KEY: frame_id.clone()
            }]
        }),
    )
    .await;
    assert_eq!(frame_round_trip, json!({ "value": true }));

    let object_identifier_not_first = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0] === window.frames[0];",
            "args": [{
                "foo": "bar",
                CLASSIC_FRAME_REFERENCE_KEY: frame_id.clone(),
                "baz": 1314
            }]
        }),
    )
    .await;
    assert_eq!(object_identifier_not_first, json!({ "value": true }));

    let async_frame_round_trip = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1](arguments[0] === window.frames[0]);",
            "args": [{
                CLASSIC_FRAME_REFERENCE_KEY: frame_id.clone()
            }]
        }),
    )
    .await;
    assert_eq!(async_frame_round_trip, json!({ "value": true }));

    let (invalid_frame_status, invalid_frame) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return true;",
            "args": [{
                CLASSIC_FRAME_REFERENCE_KEY: 42
            }]
        }),
    )
    .await;
    assert_eq!(invalid_frame_status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_frame["value"]["error"], json!("invalid argument"));

    let (invalid_window_status, invalid_window) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return true;",
            "args": [{
                CLASSIC_WINDOW_REFERENCE_KEY: false
            }]
        }),
    )
    .await;
    assert_eq!(invalid_window_status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_window["value"]["error"], json!("invalid argument"));

    let (wrong_window_status, wrong_window) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return true;",
            "args": [{
                CLASSIC_WINDOW_REFERENCE_KEY: frame_id.clone()
            }]
        }),
    )
    .await;
    assert_eq!(wrong_window_status, StatusCode::NOT_FOUND);
    assert_eq!(wrong_window["value"]["error"], json!("no such window"));

    let (wrong_frame_status, wrong_frame) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return true;",
            "args": [{
                CLASSIC_FRAME_REFERENCE_KEY: window_id
            }]
        }),
    )
    .await;
    assert_eq!(
        wrong_frame_status,
        StatusCode::NOT_FOUND,
        "wrong frame reference response: {wrong_frame:?}"
    );
    assert_eq!(wrong_frame["value"]["error"], json!("no such frame"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_execute_script_window_reference_keeps_id_after_cross_origin_navigation()
{
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/window.py test_same_id_after_cross_origin_navigation.
    let app = build_router(test_state());
    let (first_addr, first_server) = spawn_classic_cookie_fixture_server().await;
    let (second_addr, second_server) = spawn_classic_cookie_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let current_window = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    let current_window_id = current_window["value"]
        .as_str()
        .expect("current window handle")
        .to_owned();

    let first_url = format!("http://{first_addr}/page");
    let first_navigation = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": first_url }),
    )
    .await;
    assert_eq!(first_navigation, json!({ "value": null }));
    let window_before = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        window_before["value"][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(current_window_id)
    );

    let second_url = format!("http://{second_addr}/page");
    let second_navigation = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": second_url }),
    )
    .await;
    assert_eq!(second_navigation, json!({ "value": null }));
    let window_after = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        window_after["value"][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(current_window_id)
    );

    let current_after_navigation = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    assert_eq!(
        current_after_navigation,
        json!({ "value": current_window_id })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    first_server.abort();
    second_server.abort();
}

#[tokio::test]
async fn webdriver_classic_alert_routes_match_selenium_prompt_flow() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let alert_text_path = format!("/session/{session_id}/alert/text");
    let alert_accept_path = format!("/session/{session_id}/alert/accept");
    let alert_dismiss_path = format!("/session/{session_id}/alert/dismiss");

    let (missing_status, missing) =
        classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(missing["value"]["error"], json!("no such alert"));
    let (missing_accept_status, missing_accept) =
        classic_request_status_and_json(app.clone(), Method::POST, &alert_accept_path).await;
    assert_eq!(missing_accept_status, StatusCode::NOT_FOUND);
    assert_eq!(missing_accept["value"]["error"], json!("no such alert"));

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "setTimeout(() => { alert('classic alert'); }, 0); return 'opened';",
        "classic alert",
    )
    .await;
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &format!("{alert_text_path}/")).await,
        json!({ "value": "classic alert" }),
        "reading alert text should not consume the pending alert"
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::POST, &alert_accept_path).await,
        json!({ "value": null })
    );
    let (closed_status, closed) =
        classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
    assert_eq!(closed_status, StatusCode::NOT_FOUND);
    assert_eq!(closed["value"]["error"], json!("no such alert"));

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "setTimeout(() => { alert('classic dismiss'); }, 0); return 'opened';",
        "classic dismiss",
    )
    .await;
    assert_eq!(
        classic_request_json(app.clone(), Method::POST, &alert_dismiss_path).await,
        json!({ "value": null })
    );
    let (dismissed_status, dismissed) =
        classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
    assert_eq!(dismissed_status, StatusCode::NOT_FOUND);
    assert_eq!(dismissed["value"]["error"], json!("no such alert"));

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "setTimeout(() => { prompt('Prompt?', 'default'); }, 0); return 'opened';",
        "Prompt?",
    )
    .await;
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &alert_text_path,
            json!({ "text": "cheese" })
        )
        .await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::POST, &alert_accept_path).await,
        json!({ "value": null })
    );

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "setTimeout(() => { alert('not a prompt'); }, 0); return 'opened';",
        "not a prompt",
    )
    .await;
    let (send_to_alert_status, send_to_alert) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &alert_text_path,
        json!({ "text": "ignored" }),
    )
    .await;
    assert_eq!(send_to_alert_status, StatusCode::BAD_REQUEST);
    assert_eq!(
        send_to_alert["value"]["error"],
        json!("element not interactable")
    );
    let _ = classic_request_json(app, Method::POST, &alert_accept_path).await;
}

#[tokio::test]
async fn webdriver_classic_timer_dialog_completion_resumes_javascript_return_values() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let execute_path = format!("/session/{session_id}/execute/sync");
    let alert_text_path = format!("/session/{session_id}/alert/text");
    let alert_accept_path = format!("/session/{session_id}/alert/accept");
    let alert_dismiss_path = format!("/session/{session_id}/alert/dismiss");

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "window.__dialogResults = {}; setTimeout(() => { window.__dialogResults.confirmAccept = confirm('confirm accept'); }, 0); return 'opened';",
        "confirm accept",
    )
    .await;
    assert_eq!(
        classic_request_json(app.clone(), Method::POST, &alert_accept_path).await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return window.__dialogResults.confirmAccept;",
                "args": []
            }),
        )
        .await,
        json!({ "value": true })
    );

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "setTimeout(() => { window.__dialogResults.confirmDismiss = confirm('confirm dismiss'); }, 0); return 'opened';",
        "confirm dismiss",
    )
    .await;
    assert_eq!(
        classic_request_json(app.clone(), Method::POST, &alert_dismiss_path).await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return window.__dialogResults.confirmDismiss;",
                "args": []
            }),
        )
        .await,
        json!({ "value": false })
    );

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "setTimeout(() => { window.__dialogResults.promptAccept = prompt('prompt accept', 'default'); }, 0); return 'opened';",
        "prompt accept",
    )
    .await;
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &alert_text_path,
            json!({ "text": "entered" })
        )
        .await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::POST, &alert_accept_path).await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return window.__dialogResults.promptAccept;",
                "args": []
            }),
        )
        .await,
        json!({ "value": "entered" })
    );

    classic_open_dialog_and_wait(
        app.clone(),
        session_id,
        "setTimeout(() => { window.__dialogResults.promptDismiss = prompt('prompt dismiss', 'default'); }, 0); return 'opened';",
        "prompt dismiss",
    )
    .await;
    assert_eq!(
        classic_request_json(app.clone(), Method::POST, &alert_dismiss_path).await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return window.__dialogResults.promptDismiss;",
                "args": []
            }),
        )
        .await,
        json!({ "value": null })
    );

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}

#[tokio::test]
async fn webdriver_classic_unhandled_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // new_session/unhandled_prompt_behavior.py and get_computed_{label,role}/user_prompts.py.
    let app = build_router(test_state());

    struct PromptCase {
        capability: Option<serde_json::Value>,
        expected_capability: serde_json::Value,
        endpoint: &'static str,
        expected_value: serde_json::Value,
        expect_notify: bool,
        expect_closed: bool,
    }

    let cases = [
        PromptCase {
            capability: None,
            expected_capability: json!("dismiss and notify"),
            endpoint: "computedlabel",
            expected_value: json!("ok"),
            expect_notify: true,
            expect_closed: true,
        },
        PromptCase {
            capability: Some(json!("accept")),
            expected_capability: json!("accept"),
            endpoint: "computedlabel",
            expected_value: json!("ok"),
            expect_notify: false,
            expect_closed: true,
        },
        PromptCase {
            capability: Some(json!("accept and notify")),
            expected_capability: json!("accept and notify"),
            endpoint: "computedrole",
            expected_value: json!("searchbox"),
            expect_notify: true,
            expect_closed: true,
        },
        PromptCase {
            capability: Some(json!("dismiss")),
            expected_capability: json!("dismiss"),
            endpoint: "computedrole",
            expected_value: json!("searchbox"),
            expect_notify: false,
            expect_closed: true,
        },
        PromptCase {
            capability: Some(json!("ignore")),
            expected_capability: json!("ignore"),
            endpoint: "computedlabel",
            expected_value: json!("ok"),
            expect_notify: true,
            expect_closed: false,
        },
        PromptCase {
            capability: Some(json!({"default": "accept", "alert": "ignore"})),
            expected_capability: json!({"default": "accept", "alert": "ignore"}),
            endpoint: "computedrole",
            expected_value: json!("searchbox"),
            expect_notify: true,
            expect_closed: false,
        },
        PromptCase {
            capability: Some(json!({"default": "accept"})),
            expected_capability: json!({"default": "accept"}),
            endpoint: "computedlabel",
            expected_value: json!("ok"),
            expect_notify: false,
            expect_closed: true,
        },
    ];

    for case in cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        assert_eq!(
            session["value"]["capabilities"]["unhandledPromptBehavior"], case.expected_capability,
            "returned unhandledPromptBehavior for {:?}",
            case.capability
        );

        let url = classic_data_url(
            "<button id='labelled' aria-label='ok'>ignored</button>\
             <input id='role' role='searchbox'>",
        );
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({ "url": url }),
            )
            .await,
            json!({ "value": null })
        );
        let selector = if case.endpoint == "computedlabel" {
            "#labelled"
        } else {
            "#role"
        };
        let found = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/element"),
            json!({ "using": "css selector", "value": selector }),
        )
        .await;
        let element_id = found["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .expect("element id");

        classic_open_dialog_and_wait(
            app.clone(),
            session_id,
            "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            "cheese",
        )
        .await;

        let command_path = format!(
            "/session/{session_id}/element/{element_id}/{}",
            case.endpoint
        );
        if case.expect_notify {
            let (status, response) =
                classic_request_status_and_json(app.clone(), Method::GET, &command_path).await;
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "capability {:?} endpoint {} response {response:?}",
                case.capability,
                case.endpoint
            );
            assert_eq!(
                response["value"]["error"],
                json!("unexpected alert open"),
                "{:?}",
                case.capability
            );
            assert_eq!(response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                classic_request_json(app.clone(), Method::GET, &command_path).await,
                json!({ "value": case.expected_value })
            );
        }

        let alert_text_path = format!("/session/{session_id}/alert/text");
        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }

    for invalid in [
        json!(false),
        json!("ACCEPT"),
        json!("ignore "),
        json!({"foo": "accept"}),
        json!({"beforeunload": "accept"}),
        json!({"alert": null}),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            "/session",
            json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": invalid
                    }
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response:?}");
        assert_eq!(response["value"]["error"], json!("invalid argument"));
    }
}

#[tokio::test]
async fn webdriver_classic_unhandled_prompt_command_sweep_matches_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_title/user_prompts.py, get_page_source/user_prompts.py,
    // get_current_url/user_prompts.py, get_window_handle/user_prompts.py,
    // and get_window_rect/user_prompts.py. Timer-triggered dialogs let the
    // setup command return before the modal handler suspends its callback.
    let app = build_router(test_state());

    struct PromptCommandCase {
        capability: serde_json::Value,
        dialog_script: &'static str,
        command_path_suffix: &'static str,
        expect_notify: bool,
        expect_closed: bool,
    }

    let cases = [
        PromptCommandCase {
            capability: json!("accept"),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            command_path_suffix: "title",
            expect_notify: false,
            expect_closed: true,
        },
        PromptCommandCase {
            capability: json!("accept and notify"),
            dialog_script: "setTimeout(() => { prompt('cheese', ''); }, 0); return 'opened';",
            command_path_suffix: "source",
            expect_notify: true,
            expect_closed: true,
        },
        PromptCommandCase {
            capability: json!("dismiss"),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            command_path_suffix: "window/rect",
            expect_notify: false,
            expect_closed: true,
        },
        PromptCommandCase {
            capability: json!("dismiss and notify"),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            command_path_suffix: "url",
            expect_notify: true,
            expect_closed: true,
        },
        PromptCommandCase {
            capability: json!("ignore"),
            dialog_script: "setTimeout(() => { prompt('cheese', ''); }, 0); return 'opened';",
            command_path_suffix: "source",
            expect_notify: true,
            expect_closed: false,
        },
        PromptCommandCase {
            capability: json!({"default": "accept", "prompt": "ignore"}),
            dialog_script: "setTimeout(() => { prompt('cheese', ''); }, 0); return 'opened';",
            command_path_suffix: "title",
            expect_notify: true,
            expect_closed: false,
        },
    ];

    for case in cases {
        let session = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            "/session",
            json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": case.capability
                    }
                }
            }),
        )
        .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({
                    "url": classic_data_url(
                        "<title>Prompt sweep</title><main id='content'>prompt sweep</main>",
                    )
                }),
            )
            .await,
            json!({ "value": null })
        );
        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let command_path = format!("/session/{session_id}/{}", case.command_path_suffix);
        let (status, response) =
            classic_request_status_and_json(app.clone(), Method::GET, &command_path).await;
        if case.expect_notify {
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "capability {:?} command {} response {response:?}",
                case.capability,
                case.command_path_suffix
            );
            assert_eq!(response["value"]["error"], json!("unexpected alert open"));
            assert_eq!(response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                status,
                StatusCode::OK,
                "capability {:?} command {} response {response:?}",
                case.capability,
                case.command_path_suffix
            );
        }

        let alert_text_path = format!("/session/{session_id}/alert/text");
        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn webdriver_classic_execute_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // execute_script/user_prompts.py and execute_async_script/user_prompts.py
    // alert/confirm/prompt cases. beforeunload navigation remains covered by
    // prompt-neutral navigation/window command tests.
    let app = build_router(test_state());

    struct ExecutePromptCase {
        capability: Option<serde_json::Value>,
        script_kind: &'static str,
        dialog_script: &'static str,
        expect_notify: bool,
        expect_closed: bool,
    }

    let cases = [
        ExecutePromptCase {
            capability: Some(json!("accept")),
            script_kind: "sync",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("accept and notify")),
            script_kind: "sync",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("dismiss")),
            script_kind: "sync",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("dismiss and notify")),
            script_kind: "sync",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("ignore")),
            script_kind: "sync",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        ExecutePromptCase {
            capability: None,
            script_kind: "sync",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("accept")),
            script_kind: "async",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("accept and notify")),
            script_kind: "async",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("dismiss")),
            script_kind: "async",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("dismiss and notify")),
            script_kind: "async",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ExecutePromptCase {
            capability: Some(json!("ignore")),
            script_kind: "async",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        ExecutePromptCase {
            capability: None,
            script_kind: "async",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
    ];

    for case in cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        let url = classic_data_url("<title>execute prompt</title>");
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({ "url": url }),
            )
            .await,
            json!({ "value": null })
        );

        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let execute_path = format!("/session/{session_id}/execute/{}", case.script_kind);
        let execute_body = if case.script_kind == "sync" {
            json!({
                "script": "window.result = 1; return 1;",
                "args": []
            })
        } else {
            json!({
                "script": "window.result = 1; arguments[arguments.length - 1](1);",
                "args": []
            })
        };
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            execute_body,
        )
        .await;
        if case.expect_notify {
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "capability {:?} {} response {response:?}",
                case.capability,
                case.script_kind
            );
            assert_eq!(response["value"]["error"], json!("unexpected alert open"));
            assert_eq!(response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                status,
                StatusCode::OK,
                "capability {:?} {} response {response:?}",
                case.capability,
                case.script_kind
            );
            assert_eq!(response, json!({ "value": 1 }));
        }

        let alert_text_path = format!("/session/{session_id}/alert/text");
        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let result = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return window.result ?? null;",
                "args": []
            }),
        )
        .await;
        if case.expect_notify {
            assert_eq!(
                result,
                json!({ "value": null }),
                "notified {} command must not run after prompt preflight",
                case.script_kind
            );
        } else {
            assert_eq!(
                result,
                json!({ "value": 1 }),
                "non-notifying {} command should run after prompt handling",
                case.script_kind
            );
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn webdriver_classic_locator_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // find_element*/user_prompts.py, find_elements*/user_prompts.py,
    // get_element_shadow_root/user_prompts.py, and
    // find_element(s)_from_shadow_root/user_prompts.py.
    let app = build_router(test_state());

    struct LocatorPromptCase {
        capability: Option<serde_json::Value>,
        endpoint: &'static str,
        dialog_script: &'static str,
        expect_notify: bool,
        expect_closed: bool,
    }

    let cases = [
        LocatorPromptCase {
            capability: Some(json!("accept")),
            endpoint: "find_element",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        LocatorPromptCase {
            capability: Some(json!("accept and notify")),
            endpoint: "find_elements",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        LocatorPromptCase {
            capability: Some(json!("dismiss")),
            endpoint: "find_element_from_element",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        LocatorPromptCase {
            capability: Some(json!("dismiss and notify")),
            endpoint: "find_elements_from_element",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        LocatorPromptCase {
            capability: Some(json!("ignore")),
            endpoint: "get_element_shadow_root",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        LocatorPromptCase {
            capability: None,
            endpoint: "find_element_from_shadow_root",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        LocatorPromptCase {
            capability: Some(json!({"default": "accept", "prompt": "ignore"})),
            endpoint: "find_elements_from_shadow_root",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
    ];

    for case in cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        let url = classic_data_url(
            r##"
            <div id="outer"><p id="target">bar</p></div>
            <custom-element id="host"></custom-element>
            <script>
              document.querySelector("#host")
                .attachShadow({ mode: "open" })
                .innerHTML = "<input id='shadowTarget' value='bar'>";
            </script>
            "##,
        );
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({ "url": url }),
            )
            .await,
            json!({ "value": null })
        );

        let outer_id = classic_find_css_element_id(app.clone(), session_id, "#outer").await;
        let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
        let shadow = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{host_id}/shadow"),
        )
        .await;
        let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("shadow root setup response: {shadow:?}"));

        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let locator_body = json!({
            "using": "css selector",
            "value": if case.endpoint.contains("shadow_root") {
                "#shadowTarget"
            } else {
                "#target"
            }
        });
        let (status, response) = match case.endpoint {
            "find_element" => {
                classic_request_status_and_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/element"),
                    locator_body,
                )
                .await
            }
            "find_elements" => {
                classic_request_status_and_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/elements"),
                    locator_body,
                )
                .await
            }
            "find_element_from_element" => {
                classic_request_status_and_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/element/{outer_id}/element"),
                    locator_body,
                )
                .await
            }
            "find_elements_from_element" => {
                classic_request_status_and_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/element/{outer_id}/elements"),
                    locator_body,
                )
                .await
            }
            "get_element_shadow_root" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{host_id}/shadow"),
                )
                .await
            }
            "find_element_from_shadow_root" => {
                classic_request_status_and_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/shadow/{shadow_id}/element"),
                    locator_body,
                )
                .await
            }
            "find_elements_from_shadow_root" => {
                classic_request_status_and_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/shadow/{shadow_id}/elements"),
                    locator_body,
                )
                .await
            }
            endpoint => panic!("unknown locator prompt endpoint: {endpoint}"),
        };
        if case.expect_notify {
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "capability {:?} endpoint {} response {response:?}",
                case.capability,
                case.endpoint
            );
            assert_eq!(response["value"]["error"], json!("unexpected alert open"));
            assert_eq!(response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                status,
                StatusCode::OK,
                "capability {:?} endpoint {} response {response:?}",
                case.capability,
                case.endpoint
            );
            match case.endpoint {
                "find_elements"
                | "find_elements_from_element"
                | "find_elements_from_shadow_root" => {
                    let values = response["value"]
                        .as_array()
                        .unwrap_or_else(|| panic!("element array response: {response:?}"));
                    assert_eq!(values.len(), 1, "{response:?}");
                    assert!(
                        values[0][CLASSIC_ELEMENT_REFERENCE_KEY].is_string(),
                        "{response:?}"
                    );
                }
                "get_element_shadow_root" => {
                    assert!(
                        response["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY].is_string(),
                        "{response:?}"
                    );
                }
                _ => {
                    assert!(
                        response["value"][CLASSIC_ELEMENT_REFERENCE_KEY].is_string(),
                        "{response:?}"
                    );
                }
            }
        }

        let alert_text_path = format!("/session/{session_id}/alert/text");
        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn webdriver_classic_element_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_element_{attribute,property,css_value,text,tag_name}/user_prompts.py,
    // is_element_{enabled,selected}/user_prompts.py,
    // get_element_rect/user_prompts.py,
    // get_active_element/user_prompts.py, and
    // element_{clear,click,send_keys}/user_prompts.py.
    let app = build_router(test_state());

    struct ElementPromptCase {
        capability: Option<serde_json::Value>,
        endpoint: &'static str,
        dialog_script: &'static str,
        expect_notify: bool,
        expect_closed: bool,
    }

    let cases = [
        ElementPromptCase {
            capability: Some(json!("accept")),
            endpoint: "attribute",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("accept and notify")),
            endpoint: "property",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("dismiss")),
            endpoint: "css",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("dismiss and notify")),
            endpoint: "text",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("ignore")),
            endpoint: "name",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        ElementPromptCase {
            capability: None,
            endpoint: "enabled",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!({"default": "accept", "prompt": "ignore"})),
            endpoint: "selected",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        ElementPromptCase {
            capability: Some(json!("accept")),
            endpoint: "rect",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("ignore")),
            endpoint: "rect",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        ElementPromptCase {
            capability: Some(json!("accept")),
            endpoint: "active",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("dismiss")),
            endpoint: "clear",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("accept and notify")),
            endpoint: "clear",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("accept")),
            endpoint: "click",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("dismiss and notify")),
            endpoint: "click",
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("dismiss")),
            endpoint: "send_keys",
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        ElementPromptCase {
            capability: Some(json!("ignore")),
            endpoint: "send_keys",
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
    ];

    for case in cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        let url = classic_data_url(
            r#"
            <input id="foo" style="display:block;width:120px;height:40px" value="foo">
            <p id="text">bar</p>
            <input id="checked" type="checkbox" checked>
            <input id="active">
            <input id="clear" value="foo">
            <button id="click" onclick="window.__clicked = true">click</button>
            <input id="send" value="">
            <script>window.__clicked = false;</script>
            "#,
        );
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({ "url": url }),
            )
            .await,
            json!({ "value": null })
        );

        let foo_id = classic_find_css_element_id(app.clone(), session_id, "#foo").await;
        let text_id = classic_find_css_element_id(app.clone(), session_id, "#text").await;
        let checked_id = classic_find_css_element_id(app.clone(), session_id, "#checked").await;
        let clear_id = classic_find_css_element_id(app.clone(), session_id, "#clear").await;
        let click_id = classic_find_css_element_id(app.clone(), session_id, "#click").await;
        let send_id = classic_find_css_element_id(app.clone(), session_id, "#send").await;

        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/execute/sync"),
                json!({
                    "script": "document.getElementById('active').focus(); window.__clicked = false; return 'prepared';",
                    "args": []
                }),
            )
            .await,
            json!({ "value": "prepared" })
        );
        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let (status, response) = match case.endpoint {
            "attribute" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{foo_id}/attribute/id"),
                )
                .await
            }
            "property" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{foo_id}/property/id"),
                )
                .await
            }
            "css" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{foo_id}/css/display"),
                )
                .await
            }
            "text" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{text_id}/text"),
                )
                .await
            }
            "name" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{foo_id}/name"),
                )
                .await
            }
            "enabled" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{foo_id}/enabled"),
                )
                .await
            }
            "selected" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{checked_id}/selected"),
                )
                .await
            }
            "rect" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/{foo_id}/rect"),
                )
                .await
            }
            "active" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::GET,
                    &format!("/session/{session_id}/element/active"),
                )
                .await
            }
            "clear" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/element/{clear_id}/clear"),
                )
                .await
            }
            "click" => {
                classic_request_status_and_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/element/{click_id}/click"),
                )
                .await
            }
            "send_keys" => {
                classic_request_status_and_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/element/{send_id}/value"),
                    json!({ "text": "typed" }),
                )
                .await
            }
            endpoint => panic!("unknown element prompt endpoint: {endpoint}"),
        };
        if case.expect_notify {
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "capability {:?} endpoint {} response {response:?}",
                case.capability,
                case.endpoint
            );
            assert_eq!(response["value"]["error"], json!("unexpected alert open"));
            assert_eq!(response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                status,
                StatusCode::OK,
                "capability {:?} endpoint {} response {response:?}",
                case.capability,
                case.endpoint
            );
            match case.endpoint {
                "attribute" | "property" => assert_eq!(response, json!({ "value": "foo" })),
                "css" => assert_eq!(response, json!({ "value": "block" })),
                "text" => assert_eq!(response, json!({ "value": "bar" })),
                "name" => assert_eq!(response, json!({ "value": "input" })),
                "enabled" | "selected" => assert_eq!(response, json!({ "value": true })),
                "rect" => {
                    let value = &response["value"];
                    assert_eq!(value["x"].as_f64(), Some(8.0), "{response:?}");
                    assert_eq!(value["y"].as_f64(), Some(8.0), "{response:?}");
                    assert_eq!(value["width"].as_f64(), Some(120.0), "{response:?}");
                    assert_eq!(value["height"].as_f64(), Some(40.0), "{response:?}");
                }
                "active" => {
                    let active_id = response["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
                        .as_str()
                        .unwrap_or_else(|| panic!("active element response: {response:?}"));
                    let active_property = classic_request_json(
                        app.clone(),
                        Method::GET,
                        &format!("/session/{session_id}/element/{active_id}/property/id"),
                    )
                    .await;
                    assert_eq!(active_property, json!({ "value": "active" }));
                }
                "clear" | "click" | "send_keys" => {
                    assert_eq!(response, json!({ "value": null }))
                }
                endpoint => panic!("unknown successful endpoint: {endpoint}"),
            }
        }

        let alert_text_path = format!("/session/{session_id}/alert/text");
        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let command_ran = !case.expect_notify;
        if case.endpoint == "clear" {
            let value = classic_request_json(
                app.clone(),
                Method::GET,
                &format!("/session/{session_id}/element/{clear_id}/property/value"),
            )
            .await;
            assert_eq!(
                value,
                json!({ "value": if command_ran { "" } else { "foo" } })
            );
        } else if case.endpoint == "click" {
            let clicked = classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/execute/sync"),
                json!({
                    "script": "return Boolean(window.__clicked);",
                    "args": []
                }),
            )
            .await;
            assert_eq!(clicked, json!({ "value": command_ran }));
        } else if case.endpoint == "send_keys" {
            let value = classic_request_json(
                app.clone(),
                Method::GET,
                &format!("/session/{session_id}/element/{send_id}/property/value"),
            )
            .await;
            assert_eq!(
                value,
                json!({ "value": if command_ran { "typed" } else { "" } })
            );
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn webdriver_classic_get_element_tag_name_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_element_tag_name/get.py test_no_such_element_with_invalid_value
    // and test_get_element_tag_name.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let (invalid_status, invalid) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/foo/name"),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::NOT_FOUND);
    assert_eq!(invalid["value"]["error"], json!("no such element"));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": "data:text/html,<input id=foo>"
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "input"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("input element reference id");

    let tag_name = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/name"),
    )
    .await;
    assert_eq!(tag_name, json!({ "value": "input" }));
}

#[tokio::test]
async fn webdriver_classic_get_element_rect_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_element_rect/get.py invalid element and rect payload shape cases.
    // The response is now backed by the same real layout box model used by
    // CDP and CSSOM View.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let (invalid_status, invalid) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/foo/rect"),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::NOT_FOUND);
    assert_eq!(invalid["value"]["error"], json!("no such element"));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": "data:text/html,<div id=target style='width:120px;height:40px'></div>"
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    let rect = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/rect"),
    )
    .await;
    let value = &rect["value"];
    assert_eq!(value["x"].as_f64(), Some(8.0));
    assert_eq!(value["y"].as_f64(), Some(8.0));
    assert_eq!(value["width"].as_f64(), Some(120.0));
    assert_eq!(value["height"].as_f64(), Some(40.0));
}

#[tokio::test]
async fn webdriver_classic_element_state_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // is_element_selected/selected.py checked/option cases and
    // is_element_enabled/enabled.py direct form-control enabled/disabled cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    for endpoint in ["enabled", "selected"] {
        let (invalid_status, invalid) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/foo/{endpoint}"),
        )
        .await;
        assert_eq!(invalid_status, StatusCode::NOT_FOUND, "{endpoint}");
        assert_eq!(
            invalid["value"]["error"],
            json!("no such element"),
            "{endpoint}"
        );
    }

    let html = concat!(
        "<input id=checked type=checkbox checked>",
        "<input id=notChecked type=checkbox>",
        "<select><option id=notSelected>r-</option><option id=selected selected>r+</option></select>",
        "<input id=enabledInput>",
        "<input id=disabledInput disabled>",
        "<button id=enabledButton></button>",
        "<button id=disabledButton disabled></button>",
        "<textarea id=enabledTextarea></textarea>",
        "<textarea id=disabledTextarea disabled></textarea>",
        "<select id=enabledSelect></select>",
        "<select id=disabledSelect disabled></select>",
        "<select><optgroup id=enabledOptgroup><option id=optionInEnabledOptgroup>og+</option></optgroup></select>",
        "<select><optgroup id=disabledOptgroup disabled><option id=optionInDisabledOptgroup>og-</option></optgroup></select>",
        "<select id=disabledSelectWithOptions disabled><optgroup id=optgroupInDisabledSelect><option id=optionInDisabledSelect>ds-</option></optgroup></select>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": format!("data:text/html,{html}")
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    for (selector, expected) in [
        ("#checked", true),
        ("#notChecked", false),
        ("#selected", true),
        ("#notSelected", false),
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/selected"),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{selector}");
    }

    for (selector, expected) in [
        ("#enabledInput", true),
        ("#disabledInput", false),
        ("#enabledButton", true),
        ("#disabledButton", false),
        ("#enabledTextarea", true),
        ("#disabledTextarea", false),
        ("#enabledSelect", true),
        ("#disabledSelect", false),
        ("#enabledOptgroup", true),
        ("#optionInEnabledOptgroup", true),
        ("#disabledOptgroup", false),
        ("#optionInDisabledOptgroup", false),
        ("#disabledSelectWithOptions", false),
        ("#optgroupInDisabledSelect", false),
        ("#optionInDisabledSelect", false),
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/enabled"),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{selector}");
    }

    let stale_id = classic_find_css_element_id(app.clone(), session_id, "#enabledInput").await;
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "document.querySelector('#enabledInput').remove(); return 'removed';",
                "args": []
            }),
        )
        .await,
        json!({ "value": "removed" })
    );
    let (stale_status, stale) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{stale_id}/enabled"),
    )
    .await;
    assert_eq!(stale_status, StatusCode::NOT_FOUND, "{stale:?}");
    assert_eq!(stale["value"]["error"], json!("stale element reference"));
}

#[tokio::test]
async fn webdriver_classic_enabled_form_control_matrix_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // is_element_enabled/enabled.py button/input type matrix and fieldset
    // descendant cases. XML/XHTML parser-mode cases stay out of this Classic
    // route test because Moli's Classic data-url helpers exercise the
    // HTML document path.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let mut html = String::from("<!doctype html><main>");
    let mut cases: Vec<(String, bool)> = Vec::new();

    for button_type in ["button", "reset", "submit"] {
        for (status, expected) in [("enabled", true), ("disabled", false)] {
            let id = format!("{status}-button-{button_type}");
            let disabled = if expected { "" } else { " disabled" };
            html.push_str(&format!(
                r#"<button id="{id}" type="{button_type}"{disabled}>{button_type}</button>"#
            ));
            cases.push((format!("#{id}"), expected));
        }
    }

    for input_type in [
        "button",
        "checkbox",
        "color",
        "date",
        "datetime-local",
        "email",
        "file",
        "image",
        "month",
        "number",
        "password",
        "radio",
        "range",
        "reset",
        "search",
        "submit",
        "tel",
        "text",
        "time",
        "url",
        "week",
    ] {
        for (status, expected) in [("enabled", true), ("disabled", false)] {
            let id = format!("{status}-input-{input_type}");
            let disabled = if expected { "" } else { " disabled" };
            html.push_str(&format!(
                r#"<input id="{id}" type="{input_type}"{disabled}>"#
            ));
            cases.push((format!("#{id}"), expected));
        }
    }

    html.push_str(
        r#"
        <textarea id="enabled-textarea"></textarea>
        <textarea id="disabled-textarea" disabled></textarea>
        <select id="enabled-select"></select>
        <select id="disabled-select" disabled></select>
        <fieldset id="enabled-fieldset"><input id="enabled-fieldset-child"></fieldset>
        <fieldset id="disabled-fieldset" disabled>
          <legend><input id="disabled-fieldset-first-legend-input"></legend>
          <input id="disabled-fieldset-child">
          <legend><input id="disabled-fieldset-second-legend-input"></legend>
        </fieldset>
        </main>
        "#,
    );
    cases.extend([
        ("#enabled-textarea".to_owned(), true),
        ("#disabled-textarea".to_owned(), false),
        ("#enabled-select".to_owned(), true),
        ("#disabled-select".to_owned(), false),
        ("#enabled-fieldset".to_owned(), true),
        ("#enabled-fieldset-child".to_owned(), true),
        ("#disabled-fieldset".to_owned(), false),
        ("#disabled-fieldset-first-legend-input".to_owned(), true),
        ("#disabled-fieldset-child".to_owned(), false),
        ("#disabled-fieldset-second-legend-input".to_owned(), false),
    ]);

    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/url"),
            json!({ "url": classic_data_url(&html) }),
        )
        .await,
        json!({ "value": null })
    );

    for (selector, expected) in cases {
        let element_id = classic_find_css_element_id(app.clone(), session_id, &selector).await;
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/enabled"),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{selector}");
    }
}

#[tokio::test]
async fn webdriver_classic_get_element_attribute_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_element_attribute/get.py normal, boolean attribute, global boolean
    // attribute, and anchor href cases. ChromeDriver implements the boolean
    // branch in chrome/test/chromedriver/element_commands.cc:
    // ExecuteGetElementAttribute.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = concat!(
        "<input id='checkbox' type='checkbox'>",
        "<input id='checked' type='checkbox' checked>",
        "<input id='disabled' disabled='false'>",
        "<p id='hidden' hidden>foo</p>",
        "<p id='plain'>foo</p>",
        "<p id='scoped' itemscope>foo</p>",
        "<a id='relative' href='/foo.html'>foo</a>",
        "<a id='absolute' href='https://example.test/foo.html'>foo</a>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": classic_data_url(html)
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let checkbox_id = classic_find_css_element_id(app.clone(), session_id, "#checkbox").await;
    let missing_checked = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{checkbox_id}/attribute/checked"),
    )
    .await;
    assert_eq!(missing_checked, json!({ "value": null }));

    let property_only_checked = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('#checkbox').checked = true; return document.querySelector('#checkbox').checked;",
            "args": []
        }),
    )
    .await;
    assert_eq!(property_only_checked, json!({ "value": true }));
    let still_missing_checked = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{checkbox_id}/attribute/checked"),
    )
    .await;
    assert_eq!(still_missing_checked, json!({ "value": null }));

    for (selector, name) in [
        ("#checked", "checked"),
        ("#disabled", "disabled"),
        ("#hidden", "hidden"),
        ("#scoped", "itemscope"),
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/attribute/{name}"),
        )
        .await;
        assert_eq!(response, json!({ "value": "true" }), "{selector} {name}");
    }

    let plain_id = classic_find_css_element_id(app.clone(), session_id, "#plain").await;
    let absent_hidden = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{plain_id}/attribute/hidden"),
    )
    .await;
    assert_eq!(absent_hidden, json!({ "value": null }));

    let relative_id = classic_find_css_element_id(app.clone(), session_id, "#relative").await;
    let relative_href = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{relative_id}/attribute/href"),
    )
    .await;
    assert_eq!(relative_href, json!({ "value": "/foo.html" }));

    let absolute_id = classic_find_css_element_id(app.clone(), session_id, "#absolute").await;
    let absolute_href = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{absolute_id}/attribute/href"),
    )
    .await;
    assert_eq!(
        absolute_href,
        json!({ "value": "https://example.test/foo.html" })
    );
}

#[tokio::test]
async fn webdriver_classic_get_element_property_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_element_property/get.py content/IDL attribute, primitive,
    // DOMTokenList, WebElement/WebFrame/ShadowRoot/WebWindow, mutated checkbox,
    // and anchor href cases. ChromeDriver implements this by evaluating
    // `function(elem, name) { return elem[name] }`.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page_url = format!("http://{fixture_addr}/page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let body_id = classic_find_css_element_id(app.clone(), session_id, "body").await;
    let seeded = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": r#"
                const parent = document.body;
                const div = document.querySelector('#top-main');
                const input = document.createElement('input');
                input.id = 'property-input';
                input.value = 'foobar';
                document.body.appendChild(input);
                const box = document.createElement('input');
                box.id = 'property-checkbox';
                box.type = 'checkbox';
                document.body.appendChild(box);
                const classes = document.createElement('div');
                classes.id = 'property-classes';
                classes.className = 'no cheese';
                document.body.appendChild(classes);
                const host = document.createElement('div');
                host.id = 'property-host';
                host.attachShadow({ mode: 'open' }).innerHTML = '<span>shadow</span>';
                document.body.appendChild(host);
                const link = document.createElement('a');
                link.id = 'property-link';
                link.href = '/foo.html';
                document.body.appendChild(link);

                parent.__string = 'foobar';
                parent.__number = 42;
                parent.__array = [];
                parent.__object = {};
                parent.__null = null;
                parent.__undefined = undefined;
                parent.__element = div;
                parent.__frame = document.querySelector('#child').contentWindow;
                parent.__shadowRoot = host.shadowRoot;
                parent.__window = document.defaultView;
                return 'seeded';
            "#,
            "args": []
        }),
    )
    .await;
    assert_eq!(seeded, json!({ "value": "seeded" }));

    for (property, expected) in [
        ("__string", json!("foobar")),
        ("__number", json!(42)),
        ("__array", json!([])),
        ("__object", json!({})),
        ("__null", json!(null)),
        ("__undefined", json!(null)),
        ("doesNotExist", json!(null)),
    ] {
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{body_id}/property/{property}"),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{property}");
    }

    let input_id = classic_find_css_element_id(app.clone(), session_id, "#property-input").await;
    let content_attribute_value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{input_id}/property/value"),
    )
    .await;
    assert_eq!(content_attribute_value, json!({ "value": "foobar" }));

    let updated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('#property-input').value = 'bar'; return document.querySelector('#property-input').value;",
            "args": []
        }),
    )
    .await;
    assert_eq!(updated, json!({ "value": "bar" }));
    let idl_attribute_value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{input_id}/property/value"),
    )
    .await;
    assert_eq!(idl_attribute_value, json!({ "value": "bar" }));

    let classes_id =
        classic_find_css_element_id(app.clone(), session_id, "#property-classes").await;
    let class_list = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{classes_id}/property/classList"),
    )
    .await;
    assert_eq!(class_list, json!({ "value": ["no", "cheese"] }));

    let element_reference = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/property/__element"),
    )
    .await;
    assert!(
        element_reference["value"][CLASSIC_ELEMENT_REFERENCE_KEY].is_string(),
        "element property should return a WebElement reference: {element_reference:?}"
    );

    let frame_reference = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/property/__frame"),
    )
    .await;
    assert!(
        frame_reference["value"][CLASSIC_FRAME_REFERENCE_KEY].is_string(),
        "frame property should return a WebFrame reference: {frame_reference:?}"
    );

    let shadow_root_reference = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/property/__shadowRoot"),
    )
    .await;
    assert!(
        shadow_root_reference["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY].is_string(),
        "shadowRoot property should return a ShadowRoot reference: {shadow_root_reference:?}"
    );

    let window_reference = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{body_id}/property/__window"),
    )
    .await;
    assert!(
        window_reference["value"][CLASSIC_WINDOW_REFERENCE_KEY].is_string(),
        "window property should return a WebWindow reference: {window_reference:?}"
    );

    let checkbox_id =
        classic_find_css_element_id(app.clone(), session_id, "#property-checkbox").await;
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{checkbox_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));
    let checked_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{checkbox_id}/property/checked"),
    )
    .await;
    assert_eq!(checked_property, json!({ "value": true }));

    let link_id = classic_find_css_element_id(app.clone(), session_id, "#property-link").await;
    let href_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{link_id}/property/href"),
    )
    .await;
    assert_eq!(
        href_property,
        json!({ "value": format!("http://{fixture_addr}/foo.html") })
    );

    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_computed_label_and_role_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_computed_label/get.py and get_computed_role/get.py.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r##"<!doctype html>
        <button id="plain">ok</button>
        <button id="labelled" aria-labelledby="one two"></button>
        <div id="one">ok</div>
        <div id="two">go</div>
        <button id="aria-label" aria-label="foo">bar</button>
        <label><input id="wrapped"> foo</label>
        <label for="for-input">foo</label><input id="for-input">
        <h1 id="heading">Level 1 Header</h1>
        <a id="link" href="/target">Accessible Link</a>
        <img id="logo" alt="Logo Alt">
        <label for="textarea">Biography</label><textarea id="textarea"></textarea>
        <label for="select">Favorite Food</label><select id="select"><option>Pizza</option></select>
        <input id="submit" type="submit" value="Send Form">
        <article id="article">foo</article>
        <input id="search" role="searchbox">
        <img id="img-button" role="button" tabindex="0">
        <custom-element id="host"></custom-element>
        <script>
          document.querySelector("#host").attachShadow({ mode: "open" }).innerHTML =
            "<input id='inside-shadow'>";
        </script>"##;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": classic_data_url(html)
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    for (selector, expected) in [
        ("#plain", "ok"),
        ("#labelled", "ok go"),
        ("#aria-label", "foo"),
        ("#wrapped", "foo"),
        ("#for-input", "foo"),
        ("#heading", "Level 1 Header"),
        ("#link", "Accessible Link"),
        ("#logo", "Logo Alt"),
        ("#textarea", "Biography"),
        ("#select", "Favorite Food"),
        ("#submit", "Send Form"),
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let label = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/computedlabel"),
        )
        .await;
        assert_eq!(label, json!({ "value": expected }), "{selector}");
    }

    for (selector, expected) in [
        ("#article", "article"),
        ("#heading", "heading"),
        ("#link", "link"),
        ("#logo", "img"),
        ("#textarea", "textbox"),
        ("#select", "combobox"),
        ("#submit", "button"),
        ("#search", "searchbox"),
        ("#img-button", "button"),
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let role = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/computedrole"),
        )
        .await;
        assert_eq!(role, json!({ "value": expected }), "{selector}");
    }

    for endpoint in ["computedlabel", "computedrole"] {
        let (invalid_status, invalid) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/foo/{endpoint}"),
        )
        .await;
        assert_eq!(invalid_status, StatusCode::NOT_FOUND, "{endpoint}");
        assert_eq!(invalid["value"]["error"], json!("no such element"));
    }

    let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
    let shadow = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("computed label/role shadow id: {shadow:?}"));
    for endpoint in ["computedlabel", "computedrole"] {
        let (shadow_status, shadow_response) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{shadow_id}/{endpoint}"),
        )
        .await;
        assert_eq!(shadow_status, StatusCode::NOT_FOUND, "{endpoint}");
        assert_eq!(
            shadow_response["value"]["error"],
            json!("no such element"),
            "{endpoint}: {shadow_response:?}"
        );
    }

    let plain_id = classic_find_css_element_id(app.clone(), session_id, "#plain").await;
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('#plain').remove();",
            "args": []
        }),
    )
    .await;
    for endpoint in ["computedlabel", "computedrole"] {
        let (stale_status, stale) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{plain_id}/{endpoint}"),
        )
        .await;
        assert_eq!(stale_status, StatusCode::NOT_FOUND, "{endpoint}: {stale:?}");
        assert_eq!(
            stale["value"]["error"],
            json!("stale element reference"),
            "{endpoint}: {stale:?}"
        );
    }
}

#[tokio::test]
async fn webdriver_classic_shadow_root_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // get_element_shadow_root/get.py and find_element(s)_from_shadow_root/find.py.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r##"<!doctype html>
        <div id="host"></div>
        <div id="closed-host"></div>
        <div id="outside" class="item">outside</div>
        <select id="select-no-shadow"></select>
        <video id="video-no-shadow"></video>
        <script>
          const root = document.querySelector("#host").attachShadow({ mode: "open" });
          root.innerHTML = `
            <main id="shadow-main">
              <input id="inside" class="item" value="shadow">
              <button id="button" class="item">Press</button>
              <a id="link" href="#docs">Docs</a>
            </main>`;
          const closedRoot = document.querySelector("#closed-host").attachShadow({ mode: "closed" });
          closedRoot.innerHTML = `<span id="closed-inside">closed text</span>`;
        </script>"##;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": classic_data_url(html)
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
    let (shadow_status, shadow) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    assert_eq!(
        shadow_status,
        StatusCode::OK,
        "shadow root response: {shadow}"
    );
    let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("shadow root reference id: {shadow:?}"))
        .to_owned();

    let inside = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/element"),
        json!({
            "using": "css selector",
            "value": "#inside"
        }),
    )
    .await;
    let inside_id = inside["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("shadow child element reference id: {inside:?}"));
    let inside_tag = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{inside_id}/name"),
    )
    .await;
    assert_eq!(inside_tag, json!({ "value": "input" }));

    let buttons = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/elements"),
        json!({
            "using": "class name",
            "value": "item"
        }),
    )
    .await;
    assert_eq!(
        buttons["value"].as_array().expect("shadow elements").len(),
        2
    );

    let link = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/element"),
        json!({
            "using": "link text",
            "value": "Docs"
        }),
    )
    .await;
    assert!(link["value"][CLASSIC_ELEMENT_REFERENCE_KEY].is_string());

    let closed_host_id = classic_find_css_element_id(app.clone(), session_id, "#closed-host").await;
    let (closed_shadow_status, closed_shadow) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{closed_host_id}/shadow"),
    )
    .await;
    assert_eq!(
        closed_shadow_status,
        StatusCode::OK,
        "closed shadow root response: {closed_shadow}"
    );
    let closed_shadow_id = closed_shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("closed shadow root reference id: {closed_shadow:?}"))
        .to_owned();
    let closed_inside = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{closed_shadow_id}/element"),
        json!({
            "using": "css selector",
            "value": "#closed-inside"
        }),
    )
    .await;
    let closed_inside_id = closed_inside["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("closed shadow child element reference id: {closed_inside:?}"));
    let closed_inside_text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{closed_inside_id}/text"),
    )
    .await;
    assert_eq!(closed_inside_text, json!({ "value": "closed text" }));

    let (outside_status, outside) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/element"),
        json!({
            "using": "css selector",
            "value": "#outside"
        }),
    )
    .await;
    assert_eq!(outside_status, StatusCode::NOT_FOUND);
    assert_eq!(outside["value"]["error"], json!("no such element"));

    let none = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/elements"),
        json!({
            "using": "css selector",
            "value": "#outside"
        }),
    )
    .await;
    assert_eq!(none, json!({ "value": [] }));

    for selector in ["#outside", "#select-no-shadow", "#video-no-shadow"] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let (no_shadow_status, no_shadow) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/shadow"),
        )
        .await;
        assert_eq!(no_shadow_status, StatusCode::NOT_FOUND, "{selector}");
        assert_eq!(
            no_shadow["value"]["error"],
            json!("no such shadow root"),
            "{selector}: {no_shadow:?}"
        );
    }

    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('#host').remove();",
            "args": []
        }),
    )
    .await;
    let (detached_status, detached) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/element"),
        json!({
            "using": "css selector",
            "value": "#inside"
        }),
    )
    .await;
    assert_eq!(detached_status, StatusCode::NOT_FOUND);
    assert_eq!(detached["value"]["error"], json!("detached shadow root"));
}

#[tokio::test]
async fn webdriver_classic_shadow_root_find_edges_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/find_element_from_shadow_root/find.py
    // and find_elements_from_shadow_root/find.py strategy, nested shadow root,
    // and implicit wait cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r##"<!doctype html>
        <div id="open-host"></div>
        <div id="closed-host"></div>
        <script>
          function buildShadow(host, mode) {
            const root = host.attachShadow({ mode });
            root.innerHTML = `
              <section>
                <a id="linkText" href="#docs">full link text</a>
                <inner-host id="inner"></inner-host>
              </section>`;
            const inner = root.querySelector("inner-host");
            inner.attachShadow({ mode }).innerHTML =
              `<a id="nestedLink" href="#nested">nested link text</a>`;
          }
          buildShadow(document.querySelector("#open-host"), "open");
          buildShadow(document.querySelector("#closed-host"), "closed");
        </script>"##;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    for host_selector in ["#open-host", "#closed-host"] {
        let host_id = classic_find_css_element_id(app.clone(), session_id, host_selector).await;
        let shadow = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{host_id}/shadow"),
        )
        .await;
        let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{host_selector} shadow id: {shadow:?}"))
            .to_owned();

        for (using, value) in [
            ("css selector", "#linkText"),
            ("link text", "full link text"),
            ("partial link text", "link text"),
            ("tag name", "a"),
            ("xpath", "//a"),
        ] {
            let found = classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/shadow/{shadow_id}/element"),
                json!({
                    "using": using,
                    "value": value
                }),
            )
            .await;
            let found_id = found["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
                .as_str()
                .unwrap_or_else(|| panic!("{host_selector} {using}={value} response: {found:?}"));
            let text = classic_request_json(
                app.clone(),
                Method::GET,
                &format!("/session/{session_id}/element/{found_id}/text"),
            )
            .await;
            assert_eq!(
                text,
                json!({ "value": "full link text" }),
                "{host_selector} {using}={value}"
            );
        }

        let partials = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/shadow/{shadow_id}/elements"),
            json!({
                "using": "partial link text",
                "value": "link text"
            }),
        )
        .await;
        assert_eq!(
            partials["value"]
                .as_array()
                .expect("partial link elements")
                .len(),
            1,
            "{host_selector} partial link elements: {partials:?}"
        );

        let inner_host = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/shadow/{shadow_id}/element"),
            json!({
                "using": "css selector",
                "value": "inner-host"
            }),
        )
        .await;
        let inner_host_id = inner_host["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{host_selector} inner host: {inner_host:?}"));
        let nested_shadow = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{inner_host_id}/shadow"),
        )
        .await;
        let nested_shadow_id = nested_shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{host_selector} nested shadow id: {nested_shadow:?}"));
        let nested = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/shadow/{nested_shadow_id}/element"),
            json!({
                "using": "css selector",
                "value": "#nestedLink"
            }),
        )
        .await;
        let nested_id = nested["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{host_selector} nested link: {nested:?}"));
        let nested_text = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{nested_id}/text"),
        )
        .await;
        assert_eq!(
            nested_text,
            json!({ "value": "nested link text" }),
            "{host_selector} nested shadow text"
        );
    }

    let timeouts = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/timeouts"),
        json!({ "implicit": 1000 }),
    )
    .await;
    assert_eq!(timeouts, json!({ "value": null }));
    let open_host_id = classic_find_css_element_id(app.clone(), session_id, "#open-host").await;
    let open_shadow = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{open_host_id}/shadow"),
    )
    .await;
    let open_shadow_id = open_shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("open shadow id for implicit wait: {open_shadow:?}"));
    let armed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "setTimeout(() => { const input = document.createElement('input'); input.id = 'delayed'; document.querySelector('#open-host').shadowRoot.appendChild(input); }, 300); return 'armed';",
            "args": []
        }),
    )
    .await;
    assert_eq!(armed, json!({ "value": "armed" }));
    let delayed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{open_shadow_id}/element"),
        json!({
            "using": "css selector",
            "value": "#delayed"
        }),
    )
    .await;
    assert!(
        delayed["value"][CLASSIC_ELEMENT_REFERENCE_KEY].is_string(),
        "implicit wait should find delayed shadow child: {delayed:?}"
    );
}

#[tokio::test]
async fn webdriver_classic_shadow_root_find_argument_edges_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/find_element_from_shadow_root/find.py
    // and find_elements_from_shadow_root/find.py request parsing and shadow-root id cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r##"<!doctype html>
        <div id="host"></div>
        <script>
          document.querySelector("#host")
            .attachShadow({ mode: "open" })
            .innerHTML = `<input id="inside" value="shadow">`;
        </script>"##;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
    let shadow = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("shadow root id for argument edges: {shadow:?}"));

    for suffix in ["element", "elements"] {
        let path = format!("/session/{session_id}/shadow/{shadow_id}/{suffix}");
        let (null_status, null_body) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &path,
            json!(null),
        )
        .await;
        assert_eq!(
            null_status,
            StatusCode::BAD_REQUEST,
            "{suffix} null body: {null_body:?}"
        );
        assert_eq!(null_body["value"]["error"], json!("invalid argument"));

        let (element_id_status, element_id_response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/shadow/{host_id}/{suffix}"),
            json!({
                "using": "css selector",
                "value": "input"
            }),
        )
        .await;
        assert_eq!(
            element_id_status,
            StatusCode::NOT_FOUND,
            "{suffix} element id as shadow root id: {element_id_response:?}"
        );
        assert_eq!(
            element_id_response["value"]["error"],
            json!("no such shadow root")
        );

        for shadow_root_id in ["foo", "true", "null", "1", "[]", "{}"] {
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/shadow/{shadow_root_id}/{suffix}"),
                json!({
                    "using": "css selector",
                    "value": "input"
                }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{suffix} invalid shadow id {shadow_root_id}: {response:?}"
            );
            assert_eq!(response["value"]["error"], json!("no such shadow root"));
        }

        for using in [
            json!("a"),
            json!(true),
            json!(null),
            json!(1),
            json!([]),
            json!({}),
        ] {
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &path,
                json!({
                    "using": using,
                    "value": "input"
                }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{suffix} invalid using: {response:?}"
            );
            assert_eq!(response["value"]["error"], json!("invalid argument"));
        }

        for value in [json!(null), json!([]), json!({})] {
            let (status, response) = classic_request_status_and_json_with_body(
                app.clone(),
                Method::POST,
                &path,
                json!({
                    "using": "css selector",
                    "value": value
                }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{suffix} invalid selector value: {response:?}"
            );
            assert_eq!(response["value"]["error"], json!("invalid argument"));
        }
    }
}

#[tokio::test]
async fn webdriver_classic_shadow_root_link_text_edges_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/find_element_from_shadow_root/find.py
    // and find_elements_from_shadow_root/find.py link text and partial link text cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    for (using, document, value) in [
        (
            "link text",
            r##"<a id="target" href="#">link text</a>"##,
            "link text",
        ),
        (
            "link text",
            r##"<a id="target" href="#">&nbsp;link text&nbsp;</a>"##,
            "link text",
        ),
        (
            "link text",
            r##"<a id="target" href="#">link<br>text</a>"##,
            "link\ntext",
        ),
        (
            "link text",
            r##"<a id="target" href="#">link&amp;text</a>"##,
            "link&text",
        ),
        (
            "link text",
            r##"<a id="target" href="#">LINK TEXT</a>"##,
            "LINK TEXT",
        ),
        (
            "link text",
            r##"<a id="target" href="#" style="text-transform: uppercase">link text</a>"##,
            "LINK TEXT",
        ),
        (
            "partial link text",
            r##"<a id="target" href="#">partial link text</a>"##,
            "link",
        ),
        (
            "partial link text",
            r##"<a id="target" href="#">&nbsp;partial link text&nbsp;</a>"##,
            "link",
        ),
        (
            "partial link text",
            r##"<a id="target" href="#">partial link text</a>"##,
            "k t",
        ),
        (
            "partial link text",
            r##"<a id="target" href="#">partial link<br>text</a>"##,
            "k\nt",
        ),
        (
            "partial link text",
            r##"<a id="target" href="#">partial link&amp;text</a>"##,
            "k&t",
        ),
        (
            "partial link text",
            r##"<a id="target" href="#">PARTIAL LINK TEXT</a>"##,
            "LINK",
        ),
        (
            "partial link text",
            r##"<a id="target" href="#" style="text-transform: uppercase">partial link text</a>"##,
            "LINK",
        ),
    ] {
        let html = format!(
            r##"<!doctype html>
            <div id="host"></div>
            <script>
              document.querySelector("#host").attachShadow({{ mode: "open" }}).innerHTML =
                `<div><a id="not-wanted" href="#">not wanted</a><br>{document}</div>`;
            </script>"##
        );
        let navigated = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/url"),
            json!({ "url": classic_data_url(&html) }),
        )
        .await;
        assert_eq!(navigated, json!({ "value": null }));

        let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
        let shadow = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{host_id}/shadow"),
        )
        .await;
        let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("shadow id for {using}={value:?}: {shadow:?}"));

        let found = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/shadow/{shadow_id}/element"),
            json!({
                "using": using,
                "value": value
            }),
        )
        .await;
        let found_id = found["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{using}={value:?} should find target: {found:?}"));
        let found_attribute = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{found_id}/attribute/id"),
        )
        .await;
        assert_eq!(
            found_attribute,
            json!({ "value": "target" }),
            "{using}={value:?} find element should return target"
        );

        let found_many = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/shadow/{shadow_id}/elements"),
            json!({
                "using": using,
                "value": value
            }),
        )
        .await;
        let found_many = found_many["value"]
            .as_array()
            .unwrap_or_else(|| panic!("{using}={value:?} should return element list"));
        assert_eq!(
            found_many.len(),
            1,
            "{using}={value:?} find elements should return one target: {found_many:?}"
        );
        let found_many_id = found_many[0][CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("{using}={value:?} element list item: {found_many:?}"));
        let found_many_attribute = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{found_many_id}/attribute/id"),
        )
        .await;
        assert_eq!(
            found_many_attribute,
            json!({ "value": "target" }),
            "{using}={value:?} find elements should return target"
        );
    }
}

#[tokio::test]
async fn webdriver_classic_child_frame_closed_shadow_root_uses_pierced_dom_snapshot() {
    // Extends Chromium WPT get_element_shadow_root/find_element_from_shadow_root
    // coverage into a selected child browsing context. Closed shadow roots are
    // not reachable through element.shadowRoot, so this must use the shared
    // DOM snapshot path instead of page-visible JavaScript.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page_url = format!("http://{fixture_addr}/shadow-page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let frame_id = classic_find_css_element_id(app.clone(), session_id, "#shadow-child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let host_id = classic_find_css_element_id(app.clone(), session_id, "#child-closed-host").await;
    let (shadow_status, shadow) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    assert_eq!(
        shadow_status,
        StatusCode::OK,
        "child-frame closed shadow root response: {shadow:?}"
    );
    let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("child-frame closed shadow root id: {shadow:?}"));

    let (closed_inside_status, closed_inside) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/element"),
        json!({
            "using": "css selector",
            "value": "#child-closed-inside"
        }),
    )
    .await;
    assert_eq!(
        closed_inside_status,
        StatusCode::OK,
        "child-frame closed shadow scoped find response: {closed_inside:?}"
    );
    let closed_inside_id = closed_inside["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("child-frame closed shadow child id: {closed_inside:?}"));
    let text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{closed_inside_id}/text"),
    )
    .await;
    assert_eq!(text, json!({ "value": "child closed text" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_shadow_root_owner_context_errors_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/get_element_shadow_root/get.py
    // and find_element(s)_from_shadow_root/find.py owner-context and stale cases.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let frame_path = format!("/session/{session_id}/frame");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();

    let html = r##"<!doctype html>
        <div id="host"></div>
        <script>
          document.querySelector("#host")
            .attachShadow({ mode: "open" })
            .innerHTML = `<input id="inside" value="shadow">`;
        </script>"##;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
    let shadow = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("top shadow root id: {shadow:?}"))
        .to_owned();

    let new_window = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/new"),
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = new_window["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let (other_window_get_shadow_status, other_window_get_shadow) =
        classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{host_id}/shadow"),
        )
        .await;
    assert_eq!(other_window_get_shadow_status, StatusCode::NOT_FOUND);
    assert_eq!(
        other_window_get_shadow["value"]["error"],
        json!("no such element")
    );

    for path in [
        format!("/session/{session_id}/shadow/{shadow_id}/element"),
        format!("/session/{session_id}/shadow/{shadow_id}/elements"),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &path,
            json!({
                "using": "css selector",
                "value": "input"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {response:?}");
        assert_eq!(response["value"]["error"], json!("no such shadow root"));
    }

    let switched_back = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": original_handle }),
    )
    .await;
    assert_eq!(switched_back, json!({ "value": null }));
    let removed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('#host').remove(); return 'removed';",
            "args": []
        }),
    )
    .await;
    assert_eq!(removed, json!({ "value": "removed" }));
    let (stale_host_status, stale_host) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    assert_eq!(
        stale_host_status,
        StatusCode::NOT_FOUND,
        "stale host shadow response: {stale_host:?}"
    );
    assert_eq!(
        stale_host["value"]["error"],
        json!("stale element reference")
    );

    let page_url = format!("http://{fixture_addr}/shadow-page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let frame_id = classic_find_css_element_id(app.clone(), session_id, "#shadow-child").await;
    let switched_frame = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched_frame, json!({ "value": null }));

    let child_host_id =
        classic_find_css_element_id(app.clone(), session_id, "#child-closed-host").await;
    let child_shadow = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{child_host_id}/shadow"),
    )
    .await;
    let child_shadow_id = child_shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("child shadow root id: {child_shadow:?}"))
        .to_owned();

    let parent = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame/parent"),
    )
    .await;
    assert_eq!(parent, json!({ "value": null }));

    let (other_frame_get_shadow_status, other_frame_get_shadow) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{child_host_id}/shadow"),
    )
    .await;
    assert_eq!(other_frame_get_shadow_status, StatusCode::NOT_FOUND);
    assert_eq!(
        other_frame_get_shadow["value"]["error"],
        json!("no such element")
    );

    for path in [
        format!("/session/{session_id}/shadow/{child_shadow_id}/element"),
        format!("/session/{session_id}/shadow/{child_shadow_id}/elements"),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &path,
            json!({
                "using": "css selector",
                "value": "#child-closed-inside"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {response:?}");
        assert_eq!(response["value"]["error"], json!("no such shadow root"));
    }

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_screenshot_reports_unsupported_without_placeholder_payload() {
    // Ported from Selenium py/test/selenium/webdriver/common/takes_screenshots_tests.py:
    // test_get_screenshot_as_base64, test_get_screenshot_as_png and
    // test_get_element_screenshot.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r#"<!doctype html>
        <main>
            <p id="multiline">line one<br>line two</p>
        </main>"#;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": classic_data_url(html)
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (page_status, page_screenshot) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/screenshot"),
    )
    .await;
    assert_eq!(page_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        page_screenshot["value"]["error"],
        json!("unsupported operation")
    );
    assert_eq!(
        page_screenshot["value"]["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#multiline"
        }),
    )
    .await;
    let element_id = element["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("element lookup should return a classic element: {element:?}"));

    let (element_status, element_screenshot) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/screenshot"),
    )
    .await;
    assert_eq!(element_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        element_screenshot["value"]["error"],
        json!("unsupported operation")
    );
    assert_eq!(
        element_screenshot["value"]["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let (missing_status, missing) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/moli-node-999999/screenshot"),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(missing["value"]["error"], json!("no such element"));
}

#[tokio::test]
async fn webdriver_classic_print_reports_unsupported_without_placeholder_pdf() {
    // Ported from Selenium py/test/selenium/webdriver/common/print_pdf_tests.py:
    // test_pdf_with_all_pages, test_pdf_with_2_pages and test_valid_params.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r#"<!doctype html>
        <style>
            body { margin: 0; font: 16px sans-serif; }
            .page { page-break-after: always; min-height: 100vh; }
        </style>
        <section class="page">page one</section>
        <section>page two</section>"#;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": classic_data_url(html)
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (all_pages_status, all_pages) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/print"),
        json!({}),
    )
    .await;
    assert_eq!(all_pages_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(all_pages["value"]["error"], json!("unsupported operation"));
    assert_eq!(
        all_pages["value"]["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let (two_pages_status, two_pages) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/print"),
        json!({
            "pageRanges": ["1-2"]
        }),
    )
    .await;
    assert_eq!(two_pages_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(two_pages["value"]["error"], json!("unsupported operation"));
    assert_eq!(
        two_pages["value"]["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let (valid_status, valid_params) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/print"),
        json!({
            "orientation": "landscape",
            "scale": 1.0,
            "background": true,
            "shrinkToFit": true,
            "pageRanges": ["1-2"],
            "page": {
                "width": 30.0,
                "height": 29.7
            },
            "margin": {
                "top": 0.0,
                "bottom": 0.0,
                "left": 0.0,
                "right": 0.0
            }
        }),
    )
    .await;
    assert_eq!(valid_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        valid_params["value"]["error"],
        json!("unsupported operation")
    );
    assert_eq!(
        valid_params["value"]["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    for body in [
        json!({"orientation": "sideways"}),
        json!({"scale": 3.0}),
        json!({"pageRanges": ["3-2"]}),
    ] {
        let (status, invalid) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/print"),
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body should fail: {body}");
        assert_eq!(invalid["value"]["error"], json!("invalid argument"));
    }
}

#[tokio::test]
async fn webdriver_classic_displayed_cases_ported_from_selenium() {
    // Ported from Selenium py/test/selenium/webdriver/common/visibility_tests.py
    // baseline visible/display:none/hidden/ancestor-hidden cases. Moli
    // intentionally keeps this on its deterministic mock geometry rather than
    // claiming full Chromium paint/layout visibility.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let (invalid_status, invalid) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/foo/displayed"),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::NOT_FOUND);
    assert_eq!(invalid["value"]["error"], json!("no such element"));

    let html = concat!(
        "<main id=displayed>Displayed</main>",
        "<p id=none style='display:none'>none</p>",
        "<p id=hidden hidden>hidden</p>",
        "<p id=visibility style='visibility:hidden'>hidden</p>",
        "<input id=hiddenInput type=hidden value=secret>",
        "<section id=suppressed style='display:none'><a id=suppressedLink href='#'>link</a></section>",
        "<iframe id=child srcdoc=\"<main id='inside'>child</main><p id='insideNone' style='display:none'>x</p>\"></iframe>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": classic_data_url(html)
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    for (selector, expected) in [
        ("#displayed", true),
        ("#none", false),
        ("#hidden", false),
        ("#visibility", false),
        ("#hiddenInput", false),
        ("#suppressedLink", false),
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/displayed"),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{selector}");
    }

    let frame_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                "element-6066-11e4-a52e-4f735466cecf": frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    for (selector, expected) in [("#inside", true), ("#insideNone", false)] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/displayed"),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{selector}");
    }
}

#[tokio::test]
async fn webdriver_classic_get_element_text_cases_ported_from_selenium() {
    // Reduced from Selenium py/test/selenium/webdriver/common/text_handling_tests.py.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r#"
        <p id="oneline">A single line of text</p>
        <p id="hiddenline" style="visibility: hidden">A hidden line of text</p>
        <div id="multiline">
          <p>A div containing</p>
          More than one line of text<br>
          <div>and block level elements</div>
        </div>
        <span id="span">An inline element</span>
        <p id="lotsofspaces">This line has lots

            of spaces.
        </p>
        <p id="nbsp">This line has a&nbsp;non-breaking space</p>
        <p id="nbspandspaces">This line has a &nbsp; non-breaking space and spaces</p>
        <p id="privateuse">&#xE000; private use text</p>
        <p id="inline">This <span id="inlinespan">    line has <em>text</em>	</span> within elements that are meant to be displayed inline</p>
        <div id="twoblocks"><p>Some text</p><p>Some more text</p></div>
        <label id="labelforusername" for="username">
          Username: <input id="username" type="text" name="username">
          <script>document.getElementById('username').value = 'Michael';</script>
        </label>
        <div id="visible-wrapper">visible <span style="display: none">hidden</span><span>text</span></div>
        <div id="capitalize-space" style="text-transform: capitalize">foo bar</div>
        <div id="capitalize-dash" style="text-transform: capitalize">foo-bar</div>
        <div id="capitalize-underscore" style="text-transform: capitalize">foo_bar</div>
        <div id="capitalize-accent" style="text-transform: capitalize">foo b&aacute;r</div>
        <div id="slot-custom-visible">cheese</div>
        <div id="slot-custom-outside">cheese</div>
        <div id="slot-custom-hidden">cheese</div>
        <div id="slot-default-visible"></div>
        <div id="slot-default-outside"></div>
        <div id="slot-default-hidden"></div>
        <script>
          function setShadow(id, innerHTML) {
            document.getElementById(id).attachShadow({ mode: "open" }).innerHTML = innerHTML;
          }
          setShadow("slot-custom-visible", "<slot><span>foo</span>bar</slot>");
          setShadow("slot-custom-outside", "<slot><span>foo</span></slot>bar");
          setShadow("slot-custom-hidden", "<slot><span style='display: none'>foo</span>bar</slot>");
          setShadow("slot-default-visible", "<slot><span>foo</span>bar</slot>");
          setShadow("slot-default-outside", "<slot><span>foo</span></slot>bar");
          setShadow("slot-default-hidden", "<slot><span style='display: none'>foo</span>bar</slot>");
        </script>
        <div id="empty"></div>
        <p id="spaces">    </p>
    "#;
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/url"),
            json!({ "url": classic_data_url(html) }),
        )
        .await,
        json!({ "value": null })
    );

    for (selector, expected) in [
        ("#oneline", "A single line of text"),
        (
            "#multiline",
            "A div containing\nMore than one line of text\nand block level elements",
        ),
        ("#lotsofspaces", "This line has lots of spaces."),
        ("#nbsp", "This line has a non-breaking space"),
        (
            "#nbspandspaces",
            "This line has a   non-breaking space and spaces",
        ),
        ("#privateuse", "\u{E000} private use text"),
        (
            "#inline",
            "This line has text within elements that are meant to be displayed inline",
        ),
        ("#inlinespan", "line has text"),
        ("#span", "An inline element"),
        ("#twoblocks", "Some text\nSome more text"),
        ("#labelforusername", "Username:"),
        ("#visible-wrapper", "visible text"),
        ("#capitalize-space", "Foo Bar"),
        ("#capitalize-dash", "Foo-Bar"),
        ("#capitalize-underscore", "Foo_bar"),
        ("#capitalize-accent", "Foo B\u{00e1}r"),
        ("#slot-custom-visible", "cheese"),
        ("#slot-custom-outside", "cheesebar"),
        ("#slot-custom-hidden", "cheese"),
        ("#slot-default-visible", "foobar"),
        ("#slot-default-outside", "foobar"),
        ("#slot-default-hidden", "bar"),
        ("#hiddenline", ""),
        ("#empty", ""),
        ("#spaces", ""),
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let response = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/text"),
        )
        .await;
        assert_eq!(response, json!({ "value": expected }), "{selector}");
    }
}

#[tokio::test]
async fn webdriver_classic_get_element_text_rejects_closed_window_element_after_close_switches() {
    // Ported from WPT webdriver/tests/classic/get_element_text/get.py
    // test_no_top_browsing_context, adapted to Moli's selected window
    // recovery after Close Window leaves another top-level context open.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/new"),
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle.clone() }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<input id='a' value='b'>") }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let element_id = classic_find_css_element_id(app.clone(), session_id, "input").await;

    let remaining = classic_request_json(app.clone(), Method::DELETE, &window_path).await;
    assert_eq!(remaining, json!({ "value": [original_handle.clone()] }));
    assert_eq!(
        classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/window/handles"),
        )
        .await,
        json!({ "value": [original_handle.clone()] })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &window_path).await,
        json!({ "value": original_handle.clone() })
    );

    for element_id in [element_id.as_str(), "foo"] {
        let (status, response) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/text"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{element_id}: {response:?}");
        assert_eq!(
            response["value"]["error"],
            json!("no such element"),
            "{element_id}: {response:?}"
        );
    }

    let switched_back = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": original_handle }),
    )
    .await;
    assert_eq!(switched_back, json!({ "value": null }));

    let (status, response) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/text"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(response["value"]["error"], json!("no such element"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_clear_element_cases_ported_from_selenium() {
    // Ported from Selenium py/test/selenium/webdriver/common/clear_tests.py
    // and common/src/web/readOnlyPage.html.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = concat!(
        "<input id='writableTextInput' type='text' value='Test'>",
        "<input id='readOnlyTextInput' type='text' readonly value='Test'>",
        "<input id='textInputNotEnabled' type='text' disabled value='Test'>",
        "<textarea id='writableTextArea'>This is a sample text area which is supposed to be cleared</textarea>",
        "<textarea id='textAreaReadOnly' readonly>text area which is not supposed to be cleared</textarea>",
        "<textarea id='textAreaNotEnabled' disabled>text area which is not supposed to be cleared</textarea>",
        "<div id='content-editable' contenteditable='true'><h1>This</h1><h2>is a</h2><p>contentEditable area</p></div>",
        "<button id='not-clearable'>button</button>",
        "<script>",
        "window.__clearEvents=[];",
        "for (const id of ['writableTextInput','writableTextArea']) {",
        "  const element = document.getElementById(id);",
        "  for (const type of ['input','change']) {",
        "    element.addEventListener(type, event => window.__clearEvents.push(`${id}:${event.type}:${event.composed}:${element.value}`));",
        "  }",
        "}",
        "</script>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({
            "url": format!("data:text/html,{html}")
        }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    for selector in ["#writableTextInput", "#writableTextArea"] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let cleared = classic_request_json(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/clear"),
        )
        .await;
        assert_eq!(cleared, json!({ "value": null }), "{selector}");

        let value = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/property/value"),
        )
        .await;
        assert_eq!(value, json!({ "value": "" }), "{selector}");
    }
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return window.__clearEvents.join('|');",
                "args": []
            }),
        )
        .await,
        json!({ "value": "writableTextInput:input:true:|writableTextInput:change:false:|writableTextArea:input:true:|writableTextArea:change:false:" })
    );

    let editable_id =
        classic_find_css_element_id(app.clone(), session_id, "#content-editable").await;
    let cleared = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{editable_id}/clear"),
    )
    .await;
    assert_eq!(cleared, json!({ "value": null }));
    let editable_text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{editable_id}/text"),
    )
    .await;
    assert_eq!(editable_text, json!({ "value": "" }));

    for selector in [
        "#readOnlyTextInput",
        "#textInputNotEnabled",
        "#textAreaReadOnly",
        "#textAreaNotEnabled",
        "#not-clearable",
    ] {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let (status, response) = classic_request_status_and_json(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/clear"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{selector}");
        assert_eq!(
            response["value"]["error"],
            json!("invalid element state"),
            "{selector}"
        );
    }
}

#[tokio::test]
async fn webdriver_classic_clear_disabled_form_cases_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/element_clear/disabled.py
    // and element_clear/clear.py non-editable input cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r#"<!doctype html>
        <input id="enabled" value="enabled">
        <fieldset disabled>
          <input id="fieldsetChild" value="blocked">
          <legend><input id="firstLegendInput" value="allowed"></legend>
          <legend><input id="secondLegendInput" value="blocked"></legend>
        </fieldset>
        <select id="disabledSelect" disabled><option id="selectOption">select</option></select>
        <select>
          <option id="disabledOption" disabled>option</option>
          <optgroup id="disabledOptgroup" disabled><option id="optgroupOption">group</option></optgroup>
        </select>
        <input id="checkbox" type="checkbox">
        <input id="hiddenInput" type="hidden" value="hidden">
    "#;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    async fn clear_element(
        app: Router,
        session_id: &str,
        selector: &str,
    ) -> (StatusCode, serde_json::Value) {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_status_and_json(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/clear"),
        )
        .await
    }

    async fn element_value(app: Router, session_id: &str, selector: &str) -> serde_json::Value {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_json(
            app,
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/property/value"),
        )
        .await
    }

    let (enabled_status, enabled_response) =
        clear_element(app.clone(), session_id, "#enabled").await;
    assert_eq!(enabled_status, StatusCode::OK, "{enabled_response:?}");
    assert_eq!(enabled_response, json!({ "value": null }));
    assert_eq!(
        element_value(app.clone(), session_id, "#enabled").await,
        json!({ "value": "" })
    );

    let (legend_status, legend_response) =
        clear_element(app.clone(), session_id, "#firstLegendInput").await;
    assert_eq!(legend_status, StatusCode::OK, "{legend_response:?}");
    assert_eq!(legend_response, json!({ "value": null }));
    assert_eq!(
        element_value(app.clone(), session_id, "#firstLegendInput").await,
        json!({ "value": "" })
    );

    for selector in [
        "#fieldsetChild",
        "#secondLegendInput",
        "#disabledSelect",
        "#selectOption",
        "#disabledOption",
        "#disabledOptgroup",
        "#optgroupOption",
        "#checkbox",
        "#hiddenInput",
    ] {
        let (status, response) = clear_element(app.clone(), session_id, selector).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{selector}: {response:?}");
        assert_eq!(
            response["value"]["error"],
            json!("invalid element state"),
            "{selector}: {response:?}"
        );
    }
}

#[tokio::test]
async fn webdriver_classic_active_element_uses_current_browsing_context() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = "data:text/html,<body id='top-body'><input id='top-input'><iframe id='child' srcdoc=\"<body id='child-body'><input id='child-input' autofocus></body>\"></iframe></body>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (active_status, active) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/active"),
    )
    .await;
    assert_eq!(active_status, StatusCode::OK, "{active:?}");
    let active_id = active["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("active element should return body: {active:?}"));
    let active_tag = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{active_id}/name"),
    )
    .await;
    assert_eq!(active_tag, json!({ "value": "body" }));

    let focused = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.getElementById('top-input').focus(); return document.activeElement.id;",
            "args": []
        }),
    )
    .await;
    assert_eq!(focused, json!({ "value": "top-input" }));

    let active = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/active/"),
    )
    .await;
    let active_id = active["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("focused input should be active: {active:?}"));
    let active_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{active_id}/property/id"),
    )
    .await;
    assert_eq!(active_property, json!({ "value": "top-input" }));

    let frame_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                "element-6066-11e4-a52e-4f735466cecf": frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let child_focused = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.getElementById('child-input').focus(); return document.activeElement.id;",
            "args": []
        }),
    )
    .await;
    assert_eq!(child_focused, json!({ "value": "child-input" }));

    let (active_status, active) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/active"),
    )
    .await;
    assert_eq!(active_status, StatusCode::OK, "{active:?}");
    let active_id = active["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("child frame active element should return input: {active:?}"));
    let active_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{active_id}/property/id"),
    )
    .await;
    assert_eq!(active_property, json!({ "value": "child-input" }));
}

#[tokio::test]
async fn webdriver_classic_css_value_uses_current_browsing_context() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = "data:text/html,<body><main id='top' style='display:flex;width:123px'></main><iframe id='child' srcdoc=\"<main id='inside' style='display:grid;width:321px'></main>\"></iframe></body>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let top_id = classic_find_css_element_id(app.clone(), session_id, "#top").await;
    let top_display = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{top_id}/css/display"),
    )
    .await;
    assert_eq!(top_display, json!({ "value": "flex" }));
    let top_width = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{top_id}/css/width/"),
    )
    .await;
    assert_eq!(top_width, json!({ "value": "123px" }));
    let unknown = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{top_id}/css/not-a-property"),
    )
    .await;
    assert_eq!(unknown, json!({ "value": "" }));

    let frame_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                "element-6066-11e4-a52e-4f735466cecf": frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let child_id = classic_find_css_element_id(app.clone(), session_id, "#inside").await;
    let child_display = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{child_id}/css/display"),
    )
    .await;
    assert_eq!(child_display, json!({ "value": "grid" }));
    let child_width = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{child_id}/css/width"),
    )
    .await;
    assert_eq!(child_width, json!({ "value": "321px" }));
}

#[tokio::test]
async fn webdriver_classic_file_navigation_returns_stable_unknown_error_without_replacement() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let (status, headers, rejected) = classic_request_status_headers_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "file:///moli-policy-must-not-open" }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_classic_webdriver_json_headers(&headers);
    assert_eq!(
        rejected,
        json!({
            "value": {
                "error": "unknown error",
                "message": "Navigation to a local file URL requires an explicitly granted browser capability.",
                "stacktrace": "",
            }
        })
    );

    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": "about:blank" }));

    let deleted =
        classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
    assert_eq!(deleted, json!({ "value": null }));
}

#[tokio::test]
async fn webdriver_classic_url_routes_execute_through_devtools_runtime() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let first_url = "data:text/html,classic-first";
    let navigate_url = "data:text/html,<title>ClassicTitle</title><script>window.__classicClicked=0;document.addEventListener('click',function(){window.__classicClicked+=1;});</script><button id='action'>go</button><main id='source' name='sourceName' class='primary item' data-kind='primary'>classic-source</main><section id='child-root'><a id='child-link' name='childLink' href='child.html'>Child Link</a><span class='child'>nested</span></section><a id='top-link' href='top.html'>Top Link</a><input id='field' name='fieldName' value='classic-value'>";

    let first_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": first_url }),
    )
    .await;
    assert_eq!(first_navigated, json!({ "value": null }));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": navigate_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": navigate_url }));

    let title = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/title"),
    )
    .await;
    assert_eq!(title, json!({ "value": "ClassicTitle" }));

    let source = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/source"),
    )
    .await;
    let source = source["value"].as_str().expect("page source string");
    assert!(source.contains("<title>ClassicTitle</title>"));
    assert!(source.contains("classic-source"));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#source"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");
    assert!(
        element_id.starts_with("moli-node-"),
        "unexpected element id: {element_id}"
    );

    for (using, value) in [
        ("id", "source"),
        ("name", "sourceName"),
        ("class name", "primary"),
        ("tag name", "main"),
        ("link text", "Top Link"),
        ("partial link text", "Top"),
    ] {
        let located = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/element"),
            json!({
                "using": using,
                "value": value
            }),
        )
        .await;
        let located_id = located["value"]["element-6066-11e4-a52e-4f735466cecf"]
            .as_str()
            .unwrap_or_else(|| panic!("{using} locator should return an element: {located:?}"));
        assert!(
            located_id.starts_with("moli-node-"),
            "{using} locator returned unexpected element id: {located_id}"
        );
    }

    let named_field = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "name",
            "value": "fieldName"
        }),
    )
    .await;
    let named_field_id = named_field["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("name locator should find input element");
    let named_field_value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{named_field_id}/property/value"),
    )
    .await;
    assert_eq!(named_field_value, json!({ "value": "classic-value" }));

    let (compound_class_status, compound_class) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "class name",
            "value": "primary item"
        }),
    )
    .await;
    assert_eq!(compound_class_status, StatusCode::BAD_REQUEST);
    assert_eq!(compound_class["value"]["error"], json!("invalid selector"));

    let text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/text"),
    )
    .await;
    assert_eq!(text, json!({ "value": "classic-source" }));

    let id_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/property/id"),
    )
    .await;
    assert_eq!(id_property, json!({ "value": "source" }));

    let missing_property = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/property/doesNotExist"),
    )
    .await;
    assert_eq!(missing_property, json!({ "value": null }));

    let attribute = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/attribute/data-kind"),
    )
    .await;
    assert_eq!(attribute, json!({ "value": "primary" }));

    let missing_attribute = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/attribute/data-missing"),
    )
    .await;
    assert_eq!(missing_attribute, json!({ "value": null }));

    let (invalid_element_status, invalid_element) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/not-a-node/attribute/data-kind"),
    )
    .await;
    assert_eq!(invalid_element_status, StatusCode::NOT_FOUND);
    assert_eq!(invalid_element["value"]["error"], json!("no such element"));

    let field = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#field"
        }),
    )
    .await;
    let field_id = field["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("field element reference id");
    let field_value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{field_id}/property/value"),
    )
    .await;
    assert_eq!(field_value, json!({ "value": "classic-value" }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 10 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));
    let clicked = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicClicked || 0;",
            "args": []
        }),
    )
    .await;
    assert_eq!(clicked, json!({ "value": 1 }));
    let released = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/actions"),
    )
    .await;
    assert_eq!(released, json!({ "value": null }));

    let elements = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "css selector",
            "value": "main"
        }),
    )
    .await;
    let elements = elements["value"].as_array().expect("elements array");
    assert_eq!(elements.len(), 1);
    assert!(
        elements[0]["element-6066-11e4-a52e-4f735466cecf"]
            .as_str()
            .is_some()
    );

    let missing_elements = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/elements"),
        json!({
            "using": "css selector",
            "value": ".missing"
        }),
    )
    .await;
    assert_eq!(missing_elements, json!({ "value": [] }));

    let (missing_element_status, missing_element) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": ".missing"
        }),
    )
    .await;
    assert_eq!(missing_element_status, StatusCode::NOT_FOUND);
    assert_eq!(missing_element["value"]["error"], json!("no such element"));

    let child_root = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#child-root"
        }),
    )
    .await;
    let child_root_id = child_root["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("child root locator should return an element: {child_root:?}"));

    let child = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{child_root_id}/element"),
        json!({
            "using": "css selector",
            "value": ".child"
        }),
    )
    .await;
    let child_id = child["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("child CSS locator should return an element: {child:?}"));
    let child_text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{child_id}/text"),
    )
    .await;
    assert_eq!(child_text, json!({ "value": "nested" }));

    let child_links = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{child_root_id}/elements"),
        json!({
            "using": "link text",
            "value": "Child Link"
        }),
    )
    .await;
    let child_links = child_links["value"].as_array().expect("child links array");
    assert_eq!(child_links.len(), 1);

    let root_not_returned = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{child_root_id}/elements"),
        json!({
            "using": "id",
            "value": "child-root"
        }),
    )
    .await;
    assert_eq!(root_not_returned, json!({ "value": [] }));

    let (missing_child_status, missing_child) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{child_root_id}/element"),
        json!({
            "using": "partial link text",
            "value": "Top"
        }),
    )
    .await;
    assert_eq!(missing_child_status, StatusCode::NOT_FOUND);
    assert_eq!(missing_child["value"]["error"], json!("no such element"));

    let back = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/back"),
    )
    .await;
    assert_eq!(back, json!({ "value": null }));
    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": first_url }));

    let forward = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/forward"),
    )
    .await;
    assert_eq!(forward, json!({ "value": null }));
    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": navigate_url }));

    let refresh = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/refresh"),
    )
    .await;
    assert_eq!(refresh, json!({ "value": null }));
    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": navigate_url }));

    let execute = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return arguments[0].nested + arguments[1];",
            "args": [
                { "nested": 4 },
                3
            ]
        }),
    )
    .await;
    assert_eq!(execute, json!({ "value": 7 }));

    let execute_async = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/async"),
        json!({
            "script": "arguments[arguments.length - 1]({ asyncValue: arguments[0] + arguments[1] });",
            "args": [
                5,
                6
            ]
        }),
    )
    .await;
    assert_eq!(execute_async, json!({ "value": { "asyncValue": 11 } }));

    let (execute_invalid_status, execute_invalid) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return 1;",
            "args": false
        }),
    )
    .await;
    assert_eq!(execute_invalid_status, StatusCode::BAD_REQUEST);
    assert_eq!(execute_invalid["value"]["error"], json!("invalid argument"));

    let (execute_async_invalid_status, execute_async_invalid) =
        classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": "arguments[arguments.length - 1](1);",
                "args": false
            }),
        )
        .await;
    assert_eq!(execute_async_invalid_status, StatusCode::BAD_REQUEST);
    assert_eq!(
        execute_async_invalid["value"]["error"],
        json!("invalid argument")
    );

    let (execute_throw_status, execute_throw) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "throw new Error('classic boom');",
            "args": []
        }),
    )
    .await;
    assert_eq!(execute_throw_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(execute_throw["value"]["error"], json!("javascript error"));

    for message in [
        "stale element reference",
        "detached shadow root",
        "no such frame",
        "no such frame is acceptable here",
    ] {
        let script = format!(
            "throw new Error({});",
            serde_json::to_string(message).expect("serialize test error message")
        );
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response["value"]["error"],
            json!("javascript error"),
            "user JavaScript exception should not be reclassified: {response:?}"
        );
    }

    let (execute_async_throw_status, execute_async_throw) =
        classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": "throw new Error('classic async boom');",
                "args": []
            }),
        )
        .await;
    assert_eq!(
        execute_async_throw_status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        execute_async_throw["value"]["error"],
        json!("javascript error")
    );

    let (execute_async_contains_status, execute_async_contains) =
        classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/async"),
            json!({
                "script": "throw new Error('no such frame is acceptable here');",
                "args": []
            }),
        )
        .await;
    assert_eq!(
        execute_async_contains_status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        execute_async_contains["value"]["error"],
        json!("javascript error")
    );

    let (invalid_status, invalid) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": false }),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid["value"]["error"], json!("invalid argument"));

    let (malformed_status, malformed) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "foo" }),
    )
    .await;
    assert_eq!(malformed_status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed["value"]["error"], json!("invalid argument"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_history_traversal_preserves_live_same_document_and_falls_back_after_restore()
 {
    async fn page() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><main id='kept'>same-document</main>",
        )
    }

    async fn other() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><main>other-document</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Classic same-document history fixture listener");
    let fixture_addr = listener
        .local_addr()
        .expect("Classic same-document history fixture addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/other", get(other)),
        )
        .await
        .expect("serve Classic history fixture");
    });
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = format!("http://{fixture_addr}/page");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#kept"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");

    let pushed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "
                window.__sameDocumentRealmMarker = { kept: true };
                history.pushState(null, '', '#first');
                history.pushState(null, '', '#second');
                return location.hash;
            ",
            "args": []
        }),
    )
    .await;
    assert_eq!(pushed, json!({ "value": "#second" }));

    let back = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/back"),
    )
    .await;
    assert_eq!(back, json!({ "value": null }));

    let realm_preserved = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__sameDocumentRealmMarker?.kept === true && location.hash === '#first';",
            "args": []
        }),
    )
    .await;
    assert_eq!(realm_preserved, json!({ "value": true }));

    let text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/text"),
    )
    .await;
    assert_eq!(text, json!({ "value": "same-document" }));

    let prepared_restore_chain = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "
                history.replaceState(null, '', '?step=one');
                history.pushState(null, '', '?step=two');
                return location.search;
            ",
            "args": []
        }),
    )
    .await;
    assert_eq!(prepared_restore_chain, json!({ "value": "?step=two" }));

    let other_url = format!("http://{fixture_addr}/other");
    let navigated_other = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": other_url }),
    )
    .await;
    assert_eq!(navigated_other, json!({ "value": null }));

    let restored = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/back"),
    )
    .await;
    assert_eq!(restored, json!({ "value": null }));

    let restored_element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#kept"
        }),
    )
    .await;
    let restored_element_id = restored_element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("restored element reference id");
    let marked_restored_realm = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "window.__restoredRealmMarker = true; return location.search;",
            "args": []
        }),
    )
    .await;
    assert_eq!(marked_restored_realm, json!({ "value": "?step=two" }));

    let fallback_back = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/back"),
    )
    .await;
    assert_eq!(fallback_back, json!({ "value": null }));

    let fallback_state = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return { search: location.search, marker: window.__restoredRealmMarker === true };",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        fallback_state,
        json!({
            "value": {
                "search": "?step=one",
                "marker": false,
            }
        })
    );

    let (stale_status, stale) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{restored_element_id}/text"),
    )
    .await;
    assert_eq!(stale_status, StatusCode::NOT_FOUND);
    assert_eq!(stale["value"]["error"], json!("stale element reference"));

    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_document_routes_match_wpt_basic_payload_semantics() {
    // Ported from Chromium/WPT webdriver/tests/classic/get_current_url/get.py,
    // get_title/get.py, get_page_source/source.py, get_window_handle/get.py, and
    // get_window_handles/get.py.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let initial_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert!(
        initial_url["value"].as_str().is_some(),
        "current URL payload should be a string: {initial_url:?}"
    );

    let (initial_title_status, initial_title) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/title"),
    )
    .await;
    assert_eq!(
        initial_title_status,
        StatusCode::OK,
        "initial title status: {initial_title:?}"
    );
    assert!(
        initial_title["value"].as_str().is_some(),
        "title payload should be a string: {initial_title:?}"
    );

    let current_window = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    let initial_handle = current_window["value"]
        .as_str()
        .expect("initial window handle")
        .to_owned();
    assert!(!initial_handle.is_empty());

    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    assert_eq!(handles["value"], json!([initial_handle.clone()]));

    for (html, expected_title) in [
        (
            "<title>First</title><title>Second</title><main>duplicated</main>",
            "First",
        ),
        ("<h2>Hello</h2>", ""),
        (
            "<title>   a b\tc\nd\t \n e\t\n </title><h2>Hello</h2>",
            "a b c d e",
        ),
        (
            "<title>&reg; &copy; &cent; &pound; &yen;</title>",
            "® © ¢ £ ¥",
        ),
        ("<title>日本語</title>", "日本語"),
    ] {
        let url = classic_data_url(html);
        let navigated = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/url"),
            json!({ "url": url }),
        )
        .await;
        assert_eq!(navigated, json!({ "value": null }));

        let (title_status, title) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/title"),
        )
        .await;
        assert_eq!(
            title_status,
            StatusCode::OK,
            "title status for {html}: {title:?}"
        );
        assert_eq!(
            title,
            json!({ "value": expected_title }),
            "title for {html}"
        );
    }

    let source_url = classic_data_url("<html><head><title>Cheese</title><body>Peas");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": source_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let expected_source = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.documentElement.outerHTML",
            "args": []
        }),
    )
    .await;
    let page_source = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/source"),
    )
    .await;
    assert_eq!(page_source, expected_source);

    let hash_doc = format!("{}#foo", classic_data_url("<p>frame</p>"));
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": hash_doc }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": hash_doc }));

    let new_window = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/new"),
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = new_window["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();
    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    let handles = handles["value"].as_array().expect("window handles");
    assert_eq!(handles.len(), 2);
    assert!(handles.contains(&json!(initial_handle)));
    assert!(handles.contains(&json!(new_handle)));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_frame_switching_tracks_current_browsing_context() {
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page_url = format!("http://{fixture_addr}/page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let top_marker = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.body.dataset.context",
            "args": []
        }),
    )
    .await;
    assert_eq!(top_marker, json!({ "value": "top" }));

    let frame_element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#child"
        }),
    )
    .await;
    let frame_element_id = frame_element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("frame element reference: {frame_element:?}"));

    let (switched_status, switched) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                "element-6066-11e4-a52e-4f735466cecf": frame_element_id
            }
        }),
    )
    .await;
    assert_eq!(switched_status, StatusCode::OK, "{switched:?}");
    assert_eq!(switched, json!({ "value": null }));

    let child_marker = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.body.dataset.context",
            "args": []
        }),
    )
    .await;
    assert_eq!(child_marker, json!({ "value": "child" }));

    let child_element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#inside-frame"
        }),
    )
    .await;
    assert!(
        child_element["value"]["element-6066-11e4-a52e-4f735466cecf"]
            .as_str()
            .is_some(),
        "find element should use current frame: {child_element:?}"
    );

    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": page_url }));

    let parent = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame/parent"),
    )
    .await;
    assert_eq!(parent, json!({ "value": null }));

    let back_to_top = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.body.dataset.context",
            "args": []
        }),
    )
    .await;
    assert_eq!(back_to_top, json!({ "value": "top" }));

    let index_switch = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({ "id": 0 }),
    )
    .await;
    assert_eq!(index_switch, json!({ "value": null }));

    let index_child_marker = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.body.dataset.context",
            "args": []
        }),
    )
    .await;
    assert_eq!(index_child_marker, json!({ "value": "child" }));

    let default_content = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({ "id": null }),
    )
    .await;
    assert_eq!(default_content, json!({ "value": null }));

    let top_main = classic_find_css_element_id(app.clone(), session_id, "#top-main").await;
    let (non_frame_status, non_frame) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                "element-6066-11e4-a52e-4f735466cecf": top_main
            }
        }),
    )
    .await;
    assert_eq!(non_frame_status, StatusCode::NOT_FOUND);
    assert_eq!(non_frame["value"]["error"], json!("no such frame"));

    let (missing_status, missing) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({ "id": 99 }),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(missing["value"]["error"], json!("no such frame"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_switch_to_parent_frame_cases_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_parent_frame/
    // switch.py test_null_response_value, test_switch_from_iframe, and
    // test_switch_from_top_level.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let frame_path = format!("/session/{session_id}/frame");
    let parent_frame_path = format!("/session/{session_id}/frame/parent");

    let page_url = format!("http://{fixture_addr}/page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let top_main_id = classic_find_css_element_id(app.clone(), session_id, "#top-main").await;
    let parent_from_top = classic_request_json(app.clone(), Method::POST, &parent_frame_path).await;
    assert_eq!(parent_from_top, json!({ "value": null }));
    let top_main_text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{top_main_id}/text"),
    )
    .await;
    assert_eq!(top_main_text, json!({ "value": "top" }));

    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id,
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    let child_element_id =
        classic_find_css_element_id(app.clone(), session_id, "#inside-frame").await;

    let parent_from_child =
        classic_request_json(app.clone(), Method::POST, &parent_frame_path).await;
    assert_eq!(parent_from_child, json!({ "value": null }));

    let (child_text_status, child_text) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{child_element_id}/text"),
    )
    .await;
    assert_eq!(child_text_status, StatusCode::NOT_FOUND);
    assert_eq!(child_text["value"]["error"], json!("no such element"));
    let top_main_after_parent =
        classic_find_css_element_id(app.clone(), session_id, "#top-main").await;
    assert!(!top_main_after_parent.is_empty());

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_parent_frame_restores_top_level_shadow_root_lookup() {
    // Selenium's ShadowRoot client path first gets the shadow root reference,
    // then runs a shadow-scoped find. After switching into a child frame and
    // back to parent, parent must be represented as top-level, not as a child
    // frame id equal to the top-level target.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let html = r##"<!doctype html>
        <main id="top-main">top</main>
        <iframe id="child" srcdoc="<main id='inside-frame'>child</main>"></iframe>
        <div id="host"></div>
        <script>
          document.querySelector("#host").attachShadow({ mode: "open" }).innerHTML =
            "<span id='shadow-text'>shadow ready</span>";
        </script>"##;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let frame_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_id,
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    let inside_frame = classic_find_css_element_id(app.clone(), session_id, "#inside-frame").await;
    assert!(!inside_frame.is_empty());

    let parent = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame/parent"),
    )
    .await;
    assert_eq!(parent, json!({ "value": null }));

    let host_id = classic_find_css_element_id(app.clone(), session_id, "#host").await;
    let shadow = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{host_id}/shadow"),
    )
    .await;
    let shadow_id = shadow["value"][CLASSIC_SHADOW_ROOT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("shadow root after parent frame: {shadow:?}"));
    let shadow_text = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/shadow/{shadow_id}/element"),
        json!({
            "using": "css selector",
            "value": "#shadow-text"
        }),
    )
    .await;
    let shadow_text_id = shadow_text["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("shadow-scoped find after parent frame: {shadow_text:?}"));
    let text = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{shadow_text_id}/text"),
    )
    .await;
    assert_eq!(text, json!({ "value": "shadow ready" }));

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}

#[tokio::test]
async fn webdriver_classic_switch_frame_null_resets_to_top_level_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_frame/
    // switch.py test_frame_id_null.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let frame_path = format!("/session/{session_id}/frame");

    let page_url = format!("http://{fixture_addr}/nested");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let outer_frame_id = classic_find_css_element_id(app.clone(), session_id, "#outerById").await;
    let switched_outer = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: outer_frame_id.clone(),
            }
        }),
    )
    .await;
    assert_eq!(switched_outer, json!({ "value": null }));
    let outer_element_id =
        classic_find_css_element_id(app.clone(), session_id, "#outer-main").await;

    let inner_frame_id = classic_find_css_element_id(app.clone(), session_id, "#innerById").await;
    let switched_inner = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: inner_frame_id,
            }
        }),
    )
    .await;
    assert_eq!(switched_inner, json!({ "value": null }));
    let inner_element_id =
        classic_find_css_element_id(app.clone(), session_id, "#inner-text").await;

    let default_content = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({ "id": null }),
    )
    .await;
    assert_eq!(default_content, json!({ "value": null }));

    for (label, element_id) in [
        ("outer frame-local element", outer_element_id),
        ("inner frame-local element", inner_element_id),
    ] {
        let (status, response) = classic_request_status_and_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/text"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{label}: {response:?}");
        assert_eq!(response["value"]["error"], json!("no such element"));
    }

    let refound_outer_frame_id =
        classic_find_css_element_id(app.clone(), session_id, "#outerById").await;
    let same_frame = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{refound_outer_frame_id}/equals/{outer_frame_id}"),
    )
    .await;
    assert_eq!(same_frame, json!({ "value": true }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_switch_frame_argument_edges_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_frame/switch.py
    // and switch_number.py argument, bounds, and index semantics.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let frame_path = format!("/session/{session_id}/frame");

    let (empty_status, empty_body) =
        classic_request_status_and_json(app.clone(), Method::POST, &frame_path).await;
    assert_eq!(empty_status, StatusCode::BAD_REQUEST);
    assert_eq!(empty_body["value"]["error"], json!("invalid argument"));

    for value in [
        json!("foo"),
        json!(true),
        json!([]),
        json!({}),
        json!({ "shadow-6066-11e4-a52e-4f735466cecf": "shadow-1" }),
        json!(-1),
        json!(65_536),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({ "id": value }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response:?}");
        assert_eq!(response["value"]["error"], json!("invalid argument"));
    }

    let html = concat!(
        "<iframe srcdoc=\"<p>foo</p>\"></iframe>",
        "<iframe srcdoc=\"<p>bar</p>\"></iframe>",
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (missing_status, missing) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({ "id": 65_535 }),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(missing["value"]["error"], json!("no such frame"));

    for (index, expected) in [(0, "foo"), (1, "bar")] {
        let switched = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({ "id": index }),
        )
        .await;
        assert_eq!(switched, json!({ "value": null }));

        let marker = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return document.querySelector('p').textContent;",
                "args": []
            }),
        )
        .await;
        assert_eq!(marker, json!({ "value": expected }));

        let top = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({ "id": null }),
        )
        .await;
        assert_eq!(top, json!({ "value": null }));
    }

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_nested_frame_switching_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // switch_to_frame/switch_number.py, get_current_url/iframe.py,
    // and get_title/iframe.py. Selenium's string frame API resolves id/name
    // on the client, so this test exercises the same wire shape by finding the
    // frame element with id/name locators before POST /frame.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page_url = format!("http://{fixture_addr}/nested");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let outer_by_id = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "id",
            "value": "outerById"
        }),
    )
    .await;
    let outer_frame_id = outer_by_id["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("outer frame by id reference: {outer_by_id:?}"));
    let (switched_outer_status, switched_outer) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: outer_frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched_outer_status, StatusCode::OK, "{switched_outer:?}");
    assert_eq!(switched_outer, json!({ "value": null }));

    let outer_marker = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.body.dataset.context",
            "args": []
        }),
    )
    .await;
    assert_eq!(outer_marker, json!({ "value": "outer" }));

    let inner_by_name = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "name",
            "value": "innerByName"
        }),
    )
    .await;
    let inner_frame_id = inner_by_name["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .unwrap_or_else(|| panic!("inner frame by name reference: {inner_by_name:?}"));
    let (switched_inner_status, switched_inner) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: inner_frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched_inner_status, StatusCode::OK, "{switched_inner:?}");
    assert_eq!(switched_inner, json!({ "value": null }));

    let (inner_marker_status, inner_marker) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.body.dataset.context",
            "args": []
        }),
    )
    .await;
    assert_eq!(inner_marker_status, StatusCode::OK, "{inner_marker:?}");
    assert_eq!(inner_marker, json!({ "value": "inner" }));

    let inner_text = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#inner-text"
        }),
    )
    .await;
    assert!(inner_text["value"][CLASSIC_ELEMENT_REFERENCE_KEY].is_string());

    let current_url = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(current_url, json!({ "value": page_url }));
    let title = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/title"),
    )
    .await;
    assert_eq!(title, json!({ "value": "top nested" }));

    let parent = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame/parent"),
    )
    .await;
    assert_eq!(parent, json!({ "value": null }));
    let outer_main = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#outer-main"
        }),
    )
    .await;
    assert!(outer_main["value"][CLASSIC_ELEMENT_REFERENCE_KEY].is_string());

    let default_content = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({ "id": null }),
    )
    .await;
    assert_eq!(default_content, json!({ "value": null }));
    let top_main = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#top-nested"
        }),
    )
    .await;
    assert!(top_main["value"][CLASSIC_ELEMENT_REFERENCE_KEY].is_string());

    let index_outer = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({ "id": 0 }),
    )
    .await;
    assert_eq!(index_outer, json!({ "value": null }));
    let index_inner = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({ "id": 0 }),
    )
    .await;
    assert_eq!(index_inner, json!({ "value": null }));
    let index_inner_marker = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.body.dataset.context",
            "args": []
        }),
    )
    .await;
    assert_eq!(index_inner_marker, json!({ "value": "inner" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_switch_frame_webelement_cases_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_frame/
    // switch_webelement.py. The cross-origin companion cases are covered in a
    // separate test with a local multi-origin fixture.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let frame_path = format!("/session/{session_id}/frame");

    let page_url = format!("http://{fixture_addr}/page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (missing_element_status, missing_element) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: "bar"
            }
        }),
    )
    .await;
    assert_eq!(missing_element_status, StatusCode::NOT_FOUND);
    assert_eq!(missing_element["value"]["error"], json!("no such element"));

    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let stale_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<main>replacement</main>") }),
    )
    .await;
    assert_eq!(stale_navigated, json!({ "value": null }));

    let (stale_status, stale) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id
            }
        }),
    )
    .await;
    assert_eq!(stale_status, StatusCode::NOT_FOUND);
    assert_eq!(stale["value"]["error"], json!("stale element reference"));

    let no_frame_url = classic_data_url("<p id='not-a-frame'>foo</p>");
    let no_frame_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": no_frame_url }),
    )
    .await;
    assert_eq!(no_frame_navigated, json!({ "value": null }));
    let no_frame_element_id =
        classic_find_css_element_id(app.clone(), session_id, "#not-a-frame").await;
    let (no_frame_status, no_frame) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: no_frame_element_id
            }
        }),
    )
    .await;
    assert_eq!(no_frame_status, StatusCode::NOT_FOUND);
    assert_eq!(no_frame["value"]["error"], json!("no such frame"));

    let foo_doc = classic_data_url("<p>foo</p>");
    let bar_doc = classic_data_url("<p>bar</p>");
    let frame_page = classic_data_url(&format!(
        "<frameset rows='*,*'><frame id='frame-foo' src='{foo_doc}'></frame><frame id='frame-bar' src='{bar_doc}'></frame></frameset>"
    ));
    let frame_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": frame_page }),
    )
    .await;
    assert_eq!(frame_navigated, json!({ "value": null }));
    for (selector, expected) in [("#frame-foo", "foo"), ("#frame-bar", "bar")] {
        let frame_element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let switched = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({
                "id": {
                    CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id
                }
            }),
        )
        .await;
        assert_eq!(switched, json!({ "value": null }), "switch {selector}");

        let text = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return document.querySelector('p').textContent;",
                "args": []
            }),
        )
        .await;
        assert_eq!(text, json!({ "value": expected }), "frame {selector}");

        let top = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({ "id": null }),
        )
        .await;
        assert_eq!(top, json!({ "value": null }));
    }

    let iframe_page = classic_data_url(concat!(
        "<iframe id='iframe-foo' srcdoc='<p>foo</p>'></iframe>",
        "<iframe id='iframe-bar' srcdoc='<p>bar</p>'></iframe>",
    ));
    let iframe_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": iframe_page }),
    )
    .await;
    assert_eq!(iframe_navigated, json!({ "value": null }));
    for (selector, expected) in [("#iframe-foo", "foo"), ("#iframe-bar", "bar")] {
        let frame_element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let switched = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({
                "id": {
                    CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id
                }
            }),
        )
        .await;
        assert_eq!(switched, json!({ "value": null }), "switch {selector}");

        let text = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return document.querySelector('p').textContent;",
                "args": []
            }),
        )
        .await;
        assert_eq!(text, json!({ "value": expected }), "iframe {selector}");

        let top = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({ "id": null }),
        )
        .await;
        assert_eq!(top, json!({ "value": null }));
    }

    let nested_url = format!("http://{fixture_addr}/nested");
    let nested_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": nested_url }),
    )
    .await;
    assert_eq!(nested_navigated, json!({ "value": null }));
    for (selector, expected) in [("#outerById", "outer"), ("#innerById", "inner")] {
        let frame_element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let switched = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &frame_path,
            json!({
                "id": {
                    CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id
                }
            }),
        )
        .await;
        assert_eq!(switched, json!({ "value": null }), "switch {selector}");

        let marker = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return document.body.dataset.context;",
                "args": []
            }),
        )
        .await;
        assert_eq!(marker, json!({ "value": expected }), "nested {selector}");
    }
    let top_after_nested = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({ "id": null }),
    )
    .await;
    assert_eq!(top_after_nested, json!({ "value": null }));

    let append_url = format!("http://{fixture_addr}/page");
    let append_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": append_url }),
    )
    .await;
    assert_eq!(append_navigated, json!({ "value": null }));
    let appended = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const iframe = document.querySelector('#child'); const div = document.createElement('div'); div.id = 'top-created'; div.textContent = 'I am a div created in top window and appended into the iframe'; iframe.contentWindow.document.body.appendChild(div); return div.textContent;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        appended,
        json!({ "value": "I am a div created in top window and appended into the iframe" })
    );
    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    let appended_text = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.querySelector('#top-created').textContent;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        appended_text,
        json!({ "value": "I am a div created in top window and appended into the iframe" })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_switch_frame_cross_origin_cases_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_frame/
    // cross_origin.py. WPT uses alternate hostnames; this local route test uses
    // separate loopback ports so each frame has a distinct origin without
    // relying on external DNS or localhost IPv6/v4 resolution order.
    let app = build_router(test_state());
    let (browser_addr, alt_addr, www_alt_addr, fixture_servers) =
        spawn_classic_cross_origin_frame_fixture_servers().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let frame_path = format!("/session/{session_id}/frame");
    let browser_origin = format!("http://{browser_addr}");
    let alt_origin = format!("http://{alt_addr}");
    let www_alt_origin = format!("http://{www_alt_addr}");

    let top_url = format!("{browser_origin}/top");
    let child_url = format!("{alt_origin}/child");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": top_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#cross").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    let child_location = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.location.href;",
            "args": []
        }),
    )
    .await;
    assert_eq!(child_location, json!({ "value": child_url }));
    assert_ne!(alt_origin, browser_origin);

    let top = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({ "id": null }),
    )
    .await;
    assert_eq!(top, json!({ "value": null }));

    let nested_top_url = format!("{alt_origin}/nested-top");
    let nested_navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": nested_top_url }),
    )
    .await;
    assert_eq!(nested_navigated, json!({ "value": null }));
    let top_location = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.location.href;",
            "args": []
        }),
    )
    .await;
    assert_eq!(top_location, json!({ "value": nested_top_url }));

    let browser_frame_id =
        classic_find_css_element_id(app.clone(), session_id, "#to-browser").await;
    let switched_browser = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: browser_frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched_browser, json!({ "value": null }));
    let browser_child_url = format!("{browser_origin}/middle");
    let browser_location = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.location.href;",
            "args": []
        }),
    )
    .await;
    assert_eq!(browser_location, json!({ "value": browser_child_url }));

    let leaf_frame_id = classic_find_css_element_id(app.clone(), session_id, "#to-www-alt").await;
    let switched_leaf = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: leaf_frame_id
            }
        }),
    )
    .await;
    assert_eq!(switched_leaf, json!({ "value": null }));
    let leaf_url = format!("{www_alt_origin}/leaf");
    let leaf_location = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return document.location.href;",
            "args": []
        }),
    )
    .await;
    assert_eq!(leaf_location, json!({ "value": leaf_url }));
    assert_ne!(www_alt_origin, browser_origin);
    assert_ne!(www_alt_origin, alt_origin);

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    for server in fixture_servers {
        server.abort();
    }
}

#[tokio::test]
async fn webdriver_classic_detached_current_frame_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/support/fixtures_http.py
    // closed_frame and the no_browsing_context cases in switch_to_frame,
    // switch_to_parent_frame, execute_script, get_page_source, and find_element.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = format!("http://{fixture_addr}/page");
    let frame_path = format!("/session/{session_id}/frame");

    classic_switch_to_child_frame_and_remove_current_frame(app.clone(), session_id, &page_url)
        .await;

    let (url_status, url) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/url"),
    )
    .await;
    assert_eq!(url_status, StatusCode::NOT_FOUND);
    assert_eq!(url["value"]["error"], json!("no such window"));

    let (source_status, source) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/source"),
    )
    .await;
    assert_eq!(source_status, StatusCode::NOT_FOUND);
    assert_eq!(source["value"]["error"], json!("no such window"));

    let (execute_status, execute) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return 1;",
            "args": []
        }),
    )
    .await;
    assert_eq!(execute_status, StatusCode::NOT_FOUND);
    assert_eq!(execute["value"]["error"], json!("no such window"));

    let (find_status, find) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#top-main"
        }),
    )
    .await;
    assert_eq!(find_status, StatusCode::NOT_FOUND);
    assert_eq!(find["value"]["error"], json!("no such window"));

    let (active_status, active) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/active"),
    )
    .await;
    assert_eq!(active_status, StatusCode::NOT_FOUND);
    assert_eq!(active["value"]["error"], json!("no such window"));

    let (indexed_frame_status, indexed_frame) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({ "id": 0 }),
    )
    .await;
    assert_eq!(indexed_frame_status, StatusCode::NOT_FOUND);
    assert_eq!(indexed_frame["value"]["error"], json!("no such window"));

    let parent = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame/parent"),
    )
    .await;
    assert_eq!(parent, json!({ "value": null }));
    let top_main_after_parent =
        classic_find_css_element_id(app.clone(), session_id, "#top-main").await;
    assert!(!top_main_after_parent.is_empty());

    classic_switch_to_child_frame_and_remove_current_frame(app.clone(), session_id, &page_url)
        .await;
    let default_content = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({ "id": null }),
    )
    .await;
    assert_eq!(default_content, json!({ "value": null }));
    let top_main_after_default_content =
        classic_find_css_element_id(app.clone(), session_id, "#top-main").await;
    assert!(!top_main_after_default_content.is_empty());

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_navigate_from_detached_current_frame_resets_current_context() {
    // Ported from WPT webdriver/tests/classic/navigate_to/navigate.py and
    // get_title/get.py no_browsing_context: top-level navigation is allowed
    // from a removed current frame, and the selected context is top-level after
    // navigation completes.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = format!("http://{fixture_addr}/page");

    classic_switch_to_child_frame_and_remove_current_frame(app.clone(), session_id, &page_url)
        .await;

    let after_navigation_url =
        classic_data_url("<title>Foo</title><main id='after-navigation'>after</main>");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": after_navigation_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let title = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/title"),
    )
    .await;
    assert_eq!(title, json!({ "value": "Foo" }));
    let after_navigation =
        classic_find_css_element_id(app.clone(), session_id, "#after-navigation").await;
    assert!(!after_navigation.is_empty());

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_click_inside_frame_observes_removed_current_frame() {
    // Mirrors Selenium's deleted-frame recovery flow: a click dispatched inside
    // the current frame removes that frame from its parent document.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = format!("http://{fixture_addr}/page");
    let frame_path = format!("/session/{session_id}/frame");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let switched =
        classic_request_json_with_body(app.clone(), Method::POST, &frame_path, json!({ "id": 0 }))
            .await;
    assert_eq!(switched, json!({ "value": null }));

    let remove_button_id =
        classic_find_css_element_id(app.clone(), session_id, "#remove-current-frame").await;
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{remove_button_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let (find_status, find) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#inside-frame"
        }),
    )
    .await;
    assert_eq!(find_status, StatusCode::NOT_FOUND);
    assert_eq!(find["value"]["error"], json!("no such window"));

    let default_content = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({ "id": null }),
    )
    .await;
    assert_eq!(default_content, json!({ "value": null }));
    let top_main_after_default_content =
        classic_find_css_element_id(app.clone(), session_id, "#top-main").await;
    assert!(!top_main_after_default_content.is_empty());

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_detached_current_frame_endpoint_sweep_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/support/fixtures_http.py closed_frame
    // and no_browsing_context cases across element, shadow-root, cookie, and actions
    // commands. These commands inspect the current browsing context, so a bogus
    // element or shadow id must still report no such window before id lookup.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let nested_url = format!("http://{fixture_addr}/nested");

    classic_switch_to_nested_frame_and_remove_parent_frame(app.clone(), session_id, &nested_url)
        .await;

    let bogus_element_paths = [
        format!("/session/{session_id}/element/foo/attribute/id"),
        format!("/session/{session_id}/element/foo/text"),
        format!("/session/{session_id}/element/foo/name"),
        format!("/session/{session_id}/element/foo/property/id"),
        format!("/session/{session_id}/element/foo/css/display"),
        format!("/session/{session_id}/element/foo/computedlabel"),
        format!("/session/{session_id}/element/foo/computedrole"),
        format!("/session/{session_id}/element/foo/enabled"),
        format!("/session/{session_id}/element/foo/displayed"),
        format!("/session/{session_id}/element/foo/selected"),
        format!("/session/{session_id}/element/foo/rect"),
        format!("/session/{session_id}/element/foo/screenshot"),
        format!("/session/{session_id}/element/foo/shadow"),
        format!("/session/{session_id}/element/foo/equals/bar"),
        format!("/session/{session_id}/cookie"),
        format!("/session/{session_id}/cookie/foo"),
    ];
    for path in bogus_element_paths {
        classic_assert_no_such_window(app.clone(), Method::GET, &path).await;
    }

    for path in [
        format!("/session/{session_id}/element/foo/clear"),
        format!("/session/{session_id}/element/foo/click"),
    ] {
        classic_assert_no_such_window(app.clone(), Method::POST, &path).await;
    }

    for path in [
        format!("/session/{session_id}/element/foo/element"),
        format!("/session/{session_id}/element/foo/elements"),
        format!("/session/{session_id}/shadow/foo/element"),
        format!("/session/{session_id}/shadow/foo/elements"),
    ] {
        classic_assert_no_such_window_with_body(
            app.clone(),
            Method::POST,
            &path,
            json!({
                "using": "css selector",
                "value": "foo"
            }),
        )
        .await;
    }

    classic_assert_no_such_window_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/foo/value"),
        json!({ "text": "abc" }),
    )
    .await;
    classic_assert_no_such_window_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/cookie"),
        json!({
            "cookie": {
                "name": "hello",
                "value": "world"
            }
        }),
    )
    .await;
    classic_assert_no_such_window_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "none",
                "id": "pause",
                "actions": [{ "type": "pause", "duration": 0 }]
            }]
        }),
    )
    .await;

    classic_assert_no_such_window(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/cookie"),
    )
    .await;
    classic_assert_no_such_window(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/cookie/foo"),
    )
    .await;
    classic_assert_no_such_window(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/actions"),
    )
    .await;

    let parent = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame/parent"),
    )
    .await;
    assert_eq!(parent, json!({ "value": null }));
    let top = classic_find_css_element_id(app.clone(), session_id, "#top-nested").await;
    assert!(!top.is_empty());

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
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

#[tokio::test]
async fn webdriver_classic_element_click_uses_shared_dom_geometry_and_input() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<button id='target' onclick='window.__clicked = true'>go</button>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#target"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");

    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let clicked_state = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return Boolean(window.__clicked);",
            "args": []
        }),
    )
    .await;
    assert_eq!(clicked_state, json!({ "value": true }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_anchor_target_blank_click_opens_window_handle() {
    // Mirrors ChromeDriver's link-click new window smoke: <a target=_blank>
    // should create a switchable top-level browsing context.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let url_path = format!("/session/{session_id}/url");
    let title_path = format!("/session/{session_id}/title");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();
    let popup_url =
        classic_data_url("<!doctype html><title>Popup Target</title><main>popup</main>");
    let page_url = classic_data_url(&format!(
        "<!doctype html><title>Popup Source</title><a id='popup' href='{popup_url}' target='_blank'>open</a>"
    ));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &url_path,
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let link_id = classic_find_css_element_id(app.clone(), session_id, "#popup").await;
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{link_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handles = handles["value"].as_array().expect("window handles");
    assert_eq!(handles.len(), 2, "{handles:?}");
    let popup_handle = handles
        .iter()
        .filter_map(Value::as_str)
        .find(|handle| *handle != original_handle)
        .expect("popup window handle")
        .to_owned();

    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": popup_handle }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    let current_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(current_url["value"], json!(popup_url));
    let title = classic_request_json(app.clone(), Method::GET, &title_path).await;
    assert_eq!(title, json!({ "value": "Popup Target" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_window_open_self_click_waits_for_current_url() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let handles_path = format!("/session/{session_id}/window/handles");
    let url_path = format!("/session/{session_id}/url");
    let execute_path = format!("/session/{session_id}/execute/sync");

    let self_url =
        classic_data_url("<!doctype html><title>Self Target</title><main>popup self</main>");
    let page_url = classic_data_url(&format!(
        "<!doctype html><title>Self Source</title>\
         <button id='self' onclick=\"window.open('{self_url}', '_self')\">self</button>"
    ));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &url_path,
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let handle_count = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handle_count = handle_count["value"]
        .as_array()
        .expect("window handles")
        .len();

    let button_id = classic_find_css_element_id(app.clone(), session_id, "#self").await;
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{button_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let current_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(current_url["value"], json!(self_url));
    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    assert_eq!(
        handles["value"].as_array().expect("window handles").len(),
        handle_count
    );
    let text = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return document.querySelector('main').textContent;",
            "args": []
        }),
    )
    .await;
    assert_eq!(text, json!({ "value": "popup self" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_named_popup_reuse_navigates_existing_window() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let url_path = format!("/session/{session_id}/url");
    let execute_path = format!("/session/{session_id}/execute/sync");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();
    let first_url = classic_data_url("<!doctype html><title>Named First</title><main>first</main>");
    let second_url =
        classic_data_url("<!doctype html><title>Named Second</title><main>second</main>");
    let page_url = classic_data_url(&format!(
        "<!doctype html><title>Named Source</title>\
         <button id='first' onclick=\"window.open('{first_url}', 'classicNamedPopup')\">first</button>\
         <button id='second' onclick=\"window.open('{second_url}', 'classicNamedPopup')\">second</button>"
    ));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &url_path,
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let first_button_id = classic_find_css_element_id(app.clone(), session_id, "#first").await;
    let clicked_first = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{first_button_id}/click"),
    )
    .await;
    assert_eq!(clicked_first, json!({ "value": null }));
    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handles = handles["value"].as_array().expect("window handles");
    assert_eq!(handles.len(), 2, "{handles:?}");
    let named_handle = handles
        .iter()
        .filter_map(Value::as_str)
        .find(|handle| *handle != original_handle)
        .expect("named popup handle")
        .to_owned();

    let switched_first = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": named_handle }),
    )
    .await;
    assert_eq!(switched_first, json!({ "value": null }));
    let first_current_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(first_current_url["value"], json!(first_url));
    let first_text = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return document.querySelector('main').textContent;",
            "args": []
        }),
    )
    .await;
    assert_eq!(first_text, json!({ "value": "first" }));

    let switched_opener = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": original_handle }),
    )
    .await;
    assert_eq!(switched_opener, json!({ "value": null }));
    let handle_count = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handle_count = handle_count["value"]
        .as_array()
        .expect("window handles")
        .len();

    let second_button_id = classic_find_css_element_id(app.clone(), session_id, "#second").await;
    let clicked_second = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{second_button_id}/click"),
    )
    .await;
    assert_eq!(clicked_second, json!({ "value": null }));
    let handles_after_reuse = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    assert_eq!(
        handles_after_reuse["value"]
            .as_array()
            .expect("window handles")
            .len(),
        handle_count
    );

    let switched_second = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": named_handle }),
    )
    .await;
    assert_eq!(switched_second, json!({ "value": null }));
    let second_current_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(second_current_url["value"], json!(second_url));
    let second_text = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return document.querySelector('main').textContent;",
            "args": []
        }),
    )
    .await;
    assert_eq!(second_text, json!({ "value": "second" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_option_click_updates_select_state_ported_from_selenium_select() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let html = concat!(
        "<select id='single'>",
        "<option id='cheddar' value='cheddar'>Cheddar</option>",
        "<option id='brie' value='brie'>Brie</option>",
        "</select>",
        "<select id='multi' multiple>",
        "<option id='eggs' value='eggs' selected>Eggs</option>",
        "<option id='ham' value='ham'>Ham</option>",
        "</select>",
        "<select id='blocked'>",
        "<option id='safe' value='safe' selected>Safe</option>",
        "<optgroup disabled><option id='blockedOptgroup' value='blocked'>Blocked</option></optgroup>",
        "</select>",
        "<select id='disabledSelectForClick' disabled>",
        "<option id='locked' value='locked' selected>Locked</option>",
        "<option id='disabledSelectTarget' value='target'>Target</option>",
        "</select>",
        "<script>",
        "window.__selectLog=[];",
        "for (const select of document.querySelectorAll('select')) {",
        "select.addEventListener('input', () => window.__selectLog.push(select.id + ':input:' + Array.from(select.selectedOptions).map(o => o.value).join('/')));",
        "select.addEventListener('change', () => window.__selectLog.push(select.id + ':change:' + Array.from(select.selectedOptions).map(o => o.value).join('/')));",
        "}",
        "</script>",
    );

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": format!("data:text/html,{html}") }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let cheddar_id = classic_find_css_element_id(app.clone(), session_id, "#cheddar").await;
    let brie_id = classic_find_css_element_id(app.clone(), session_id, "#brie").await;
    let eggs_id = classic_find_css_element_id(app.clone(), session_id, "#eggs").await;
    let ham_id = classic_find_css_element_id(app.clone(), session_id, "#ham").await;
    let safe_id = classic_find_css_element_id(app.clone(), session_id, "#safe").await;
    let blocked_optgroup_id =
        classic_find_css_element_id(app.clone(), session_id, "#blockedOptgroup").await;
    let locked_id = classic_find_css_element_id(app.clone(), session_id, "#locked").await;
    let disabled_select_target_id =
        classic_find_css_element_id(app.clone(), session_id, "#disabledSelectTarget").await;

    let clicked_brie = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{brie_id}/click"),
    )
    .await;
    assert_eq!(clicked_brie, json!({ "value": null }));
    let clicked_ham = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{ham_id}/click"),
    )
    .await;
    assert_eq!(clicked_ham, json!({ "value": null }));
    let clicked_eggs = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{eggs_id}/click"),
    )
    .await;
    assert_eq!(clicked_eggs, json!({ "value": null }));
    let clicked_blocked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{blocked_optgroup_id}/click"),
    )
    .await;
    assert_eq!(clicked_blocked, json!({ "value": null }));
    let clicked_disabled_select = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{disabled_select_target_id}/click"),
    )
    .await;
    assert_eq!(clicked_disabled_select, json!({ "value": null }));

    for (element_id, expected) in [
        (&cheddar_id, false),
        (&brie_id, true),
        (&eggs_id, false),
        (&ham_id, true),
        (&safe_id, true),
        (&blocked_optgroup_id, false),
        (&locked_id, true),
        (&disabled_select_target_id, false),
    ] {
        let selected = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/selected"),
        )
        .await;
        assert_eq!(selected, json!({ "value": expected }), "{element_id}");
    }

    let script_state = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [document.getElementById('single').value, Array.from(document.getElementById('multi').selectedOptions).map(o => o.value).join('/'), document.getElementById('blocked').value, document.getElementById('disabledSelectForClick').value, window.__selectLog.join(',')].join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        script_state,
        json!({
            "value": "brie|ham|safe|locked|single:input:brie,single:change:brie,multi:input:eggs/ham,multi:change:eggs/ham,multi:input:ham,multi:change:ham"
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_option_click_edges_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/element_click/select.py.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let html = r#"<!doctype html>
        <select id="preselectedSingle">
          <option id="psFirst">first</option>
          <option id="psSecond" selected>second</option>
        </select>
        <select id="singleDeselects">
          <option id="sdFirst">first</option>
          <option id="sdSecond">second</option>
          <option id="sdThird">third</option>
        </select>
        <select id="singleRepeated">
          <option id="srFirst">first</option>
          <option id="srSecond">second</option>
        </select>
        <select id="preselectedMultiple" multiple>
          <option id="pmFirst">first</option>
          <option id="pmSecond" selected>second</option>
        </select>
        <select id="multiKeepsOthers" multiple>
          <option id="mkFirst">first</option>
          <option id="mkSecond">second</option>
          <option id="mkThird">third</option>
        </select>
        <select id="multiToggle" multiple>
          <option id="mtFirst">first</option>
          <option id="mtSecond">second</option>
        </select>
        <select id="outSingle">
          <option id="outSingle1">1</option>
          <option id="outSingle2">2</option>
          <option id="outSingle3">3</option>
          <option id="outSingle4">4</option>
          <option id="outSingle5">5</option>
          <option id="outSingle6">6</option>
          <option id="outSingle7">7</option>
          <option id="outSingle8">8</option>
          <option id="outSingle9">9</option>
          <option id="outSingle10">10</option>
          <option id="outSingle11">11</option>
          <option id="outSingle12">12</option>
          <option id="outSingle13">13</option>
          <option id="outSingle14">14</option>
          <option id="outSingle15">15</option>
          <option id="outSingle16">16</option>
          <option id="outSingle17">17</option>
          <option id="outSingle18">18</option>
          <option id="outSingle19">19</option>
          <option id="outSingle20">20</option>
        </select>
        <select id="outMulti" multiple>
          <option id="outMulti1">1</option>
          <option id="outMulti2">2</option>
          <option id="outMulti3">3</option>
          <option id="outMulti4">4</option>
          <option id="outMulti5">5</option>
          <option id="outMulti6">6</option>
          <option id="outMulti7">7</option>
          <option id="outMulti8">8</option>
          <option id="outMulti9">9</option>
          <option id="outMulti10">10</option>
          <option id="outMulti11">11</option>
          <option id="outMulti12">12</option>
          <option id="outMulti13">13</option>
          <option id="outMulti14">14</option>
          <option id="outMulti15">15</option>
          <option id="outMulti16">16</option>
          <option id="outMulti17">17</option>
          <option id="outMulti18">18</option>
          <option id="outMulti19">19</option>
          <option id="outMulti20">20</option>
        </select>
        <select id="disabledOption">
          <option id="disabledFirst" disabled>foo</option>
          <option id="enabledSecond">bar</option>
        </select>"#;
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url(html) }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    async fn option_id(app: Router, session_id: &str, selector: &str) -> String {
        classic_find_css_element_id(app, session_id, selector).await
    }

    async fn click_option(app: Router, session_id: &str, element_id: &str) {
        let clicked = classic_request_json(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/click"),
        )
        .await;
        assert_eq!(clicked, json!({ "value": null }));
    }

    async fn assert_selected(
        app: Router,
        session_id: &str,
        element_id: &str,
        label: &str,
        expected: bool,
    ) {
        let selected = classic_request_json(
            app,
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/selected"),
        )
        .await;
        assert_eq!(
            selected,
            json!({ "value": expected }),
            "{label}: element {element_id}"
        );
    }

    let ps_first = option_id(app.clone(), session_id, "#psFirst").await;
    let ps_second = option_id(app.clone(), session_id, "#psSecond").await;
    assert_selected(app.clone(), session_id, &ps_first, "psFirst initial", false).await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_second,
        "psSecond initial",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &ps_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_second,
        "psSecond after repeated click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_first,
        "psFirst after repeated click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &ps_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_first,
        "psFirst after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &ps_second,
        "psSecond after psFirst click",
        false,
    )
    .await;

    let sd_first = option_id(app.clone(), session_id, "#sdFirst").await;
    let sd_second = option_id(app.clone(), session_id, "#sdSecond").await;
    let sd_third = option_id(app.clone(), session_id, "#sdThird").await;
    click_option(app.clone(), session_id, &sd_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_first,
        "sdFirst after click",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &sd_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_second,
        "sdSecond after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_first,
        "sdFirst after sdSecond click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &sd_third).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_third,
        "sdThird after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_second,
        "sdSecond after sdThird click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &sd_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_first,
        "sdFirst after second click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &sd_third,
        "sdThird after sdFirst click",
        false,
    )
    .await;

    let sr_second = option_id(app.clone(), session_id, "#srSecond").await;
    click_option(app.clone(), session_id, &sr_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &sr_second,
        "srSecond after first click",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &sr_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &sr_second,
        "srSecond after repeated click",
        true,
    )
    .await;

    let pm_first = option_id(app.clone(), session_id, "#pmFirst").await;
    let pm_second = option_id(app.clone(), session_id, "#pmSecond").await;
    assert_selected(app.clone(), session_id, &pm_first, "pmFirst initial", false).await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_second,
        "pmSecond initial",
        true,
    )
    .await;
    click_option(app.clone(), session_id, &pm_second).await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_second,
        "pmSecond after click",
        false,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_first,
        "pmFirst after pmSecond click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &pm_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_first,
        "pmFirst after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &pm_second,
        "pmSecond after pmFirst click",
        false,
    )
    .await;

    let mk_first = option_id(app.clone(), session_id, "#mkFirst").await;
    let mk_second = option_id(app.clone(), session_id, "#mkSecond").await;
    let mk_third = option_id(app.clone(), session_id, "#mkThird").await;
    click_option(app.clone(), session_id, &mk_first).await;
    click_option(app.clone(), session_id, &mk_second).await;
    click_option(app.clone(), session_id, &mk_third).await;
    assert_selected(app.clone(), session_id, &mk_first, "mkFirst final", true).await;
    assert_selected(app.clone(), session_id, &mk_second, "mkSecond final", true).await;
    assert_selected(app.clone(), session_id, &mk_third, "mkThird final", true).await;

    let mt_first = option_id(app.clone(), session_id, "#mtFirst").await;
    let mt_second = option_id(app.clone(), session_id, "#mtSecond").await;
    assert_selected(app.clone(), session_id, &mt_first, "mtFirst initial", false).await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_second,
        "mtSecond initial",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &mt_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_first,
        "mtFirst after click",
        true,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_second,
        "mtSecond after mtFirst click",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &mt_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_first,
        "mtFirst after repeated click",
        false,
    )
    .await;
    assert_selected(
        app.clone(),
        session_id,
        &mt_second,
        "mtSecond after repeated click",
        false,
    )
    .await;

    let out_single_15 = option_id(app.clone(), session_id, "#outSingle15").await;
    click_option(app.clone(), session_id, &out_single_15).await;
    assert_selected(
        app.clone(),
        session_id,
        &out_single_15,
        "outSingle15 after click",
        true,
    )
    .await;
    let out_multi_20 = option_id(app.clone(), session_id, "#outMulti20").await;
    click_option(app.clone(), session_id, &out_multi_20).await;
    assert_selected(
        app.clone(),
        session_id,
        &out_multi_20,
        "outMulti20 after click",
        true,
    )
    .await;

    let disabled_first = option_id(app.clone(), session_id, "#disabledFirst").await;
    assert_selected(
        app.clone(),
        session_id,
        &disabled_first,
        "disabledFirst initial",
        false,
    )
    .await;
    click_option(app.clone(), session_id, &disabled_first).await;
    assert_selected(
        app.clone(),
        session_id,
        &disabled_first,
        "disabledFirst after click",
        false,
    )
    .await;

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_element_send_keys_uses_shared_input() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<input id='field' value=''>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#field"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");

    let sent = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/value"),
        json!({ "text": "typed" }),
    )
    .await;
    assert_eq!(sent, json!({ "value": null }));

    let value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/property/value"),
    )
    .await;
    assert_eq!(value, json!({ "value": "typed" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_send_keys_form_control_cases_ported_from_chromium_wpt() {
    // Ported from Chromium's WPT checkout:
    // third_party/blink/web_tests/external/wpt/webdriver/tests/classic/
    // element_send_keys/form_controls.py input, textarea, append,
    // focused-selection insertion, and date cases.
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let page = classic_data_url(
        r#"<!doctype html>
        <input id="input-empty">
        <textarea id="textarea-empty"></textarea>
        <input id="input-append" value="a">
        <textarea id="textarea-append">a</textarea>
        <input id="input-insert" value="a">
        <textarea id="textarea-insert">a</textarea>
        <input id="disabled-text" disabled>
        <input id="date" type="date">
        "#,
    );
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    async fn send_keys(app: Router, session_id: &str, selector: &str, text: &str) {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        let sent = classic_request_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/value"),
            json!({ "text": text }),
        )
        .await;
        assert_eq!(sent, json!({ "value": null }), "{selector} send keys");
    }

    async fn send_keys_status(
        app: Router,
        session_id: &str,
        selector: &str,
        text: &str,
    ) -> (StatusCode, serde_json::Value) {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_status_and_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/value"),
            json!({ "text": text }),
        )
        .await
    }

    async fn property_value(app: Router, session_id: &str, selector: &str) -> serde_json::Value {
        let element_id = classic_find_css_element_id(app.clone(), session_id, selector).await;
        classic_request_json(
            app,
            Method::GET,
            &format!("/session/{session_id}/element/{element_id}/property/value"),
        )
        .await
    }

    async fn active_element_id(app: Router, session_id: &str) -> serde_json::Value {
        classic_request_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return document.activeElement && document.activeElement.id;",
                "args": []
            }),
        )
        .await
    }

    send_keys(app.clone(), session_id, "#input-empty", "foo").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-empty").await,
        json!({ "value": "foo" })
    );
    assert_eq!(
        active_element_id(app.clone(), session_id).await,
        json!({ "value": "input-empty" })
    );

    send_keys(app.clone(), session_id, "#textarea-empty", "foo").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-empty").await,
        json!({ "value": "foo" })
    );
    assert_eq!(
        active_element_id(app.clone(), session_id).await,
        json!({ "value": "textarea-empty" })
    );

    send_keys(app.clone(), session_id, "#input-append", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-append").await,
        json!({ "value": "ab" })
    );
    send_keys(app.clone(), session_id, "#input-append", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-append").await,
        json!({ "value": "abc" })
    );

    send_keys(app.clone(), session_id, "#textarea-append", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-append").await,
        json!({ "value": "ab" })
    );
    send_keys(app.clone(), session_id, "#textarea-append", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-append").await,
        json!({ "value": "abc" })
    );

    let prepared_input = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const elem = document.getElementById('input-insert'); elem.focus(); elem.setSelectionRange(0, 0); return [elem.selectionStart, elem.selectionEnd].join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(prepared_input, json!({ "value": "0|0" }));
    send_keys(app.clone(), session_id, "#input-insert", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-insert").await,
        json!({ "value": "ba" })
    );
    send_keys(app.clone(), session_id, "#input-insert", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#input-insert").await,
        json!({ "value": "bca" })
    );

    let prepared_textarea = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const elem = document.getElementById('textarea-insert'); elem.focus(); elem.setSelectionRange(0, 0); return [elem.selectionStart, elem.selectionEnd].join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(prepared_textarea, json!({ "value": "0|0" }));
    send_keys(app.clone(), session_id, "#textarea-insert", "b").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-insert").await,
        json!({ "value": "ba" })
    );
    send_keys(app.clone(), session_id, "#textarea-insert", "c").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#textarea-insert").await,
        json!({ "value": "bca" })
    );

    send_keys(app.clone(), session_id, "#date", "2000-01-01").await;
    assert_eq!(
        property_value(app.clone(), session_id, "#date").await,
        json!({ "value": "2000-01-01" })
    );

    let (disabled_status, disabled_response) =
        send_keys_status(app.clone(), session_id, "#disabled-text", "blocked").await;
    assert_eq!(disabled_status, StatusCode::BAD_REQUEST);
    assert_eq!(
        disabled_response["value"]["error"],
        json!("element not interactable")
    );
}

#[tokio::test]
async fn webdriver_classic_file_input_send_keys_sets_selected_files() {
    let first_file = TempPath::new("classic-file-upload-first");
    let second_file = TempPath::new("classic-file-upload-second");
    let third_file = TempPath::new("classic-file-upload-third");
    fs::write(&first_file.path, b"alpha").expect("write first upload file");
    fs::write(&second_file.path, b"bravo!").expect("write second upload file");
    fs::write(&third_file.path, b"charlie").expect("write third upload file");
    let first_name = first_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("first file should have a filename")
        .to_owned();
    let second_name = second_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("second file should have a filename")
        .to_owned();
    let third_name = third_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("third file should have a filename")
        .to_owned();
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page = classic_data_url(
        "<input id='multi' type='file' multiple style='display:none'>\
         <input id='single' type='file'>\
         <script>\
         window.__events=[];\
         for (const id of ['multi','single']) {\
           const el = document.getElementById(id);\
           el.addEventListener('input', () => window.__events.push(id + ':input:' + el.files.length));\
           el.addEventListener('change', () => window.__events.push(id + ':change:' + el.files.length));\
         }\
         </script>",
    );

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let multi = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#multi"
        }),
    )
    .await;
    let multi_id = multi["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .expect("multi file input id");
    let uploaded = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{multi_id}/value"),
        json!({
            "text": format!(
                "{}\n{}",
                first_file.path.to_string_lossy(),
                second_file.path.to_string_lossy()
            )
        }),
    )
    .await;
    assert_eq!(uploaded, json!({ "value": null }));

    let summary = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const input = document.getElementById('multi'); return JSON.stringify({ length: input.files.length, names: Array.from(input.files).map(file => file.name).join('|'), sizes: Array.from(input.files).map(file => file.size).join('|'), value: input.value, events: window.__events.join(',') });",
            "args": []
        }),
    )
    .await;
    let summary: serde_json::Value = serde_json::from_str(
        summary["value"]
            .as_str()
            .expect("summary should be JSON string"),
    )
    .expect("summary JSON");
    assert_eq!(summary["length"], json!(2));
    assert_eq!(
        summary["names"],
        json!(format!("{first_name}|{second_name}"))
    );
    assert_eq!(summary["sizes"], json!("5|6"));
    assert_eq!(
        summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(summary["events"], json!("multi:input:2,multi:change:2"));

    let appended = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{multi_id}/value"),
        json!({
            "text": third_file.path.to_string_lossy().to_string()
        }),
    )
    .await;
    assert_eq!(appended, json!({ "value": null }));

    let appended_summary = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const input = document.getElementById('multi'); return JSON.stringify({ length: input.files.length, names: Array.from(input.files).map(file => file.name).join('|'), sizes: Array.from(input.files).map(file => file.size).join('|'), value: input.value, events: window.__events.join(',') });",
            "args": []
        }),
    )
    .await;
    let appended_summary: serde_json::Value = serde_json::from_str(
        appended_summary["value"]
            .as_str()
            .expect("appended summary should be JSON string"),
    )
    .expect("appended summary JSON");
    assert_eq!(appended_summary["length"], json!(3));
    assert_eq!(
        appended_summary["names"],
        json!(format!("{first_name}|{second_name}|{third_name}"))
    );
    assert_eq!(appended_summary["sizes"], json!("5|6|7"));
    assert_eq!(
        appended_summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(
        appended_summary["events"],
        json!("multi:input:2,multi:change:2,multi:input:3,multi:change:3")
    );

    let single = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#single"
        }),
    )
    .await;
    let single_id = single["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
        .as_str()
        .expect("single file input id");
    let (non_multiple_status, non_multiple) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{single_id}/value"),
        json!({
            "text": format!(
                "{}\n{}",
                first_file.path.to_string_lossy(),
                second_file.path.to_string_lossy()
            )
        }),
    )
    .await;
    assert_eq!(non_multiple_status, StatusCode::BAD_REQUEST);
    assert_eq!(non_multiple["value"]["error"], json!("invalid argument"));

    let trailing_newlines = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{single_id}/value"),
        json!({
            "text": format!("{}\n\n", first_file.path.to_string_lossy())
        }),
    )
    .await;
    assert_eq!(trailing_newlines, json!({ "value": null }));

    let single_summary = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const input = document.getElementById('single'); return JSON.stringify({ length: input.files.length, names: Array.from(input.files).map(file => file.name).join('|'), value: input.value, events: window.__events.join(',') });",
            "args": []
        }),
    )
    .await;
    let single_summary: serde_json::Value = serde_json::from_str(
        single_summary["value"]
            .as_str()
            .expect("single summary should be JSON string"),
    )
    .expect("single summary JSON");
    assert_eq!(single_summary["length"], json!(1));
    assert_eq!(single_summary["names"], json!(first_name));
    assert_eq!(
        single_summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(
        single_summary["events"],
        json!(
            "multi:input:2,multi:change:2,multi:input:3,multi:change:3,single:input:1,single:change:1"
        )
    );

    let (empty_paths_status, empty_paths) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{single_id}/value"),
        json!({
            "text": "\n \n"
        }),
    )
    .await;
    assert_eq!(empty_paths_status, StatusCode::BAD_REQUEST);
    assert_eq!(empty_paths["value"]["error"], json!("invalid argument"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_key_source_uses_shared_input() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<input id='field' value='abc'>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#field"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");

    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let actions = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "\u{E009}" },
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" },
                    { "type": "keyUp", "value": "\u{E009}" },
                    { "type": "keyDown", "value": "\u{E003}" },
                    { "type": "keyUp", "value": "\u{E003}" },
                    { "type": "keyDown", "value": "x" },
                    { "type": "keyUp", "value": "x" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions, json!({ "value": null }));

    let value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/property/value"),
    )
    .await;
    assert_eq!(value, json!({ "value": "x" }));

    let cleared = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/clear"),
    )
    .await;
    assert_eq!(cleared, json!({ "value": null }));
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let shifted_actions = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "f" },
                    { "type": "keyUp", "value": "f" },
                    { "type": "keyDown", "value": "\u{E008}" },
                    { "type": "keyDown", "value": "o" },
                    { "type": "keyUp", "value": "o" },
                    { "type": "keyDown", "value": "b" },
                    { "type": "keyUp", "value": "b" },
                    { "type": "keyUp", "value": "\u{E008}" },
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" },
                    { "type": "keyDown", "value": "r" },
                    { "type": "keyUp", "value": "r" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(shifted_actions, json!({ "value": null }));

    let shifted_value = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{element_id}/property/value"),
    )
    .await;
    assert_eq!(shifted_value, json!({ "value": "fOBar" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_wheel_source_dispatches_with_real_geometry() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicWheel=null;document.addEventListener('wheel',function(event){window.__classicWheel={type:event.type,deltaX:event.deltaX,deltaY:event.deltaY,clientX:event.clientX,clientY:event.clientY};});</script><div style='width:200px;height:200px'>wheel-target</div>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": "viewport",
                    "x": 10,
                    "y": 11,
                    "deltaX": 7,
                    "deltaY": 13
                }]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let wheel = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicWheel;",
            "args": []
        }),
    )
    .await;
    assert_eq!(wheel["value"]["type"], json!("wheel"));
    assert_eq!(wheel["value"]["deltaX"], json!(7));
    assert_eq!(wheel["value"]["deltaY"], json!(13));
    assert_eq!(wheel["value"]["clientX"], json!(10));
    assert_eq!(wheel["value"]["clientY"], json!(11));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_touch_pointer_dispatches_with_real_geometry() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicTouch=[];['touchstart','touchmove','touchend'].forEach(function(type){document.addEventListener(type,function(event){var point=event.changedTouches[0];window.__classicTouch.push(type+':' +(event instanceof TouchEvent)+':' + point.clientX + ':' + point.clientY);});});</script><main style='width:200px;height:200px'>touch-target</main>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "finger",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown" },
                    { "type": "pointerMove", "origin": "pointer", "x": 3, "y": 4 },
                    { "type": "pointerUp" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let touch_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicTouch;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        touch_events,
        json!({
            "value": [
                "touchstart:true:10:11",
                "touchmove:true:13:15",
                "touchend:true:13:15"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_touch_pointer_capture_uses_real_geometry() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='button'>button</div><div id='target0'>capture</div><script>const button=document.getElementById('button');const target0=document.getElementById('target0');window.__classicTouchCapture=[];button.addEventListener('pointerdown',event=>{window.__classicTouchCapture.push('pointerdown@button:'+event.pointerType);target0.setPointerCapture(event.pointerId);window.__classicTouchCapture.push('has:'+target0.hasPointerCapture(event.pointerId));});button.addEventListener('pointermove',()=>window.__classicTouchCapture.push('pointermove@button'));target0.addEventListener('gotpointercapture',event=>window.__classicTouchCapture.push('gotpointercapture@target0:'+event.pointerType));target0.addEventListener('pointermove',event=>window.__classicTouchCapture.push('pointermove@target0:'+event.pointerType));target0.addEventListener('pointerup',event=>window.__classicTouchCapture.push('pointerup@target0:'+event.pointerType));target0.addEventListener('lostpointercapture',event=>window.__classicTouchCapture.push('lostpointercapture@target0:'+event.pointerType));</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "finger",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown" },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 35 },
                    { "type": "pointerUp" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicTouchCapture;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@button:touch",
                "has:true",
                "gotpointercapture@target0:touch",
                "pointermove@target0:touch",
                "pointerup@target0:touch",
                "lostpointercapture@target0:touch"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_pen_pointer_preserves_pointer_properties() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<main id='target' style='width:200px;height:200px'>pen-target</main><script>window.__classicPen=[];function v(value){if(value===undefined)return '';return value;}const target=document.getElementById('target');['pointerover','pointerenter','pointermove','pointerdown','pointerup','mouseover','mouseenter','mousemove','mousedown','mouseup','click'].forEach(function(type){target.addEventListener(type,function(event){window.__classicPen.push([type,event.pointerType||'',v(event.pressure),v(event.tangentialPressure),v(event.tiltX),v(event.tiltY),v(event.twist),event.clientX,event.clientY,event.button,event.buttons].join(':'));});});</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "pen",
                "parameters": { "pointerType": "pen" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    {
                        "type": "pointerDown",
                        "button": 0,
                        "pressure": 0.75,
                        "tangentialPressure": -0.25,
                        "tiltX": 12,
                        "tiltY": -8,
                        "twist": 45
                    },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let pen_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicPen;",
            "args": []
        }),
    )
    .await;
    let pen_events = pen_events["value"]
        .as_array()
        .expect("pen event log should be an array");
    assert!(
        pen_events.iter().any(|event| {
            event.as_str().is_some_and(|event| {
                event.starts_with("pointerdown:pen:0.75:-0.25:12:-8:45:10:11:0:1")
            })
        }),
        "pen events: {pen_events:?}"
    );
    assert!(
        pen_events.iter().any(|event| {
            event
                .as_str()
                .is_some_and(|event| event.starts_with("pointerup:pen:0:0:0:0:0:10:11:0:0"))
        }),
        "pen events: {pen_events:?}"
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_cancelled_pointerdown_suppresses_compat_mouse_events() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='target0'>first</div><div id='target1'>second</div><script>window.__classicCompat=[];for(const id of ['target0','target1']){const target=document.getElementById(id);for(const type of ['pointerdown','pointerup','mousedown','mouseup','click']){target.addEventListener(type,function(event){window.__classicCompat.push(type+'@'+id);if(id==='target0'&&type==='pointerdown')event.preventDefault();});}}</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 35 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let compat_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicCompat;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        compat_events,
        json!({
            "value": [
                "pointerdown@target0",
                "pointerup@target0",
                "click@target0",
                "pointerdown@target1",
                "mousedown@target1",
                "pointerup@target1",
                "mouseup@target1",
                "click@target1"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_pointer_capture_routes_real_coordinate_input() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='target0'>first</div><div id='target1'>second</div><script>window.__captureStarted=false;window.__classicCapture=[];for(const id of ['target0','target1']){const target=document.getElementById(id);for(const type of ['pointerdown','gotpointercapture','pointermove','pointerup','lostpointercapture']){target.addEventListener(type,function(event){if(type==='pointermove'&&!window.__captureStarted)return;window.__classicCapture.push(type+'@'+id);if(id==='target0'&&type==='pointerdown'){window.__captureStarted=true;target.setPointerCapture(event.pointerId);window.__classicCapture.push('has:'+target.hasPointerCapture(event.pointerId));}});}}</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 35 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicCapture;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@target0",
                "has:true",
                "gotpointercapture@target0",
                "pointermove@target0",
                "pointerup@target0",
                "lostpointercapture@target0"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_removed_capture_target_retargets_to_document() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='button'>button</div><div id='target0'>capture</div><script>const button=document.getElementById('button');const target0=document.getElementById('target0');window.__classicCaptureRemoval=[];button.addEventListener('pointerdown',event=>{window.__classicCaptureRemoval.push('pointerdown@button');target0.setPointerCapture(event.pointerId);});button.addEventListener('pointerup',()=>window.__classicCaptureRemoval.push('pointerup@button'));target0.addEventListener('gotpointercapture',()=>{window.__classicCaptureRemoval.push('gotpointercapture@target0');target0.remove();});target0.addEventListener('lostpointercapture',()=>window.__classicCaptureRemoval.push('lostpointercapture@target0'));target0.addEventListener('pointerup',()=>window.__classicCaptureRemoval.push('pointerup@target0'));document.addEventListener('lostpointercapture',event=>{if(event.target===document)window.__classicCaptureRemoval.push('lostpointercapture@document');});</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicCaptureRemoval;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@button",
                "gotpointercapture@target0",
                "lostpointercapture@document",
                "pointerup@button"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_active_capture_move_handles_removed_target() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<div id='button'>button</div><div id='target0'>capture</div><script>const button=document.getElementById('button');const target0=document.getElementById('target0');window.__classicActiveRemoval=[];button.addEventListener('pointerdown',event=>{window.__classicActiveRemoval.push('pointerdown@button');target0.setPointerCapture(event.pointerId);});button.addEventListener('pointerup',()=>window.__classicActiveRemoval.push('pointerup@button'));target0.addEventListener('gotpointercapture',()=>window.__classicActiveRemoval.push('gotpointercapture@target0'));target0.addEventListener('pointermove',event=>{window.__classicActiveRemoval.push('pointermove@target0');target0.remove();window.__classicActiveRemoval.push('has:'+target0.hasPointerCapture(event.pointerId));});target0.addEventListener('pointerup',()=>window.__classicActiveRemoval.push('pointerup@target0'));document.addEventListener('lostpointercapture',event=>{if(event.target===document)window.__classicActiveRemoval.push('lostpointercapture@document');});</script>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let capture_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicActiveRemoval;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        capture_events,
        json!({
            "value": [
                "pointerdown@button",
                "gotpointercapture@target0",
                "pointermove@target0",
                "has:false",
                "lostpointercapture@document",
                "pointerup@button"
            ]
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_coordinate_actions_dispatch_after_tick_delay() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicEvents=[];document.addEventListener('mousemove',function(){window.__classicEvents.push({type:'move',t:performance.now()});});document.addEventListener('mousedown',function(){window.__classicEvents.push({type:'down',t:performance.now()});});</script><main style='width:200px;height:200px'>actions</main>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 10, "y": 11, "duration": 100 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicEvents.map(event => event.type).join('|');",
            "args": []
        }),
    )
    .await;
    assert_eq!(events, json!({ "value": "move|down" }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_element_origin_uses_real_geometry() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicWheel=null;document.addEventListener('wheel',function(event){window.__classicWheel={type:event.type,deltaX:event.deltaX,deltaY:event.deltaY,clientX:event.clientX,clientY:event.clientY};});</script><button id='target' onclick='window.__classicClicked=true'>go</button><div id='wheel' style='width:200px;height:200px'>wheel-target</div>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let button = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#target"
        }),
    )
    .await;
    let button_id = button["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("button element reference id");

    let wheel_target = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#wheel"
        }),
    )
    .await;
    let wheel_target_id = wheel_target["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("wheel element reference id");

    let (clicked_status, clicked) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    {
                        "type": "pointerMove",
                        "origin": { "element-6066-11e4-a52e-4f735466cecf": button_id },
                        "x": 0,
                        "y": 0
                    },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(clicked_status, StatusCode::OK, "response: {clicked:?}");
    assert_eq!(clicked, json!({ "value": null }));

    let clicked_state = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return Boolean(window.__classicClicked);",
            "args": []
        }),
    )
    .await;
    assert_eq!(clicked_state, json!({ "value": true }));

    let (scrolled_status, scrolled) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": { "element-6066-11e4-a52e-4f735466cecf": wheel_target_id },
                    "x": 1,
                    "y": 2,
                    "deltaX": 7,
                    "deltaY": 13
                }]
            }]
        }),
    )
    .await;
    assert_eq!(scrolled_status, StatusCode::OK, "response: {scrolled:?}");
    assert_eq!(scrolled, json!({ "value": null }));

    let wheel = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicWheel;",
            "args": []
        }),
    )
    .await;
    assert_eq!(wheel["value"]["type"], json!("wheel"));
    assert_eq!(wheel["value"]["deltaX"], json!(7));
    assert_eq!(wheel["value"]["deltaY"], json!(13));
    assert!(wheel["value"]["clientX"].is_number(), "wheel: {wheel:?}");
    assert!(wheel["value"]["clientY"].is_number(), "wheel: {wheel:?}");

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_release_actions_clear_pressed_sources() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<script>window.__classicEvents=[];document.addEventListener('mouseup',function(event){window.__classicEvents.push('mouseup:'+event.button+':'+event.clientX+':'+event.clientY);});document.addEventListener('keyup',function(event){window.__classicEvents.push('keyup:'+event.key);});</script><input id='field' value=''>";

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#field"
        }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("element reference id");

    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(clicked, json!({ "value": null }));

    let reset_events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "window.__classicEvents=[]; return true;",
            "args": []
        }),
    )
    .await;
    assert_eq!(reset_events, json!({ "value": true }));

    let (actions_status, actions) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [
                {
                    "type": "pointer",
                    "id": "mouse",
                    "parameters": { "pointerType": "mouse" },
                    "actions": [
                        { "type": "pointerMove", "origin": "viewport", "x": 12, "y": 13 },
                        { "type": "pointerDown", "button": 0 }
                    ]
                },
                {
                    "type": "key",
                    "id": "keyboard",
                    "actions": [{ "type": "keyDown", "value": "a" }]
                }
            ]
        }),
    )
    .await;
    assert_eq!(actions_status, StatusCode::OK, "response: {actions:?}");
    assert_eq!(actions, json!({ "value": null }));

    let (released_status, released) = classic_request_status_and_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/actions"),
    )
    .await;
    assert_eq!(released_status, StatusCode::OK, "response: {released:?}");
    assert_eq!(released, json!({ "value": null }));

    let events = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicEvents;",
            "args": []
        }),
    )
    .await;
    assert_eq!(events, json!({ "value": ["mouseup:0:12:13", "keyup:a"] }));

    let released_again = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/actions"),
    )
    .await;
    assert_eq!(released_again, json!({ "value": null }));

    let events_after_second_release = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return window.__classicEvents;",
            "args": []
        }),
    )
    .await;
    assert_eq!(events_after_second_release, events);

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_actions_reject_move_target_out_of_bounds() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": "data:text/html,<main>bounds</main>" }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let (status, response) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/actions"),
        json!({
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    { "type": "pointerMove", "origin": "viewport", "x": 5000, "y": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response["value"]["error"],
        json!("move target out of bounds")
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_window_routes_execute_through_devtools_runtime() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");

    let current_window = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    let initial_handle = current_window["value"]
        .as_str()
        .expect("initial window handle")
        .to_owned();
    assert!(!initial_handle.is_empty());

    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    assert_eq!(handles["value"], json!([initial_handle.clone()]));

    let new_window = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/new"),
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = new_window["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();
    assert_ne!(new_handle, initial_handle);
    assert_eq!(new_window["value"]["type"], json!("tab"));

    let current_window = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    assert_eq!(
        current_window["value"],
        json!(initial_handle.clone()),
        "WebDriver New Window must not switch the current window handle"
    );

    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    let handles = handles["value"].as_array().expect("window handles");
    assert!(handles.contains(&json!(initial_handle.clone())));
    assert!(handles.contains(&json!(new_handle.clone())));

    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window"),
        json!({ "handle": new_handle }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let current_window = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    assert_eq!(current_window["value"], json!(new_handle.clone()));

    let defaulted_from_null = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/new"),
        json!({ "type": null }),
    )
    .await;
    let null_type_handle = defaulted_from_null["value"]["handle"]
        .as_str()
        .expect("null type new window handle")
        .to_owned();
    assert_eq!(defaulted_from_null["value"]["type"], json!("tab"));

    let defaulted_from_unknown = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/new"),
        json!({ "type": "popup" }),
    )
    .await;
    let unknown_type_handle = defaulted_from_unknown["value"]["handle"]
        .as_str()
        .expect("unknown type new window handle")
        .to_owned();
    assert_eq!(defaulted_from_unknown["value"]["type"], json!("tab"));

    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    let handles = handles["value"].as_array().expect("window handles");
    assert!(handles.contains(&json!(null_type_handle)));
    assert!(handles.contains(&json!(unknown_type_handle)));

    let (invalid_handle_status, invalid_handle) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window"),
        json!({ "handle": false }),
    )
    .await;
    assert_eq!(invalid_handle_status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_handle["value"]["error"], json!("invalid argument"));

    let (missing_handle_status, missing_handle) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window"),
        json!({ "handle": "missing-target" }),
    )
    .await;
    assert_eq!(missing_handle_status, StatusCode::NOT_FOUND);
    assert_eq!(missing_handle["value"]["error"], json!("no such window"));

    let remaining_handles = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/window"),
    )
    .await;
    let remaining_handles = remaining_handles["value"]
        .as_array()
        .expect("remaining window handles");
    assert!(remaining_handles.contains(&json!(initial_handle.clone())));
    assert!(!remaining_handles.contains(&json!(new_handle)));
    assert!(remaining_handles.contains(&json!(null_type_handle)));
    assert!(remaining_handles.contains(&json!(unknown_type_handle)));

    let (current_window_status, current_window) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window"),
    )
    .await;
    assert_eq!(current_window_status, StatusCode::OK);
    assert_eq!(current_window["value"], remaining_handles[0]);

    let switched_after_close = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window"),
        json!({ "handle": initial_handle }),
    )
    .await;
    assert_eq!(switched_after_close, json!({ "value": null }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_service_worker_projection_does_not_pollute_window_handles() {
    let (fixture_addr, _fixture_server) = spawn_classic_service_worker_fixture_server();
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = format!("http://{fixture_addr}/");
    let script_url = format!("http://{fixture_addr}/service-worker.js");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let registered = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": r#"
                const registration = await navigator.serviceWorker.register('/service-worker.js');
                await navigator.serviceWorker.ready;
                return registration.active && registration.active.scriptURL;
            "#,
            "args": []
        }),
    )
    .await;
    assert_eq!(registered["value"], json!(script_url));

    let service_workers = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/moli/service-workers"),
    )
    .await;
    let targets = service_workers["value"]["targets"]
        .as_array()
        .expect("service worker targets");
    let target = targets
        .iter()
        .find(|target| target["url"] == json!(script_url))
        .unwrap_or_else(|| panic!("expected service worker target: {service_workers}"));
    assert_eq!(target["type"], json!("service_worker"));
    assert_eq!(target["attached"], json!(false));
    let target_id = target["targetId"]
        .as_str()
        .expect("service worker target id");

    let realms = service_workers["value"]["realms"]
        .as_array()
        .expect("service worker realms");
    assert!(
        realms.is_empty(),
        "Classic Service Worker projection must not synthesize realms before real Runtime.executionContextCreated: {service_workers}"
    );

    let logs = service_workers["value"]["logs"]
        .as_array()
        .expect("service worker logs");
    let boot_log = logs
        .iter()
        .find(|entry| {
            entry["targetId"] == json!(target_id)
                && entry["type"] == json!("log")
                && entry["text"] == json!("classic-service-worker-log")
        })
        .unwrap_or_else(|| panic!("expected service worker log projection: {service_workers}"));
    assert!(
        boot_log.get("executionContextId").is_none(),
        "service worker log must not expose a synthetic executionContextId before real Runtime.executionContextCreated: {boot_log}"
    );

    let service_workers_after_log_drain = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/moli/service-workers"),
    )
    .await;
    assert!(
        service_workers_after_log_drain["value"]["logs"]
            .as_array()
            .expect("service worker logs after drain")
            .iter()
            .all(|entry| entry["text"] != json!("classic-service-worker-log")),
        "service worker logs should not be duplicated after Classic drains them: {service_workers_after_log_drain}"
    );
    let target_after_log_drain = service_workers_after_log_drain["value"]["targets"]
        .as_array()
        .expect("service worker targets after drain")
        .iter()
        .find(|target| target["targetId"] == json!(target_id))
        .unwrap_or_else(|| {
            panic!(
                "expected service worker target after log drain: {service_workers_after_log_drain}"
            )
        });
    assert_eq!(target_after_log_drain["attached"], json!(false));

    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    let handles = handles["value"].as_array().expect("window handles");
    assert!(
        !handles.contains(&json!(target_id)),
        "service worker target must not be exposed as a Classic window handle"
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_shared_worker_reuses_instance_and_does_not_pollute_window_handles() {
    let (fixture_addr, _fixture_server) =
        spawn_shared_worker_fixture_server("classic-shared-worker");
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let handles_path = format!("/session/{session_id}/window/handles");

    let initial_handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let initial_handles = initial_handles["value"]
        .as_array()
        .expect("initial window handles")
        .clone();
    let page_url = format!("http://{fixture_addr}/");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let connected = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": r#"
                return await new Promise((resolve, reject) => {
                    const timer = setTimeout(() => reject(new Error('shared worker timeout')), 1000);
                    const worker = new SharedWorker('/shared-worker.js', 'classic-shared-worker-smoke');
                    globalThis.__classicSharedWorkerSmoke = worker;
                    worker.port.onmessage = event => {
                        if (event.data && event.data.kind === 'probe-result') {
                            clearTimeout(timer);
                            resolve(event.data);
                        }
                    };
                    worker.port.start();
                    worker.port.postMessage({ kind: 'probe', value: 'classic' });
                });
            "#,
            "args": []
        }),
    )
    .await;
    assert_eq!(
        connected["value"],
        json!({
            "kind": "probe-result",
            "echoed": "classic",
            "name": "classic-shared-worker-smoke",
            "pathname": "/shared-worker.js",
            "isSharedWorker": true,
            "selfEqualsGlobal": true,
            "connectCount": 1,
        })
    );

    let reconnected = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": r#"
                return await new Promise((resolve, reject) => {
                    const timer = setTimeout(() => reject(new Error('shared worker reconnect timeout')), 1000);
                    const worker = new SharedWorker('/shared-worker.js', 'classic-shared-worker-smoke');
                    globalThis.__classicSharedWorkerSmokeSecond = worker;
                    worker.port.onmessage = event => {
                        if (event.data && event.data.kind === 'probe-result') {
                            clearTimeout(timer);
                            resolve(event.data);
                        }
                    };
                    worker.port.start();
                    worker.port.postMessage({ kind: 'probe', value: 'classic-second' });
                });
            "#,
            "args": []
        }),
    )
    .await;
    assert_eq!(
        reconnected["value"],
        json!({
            "kind": "probe-result",
            "echoed": "classic-second",
            "name": "classic-shared-worker-smoke",
            "pathname": "/shared-worker.js",
            "isSharedWorker": true,
            "selfEqualsGlobal": true,
            "connectCount": 2,
        })
    );

    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handles = handles["value"].as_array().expect("window handles");
    assert_eq!(
        handles, &initial_handles,
        "shared worker target must not be exposed as a Classic window handle after repeated connects"
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_new_window_argument_edges_ported_from_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/new_window/new.py
    // null body and invalid type cases.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let new_window_path = format!("/session/{session_id}/window/new");

    let (empty_body_status, empty_body) =
        classic_request_status_and_json(app.clone(), Method::POST, &new_window_path).await;
    assert_eq!(empty_body_status, StatusCode::BAD_REQUEST);
    assert_eq!(empty_body["value"]["error"], json!("invalid argument"));

    let (null_body_status, null_body) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!(null),
    )
    .await;
    assert_eq!(null_body_status, StatusCode::BAD_REQUEST);
    assert_eq!(null_body["value"]["error"], json!("invalid argument"));

    for invalid_type in [json!(true), json!(42), json!(4.2), json!([]), json!({})] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &new_window_path,
            json!({ "type": invalid_type }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["value"]["error"], json!("invalid argument"));
    }

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_window_rect_cases_ported_from_wpt_and_selenium() {
    // Ported from WPT webdriver/tests/classic/get_window_rect/get.py,
    // webdriver/tests/classic/set_window_rect/set.py, and Selenium
    // py/test/selenium/webdriver/common/window_tests.py.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let rect_path = format!("/session/{session_id}/window/rect");
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<!doctype html><title>window rect</title>") }),
    )
    .await;

    let initial = classic_request_json(app.clone(), Method::GET, &rect_path).await;
    assert_eq!(initial["value"]["x"], json!(0));
    assert_eq!(initial["value"]["y"], json!(0));
    assert!(
        initial["value"]["width"]
            .as_u64()
            .is_some_and(|width| width > 0),
        "initial window rect should expose a positive width: {initial:?}"
    );
    assert!(
        initial["value"]["height"]
            .as_u64()
            .is_some_and(|height| height > 0),
        "initial window rect should expose a positive height: {initial:?}"
    );

    let unchanged = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &rect_path,
        json!({ "x": null, "y": null, "width": null, "height": null }),
    )
    .await;
    assert_eq!(unchanged, initial);

    let resized = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &rect_path,
        json!({
            "x": 150.5,
            "y": -8.9,
            "width": 650.5,
            "height": 420
        }),
    )
    .await;
    assert_eq!(
        resized["value"],
        json!({
            "x": 150,
            "y": -8,
            "width": 650,
            "height": 420
        })
    );
    let read_back = classic_request_json(app.clone(), Method::GET, &rect_path).await;
    assert_eq!(read_back, resized);

    let visible_surface = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return JSON.stringify({ innerWidth, innerHeight, outerWidth, outerHeight });",
            "args": []
        }),
    )
    .await;
    let visible_surface: serde_json::Value = serde_json::from_str(
        visible_surface["value"]
            .as_str()
            .expect("script should return JSON string"),
    )
    .expect("viewport JSON");
    assert_eq!(visible_surface["innerWidth"], json!(650));
    assert_eq!(visible_surface["innerHeight"], json!(420));
    assert_eq!(visible_surface["outerWidth"], json!(650));
    assert_eq!(visible_surface["outerHeight"], json!(420));

    let partial = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &rect_path,
        json!({ "height": 421 }),
    )
    .await;
    assert_eq!(
        partial["value"],
        json!({
            "x": 150,
            "y": -8,
            "width": 650,
            "height": 421
        })
    );

    for invalid in [
        json!(null),
        json!({ "width": "650" }),
        json!({ "x": false }),
        json!({ "width": -1 }),
        json!({ "height": 0 }),
    ] {
        let (status, response) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &rect_path,
            invalid,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["value"]["error"], json!("invalid argument"));
    }

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_window_state_routes_use_headless_viewport_contract() {
    // Ported from WPT webdriver/tests/classic/maximize_window,
    // fullscreen_window, minimize_window, with Moli's lightweight
    // headless window model.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let rect_path = format!("/session/{session_id}/window/rect");
    let maximize_path = format!("/session/{session_id}/window/maximize");
    let minimize_path = format!("/session/{session_id}/window/minimize");
    let fullscreen_path = format!("/session/{session_id}/window/fullscreen");
    let execute_path = format!("/session/{session_id}/execute/sync");
    let _ = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": classic_data_url("<!doctype html><title>window state</title>") }),
    )
    .await;

    let small = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &rect_path,
        json!({
            "x": 7,
            "y": 9,
            "width": 640,
            "height": 480
        }),
    )
    .await;
    assert_eq!(
        small["value"],
        json!({
            "x": 7,
            "y": 9,
            "width": 640,
            "height": 480
        })
    );

    let maximized = classic_request_json(app.clone(), Method::POST, &maximize_path).await;
    assert_eq!(
        maximized["value"],
        json!({
            "x": 0,
            "y": 0,
            "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
        })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &rect_path).await,
        maximized
    );
    let maximized_again = classic_request_json(app.clone(), Method::POST, &maximize_path).await;
    assert_eq!(maximized_again, maximized);

    let maximize_surface = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return JSON.stringify({ innerWidth, innerHeight, outerWidth, outerHeight, screenWidth: screen.width, screenHeight: screen.height, availWidth: screen.availWidth, availHeight: screen.availHeight, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, fullScreen: window.fullScreen, webkitIsFullScreen: document.webkitIsFullScreen });",
            "args": []
        }),
    )
    .await;
    let maximize_surface: serde_json::Value = serde_json::from_str(
        maximize_surface["value"]
            .as_str()
            .expect("script should return JSON string"),
    )
    .expect("maximize surface JSON");
    assert_eq!(
        maximize_surface,
        json!({
            "innerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "innerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
            "outerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "outerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
            "screenWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "screenHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
            "availWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "availHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible",
            "fullScreen": false,
            "webkitIsFullScreen": false,
        })
    );

    let minimized = classic_request_json(app.clone(), Method::POST, &minimize_path).await;
    assert_eq!(
        minimized, maximized,
        "minimize preserves the current restore rect in Moli's headless window model"
    );
    let minimized_surface = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, fullScreen: window.fullScreen, webkitIsFullScreen: document.webkitIsFullScreen });",
            "args": []
        }),
    )
    .await;
    let minimized_surface: serde_json::Value = serde_json::from_str(
        minimized_surface["value"]
            .as_str()
            .expect("script should return JSON string"),
    )
    .expect("minimize surface JSON");
    assert_eq!(
        minimized_surface,
        json!({
            "hasFocus": false,
            "hidden": true,
            "visibilityState": "hidden",
            "fullScreen": false,
            "webkitIsFullScreen": false,
        })
    );

    let fullscreen = classic_request_json(app.clone(), Method::POST, &fullscreen_path).await;
    assert_eq!(
        fullscreen["value"],
        json!({
            "x": 0,
            "y": 0,
            "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
        })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &rect_path).await,
        fullscreen
    );
    let fullscreen_surface = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return JSON.stringify({ innerWidth, innerHeight, outerWidth, outerHeight, screenWidth: screen.width, screenHeight: screen.height, availWidth: screen.availWidth, availHeight: screen.availHeight, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, fullScreen: window.fullScreen, webkitIsFullScreen: document.webkitIsFullScreen });",
            "args": []
        }),
    )
    .await;
    let fullscreen_surface: serde_json::Value = serde_json::from_str(
        fullscreen_surface["value"]
            .as_str()
            .expect("script should return JSON string"),
    )
    .expect("fullscreen surface JSON");
    assert_eq!(
        fullscreen_surface,
        json!({
            "innerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "innerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
            "outerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "outerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
            "screenWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "screenHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
            "availWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "availHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible",
            "fullScreen": true,
            "webkitIsFullScreen": true,
        })
    );

    let restored_from_fullscreen = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &rect_path,
        json!({
            "x": 15,
            "y": 25,
            "width": 800,
            "height": 600
        }),
    )
    .await;
    assert_eq!(
        restored_from_fullscreen["value"],
        json!({
            "x": 15,
            "y": 25,
            "width": 800,
            "height": 600
        })
    );
    let restored_surface = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return JSON.stringify({ innerWidth, innerHeight, outerWidth, outerHeight, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, fullScreen: window.fullScreen, webkitIsFullScreen: document.webkitIsFullScreen });",
            "args": []
        }),
    )
    .await;
    let restored_surface: serde_json::Value = serde_json::from_str(
        restored_surface["value"]
            .as_str()
            .expect("script should return JSON string"),
    )
    .expect("restored surface JSON");
    assert_eq!(
        restored_surface,
        json!({
            "innerWidth": 800,
            "innerHeight": 600,
            "outerWidth": 800,
            "outerHeight": 600,
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible",
            "fullScreen": false,
            "webkitIsFullScreen": false,
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[derive(Clone, Copy, Debug)]
enum WindowPromptCommand {
    GetRect,
    SetRect,
    Maximize,
    Minimize,
    Fullscreen,
}

impl WindowPromptCommand {
    fn label(self) -> &'static str {
        match self {
            Self::GetRect => "get window rect",
            Self::SetRect => "set window rect",
            Self::Maximize => "maximize window",
            Self::Minimize => "minimize window",
            Self::Fullscreen => "fullscreen window",
        }
    }

    fn expected_success_value(self) -> serde_json::Value {
        match self {
            Self::GetRect | Self::Minimize => {
                json!({
                    "x": 7,
                    "y": 9,
                    "width": 640,
                    "height": 480,
                })
            }
            Self::SetRect => {
                json!({
                    "x": 11,
                    "y": 13,
                    "width": 650,
                    "height": 490,
                })
            }
            Self::Maximize => {
                json!({
                    "x": 0,
                    "y": 0,
                    "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                    "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
                })
            }
            Self::Fullscreen => {
                json!({
                    "x": 0,
                    "y": 0,
                    "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                    "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
                })
            }
        }
    }

    fn expected_success_surface(self) -> serde_json::Value {
        match self {
            Self::Minimize => json!({
                "innerWidth": 640,
                "innerHeight": 480,
                "hidden": true,
                "visibilityState": "hidden",
                "fullScreen": false,
                "webkitIsFullScreen": false,
            }),
            Self::Fullscreen => json!({
                "innerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                "innerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
                "hidden": false,
                "visibilityState": "visible",
                "fullScreen": true,
                "webkitIsFullScreen": true,
            }),
            Self::Maximize => json!({
                "innerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                "innerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
                "hidden": false,
                "visibilityState": "visible",
                "fullScreen": false,
                "webkitIsFullScreen": false,
            }),
            Self::GetRect | Self::SetRect => json!({
                "innerWidth": self.expected_success_value()["width"],
                "innerHeight": self.expected_success_value()["height"],
                "hidden": false,
                "visibilityState": "visible",
                "fullScreen": false,
                "webkitIsFullScreen": false,
            }),
        }
    }
}

async fn assert_window_prompt_command_matches_chromium_wpt(command: WindowPromptCommand) {
    let app = build_router(test_state());

    struct WindowPromptCase {
        capability: Option<serde_json::Value>,
        dialog_script: &'static str,
        expect_notify: bool,
        expect_closed: bool,
    }

    let prompt_cases = [
        WindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("accept and notify")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss and notify")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        WindowPromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        WindowPromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        WindowPromptCase {
            capability: None,
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
    ];

    async fn window_surface(app: Router, session_id: &str) -> serde_json::Value {
        let response = classic_request_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return JSON.stringify({ innerWidth, innerHeight, hidden: document.hidden, visibilityState: document.visibilityState, fullScreen: window.fullScreen, webkitIsFullScreen: document.webkitIsFullScreen });",
                "args": []
            }),
        )
        .await;
        serde_json::from_str(
            response["value"]
                .as_str()
                .expect("window surface should be JSON string"),
        )
        .expect("window surface JSON")
    }

    async fn run_window_prompt_command(
        app: Router,
        session_id: &str,
        command: WindowPromptCommand,
    ) -> (StatusCode, serde_json::Value) {
        match command {
            WindowPromptCommand::GetRect => {
                classic_request_status_and_json(
                    app,
                    Method::GET,
                    &format!("/session/{session_id}/window/rect"),
                )
                .await
            }
            WindowPromptCommand::SetRect => {
                classic_request_status_and_json_with_body(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/rect"),
                    json!({
                        "x": 11,
                        "y": 13,
                        "width": 650,
                        "height": 490,
                    }),
                )
                .await
            }
            WindowPromptCommand::Maximize => {
                classic_request_status_and_json(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/maximize"),
                )
                .await
            }
            WindowPromptCommand::Minimize => {
                classic_request_status_and_json(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/minimize"),
                )
                .await
            }
            WindowPromptCommand::Fullscreen => {
                classic_request_status_and_json(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/fullscreen"),
                )
                .await
            }
        }
    }

    for case in &prompt_cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({ "url": classic_data_url("<!doctype html><title>window prompt</title>") }),
            )
            .await,
            json!({ "value": null })
        );
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/window/rect"),
                json!({
                    "x": 7,
                    "y": 9,
                    "width": 640,
                    "height": 480,
                }),
            )
            .await,
            json!({ "value": {
                "x": 7,
                "y": 9,
                "width": 640,
                "height": 480,
            }})
        );
        let original_rect = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/window/rect"),
        )
        .await;
        let original_surface = window_surface(app.clone(), session_id).await;

        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let (status, response) = run_window_prompt_command(app.clone(), session_id, command).await;
        if case.expect_notify {
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "{} capability {:?} response {response:?}",
                command.label(),
                case.capability
            );
            assert_eq!(response["value"]["error"], json!("unexpected alert open"));
            assert_eq!(response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                status,
                StatusCode::OK,
                "{} capability {:?} response {response:?}",
                command.label(),
                case.capability
            );
            assert_eq!(response["value"], command.expected_success_value());
        }

        let alert_text_path = format!("/session/{session_id}/alert/text");
        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let final_rect = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/window/rect"),
        )
        .await;
        let final_surface = window_surface(app.clone(), session_id).await;
        if case.expect_notify {
            assert_eq!(
                final_rect,
                original_rect,
                "{} should not run after notify preflight",
                command.label()
            );
            assert_eq!(
                final_surface,
                original_surface,
                "{} should preserve surface after notify preflight",
                command.label()
            );
        } else {
            assert_eq!(final_rect["value"], command.expected_success_value());
            assert_eq!(final_surface, command.expected_success_surface());
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn webdriver_classic_get_window_rect_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/get_window_rect/user_prompts.py.
    assert_window_prompt_command_matches_chromium_wpt(WindowPromptCommand::GetRect).await;
}

#[tokio::test]
async fn webdriver_classic_set_window_rect_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/set_window_rect/user_prompts.py.
    assert_window_prompt_command_matches_chromium_wpt(WindowPromptCommand::SetRect).await;
}

#[tokio::test]
async fn webdriver_classic_maximize_window_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/maximize_window/user_prompts.py.
    assert_window_prompt_command_matches_chromium_wpt(WindowPromptCommand::Maximize).await;
}

#[tokio::test]
async fn webdriver_classic_minimize_window_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/minimize_window/user_prompts.py.
    assert_window_prompt_command_matches_chromium_wpt(WindowPromptCommand::Minimize).await;
}

#[tokio::test]
async fn webdriver_classic_fullscreen_window_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/fullscreen_window/user_prompts.py.
    assert_window_prompt_command_matches_chromium_wpt(WindowPromptCommand::Fullscreen).await;
}

#[tokio::test]
async fn webdriver_classic_window_rect_state_ignore_detached_current_frame() {
    // Ported from Chromium/WPT webdriver/tests/classic/get_window_rect/get.py,
    // set_window_rect/set.py, maximize_window/maximize.py,
    // minimize_window/minimize.py, and fullscreen_window/fullscreen.py
    // test_no_browsing_context cases.
    let app = build_router(test_state());
    let (addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = format!("http://{addr}/page");
    classic_switch_to_child_frame_and_remove_current_frame(app.clone(), session_id, &page_url)
        .await;

    let rect_path = format!("/session/{session_id}/window/rect");
    let initial = classic_request_json(app.clone(), Method::GET, &rect_path).await;
    assert_eq!(initial["value"]["x"], json!(0));
    assert_eq!(initial["value"]["y"], json!(0));
    assert!(
        initial["value"]["width"]
            .as_u64()
            .is_some_and(|width| width > 0),
        "detached current frame should not block get rect: {initial:?}"
    );
    assert!(
        initial["value"]["height"]
            .as_u64()
            .is_some_and(|height| height > 0),
        "detached current frame should not block get rect: {initial:?}"
    );

    let resized = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &rect_path,
        json!({
            "x": 21,
            "y": 22,
            "width": 700,
            "height": 500,
        }),
    )
    .await;
    assert_eq!(
        resized["value"],
        json!({
            "x": 21,
            "y": 22,
            "width": 700,
            "height": 500,
        })
    );

    let maximized = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/maximize"),
    )
    .await;
    assert_eq!(
        maximized["value"],
        json!({
            "x": 0,
            "y": 0,
            "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
        })
    );

    let minimized = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/minimize"),
    )
    .await;
    assert_eq!(minimized, maximized);

    let fullscreen = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/fullscreen"),
    )
    .await;
    assert_eq!(
        fullscreen["value"],
        json!({
            "x": 0,
            "y": 0,
            "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
            "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
        })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_new_window_matches_wpt_tab_payload_and_context_semantics() {
    // Ported from Chromium/WPT webdriver/tests/classic/new_window/new.py and
    // webdriver/tests/classic/new_window/new_tab.py.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let new_window_path = format!("/session/{session_id}/window/new");
    let url_path = format!("/session/{session_id}/url");
    let execute_path = format!("/session/{session_id}/execute/sync");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();
    let original_handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    assert_eq!(original_handles["value"], json!([original_handle.clone()]));

    let original_url = classic_data_url("<p>foo</p>");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &url_path,
        json!({ "url": original_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(created["value"]["type"], json!("tab"));
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new tab handle")
        .to_owned();
    assert_ne!(new_handle, original_handle);

    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handles = handles["value"].as_array().expect("window handles");
    assert_eq!(handles.len(), 2);
    assert!(handles.contains(&json!(original_handle.clone())));
    assert!(handles.contains(&json!(new_handle.clone())));

    let current_window = classic_request_json(app.clone(), Method::GET, &window_path).await;
    assert_eq!(
        current_window["value"],
        json!(original_handle),
        "New Window must not switch the selected top-level browsing context"
    );
    let current_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(current_url["value"], json!(original_url));

    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let new_context_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(new_context_url, json!({ "value": "about:blank" }));

    let (window_name_status, window_name) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return window.name;",
            "args": []
        }),
    )
    .await;
    assert_eq!(window_name_status, StatusCode::OK, "{window_name:?}");
    assert_eq!(window_name, json!({ "value": "" }));

    let opener = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return window.opener;",
            "args": []
        }),
    )
    .await;
    assert_eq!(opener, json!({ "value": null }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_new_window_matches_wpt_window_payload_and_context_semantics() {
    // Ported from Chromium/WPT webdriver/tests/classic/new_window/new_window.py.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let new_window_path = format!("/session/{session_id}/window/new");
    let url_path = format!("/session/{session_id}/url");
    let execute_path = format!("/session/{session_id}/execute/sync");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();

    let original_url = classic_data_url("<p>foo</p>");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &url_path,
        json!({ "url": original_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "window" }),
    )
    .await;
    assert_eq!(created["value"]["type"], json!("window"));
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();
    assert_ne!(new_handle, original_handle);

    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handles = handles["value"].as_array().expect("window handles");
    assert_eq!(handles.len(), 2);
    assert!(handles.contains(&json!(original_handle.clone())));
    assert!(handles.contains(&json!(new_handle.clone())));

    let current_window = classic_request_json(app.clone(), Method::GET, &window_path).await;
    assert_eq!(
        current_window["value"],
        json!(original_handle),
        "New Window must not switch the selected top-level browsing context"
    );
    let current_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(current_url["value"], json!(original_url));

    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let new_context_url = classic_request_json(app.clone(), Method::GET, &url_path).await;
    assert_eq!(new_context_url, json!({ "value": "about:blank" }));

    let window_name = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return window.name;",
            "args": []
        }),
    )
    .await;
    assert_eq!(window_name, json!({ "value": "" }));

    let opener = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return window.opener;",
            "args": []
        }),
    )
    .await;
    assert_eq!(opener, json!({ "value": null }));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_switch_and_close_window_match_wpt_state_transitions() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_window/switch.py
    // and webdriver/tests/classic/close_window/close.py.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let new_window_path = format!("/session/{session_id}/window/new");

    let (null_body_status, null_body) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!(null),
    )
    .await;
    assert_eq!(null_body_status, StatusCode::BAD_REQUEST);
    assert_eq!(null_body["value"]["error"], json!("invalid argument"));

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();

    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle.clone() }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    let current_window = classic_request_json(app.clone(), Method::GET, &window_path).await;
    assert_eq!(current_window["value"], json!(new_handle.clone()));

    let remaining = classic_request_json(app.clone(), Method::DELETE, &window_path).await;
    assert_eq!(remaining["value"], json!([original_handle.clone()]));
    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    assert_eq!(handles["value"], json!([original_handle.clone()]));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &window_path).await,
        json!({ "value": original_handle.clone() })
    );

    let closed_last = classic_request_json(app.clone(), Method::DELETE, &window_path).await;
    assert_eq!(closed_last, json!({ "value": [] }));
    let (missing_session_status, missing_session) =
        classic_request_status_and_json(app, Method::GET, &handles_path).await;
    assert_eq!(missing_session_status, StatusCode::NOT_FOUND);
    assert_eq!(
        missing_session["value"]["error"],
        json!("invalid session id")
    );
}

#[tokio::test]
async fn webdriver_classic_switch_window_succeeds_after_current_top_level_is_closed() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_window/switch.py
    // test_no_top_browsing_context.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let new_window_path = format!("/session/{session_id}/window/new");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();
    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();

    let remaining = classic_request_json(app.clone(), Method::DELETE, &window_path).await;
    let remaining = remaining["value"].as_array().expect("remaining handles");
    assert!(!remaining.contains(&json!(original_handle)));
    assert!(remaining.contains(&json!(new_handle.clone())));

    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle.clone() }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &window_path).await,
        json!({ "value": new_handle.clone() })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &handles_path).await,
        json!({ "value": [new_handle.clone()] })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_switch_window_succeeds_from_detached_current_frame() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_window/switch.py
    // test_no_browsing_context.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let new_window_path = format!("/session/{session_id}/window/new");

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();

    let page_url = format!("http://{fixture_addr}/page");
    classic_switch_to_child_frame_and_remove_current_frame(app.clone(), session_id, &page_url)
        .await;

    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle.clone() }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &window_path).await,
        json!({ "value": new_handle.clone() })
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_window_handle_and_new_window_ignore_detached_current_frame() {
    // Ported from Chromium/WPT webdriver/tests/classic/get_window_handle/get.py,
    // get_window_handles/get.py, and new_window/new.py no_browsing_context cases.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let new_window_path = format!("/session/{session_id}/window/new");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();

    let page_url = format!("http://{fixture_addr}/page");
    classic_switch_to_child_frame_and_remove_current_frame(app.clone(), session_id, &page_url)
        .await;

    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &window_path).await,
        json!({ "value": original_handle.clone() })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &handles_path).await,
        json!({ "value": [original_handle.clone()] })
    );

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": null }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();
    assert_ne!(new_handle, original_handle);
    assert_eq!(created["value"]["type"], json!("tab"));

    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handles = handles["value"].as_array().expect("handles");
    assert_eq!(handles.len(), 2);
    assert!(handles.contains(&json!(original_handle)));
    assert!(handles.contains(&json!(new_handle)));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_close_window_succeeds_from_detached_current_frame() {
    // Ported from Chromium/WPT webdriver/tests/classic/close_window/close.py
    // test_no_browsing_context.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let new_window_path = format!("/session/{session_id}/window/new");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();
    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();

    let page_url = format!("http://{fixture_addr}/page");
    classic_switch_to_child_frame_and_remove_current_frame(app.clone(), session_id, &page_url)
        .await;

    let remaining = classic_request_json(app.clone(), Method::DELETE, &window_path).await;
    assert_eq!(remaining, json!({ "value": [new_handle.clone()] }));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &handles_path).await,
        json!({ "value": [new_handle.clone()] })
    );
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &window_path).await,
        json!({ "value": new_handle.clone() })
    );

    let switched_after_close = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle.clone() }),
    )
    .await;
    assert_eq!(switched_after_close, json!({ "value": null }));

    assert_ne!(new_handle, original_handle);
    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_switch_window_resets_current_frame_to_top_level_context() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_window/switch.py
    // test_switch_to_window_sets_top_level_context.
    let app = build_router(test_state());
    let (fixture_addr, fixture_server) = spawn_classic_frame_fixture_server().await;

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let frame_path = format!("/session/{session_id}/frame");

    let page_url = format!("http://{fixture_addr}/page");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let current_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let current_handle = current_handle["value"]
        .as_str()
        .expect("current window handle")
        .to_owned();

    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched_to_frame = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &frame_path,
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id,
            }
        }),
    )
    .await;
    assert_eq!(switched_to_frame, json!({ "value": null }));
    let _inside_frame = classic_find_css_element_id(app.clone(), session_id, "#inside-frame").await;

    let switched_to_same_window = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": current_handle }),
    )
    .await;
    assert_eq!(switched_to_same_window, json!({ "value": null }));

    let top_element = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": "#top-main"
        }),
    )
    .await;
    assert!(
        top_element["value"][CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .is_some(),
        "switching to a window handle should restore the top-level context: {top_element:?}"
    );

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

#[tokio::test]
async fn webdriver_classic_element_reference_is_not_found_after_tab_switch() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_window/switch.py
    // test_element_not_found_after_tab_switch.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let new_window_path = format!("/session/{session_id}/window/new");

    let page_url = classic_data_url("<p id='a'>foo</p>");
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let paragraph_id = classic_find_css_element_id(app.clone(), session_id, "p").await;

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": new_handle }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let (attribute_status, attribute) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/element/{paragraph_id}/attribute/id"),
    )
    .await;
    assert_eq!(attribute_status, StatusCode::NOT_FOUND, "{attribute:?}");
    assert_eq!(attribute["value"]["error"], json!("no such element"));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_switch_window_keeps_user_prompt_on_original_context() {
    // Ported from Chromium/WPT webdriver/tests/classic/switch_to_window/switch.py
    // test_finds_exising_user_prompt_after_tab_switch.
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let new_window_path = format!("/session/{session_id}/window/new");
    let alert_text_path = format!("/session/{session_id}/alert/text");
    let alert_accept_path = format!("/session/{session_id}/alert/accept");

    let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original_handle = original_handle["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();
    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &new_window_path,
        json!({ "type": "tab" }),
    )
    .await;
    let new_handle = created["value"]["handle"]
        .as_str()
        .expect("new window handle")
        .to_owned();

    for dialog_type in ["alert", "confirm", "prompt"] {
        let dialog_script =
            format!("setTimeout(() => {{ {dialog_type}('foo'); }}, 0); return 'opened';");
        // This Chromium sequence intentionally leaves the new target's ordinary
        // lifecycle work queued. The timeout is only a test liveness guard, not
        // a WebDriver timing contract; allow full-workspace CPU contention while
        // retaining the exact dialog text and owner assertions below.
        classic_open_dialog_and_wait_with_timeout(
            app.clone(),
            session_id,
            &dialog_script,
            "foo",
            std::time::Duration::from_secs(5),
        )
        .await;
        assert_eq!(
            classic_request_json(app.clone(), Method::GET, &window_path).await,
            json!({ "value": original_handle }),
            "getting the current window should not handle an open {dialog_type}"
        );
        let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
        assert!(
            handles["value"]
                .as_array()
                .unwrap()
                .contains(&json!(original_handle)),
            "window handles should still include the prompted original window: {handles:?}"
        );
        assert!(
            handles["value"]
                .as_array()
                .unwrap()
                .contains(&json!(new_handle)),
            "window handles should still include the target window: {handles:?}"
        );
        assert_eq!(
            classic_request_json(app.clone(), Method::GET, &alert_text_path).await,
            json!({ "value": "foo" }),
            "window handle commands should leave the original {dialog_type} open"
        );

        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                classic_request_json_with_body(
                    app.clone(),
                    Method::POST,
                    &window_path,
                    json!({ "handle": new_handle }),
                ),
            )
            .await
            .expect("switching away from prompted window should complete"),
            json!({ "value": null }),
            "switching away from the prompted window should succeed for {dialog_type}"
        );
        let (missing_status, missing) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        assert_eq!(missing_status, StatusCode::NOT_FOUND, "{missing:?}");
        assert_eq!(missing["value"]["error"], json!("no such alert"));

        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                classic_request_json_with_body(
                    app.clone(),
                    Method::POST,
                    &window_path,
                    json!({ "handle": original_handle }),
                ),
            )
            .await
            .expect("switching back to prompted window should complete"),
            json!({ "value": null }),
            "switching back should restore access to the original {dialog_type}"
        );
        assert_eq!(
            classic_request_json(app.clone(), Method::GET, &alert_text_path).await,
            json!({ "value": "foo" })
        );
        assert_eq!(
            classic_request_json(app.clone(), Method::POST, &alert_accept_path).await,
            json!({ "value": null })
        );
    }

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn webdriver_classic_close_window_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/close_window/user_prompts.py
    // for alert/confirm/prompt. beforeunload remains out of scope for
    // Moli's lightweight dialog model here.
    let app = build_router(test_state());

    struct ClosePromptCase {
        capability: Option<serde_json::Value>,
        dialog_script: &'static str,
        expect_notify: bool,
        expect_prompt_closed: bool,
        expect_window_closed: bool,
    }

    let cases = [
        ClosePromptCase {
            capability: None,
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_prompt_closed: true,
            expect_window_closed: false,
        },
        ClosePromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_prompt_closed: true,
            expect_window_closed: true,
        },
        ClosePromptCase {
            capability: Some(json!("accept and notify")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_prompt_closed: true,
            expect_window_closed: false,
        },
        ClosePromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_prompt_closed: true,
            expect_window_closed: true,
        },
        ClosePromptCase {
            capability: Some(json!("dismiss and notify")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_prompt_closed: true,
            expect_window_closed: false,
        },
        ClosePromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_prompt_closed: false,
            expect_window_closed: false,
        },
        ClosePromptCase {
            capability: Some(json!({"default": "accept", "prompt": "ignore"})),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_prompt_closed: false,
            expect_window_closed: false,
        },
    ];

    for case in cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        let window_path = format!("/session/{session_id}/window");
        let handles_path = format!("/session/{session_id}/window/handles");
        let new_window_path = format!("/session/{session_id}/window/new");
        let alert_text_path = format!("/session/{session_id}/alert/text");
        let alert_dismiss_path = format!("/session/{session_id}/alert/dismiss");

        let original_handle = classic_request_json(app.clone(), Method::GET, &window_path).await;
        let original_handle = original_handle["value"]
            .as_str()
            .expect("original window handle")
            .to_owned();
        let created = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &new_window_path,
            json!({ "type": "tab" }),
        )
        .await;
        let new_handle = created["value"]["handle"]
            .as_str()
            .expect("new window handle")
            .to_owned();
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &window_path,
                json!({ "handle": new_handle.clone() }),
            )
            .await,
            json!({ "value": null })
        );
        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let (close_status, close_response) =
            classic_request_status_and_json(app.clone(), Method::DELETE, &window_path).await;
        if case.expect_notify {
            assert_eq!(
                close_status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "capability {:?} response {close_response:?}",
                case.capability
            );
            assert_eq!(
                close_response["value"]["error"],
                json!("unexpected alert open")
            );
            assert_eq!(close_response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                close_status,
                StatusCode::OK,
                "capability {:?} response {close_response:?}",
                case.capability
            );
            assert_eq!(close_response["value"], json!([original_handle.clone()]));
        }

        let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
        let handles = handles["value"].as_array().expect("window handles");
        assert!(handles.contains(&json!(original_handle.clone())));
        assert_eq!(
            handles.contains(&json!(new_handle.clone())),
            !case.expect_window_closed,
            "capability {:?} handles {handles:?}",
            case.capability
        );

        if case.expect_window_closed {
            assert_eq!(
                classic_request_json(app.clone(), Method::GET, &window_path).await,
                json!({ "value": original_handle.clone() }),
                "closed prompt window case should select the remaining original window"
            );
            assert_eq!(
                classic_request_json_with_body(
                    app.clone(),
                    Method::POST,
                    &window_path,
                    json!({ "handle": original_handle.clone() }),
                )
                .await,
                json!({ "value": null }),
                "closed prompt window case should be able to switch back to original"
            );
        } else {
            let current_window = classic_request_json(app.clone(), Method::GET, &window_path).await;
            assert_eq!(current_window["value"], json!(new_handle.clone()));
        }

        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_prompt_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(app.clone(), Method::POST, &alert_dismiss_path).await,
                json!({ "value": null })
            );
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn webdriver_classic_new_window_user_prompt_behavior_matches_chromium_wpt() {
    // Ported from Chromium/WPT webdriver/tests/classic/new_window/user_prompts.py.
    let app = build_router(test_state());

    struct NewWindowPromptCase {
        capability: Option<serde_json::Value>,
        dialog_script: &'static str,
        expect_notify: bool,
        expect_closed: bool,
        expect_created: bool,
    }

    let cases = [
        NewWindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
            expect_created: true,
        },
        NewWindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
            expect_created: true,
        },
        NewWindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
            expect_created: true,
        },
        NewWindowPromptCase {
            capability: Some(json!("accept and notify")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
            expect_created: false,
        },
        NewWindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
            expect_created: true,
        },
        NewWindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
            expect_created: true,
        },
        NewWindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
            expect_created: true,
        },
        NewWindowPromptCase {
            capability: Some(json!("dismiss and notify")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
            expect_created: false,
        },
        NewWindowPromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
            expect_created: false,
        },
        NewWindowPromptCase {
            capability: None,
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
            expect_created: false,
        },
    ];

    for case in cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        let new_window_path = format!("/session/{session_id}/window/new");
        let handles_path = format!("/session/{session_id}/window/handles");
        let alert_text_path = format!("/session/{session_id}/alert/text");

        let original_handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
        let original_handles = original_handles["value"]
            .as_array()
            .expect("original handles")
            .clone();

        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let (new_status, new_window) = classic_request_status_and_json_with_body(
            app.clone(),
            Method::POST,
            &new_window_path,
            json!({ "type": null }),
        )
        .await;
        if case.expect_notify {
            assert_eq!(
                new_status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "capability {:?} response {new_window:?}",
                case.capability
            );
            assert_eq!(new_window["value"]["error"], json!("unexpected alert open"));
            assert_eq!(new_window["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                new_status,
                StatusCode::OK,
                "capability {:?} response {new_window:?}",
                case.capability
            );
            assert!(new_window["value"]["handle"].as_str().is_some());
            assert_eq!(new_window["value"]["type"], json!("tab"));
        }

        let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
        let handles = handles["value"].as_array().expect("handles");
        if case.expect_created {
            assert_eq!(handles.len(), original_handles.len() + 1);
            assert!(
                handles
                    .iter()
                    .any(|handle| !original_handles.contains(handle)),
                "new window should add a handle: {handles:?}"
            );
        } else {
            assert_eq!(handles, &original_handles);
        }

        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn webdriver_classic_cookie_routes_execute_through_devtools_runtime() {
    let (fixture_addr, fixture_server) = spawn_classic_cookie_fixture_server().await;
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let page_url = format!("http://{fixture_addr}/page");

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let added = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/cookie"),
        json!({
            "cookie": {
                "name": "sid",
                "value": "abc",
                "path": "/",
                "httpOnly": true,
                "sameSite": "Lax"
            }
        }),
    )
    .await;
    assert_eq!(added, json!({ "value": null }));

    let cookies = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/cookie"),
    )
    .await;
    let cookies = cookies["value"].as_array().expect("cookies");
    let sid = cookies
        .iter()
        .find(|cookie| cookie["name"] == json!("sid"))
        .expect("sid cookie");
    assert_eq!(sid["value"], json!("abc"));
    assert_eq!(sid["httpOnly"], json!(true));
    assert_eq!(sid["sameSite"], json!("Lax"));

    let named = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/cookie/sid"),
    )
    .await;
    assert_eq!(named["value"]["name"], json!("sid"));
    assert_eq!(named["value"]["value"], json!("abc"));

    let (missing_status, missing) = classic_request_status_and_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/cookie/missing"),
    )
    .await;
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(missing["value"]["error"], json!("no such cookie"));

    let (invalid_status, invalid) = classic_request_status_and_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/cookie"),
        json!({
            "cookie": {
                "name": false,
                "value": "abc"
            }
        }),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid["value"]["error"], json!("invalid argument"));

    let deleted = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/cookie/sid"),
    )
    .await;
    assert_eq!(deleted, json!({ "value": null }));

    let cookies = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/cookie"),
    )
    .await;
    assert_eq!(cookies["value"], json!([]));

    for (name, value) in [("sid", "abc"), ("theme", "dark")] {
        let added = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/cookie"),
            json!({
                "cookie": {
                    "name": name,
                    "value": value,
                    "path": "/"
                }
            }),
        )
        .await;
        assert_eq!(added, json!({ "value": null }));
    }

    let deleted = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}/cookie"),
    )
    .await;
    assert_eq!(deleted, json!({ "value": null }));
    let cookies = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/cookie"),
    )
    .await;
    assert_eq!(cookies["value"], json!([]));

    let _ = classic_request_json(
        app.clone(),
        Method::DELETE,
        &format!("/session/{session_id}"),
    )
    .await;
    fixture_server.abort();
}

fn classic_data_url(html: &str) -> String {
    fn push_hex(encoded: &mut String, byte: u8) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        encoded.push('%');
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }

    let mut encoded = String::with_capacity(html.len());
    for byte in html.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => push_hex(&mut encoded, byte),
        }
    }
    format!("data:text/html;charset=utf-8,{encoded}")
}

async fn classic_open_dialog_and_wait(
    app: Router,
    session_id: &str,
    script: &str,
    expected_text: &str,
) {
    classic_open_dialog_and_wait_with_timeout(
        app,
        session_id,
        script,
        expected_text,
        std::time::Duration::from_secs(1),
    )
    .await;
}

async fn classic_open_dialog_and_wait_with_timeout(
    app: Router,
    session_id: &str,
    script: &str,
    expected_text: &str,
    timeout: std::time::Duration,
) {
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await,
        json!({ "value": "opened" })
    );
    let alert_path = format!("/session/{session_id}/alert/text");
    let alert = tokio::time::timeout(timeout, async {
        loop {
            let (status, response) =
                classic_request_status_and_json(app.clone(), Method::GET, &alert_path).await;
            if status == StatusCode::OK {
                break response;
            }
            assert_eq!(status, StatusCode::NOT_FOUND, "{response:?}");
            assert_eq!(response["value"]["error"], json!("no such alert"));
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("scheduled JavaScript dialog should open: {script}"));
    assert_eq!(alert, json!({ "value": expected_text }));
}

async fn classic_request_json(app: Router, method: Method, path: &str) -> serde_json::Value {
    let (status, value) = classic_request_status_and_json(app, method, path).await;
    assert_eq!(status, StatusCode::OK, "path {path}: {value:?}");
    value
}

async fn spawn_classic_delayed_navigation_fixture_server(
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic delayed navigation fixture server");
    let addr = listener
        .local_addr()
        .expect("classic delayed navigation fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new().route(
            "/slow",
            get(move || async move {
                sleep(delay).await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body>slow navigation</body></html>",
                )
            }),
        );
        axum::serve(listener, app)
            .await
            .expect("classic delayed navigation fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_form_navigation_fixture_server(
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic form navigation fixture server");
    let addr = listener
        .local_addr()
        .expect("classic form navigation fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/form",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>Form Source</title></head>\
                         <body><form action='/submitted'><input name='login' value='moli'></form></body></html>",
                    )
                }),
            )
            .route(
                "/submitted",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>Submitted Target</title></head>\
                         <body><main>submitted</main></body></html>",
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("classic form navigation fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_frame_fixture_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic frame fixture server");
    let addr = listener.local_addr().expect("classic frame fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body data-context='top'><main id='top-main'>top</main><iframe id='child' src='/frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body data-context='child'><main id='inside-frame'>child</main><button id='remove-current-frame' onclick=\"parent.document.getElementById('child').remove()\">remove</button></body></html>",
                    )
                }),
            )
            .route(
                "/nested",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>top nested</title></head><body data-context='top'><main id='top-nested'>top</main><iframe id='outerById' name='outerByName' src='/outer-frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/outer-frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>outer frame</title></head><body data-context='outer'><main id='outer-main'>outer</main><iframe id='innerById' name='innerByName' src='/inner-frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/inner-frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>inner frame</title></head><body data-context='inner'><p id='inner-text'>inner</p></body></html>",
                    )
                }),
            )
            .route(
                "/shadow-page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main id='shadow-top'>top</main><iframe id='shadow-child' src='/shadow-frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/shadow-frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><div id='child-closed-host'></div><script>const root=document.getElementById('child-closed-host').attachShadow({mode:'closed'});root.innerHTML='<span id=\"child-closed-inside\">child closed text</span>';</script></body></html>",
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("classic frame fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_cross_origin_frame_fixture_servers() -> (
    std::net::SocketAddr,
    std::net::SocketAddr,
    std::net::SocketAddr,
    Vec<tokio::task::JoinHandle<()>>,
) {
    let browser_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic browser-origin frame fixture server");
    let alt_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic alt-origin frame fixture server");
    let www_alt_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic www-alt-origin frame fixture server");

    let browser_addr = browser_listener
        .local_addr()
        .expect("classic browser-origin frame fixture addr");
    let alt_addr = alt_listener
        .local_addr()
        .expect("classic alt-origin frame fixture addr");
    let www_alt_addr = www_alt_listener
        .local_addr()
        .expect("classic www-alt-origin frame fixture addr");

    let alt_child_url = format!("http://{alt_addr}/child");
    let browser_middle_url = format!("http://{browser_addr}/middle");
    let www_alt_leaf_url = format!("http://{www_alt_addr}/leaf");

    let browser_app = Router::new()
        .route(
            "/top",
            get({
                let alt_child_url = alt_child_url.clone();
                move || {
                    let alt_child_url = alt_child_url.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            format!(
                                "<!doctype html><html><body><iframe id='cross' src='{alt_child_url}'></iframe></body></html>"
                            ),
                        )
                    }
                }
            }),
        )
        .route(
            "/middle",
            get({
                let www_alt_leaf_url = www_alt_leaf_url.clone();
                move || {
                    let www_alt_leaf_url = www_alt_leaf_url.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            format!(
                                "<!doctype html><html><body data-context='browser-child'><iframe id='to-www-alt' src='{www_alt_leaf_url}'></iframe></body></html>"
                            ),
                        )
                    }
                }
            }),
        );
    let alt_app = Router::new()
        .route(
            "/child",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body data-context='alt-child'>alt child</body></html>",
                )
            }),
        )
        .route(
            "/nested-top",
            get({
                let browser_middle_url = browser_middle_url.clone();
                move || {
                    let browser_middle_url = browser_middle_url.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            format!(
                                "<!doctype html><html><body data-context='alt-top'><iframe id='to-browser' src='{browser_middle_url}'></iframe></body></html>"
                            ),
                        )
                    }
                }
            }),
        );
    let www_alt_app = Router::new().route(
        "/leaf",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body data-context='www-alt-leaf'>www alt leaf</body></html>",
            )
        }),
    );

    let browser_server = tokio::spawn(async move {
        axum::serve(browser_listener, browser_app)
            .await
            .expect("classic browser-origin frame fixture server should serve");
    });
    let alt_server = tokio::spawn(async move {
        axum::serve(alt_listener, alt_app)
            .await
            .expect("classic alt-origin frame fixture server should serve");
    });
    let www_alt_server = tokio::spawn(async move {
        axum::serve(www_alt_listener, www_alt_app)
            .await
            .expect("classic www-alt-origin frame fixture server should serve");
    });

    (
        browser_addr,
        alt_addr,
        www_alt_addr,
        vec![browser_server, alt_server, www_alt_server],
    )
}

async fn spawn_classic_page_load_strategy_fixture_server(
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic page load strategy fixture server");
    let addr = listener
        .local_addr()
        .expect("classic page load strategy fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><script>window.__classicLifecycle=[];document.addEventListener('DOMContentLoaded',()=>{window.__classicLifecycle.push('dcl:'+document.readyState);const script=document.createElement('script');script.src='/runtime-script.js';document.head.appendChild(script);});window.addEventListener('load',()=>window.__classicLifecycle.push('load:'+document.readyState));</script></head><body><main>strategy</main></body></html>",
                    )
                }),
            )
            .route(
                "/runtime-script.js",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "window.__classicLifecycle.push('external:'+document.readyState);",
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("classic page load strategy fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_cookie_fixture_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic cookie fixture server");
    let addr = listener.local_addr().expect("classic cookie fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new().route(
            "/page",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body>classic-cookie</body></html>",
                )
            }),
        );
        axum::serve(listener, app)
            .await
            .expect("classic cookie fixture server should serve");
    });
    (addr, server)
}

fn spawn_classic_service_worker_fixture_server() -> (std::net::SocketAddr, DedicatedFixtureServer) {
    let app = Router::new()
        .route(
            "/",
            get(|| async move {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body>classic service worker</body></html>",
                )
            }),
        )
        .route(
            "/service-worker.js",
            get(|| async move {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "console.log('classic-service-worker-log');\
                     self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));\
                     self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));",
                )
            }),
        );
    spawn_dedicated_fixture_server(app, "classic-service-worker")
}

async fn classic_switch_to_child_frame_and_remove_current_frame(
    app: Router,
    session_id: &str,
    page_url: &str,
) {
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id,
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let removed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const frame = window.frameElement; if (frame) frame.remove(); return frame ? 'removed' : 'missing';",
            "args": []
        }),
    )
    .await;
    assert_eq!(removed, json!({ "value": "removed" }));
}

async fn classic_switch_to_nested_frame_and_remove_parent_frame(
    app: Router,
    session_id: &str,
    page_url: &str,
) {
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let outer_frame_id = classic_find_css_element_id(app.clone(), session_id, "#outerById").await;
    let switched_outer = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: outer_frame_id,
            }
        }),
    )
    .await;
    assert_eq!(switched_outer, json!({ "value": null }));

    let inner_frame_id = classic_find_css_element_id(app.clone(), session_id, "#innerById").await;
    let switched_inner = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: inner_frame_id,
            }
        }),
    )
    .await;
    assert_eq!(switched_inner, json!({ "value": null }));

    let removed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const frame = window.parent.frameElement; if (frame) frame.remove(); return frame ? 'removed' : 'missing';",
            "args": []
        }),
    )
    .await;
    assert_eq!(removed, json!({ "value": "removed" }));
}

async fn classic_find_css_element_id(app: Router, session_id: &str, selector: &str) -> String {
    classic_request_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": selector
        }),
    )
    .await["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("{selector} element reference id"))
        .to_owned()
}

async fn classic_assert_web_element_array_eq(
    app: Router,
    session_id: &str,
    label: &str,
    response: &serde_json::Value,
    expected_element_ids: &[String],
) {
    let actual = response["value"]
        .as_array()
        .unwrap_or_else(|| panic!("{label}: expected WebElement array response: {response:?}"));
    assert_eq!(
        actual.len(),
        expected_element_ids.len(),
        "{label}: unexpected WebElement array length for response {response:?}"
    );
    for (index, (actual, expected_element_id)) in
        actual.iter().zip(expected_element_ids.iter()).enumerate()
    {
        let actual_element_id = actual[CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("expected WebElement reference at {index}: {response:?}"));
        let same = classic_request_json(
            app.clone(),
            Method::GET,
            &format!(
                "/session/{session_id}/element/{actual_element_id}/equals/{expected_element_id}"
            ),
        )
        .await;
        assert_eq!(
            same,
            json!({ "value": true }),
            "{label}: returned WebElement at {index} should match expected element"
        );
    }
}

fn classic_temp_file_basename(file: &TempPath) -> String {
    file.path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| {
            panic!(
                "temporary file should have a UTF-8 basename: {:?}",
                file.path
            )
        })
        .to_owned()
}

fn classic_assert_serialized_file_list_names(
    label: &str,
    response: &serde_json::Value,
    expected_names: &[String],
) {
    let actual = response["value"]
        .as_array()
        .unwrap_or_else(|| panic!("{label}: expected FileList array response: {response:?}"));
    assert_eq!(
        actual.len(),
        expected_names.len(),
        "{label}: unexpected FileList length for response {response:?}"
    );
    for (index, (actual, expected_name)) in actual.iter().zip(expected_names.iter()).enumerate() {
        assert!(
            actual.as_object().is_some(),
            "{label}: expected serialized File object at {index}: {response:?}"
        );
        assert!(
            actual["name"].as_str().is_some(),
            "{label}: expected serialized File name string at {index}: {response:?}"
        );
        assert_eq!(
            actual["name"],
            json!(expected_name),
            "{label}: unexpected serialized File name at {index}"
        );
    }
}

async fn classic_assert_no_such_window(app: Router, method: Method, path: &str) {
    let (status, response) = classic_request_status_and_json(app, method, path).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "path {path}: {response:?}");
    assert_eq!(
        response["value"]["error"],
        json!("no such window"),
        "path {path}: {response:?}"
    );
}

async fn classic_assert_no_such_window_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) {
    let (status, response) =
        classic_request_status_and_json_with_body(app, method, path, body).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "path {path}: {response:?}");
    assert_eq!(
        response["value"]["error"],
        json!("no such window"),
        "path {path}: {response:?}"
    );
}

async fn classic_request_json_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let (status, value) = classic_request_status_and_json_with_body(app, method, path, body).await;
    assert_eq!(status, StatusCode::OK, "path {path}: {value:?}");
    value
}

async fn classic_request_status_and_json(
    app: Router,
    method: Method,
    path: &str,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&body).expect("json response"),
    )
}

async fn classic_request_status_headers_and_json(
    app: Router,
    method: Method,
    path: &str,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        headers,
        serde_json::from_slice(&body).expect("json response"),
    )
}

async fn classic_request_status_headers_and_text(
    app: Router,
    method: Method,
    path: &str,
) -> (StatusCode, HeaderMap, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        headers,
        String::from_utf8(body.to_vec()).expect("text response"),
    )
}

async fn classic_request_status_and_json_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&body).expect("json response"),
    )
}

async fn classic_request_status_headers_and_json_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        headers,
        serde_json::from_slice(&body).expect("json response"),
    )
}

fn assert_classic_webdriver_json_headers(headers: &HeaderMap) {
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-cache");
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8"
    );
}

fn assert_classic_json_content_type_absent(headers: &HeaderMap) {
    let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
        return;
    };
    let content_type = content_type.to_str().expect("content-type should be ascii");
    assert!(
        !content_type
            .split_once(';')
            .map_or(content_type, |(media_type, _)| media_type)
            .trim()
            .eq_ignore_ascii_case("application/json"),
        "non-Classic router errors must not be relabeled as JSON: {content_type}"
    );
}
