use super::*;

#[tokio::test]
async fn websocket_child_navigation_wait_allows_peer_and_fetch_continue() {
    verify_child_navigation_wait(false).await;
}

#[tokio::test]
async fn websocket_child_navigation_wait_allows_parent_replacement() {
    verify_child_navigation_wait(true).await;
}

async fn verify_child_navigation_wait(replace_parent: bool) {
    let requested = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let resumed = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&requested);
    let release_for_route = Arc::clone(&release);
    let resumed_for_route = Arc::clone(&resumed);
    let fixture = Router::new()
        .route(
            "/resume",
            get(move || {
                let resumed = Arc::clone(&resumed_for_route);
                async move {
                    resumed.notify_one();
                    "resumed"
                }
            }),
        )
        .route(
            "/parent",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><iframe src=/blank></iframe>",
                )
            }),
        )
        .route(
            "/blank",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><p>initial</p>",
                )
            }),
        )
        .route(
            "/child",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><script src=/hold.js></script><p>child</p>",
                )
            }),
        )
        .route(
            "/replacement",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><p>replacement</p>",
                )
            }),
        )
        .route(
            "/hold.js",
            get(move || {
                let requested = Arc::clone(&requested_for_route);
                let release = Arc::clone(&release_for_route);
                async move {
                    requested.notify_one();
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "globalThis.childDone = true;",
                    )
                }
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture, "child-navigation-wait");
    let (addr, server) = spawn_test_protocol_server().await;
    let endpoint = format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}");
    let (mut socket, _) = connect_async(&endpoint).await.unwrap();
    let (mut peer, _) = connect_async(&endpoint).await.unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    send_cdp_command(
        &mut socket,
        3,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    send_cdp_command(
        &mut socket,
        4,
        "Page.setLifecycleEventsEnabled",
        Some(&session.session_id),
        json!({"enabled": true}),
    )
    .await;
    let mut observed = send_cdp_command(
        &mut socket,
        5,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": format!("http://{fixture_addr}/parent")}),
    )
    .await;
    if !observed
        .iter()
        .any(|message| message["method"] == "Page.loadEventFired")
    {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == "Page.loadEventFired"
            })
            .await,
        );
    }
    let tree = send_cdp_command(
        &mut socket,
        6,
        "Page.getFrameTree",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let child_id = tree.iter().find(|message| message["id"] == 6).unwrap()
        ["result"]["frameTree"]["childFrames"][0]["frame"]["id"].as_str().expect("the parent must own a live child frame").to_owned();
    let mut request_to_resume = None;
    if !replace_parent {
        send_cdp_command(&mut socket, 20, "Fetch.enable", Some(&session.session_id), json!({
            "patterns": [{"urlPattern": "*resume", "resourceType": "Fetch", "requestStage": "Request"}],
        })).await;
        let mut paused = send_cdp_command(&mut socket, 21, "Runtime.evaluate", Some(&session.session_id),
            json!({"expression": "globalThis.resumeResult = 'pending'; fetch('/resume').then(r => r.text()).then(v => globalThis.resumeResult = v); 'started'", "returnByValue": true})).await;
        if !paused
            .iter()
            .any(|message| message["method"] == "Fetch.requestPaused")
        {
            paused.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Fetch.requestPaused"
                })
                .await,
            );
        }
        request_to_resume = paused
            .iter()
            .find(|message| message["method"] == "Fetch.requestPaused")
            .map(|message| message["params"]["requestId"].clone());
        assert!(request_to_resume.is_some());
    }
    observed.clear();
    let child_url = format!("http://{fixture_addr}/child");
    send_cdp_command_without_wait(
        &mut socket,
        7,
        "Page.navigate",
        Some(&session.session_id),
        json!({"frameId": child_id, "url": child_url}),
    )
    .await;
    timeout(Duration::from_secs(10), requested.notified())
        .await
        .expect("the child parser must reach its held script");
    if let Some(request) = request_to_resume {
        observed.extend(
            timeout(
                Duration::from_secs(5),
                send_cdp_command(
                    &mut socket,
                    8,
                    "Fetch.continueRequest",
                    Some(&session.session_id),
                    json!({"requestId": request}),
                ),
            )
            .await
            .expect("the child lifecycle wait must allow same-Page Fetch continuation"),
        );
        timeout(Duration::from_secs(5), resumed.notified())
            .await
            .expect("same-Page HTTP work must resume before child script release");
    }
    let progress = timeout(
        Duration::from_secs(5),
        send_cdp_command(&mut peer, 1, "Target.getTargets", None, json!({})),
    )
    .await
    .expect("an independent frontend must progress before child script release");
    assert!(
        progress
            .iter()
            .any(|message| message["id"] == 1 && message["result"]["targetInfos"].is_array())
    );
    observed.extend(
        timeout(
            Duration::from_secs(5),
            send_cdp_command(&mut socket, 9, "Browser.getVersion", None, json!({})),
        )
        .await
        .expect("the source frontend must also progress before child script release"),
    );
    assert!(
        !observed.iter().any(|message| message["id"] == 7),
        "the child navigation must retain its load wait: {observed:?}"
    );
    if replace_parent {
        send_cdp_command_without_wait(
            &mut socket,
            10,
            "Page.navigate",
            Some(&session.session_id),
            json!({"url": format!("http://{fixture_addr}/replacement")}),
        )
        .await;
    } else {
        release.notify_one();
    }
    while !observed.iter().any(|message| message["id"] == 7)
        || (replace_parent && !observed.iter().any(|message| message["id"] == 10))
    {
        observed.extend(
            timeout(
                Duration::from_secs(5),
                recv_until_match(&mut socket, |message| message["id"].is_number()),
            )
            .await
            .expect(
                "child completion and replacement must settle without releasing a retired script",
            ),
        );
    }
    assert_eq!(
        observed.iter().filter(|message| message["id"] == 7).count(),
        1
    );
    if replace_parent {
        assert!(
            observed
                .iter()
                .any(|message| message["id"] == 7 && message["error"].is_object()),
            "{observed:?}"
        );
        assert_eq!(
            cdp_runtime_evaluate_string(
                &mut socket,
                &session.session_id,
                11,
                "document.querySelector('p').textContent"
            )
            .await,
            "replacement"
        );
        release.notify_one();
        assert_eq!(
            cdp_runtime_evaluate_string(
                &mut socket,
                &session.session_id,
                12,
                "document.querySelector('p').textContent"
            )
            .await,
            "replacement"
        );
    } else {
        let reply = observed
            .iter()
            .position(|message| message["id"] == 7)
            .unwrap();
        assert_eq!(observed[reply]["result"]["frameId"], child_id);
        let navigation = observed
            .iter()
            .position(|message| {
                message["method"] == "Page.frameNavigated"
                    && message["params"]["frame"]["id"] == child_id
                    && message["params"]["frame"]["url"] == child_url
            })
            .expect("child commit must precede the navigation reply");
        assert!(navigation < reply, "{observed:?}");
        assert_eq!(cdp_runtime_evaluate_string(&mut socket, &session.session_id, 11,
            "JSON.stringify([frames[0].childDone, frames[0].document.querySelector('p').textContent])").await, r#"[true,"child"]"#);
    }
    drop(peer);
    drop(socket);
    server.abort();
}
