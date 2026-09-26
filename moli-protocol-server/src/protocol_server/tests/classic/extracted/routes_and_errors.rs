use super::*;

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
    classic_capture_layout(app.clone(), session_id).await;
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
