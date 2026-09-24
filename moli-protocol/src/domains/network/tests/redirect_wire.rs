use super::*;

#[derive(Clone, Debug)]
struct WireRequest {
    path: String,
    method: String,
    headers: HeaderMap,
    body: String,
}

fn event_header<'a>(headers: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    headers.as_object()?.iter().find_map(|(key, value)| {
        key.eq_ignore_ascii_case(name)
            .then(|| value.as_str())
            .flatten()
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn form_post_redirect_chain_cdp_metadata_matches_wire_requests() {
    async fn capture(
        State(seen): State<Arc<Mutex<Vec<WireRequest>>>>,
        method: axum::http::Method,
        uri: axum::http::Uri,
        headers: HeaderMap,
        body: String,
    ) -> axum::response::Response {
        let path = uri.path().to_owned();
        seen.lock().push(WireRequest {
            path: path.clone(),
            method: method.to_string(),
            headers,
            body,
        });
        match path.as_str() {
            "/start" => (StatusCode::FOUND, [(LOCATION, "/middle")]).into_response(),
            "/middle" => (StatusCode::TEMPORARY_REDIRECT, [(LOCATION, "/final")]).into_response(),
            "/final" => axum::response::Html("<!doctype html><p>finished</p>").into_response(),
            _ => unreachable!("only the three redirect routes use this handler"),
        }
    }

    let seen = Arc::new(Mutex::new(Vec::<WireRequest>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/page",
            get(|| async {
                axum::response::Html(
                    "<!doctype html><form id='f' method='POST' action='/start'><input name='account' value='alice'></form>",
                )
            }),
        )
        // Accept any method so an incorrect redirect reaches an assertion,
        // rather than being hidden behind the router's 405 response.
        .route("/start", axum::routing::any(capture))
        .route("/middle", axum::routing::any(capture))
        .route("/final", axum::routing::any(capture))
        .with_state(seen.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-1".to_owned());
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 79_300, "method": "Network.enable", "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(79_300, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 79_301, "method": "Runtime.evaluate", "sessionId": "SID-1",
        "params": {"expression": "document.getElementById('f').submit(); 'scheduled'"}
    }))
    .await;
    let reply = ctx.take_response_by_id(79_301);
    assert!(reply.get("error").is_none(), "{reply}");
    assert!(reply["result"].get("exceptionDetails").is_none(), "{reply}");

    let final_url = format!("http://{addr}/final");
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "completed form redirect chain",
        |messages| {
            let id = messages.iter().find_map(|message| {
                (message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["request"]["url"] == json!(final_url))
                .then(|| message["params"]["requestId"].as_str())
                .flatten()
            });
            id.is_some_and(|id| {
                messages.iter().any(|message| {
                    message["method"] == json!("Network.loadingFinished")
                        && message["params"]["requestId"] == json!(id)
                })
            })
        },
    )
    .await;
    server.abort();

    let wire = seen.lock().clone();
    assert_eq!(
        wire.len(),
        3,
        "exactly one request per redirect hop: {wire:?}"
    );
    // HTTP expectations are explicit, independent of both the server records
    // and the protocol implementation, so two equally wrong views cannot pass.
    for (request, (path, method, body)) in wire.iter().zip([
        ("/start", "POST", "account=alice"),
        ("/middle", "GET", ""),
        ("/final", "GET", ""),
    ]) {
        assert_eq!(request.path, path);
        assert_eq!(request.method, method);
        assert_eq!(request.body, body);
        assert_eq!(request.headers.get("referer").unwrap(), page_url.as_str());
    }
    assert_eq!(
        wire[0].headers.get("content-type").unwrap(),
        "application/x-www-form-urlencoded"
    );
    for request in &wire[1..] {
        assert!(!request.headers.contains_key("content-type"));
        assert!(!request.headers.contains_key("content-length"));
    }

    let requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && wire.iter().any(|request| {
                    message["params"]["request"]["url"]
                        == json!(format!("http://{addr}{}", request.path))
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 3);
    let request_id = &requests[0]["params"]["requestId"];
    for (index, (event, actual)) in requests.iter().zip(&wire).enumerate() {
        assert_eq!(&event["params"]["requestId"], request_id);
        assert_eq!(event["params"]["request"]["method"], actual.method);
        assert_eq!(
            event["params"]["request"]["url"],
            format!("http://{addr}{}", actual.path)
        );
        // Ordinary request events describe the request before all transport
        // headers are finalized. Compare wire headers through ExtraInfo below;
        // Chromium can retain Content-Type here after a redirect removed it.
        if index == 0 {
            assert_eq!(event["params"]["request"]["postData"], "account=alice");
        } else {
            assert!(event["params"]["request"].get("postData").is_none());
            assert_eq!(
                event["params"]["redirectResponse"]["status"],
                [302, 307][index - 1]
            );
        }
    }
    let extra = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && &message["params"]["requestId"] == request_id
        })
        .collect::<Vec<_>>();
    assert_eq!(extra.len(), 3, "one wire header observation per hop");
    for (event, actual) in extra.iter().zip(&wire) {
        for name in ["referer", "content-type", "content-length"] {
            assert_eq!(
                event_header(&event["params"]["headers"], name),
                actual
                    .headers
                    .get(name)
                    .map(|value| value.to_str().unwrap()),
                "{} {name}",
                actual.path
            );
        }
    }
}
