use super::*;

#[tokio::test]
async fn websocket_cdp_child_script_redirect_preserves_each_hops_transport_cookies() {
    assert_child_script_redirect_transport_cookies(true).await;
}

#[tokio::test]
async fn websocket_cdp_child_script_redirect_without_set_cookie_preserves_extra_info() {
    assert_child_script_redirect_transport_cookies(false).await;
}

async fn assert_child_script_redirect_transport_cookies(set_cookie: bool) {
    let requests = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let fixture = Router::new()
        .route(
            "/parent",
            get(|| async { axum::response::Html("<iframe id='child' src='/child'></iframe>") }),
        )
        .route(
            "/child",
            get(|| async { axum::response::Html("<script src='/first.js'></script>") }),
        )
        .route(
            "/first.js",
            get({
                let requests = Arc::clone(&requests);
                move |headers: axum::http::HeaderMap| {
                    let requests = Arc::clone(&requests);
                    async move {
                        requests
                            .lock()
                            .push(headers[header::COOKIE].to_str().unwrap().to_owned());
                        let mut response_headers = axum::http::HeaderMap::new();
                        response_headers.insert(
                            header::LOCATION,
                            axum::http::HeaderValue::from_static("/final.js"),
                        );
                        if set_cookie {
                            response_headers.insert(
                                header::SET_COOKIE,
                                axum::http::HeaderValue::from_static("hop=second; Path=/"),
                            );
                        }
                        (axum::http::StatusCode::FOUND, response_headers, "")
                    }
                }
            }),
        )
        .route(
            "/final.js",
            get({
                let requests = Arc::clone(&requests);
                move |headers: axum::http::HeaderMap| {
                    let requests = Arc::clone(&requests);
                    async move {
                        requests
                            .lock()
                            .push(headers[header::COOKIE].to_str().unwrap().to_owned());
                        (
                            [(header::CONTENT_TYPE, "text/javascript")],
                            "globalThis.scriptDone = true;",
                        )
                    }
                }
            }),
        );
    let (fixture_addr, _fixture) =
        spawn_dedicated_fixture_server(fixture, "child-script-redirect-extra-info");
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    for (id, method, params) in [
        (4, "Page.enable", json!({})),
        (5, "Network.enable", json!({})),
        (
            6,
            "Network.setCookie",
            json!({"name": "fixture", "value": "one", "url": base}),
        ),
    ] {
        send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }
    let mut messages = send_cdp_command(
        &mut socket,
        7,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": format!("{base}/parent")}),
    )
    .await;
    if !messages
        .iter()
        .any(|m| m["method"] == "Page.loadEventFired")
    {
        messages
            .extend(recv_until_match(&mut socket, |m| m["method"] == "Page.loadEventFired").await);
    }
    messages.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({"expression": "String(child.contentWindow.scriptDone)"}),
        )
        .await,
    );
    assert_eq!(
        messages.iter().find(|m| m["id"] == 8).unwrap()["result"]["result"]["value"],
        "true"
    );
    messages.extend(send_cdp_command(&mut socket, 9, "Browser.getVersion", None, json!({})).await);
    let starts = messages
        .iter()
        .filter(|m| {
            m["method"] == "Network.requestWillBeSent"
                && [format!("{base}/first.js"), format!("{base}/final.js")]
                    .iter()
                    .any(|url| m["params"]["request"]["url"] == *url)
        })
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 2);
    let request_id = &starts[0]["params"]["requestId"];
    assert_eq!(&starts[1]["params"]["requestId"], request_id);
    let extra = messages
        .iter()
        .filter(|m| {
            m["method"] == "Network.requestWillBeSentExtraInfo"
                && &m["params"]["requestId"] == request_id
        })
        .collect::<Vec<_>>();
    assert_eq!(extra.len(), 2);
    let observed = requests.lock().clone();
    assert_eq!(observed.len(), 2);
    assert_eq!(observed[0], "fixture=one");
    if set_cookie {
        assert!(observed[1].contains("hop=second"));
    } else {
        assert_eq!(observed[1], "fixture=one");
    }
    for (event, cookie) in extra.iter().zip(&observed) {
        assert_eq!(event["params"]["headers"]["Cookie"], *cookie);
    }
    assert_eq!(
        extra[0]["params"]["associatedCookies"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        extra[1]["params"]["associatedCookies"]
            .as_array()
            .unwrap()
            .len(),
        if set_cookie { 2 } else { 1 }
    );
    assert_eq!(starts[1]["params"]["redirectHasExtraInfo"], true);
    let response_extra = messages
        .iter()
        .filter(|m| {
            m["method"] == "Network.responseReceivedExtraInfo"
                && &m["params"]["requestId"] == request_id
        })
        .collect::<Vec<_>>();
    assert_eq!(response_extra.len(), 2);
    assert_eq!(response_extra[0]["params"]["statusCode"], 302);
    assert_eq!(response_extra[1]["params"]["statusCode"], 200);
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
