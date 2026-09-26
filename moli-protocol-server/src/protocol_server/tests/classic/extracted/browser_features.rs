use super::*;

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
    classic_capture_layout(app.clone(), session_id).await;
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
