use super::*;

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
    classic_capture_layout(app.clone(), session_id).await;

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
    classic_capture_layout(app.clone(), session_id).await;
    assert_eq!(first_navigated, json!({ "value": null }));

    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": navigate_url }),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
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
async fn webdriver_classic_click_uses_top_level_pointer_coordinates_inside_offset_frame() {
    for transform in ["none", "scale(0.75)", "rotate(12deg)"] {
        let app = build_router(test_state());
        let session = classic_request_json(app.clone(), Method::POST, "/session").await;
        let session_id = session["value"]["sessionId"].as_str().expect("session id");
        let html = format!(
            "<iframe style='position:absolute;left:200px;top:180px;width:400px;height:200px;transform:{transform}' srcdoc=\"<input id='target' style='margin:40px'><script>window.clicks=0;target.onclick=e=>{{clicks++;window.trusted=e.isTrusted;}};</script>\"></iframe>"
        );
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/url"),
            json!({"url":format!("data:text/html,{html}")}),
        )
        .await;
        classic_capture_layout(app.clone(), session_id).await;
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/frame"),
                json!({"id":0})
            )
            .await,
            json!({"value":null})
        );
        let element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
        assert_eq!(
            classic_request_json(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/element/{element_id}/click")
            )
            .await,
            json!({"value":null})
        );
        let observed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script":"return [document.activeElement.id,window.clicks,window.trusted];", "args":[]
        }),
    )
    .await;
        assert_eq!(observed, json!({"value":["target",1,true]}), "{transform}");
        classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
    }
}
#[tokio::test]
#[ignore = "Pending iframe overlay click fix; expectations verified against Chromium"]
async fn webdriver_classic_frame_target_click_is_invariant_under_neutral_frame_wrapping() {
    for depth in 0..=2 {
        for selector in ["#target", "#container"] {
            let app = build_router(test_state());
            let session = classic_request_json(app.clone(), Method::POST, "/session").await;
            let session_id = session["value"]["sessionId"].as_str().unwrap();
            let mut html = "<div id='container' style='width:200px;height:100px'><iframe id='target' style='width:200px;height:100px;border:0' srcdoc=\"<body style='margin:0'><button style='width:200px;height:100px' onclick='window.clicks++'>go</button><script>window.clicks=0;</script>\"></iframe></div>".to_owned();
            for _ in 0..depth {
                let child = html.replace('&', "&amp;").replace('"', "&quot;");
                html = format!(
                    "<iframe style='width:600px;height:400px;border:0' srcdoc=\"{child}\"></iframe>"
                );
            }
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({"url":classic_data_url(&html)}),
            )
            .await;
            for _ in 0..depth {
                classic_request_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/frame"),
                    json!({"id":0}),
                )
                .await;
            }
            let target = classic_find_css_element_id(app.clone(), session_id, selector).await;
            let (status, clicked) = classic_request_status_and_json(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/element/{target}/click"),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "depth={depth} target={selector}: {clicked}"
            );
            assert_eq!(clicked, json!({"value":null}));
            let observed = classic_request_json_with_body(
                app.clone(), Method::POST, &format!("/session/{session_id}/execute/sync"),
                json!({"script":"return document.getElementById('target').contentWindow.clicks;","args":[]}),
            ).await;
            assert_eq!(observed["value"], 1, "depth={depth} target={selector}");
            // Another child of the same document is not a descendant of the
            // requested target. It must not become an accepted deep hit.
            classic_request_json_with_body(
                app.clone(), Method::POST, &format!("/session/{session_id}/execute/sync"),
                json!({"script":r#"
                    window.overlayClicks=0;
                    const veil=document.createElement('iframe');
                    veil.style.cssText='position:fixed;inset:0;width:100%;height:100%;border:0;z-index:999';
                    veil.srcdoc='<body style="margin:0" onclick="parent.overlayClicks++">overlay</body>';
                    document.body.appendChild(veil);
                "#,"args":[]}),
            ).await;
            let (status, blocked) = classic_request_status_and_json(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/element/{target}/click"),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "depth={depth} target={selector}: {blocked}"
            );
            assert_eq!(blocked["value"]["error"], "element click intercepted");
            let observed = classic_request_json_with_body(
                app.clone(), Method::POST, &format!("/session/{session_id}/execute/sync"),
                json!({"script":"return [document.getElementById('target').contentWindow.clicks,window.overlayClicks];","args":[]}),
            ).await;
            assert_eq!(
                observed["value"],
                json!([1, 0]),
                "rejected clicks must have no effect"
            );
            classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
        }
    }
}
#[tokio::test]
#[ignore = "Pending ancestor overlay click fix; expectations verified against Chromium"]
async fn webdriver_classic_frame_click_rejects_ancestor_overlays_without_dispatch() {
    for (depth, blocked_level) in [(1, 0), (2, 0), (2, 1)] {
        let app = build_router(test_state());
        let session = classic_request_json(app.clone(), Method::POST, "/session").await;
        let session_id = session["value"]["sessionId"].as_str().unwrap();
        let mut html = "<button id='target' onclick='window.clicks++'>go</button><script>window.clicks=0;</script>".to_owned();
        for _ in 0..depth {
            let child = html.replace('&', "&amp;").replace('"', "&quot;");
            html = format!(
                "<iframe style='width:600px;height:400px;border:0' srcdoc=\"{child}\"></iframe><div id='veil' style='display:none;position:fixed;inset:0;z-index:999' onclick='window.overlayClicks++'></div><script>window.overlayClicks=0;</script>"
            );
        }
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/url"),
            json!({"url":classic_data_url(&html)}),
        )
        .await;
        for _ in 0..depth {
            assert_eq!(
                classic_request_json_with_body(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/frame"),
                    json!({"id":0}),
                )
                .await,
                json!({"value":null})
            );
        }
        let target = classic_find_css_element_id(app.clone(), session_id, "#target").await;
        let overlay_script = format!(
            "let w=window;for(let i=0;i<{};i++)w=w.parent;w.document.getElementById('veil').style.display=arguments[0];",
            depth - blocked_level
        );
        let read_script = "let w=window,total=0;while(w!==w.parent){w=w.parent;total+=w.overlayClicks;}return [window.clicks,total];";
        for blocked in [true, false] {
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/execute/sync"),
                json!({"script":overlay_script,"args":[if blocked {"block"} else {"none"}]}),
            )
            .await;
            let (status, clicked) = classic_request_status_and_json(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/element/{target}/click"),
            )
            .await;
            if blocked {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert_eq!(
                    clicked["value"]["error"], "element click intercepted",
                    "depth={depth} level={blocked_level}: {clicked}"
                );
            } else {
                assert_eq!(status, StatusCode::OK);
                assert_eq!(clicked, json!({"value":null}));
            }
            let observed = classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/execute/sync"),
                json!({"script":read_script,"args":[]}),
            )
            .await;
            assert_eq!(observed["value"], json!([if blocked { 0 } else { 1 }, 0]));
        }
        classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
    }
}
#[tokio::test]
async fn webdriver_classic_click_mousedown_navigation_does_not_activate_successor() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"].as_str().expect("session id");
    let successor = classic_data_url(
        "<button id='target' style='width:200px;height:80px'>next</button><script>window.clicks=0;window.buttons=[];target.onclick=()=>clicks++;target.onmouseup=e=>buttons.push(e.buttons);</script>",
    );
    let source = classic_data_url(&format!(
        "<button id='target' style='width:200px;height:80px' onmousedown=\"window.open('{successor}','_self')\">first</button>"
    ));
    classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({"url":source}),
    )
    .await;
    classic_capture_layout(app.clone(), session_id).await;
    let element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    let clicked = classic_request_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    // A complete physical sequence may navigate during mousedown; no input
    // phase is retried against the successor document.
    assert_eq!(clicked, json!({"value":null}));
    let observed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script":"return window.clicks;", "args":[]
        }),
    )
    .await;
    assert_eq!(observed, json!({"value":0}));
    classic_capture_layout(app.clone(), session_id).await;
    let successor_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
    assert_eq!(
        classic_request_json(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/element/{successor_id}/click")
        )
        .await,
        json!({"value":null})
    );
    let observed = classic_request_json_with_body(app.clone(), Method::POST,
        &format!("/session/{session_id}/execute/sync"), json!({
            "script":"return [window.clicks,document.activeElement.id,window.buttons.every(b=>b===0)];", "args":[]
        })).await;
    assert_eq!(observed, json!({"value":[1,"target",true]}));
    classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}
