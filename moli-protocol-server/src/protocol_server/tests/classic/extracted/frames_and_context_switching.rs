use super::*;

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
    classic_capture_layout(app.clone(), session_id).await;
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
