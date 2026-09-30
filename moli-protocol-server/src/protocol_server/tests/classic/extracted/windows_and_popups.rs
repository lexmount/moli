use super::*;

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

    classic_capture_layout(app.clone(), session_id).await;

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
    classic_capture_layout(app.clone(), session_id).await;
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
async fn webdriver_classic_keeps_all_live_popup_proxy_aliases() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let execute_path = format!("/session/{session_id}/execute/sync");
    let original = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original = original["value"].as_str().unwrap().to_owned();

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "window.__oldReport = open('about:blank#report', 'report'); const helper = open('about:blank#helper', 'helper'); return [window.__oldReport, helper];",
            "args": []
        }),
    )
    .await;
    let report = created["value"][0][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .expect("report handle")
        .to_owned();
    let helper = created["value"][1][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .expect("helper handle")
        .to_owned();

    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &window_path,
            json!({ "handle": helper }),
        )
        .await,
        json!({ "value": null })
    );
    let second_alias = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "window.__newReport = open('about:blank#next', 'report'); return window.__newReport;",
            "args": []
        }),
    )
    .await;
    assert_eq!(
        second_alias["value"][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(report)
    );

    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &window_path,
            json!({ "handle": original }),
        )
        .await,
        json!({ "value": null })
    );
    let old_alias = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({ "script": "return window.__oldReport;", "args": [] }),
    )
    .await;
    assert_eq!(
        old_alias["value"][CLASSIC_WINDOW_REFERENCE_KEY],
        json!(report)
    );

    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    assert_eq!(
        handles["value"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|handle| **handle == json!(report))
            .count(),
        1
    );

    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &window_path,
            json!({ "handle": report }),
        )
        .await,
        json!({ "value": null })
    );
    let remaining = classic_request_json(app.clone(), Method::DELETE, &window_path).await;
    assert_eq!(remaining["value"].as_array().map(Vec::len), Some(2));

    for (owner, proxy) in [(&original, "__oldReport"), (&helper, "__newReport")] {
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &window_path,
                json!({ "handle": owner }),
            )
            .await,
            json!({ "value": null })
        );
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &execute_path,
                json!({
                    "script": format!("return window.{proxy}.closed;"),
                    "args": []
                }),
            )
            .await,
            json!({ "value": true }),
            "every live alias must observe the shared target close"
        );
    }

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}

#[tokio::test]
async fn webdriver_classic_noopener_reuses_a_related_named_popup() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let execute_path = format!("/session/{session_id}/execute/sync");

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "window.__report = open('about:blank#first', 'report'); return window.__report;",
            "args": []
        }),
    )
    .await;
    let report = created["value"][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .expect("report handle")
        .to_owned();
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return window.__report.opener === window;",
                "args": []
            }),
        )
        .await,
        json!({ "value": true }),
        "the original popup proxy must retain its opener"
    );
    let before = classic_request_json(app.clone(), Method::GET, &handles_path).await;

    let reused = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "return open('about:blank#second', 'report', 'noopener') === null;",
            "args": []
        }),
    )
    .await;
    assert_eq!(reused, json!({ "value": true }));
    assert_eq!(
        classic_request_json(app.clone(), Method::GET, &handles_path).await,
        before,
        "noopener must not create a new target when a related named target exists"
    );

    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &window_path,
            json!({ "handle": report }),
        )
        .await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return location.href;",
                "args": []
            }),
        )
        .await,
        json!({ "value": "about:blank#second" }),
        "the selected target must receive the noopener navigation"
    );

    let original = before["value"]
        .as_array()
        .and_then(|handles| {
            handles.iter().find_map(|handle| {
                let handle = handle.as_str()?;
                (handle != report).then(|| handle.to_owned())
            })
        })
        .expect("original window handle");
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &window_path,
            json!({ "handle": original }),
        )
        .await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return window.__report.opener === window;",
                "args": []
            }),
        )
        .await,
        json!({ "value": true }),
        "reusing with noopener must not clear the existing proxy's opener"
    );

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}

