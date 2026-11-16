use super::*;

async fn check_bidi_child_script_request_metadata(http_error: bool, cross_origin: bool) {
    let requested = Arc::new(tokio::sync::Notify::new());
    let (release, release_rx) = tokio::sync::watch::channel(false);
    let script_fixture = Router::new().route(
        "/pending.js",
        get({
            let requested = Arc::clone(&requested);
            move |headers: axum::http::HeaderMap| {
                let requested = Arc::clone(&requested);
                let mut release_rx = release_rx.clone();
                async move {
                    assert_eq!(
                        headers.get(header::COOKIE).map(|v| v.to_str().unwrap()),
                        (!cross_origin).then_some("fixture=one")
                    );
                    requested.notify_one();
                    release_rx.wait_for(|ready| *ready).await.unwrap();
                    (
                        if http_error {
                            StatusCode::NOT_FOUND
                        } else {
                            StatusCode::OK
                        },
                        [
                            (header::CONTENT_TYPE, "text/javascript"),
                            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                        ],
                        "window.childScriptLoaded = true;",
                    )
                }
            }
        }),
    );
    let (script_addr, _script_server) =
        spawn_dedicated_fixture_server(script_fixture.clone(), "bidi-script-source");
    let script_src = if cross_origin {
        format!("http://{script_addr}/pending.js")
    } else {
        "/pending.js".to_owned()
    };
    let fixture = script_fixture
        .route(
            "/parent",
            get(|| async { axum::response::Html("<iframe src='/child'></iframe>") }),
        )
        .route(
            "/child",
            get(move || {
                let script_src = script_src.clone();
                async move {
                    axum::response::Html(format!(
                        "<script crossorigin='anonymous' src='{script_src}'></script>"
                    ))
                }
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture, "bidi-script-metadata");
    let script_url = format!(
        "http://{}/pending.js",
        if cross_origin {
            script_addr
        } else {
            fixture_addr
        }
    );
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .unwrap();
    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], "success");
    let context = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    let context = context["result"]["context"].as_str().unwrap().to_owned();
    assert_eq!(send_bidi_command(&mut socket, 3, "storage.setCookie", json!({
        "cookie": {"name": "fixture", "value": {"type": "string", "value": "one"}, "domain": "127.0.0.1", "path": "/"},
        "partition": {"type": "context", "context": context},
    })).await["type"], "success");
    assert_eq!(send_bidi_command(&mut socket, 4, "session.subscribe", json!({
        "events": ["network.beforeRequestSent", "network.responseStarted", "network.responseCompleted", "network.fetchError"],
    })).await["type"], "success");
    send_cdp_command_without_wait(
        &mut socket,
        5,
        "browsingContext.navigate",
        None,
        json!({
            "context": context, "url": format!("http://{fixture_addr}/parent"), "wait": "none",
        }),
    )
    .await;
    timeout(Duration::from_secs(10), requested.notified())
        .await
        .expect("script request reaches fixture");
    let mut messages = timeout(
        Duration::from_secs(10),
        recv_until_match(&mut socket, |message| {
            message["method"] == "network.beforeRequestSent"
                && message["params"]["request"]["url"] == script_url
        }),
    )
    .await
    .expect("BiDi admission must arrive while the response is held");
    let before = messages
        .iter()
        .find(|message| {
            message["method"] == "network.beforeRequestSent"
                && message["params"]["request"]["url"] == script_url
        })
        .unwrap();
    let request_id = before["params"]["request"]["request"]
        .as_str()
        .unwrap()
        .to_owned();
    let assert_metadata = |message: &serde_json::Value| {
        let request = &message["params"]["request"];
        assert_eq!(request["request"], request_id);
        let cookies = request["cookies"].as_array().unwrap();
        let headers = request["headers"].as_array().unwrap();
        assert!(headers.iter().any(|header| {
            header["name"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case("accept")
        }));
        let cookie_header = headers.iter().find(|header| {
            header["name"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case("cookie")
        });
        if cross_origin {
            assert!(cookies.is_empty());
            assert!(cookie_header.is_none());
        } else {
            assert_eq!(cookies.len(), 1);
            assert_eq!(cookies[0]["name"], "fixture");
            assert_eq!(
                cookies[0]["value"],
                json!({"type": "string", "value": "one"})
            );
            assert_eq!(
                cookie_header.unwrap()["value"],
                json!({"type": "string", "value": "fixture=one"})
            );
        }
    };
    assert_metadata(before);
    assert!(
        !messages
            .iter()
            .any(|message| message["params"]["request"]["url"] == script_url
                && matches!(
                    message["method"].as_str(),
                    Some(
                        "network.responseStarted"
                            | "network.responseCompleted"
                            | "network.fetchError"
                    )
                ))
    );
    release.send(true).unwrap();
    let terminal = if http_error {
        "network.fetchError"
    } else {
        "network.responseCompleted"
    };
    messages.extend(
        timeout(
            Duration::from_secs(10),
            recv_until_match(&mut socket, |message| {
                message["method"] == terminal
                    && message["params"]["request"]["request"] == request_id
            }),
        )
        .await
        .expect("BiDi terminal must retain the admitted request metadata"),
    );
    send_cdp_command_without_wait(&mut socket, 6, "session.status", None, json!({})).await;
    messages.extend(recv_until_id(&mut socket, 6).await);
    let script_events = messages
        .iter()
        .filter(|message| message["params"]["request"]["url"] == script_url)
        .collect::<Vec<_>>();
    assert_eq!(
        script_events
            .iter()
            .filter(|message| message["method"] == "network.beforeRequestSent")
            .count(),
        1
    );
    assert_eq!(
        script_events
            .iter()
            .filter(|message| message["method"] == terminal)
            .count(),
        1
    );
    for message in script_events {
        assert_metadata(message);
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_bidi_child_script_success_preserves_admission_cookies() {
    check_bidi_child_script_request_metadata(false, false).await;
}

#[tokio::test]
async fn websocket_bidi_child_script_http_error_preserves_admission_cookies() {
    check_bidi_child_script_request_metadata(true, false).await;
}

#[tokio::test]
async fn websocket_bidi_cross_origin_anonymous_child_script_omits_cookies() {
    check_bidi_child_script_request_metadata(false, true).await;
}
