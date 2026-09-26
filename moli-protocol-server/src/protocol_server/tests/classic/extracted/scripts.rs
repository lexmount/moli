use super::*;

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