#[tokio::test]
async fn webdriver_classic_close_retires_popup_aliases_before_named_reopen() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let execute_path = format!("/session/{session_id}/execute/sync");
    let original = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original = original["value"].as_str().unwrap().to_owned();

    let created = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "window.__oldReport = open('about:blank#one', 'report'); return window.__oldReport;",
            "args": []
        }),
    )
    .await;
    let old_report = created["value"][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .expect("old report handle")
        .to_owned();
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &window_path,
            json!({ "handle": old_report }),
        )
        .await,
        json!({ "value": null })
    );
    let remaining = classic_request_json(app.clone(), Method::DELETE, &window_path).await;
    assert_eq!(remaining, json!({ "value": [original.clone()] }));

    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &window_path,
            json!({ "handle": original }),
        )
        .await,
        json!({ "value": null })
    );
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &execute_path,
            json!({
                "script": "return window.__oldReport.closed;",
                "args": []
            }),
        )
        .await,
        json!({ "value": true }),
        "a proxy must observe that its protocol target was closed"
    );
    let replacement = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "window.__newReport = open('about:blank#two', 'report'); return window.__newReport;",
            "args": []
        }),
    )
    .await;
    let new_report = replacement["value"][CLASSIC_WINDOW_REFERENCE_KEY]
        .as_str()
        .expect("replacement report handle")
        .to_owned();
    assert_ne!(new_report, old_report);

    let reused = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "window.__newReport.marker = 19; window.__newReport.document.body.textContent = 'kept'; for (let i = 0; i < 20; ++i) { if (open('', 'report') !== window.__newReport) return [false, i]; } return [true, window.__newReport.marker, window.__newReport.document.body.textContent, window.__newReport.closed];",
            "args": []
        }),
    )
    .await;
    assert_eq!(reused, json!({ "value": [true, 19, "kept", false] }));
    let handles = classic_request_json(
        app.clone(),
        Method::GET,
        &format!("/session/{session_id}/window/handles"),
    )
    .await;
    assert_eq!(handles["value"].as_array().map(Vec::len), Some(2));

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}

#[tokio::test]
async fn webdriver_classic_empty_parent_ignores_replaced_public_parent_property() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let result = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const fake = {}; let reads = 0; Object.defineProperty(window, 'parent', { configurable: true, get() { ++reads; return fake; } }); const selected = open('', '_parent'); return [selected === window, selected === fake, reads];",
            "args": []
        }),
    )
    .await;
    assert_eq!(result, json!({ "value": [true, false, 0] }));

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
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
    classic_capture_layout(app.clone(), session_id).await;

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
            "script": "return JSON.stringify({ innerWidth, innerHeight, outerWidth, outerHeight, screenWidth: screen.width, screenHeight: screen.height, availWidth: screen.availWidth, availHeight: screen.availHeight, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, hasFullScreen: 'fullScreen' in window, hasWebkitIsFullScreen: 'webkitIsFullScreen' in document });",
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
            "hasFullScreen": false,
            "hasWebkitIsFullScreen": false,
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
            "script": "return JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, hasFullScreen: 'fullScreen' in window, hasWebkitIsFullScreen: 'webkitIsFullScreen' in document });",
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
            "hasFullScreen": false,
            "hasWebkitIsFullScreen": false,
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
            "script": "return JSON.stringify({ innerWidth, innerHeight, outerWidth, outerHeight, screenWidth: screen.width, screenHeight: screen.height, availWidth: screen.availWidth, availHeight: screen.availHeight, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, hasFullScreen: 'fullScreen' in window, hasWebkitIsFullScreen: 'webkitIsFullScreen' in document });",
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
            "hasFullScreen": false,
            "hasWebkitIsFullScreen": false,
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
            "script": "return JSON.stringify({ innerWidth, innerHeight, outerWidth, outerHeight, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, hasFullScreen: 'fullScreen' in window, hasWebkitIsFullScreen: 'webkitIsFullScreen' in document });",
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
            "hasFullScreen": false,
            "hasWebkitIsFullScreen": false,
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
async fn webdriver_classic_named_popup_does_not_reuse_an_independent_tab() {
    let app = build_router(test_state());
    let session = classic_request_json(app.clone(), Method::POST, "/session").await;
    let session_id = session["value"]["sessionId"]
        .as_str()
        .expect("classic session id");
    let window_path = format!("/session/{session_id}/window");
    let handles_path = format!("/session/{session_id}/window/handles");
    let execute_path = format!("/session/{session_id}/execute/sync");

    let original = classic_request_json(app.clone(), Method::GET, &window_path).await;
    let original = original["value"]
        .as_str()
        .expect("original window handle")
        .to_owned();
    let named = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "window.name = 'independent-report'; return window.name;",
            "args": []
        }),
    )
    .await;
    assert_eq!(named, json!({ "value": "independent-report" }));

    let independent = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/window/new"),
        json!({ "type": "tab" }),
    )
    .await;
    let independent = independent["value"]["handle"]
        .as_str()
        .expect("independent tab handle")
        .to_owned();
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &window_path,
        json!({ "handle": independent }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let opened = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &execute_path,
        json!({
            "script": "const popup = window.open('about:blank#independent-popup', 'independent-report'); return popup !== null;",
            "args": []
        }),
    )
    .await;
    assert_eq!(opened, json!({ "value": true }));
    let handles = classic_request_json(app.clone(), Method::GET, &handles_path).await;
    let handles = handles["value"].as_array().expect("window handles");
    assert_eq!(
        handles.len(),
        3,
        "an unrelated same-name tab must not be selected as the popup target: {handles:?}"
    );
    assert!(handles.contains(&json!(original)));
    assert!(handles.contains(&json!(independent)));

    let _ = classic_request_json(app, Method::DELETE, &format!("/session/{session_id}")).await;
}
