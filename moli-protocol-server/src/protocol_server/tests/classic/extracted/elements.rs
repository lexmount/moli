use super::*;

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
        classic_capture_layout(app.clone(), session_id).await;

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
    classic_capture_layout(app.clone(), session_id).await;
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
    classic_capture_layout(app.clone(), session_id).await;

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
        classic_capture_layout(app.clone(), session_id).await;
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
async fn webdriver_classic_page_screenshot_publishes_layout_and_element_clip_remains_unsupported() {
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
    assert_eq!(page_status, StatusCode::OK);
    let bytes = BASE64_STANDARD
        .decode(page_screenshot["value"].as_str().expect("PNG base64"))
        .expect("valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()) > 0);
    assert!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()) > 0);

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
async fn webdriver_classic_click_respects_dom_first_and_real_layout_policies() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let state = AppState::new_with_storage_partition_and_runtime_config(
            "127.0.0.1:9222".parse().unwrap(),
            Arc::new(StoragePartitionState::open(None).unwrap()),
            NavigationRuntimeConfig::new(
                protocol_server_test_fetch_config(FetchConfig::default()),
                OptionalResourceFetchMask::NONE,
                true,
                policy,
            ),
            crate::config::DEFAULT_SCREENCAST_INTERVAL_MS,
        )
        .unwrap();
        let app = build_router(state);
        let session = classic_request_json(app.clone(), Method::POST, "/session").await;
        let session_id = session["value"]["sessionId"].as_str().unwrap();
        let navigated = classic_request_json_with_body(
            app.clone(), Method::POST, &format!("/session/{session_id}/url"),
            json!({"url": "data:text/html,<button id='target'>go</button><script>window.events=[];for(const type of ['pointerdown','mousedown','mouseup','click'])target.addEventListener(type,e=>events.push(e.type));</script>"}),
        ).await;
        if policy.uses_real_layout() {
            classic_capture_layout(app.clone(), session_id).await;
        }
        assert_eq!(navigated, json!({"value": null}));
        let element_id = classic_find_css_element_id(app.clone(), session_id, "#target").await;
        let clicked = classic_request_json(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/element/{element_id}/click"),
        )
        .await;
        assert_eq!(clicked, json!({"value": null}), "{policy:?}");
        let observed = classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({"script": "return window.events;", "args": []}),
        )
        .await;
        let expected = match policy {
            LayoutPolicy::Mock => json!(["click"]),
            LayoutPolicy::OnDemand => json!(["pointerdown", "mousedown", "mouseup", "click"]),
        };
        assert_eq!(observed["value"], expected, "{policy:?}");
        classic_request_json_with_body(
            app.clone(), Method::POST, &format!("/session/{session_id}/url"),
            json!({"url": "data:text/html,<input id='origin'><input id='target'><script>document.getElementById('origin').focus();</script>"}),
        ).await;
        if policy.uses_real_layout() {
            classic_capture_layout(app.clone(), session_id).await;
        }
        let target = classic_find_css_element_id(app.clone(), session_id, "#target").await;
        assert_eq!(
            classic_request_json(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/element/{target}/click"),
            )
            .await,
            json!({"value":null})
        );
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/actions"),
                json!({"actions":[{"type":"key","id":"keyboard","actions":[
                    {"type":"keyDown","value":"x"},{"type":"keyUp","value":"x"}
                ]}]}),
            )
            .await,
            json!({"value":null})
        );
        let focused = classic_request_json_with_body(
            app.clone(), Method::POST, &format!("/session/{session_id}/execute/sync"),
            json!({"script":"return [document.activeElement.id,document.getElementById('origin').value,document.getElementById('target').value];","args":[]}),
        ).await;
        assert_eq!(focused["value"], json!(["target", "", "x"]), "{policy:?}");
        classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
    }
}
#[tokio::test]
async fn webdriver_classic_element_click_uses_shared_dom_geometry_and_input() {
    let app = build_router(test_state());

    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let url = "data:text/html,<button id='target' onclick='window.__clicked = true'>go</button><script>window.events=[];for(const type of ['pointerdown','mousedown','focus','mouseup','click'])target.addEventListener(type,e=>events.push([e.type,e.isTrusted]));</script>";

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

    let (cold_status, cold) = classic_request_status_and_json(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/element/{element_id}/click"),
    )
    .await;
    assert_eq!(cold_status, StatusCode::OK);
    assert_eq!(cold, json!({ "value": null }));

    let clicked_state = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "return [Boolean(window.__clicked),document.activeElement.id,window.events];",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        clicked_state,
        json!({ "value": [true, "target", [
        ["pointerdown", true], ["mousedown", true], ["focus", true],
        ["mouseup", true], ["click", true]
    ]] })
    );

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
    classic_capture_layout(app.clone(), session_id).await;
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
    classic_capture_layout(app.clone(), session_id).await;
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
