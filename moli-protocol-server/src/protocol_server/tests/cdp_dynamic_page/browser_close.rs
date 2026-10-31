use super::*;

async fn instance(
    profile_dir: Option<PathBuf>,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<anyhow::Result<()>>,
    AppState,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owned instance");
    let addr = listener.local_addr().expect("owned instance address");
    let state = AppState::new_with_storage_partition_and_runtime_config(
        addr,
        Arc::new(
            StoragePartitionState::open(profile_dir.as_deref()).expect("owned storage partition"),
        ),
        protocol_server_test_runtime_config(
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
        ),
        crate::config::DEFAULT_SCREENCAST_INTERVAL_MS,
        BrowserInstanceControl::new(),
    )
    .expect("owned app state");
    let serving = state.clone();
    let task = tokio::spawn(async move { serve_listener(listener, serving).await });
    (addr, task, state)
}

async fn browser(addr: SocketAddr) -> TestCdpSocket {
    connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
        .await
        .expect("connect owned browser")
        .0
}

async fn joined(task: tokio::task::JoinHandle<anyhow::Result<()>>) {
    timeout(Duration::from_secs(7), task)
        .await
        .expect("instance exits within total shutdown budget")
        .expect("server task joins")
        .expect("server completes shutdown without a timeout or join error");
}

async fn closed(socket: &mut TestCdpSocket) {
    timeout(Duration::from_secs(2), async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, WsMessage::Close(_)) {
                break;
            }
        }
    })
    .await
    .expect("owned socket terminates");
}

async fn close_instance(socket: &mut TestCdpSocket, id: u64) {
    let response = send_cdp_command(socket, id, "Browser.close", None, json!({})).await;
    assert_eq!(
        response_by_id(&response, id),
        &json!({"id": id, "result": {}}),
        "the close response must arrive before transport EOF"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_close_joins_its_listener_connections_and_storage_only() {
    assert_instance_shutdown(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_control_joins_its_listener_connections_and_storage_only() {
    assert_instance_shutdown(true).await;
}

#[tokio::test]
async fn native_control_can_stop_a_server_without_a_protocol_connection() {
    let control = BrowserInstanceControl::new();
    // A caller may request shutdown before the serving future is polled.
    // The request must survive startup without requiring a CDP owner.
    let caller = control.clone();
    std::thread::spawn(move || caller.request(InstanceCommand::Shutdown))
        .join()
        .expect("native control caller");
    let server = ProtocolServer::new(ServerConfig {
        host: "127.0.0.1".into(),
        port: 0,
        ..ServerConfig::default()
    });
    timeout(Duration::from_secs(2), server.serve_with_control(control))
        .await
        .expect("native shutdown is observed without a protocol connection")
        .expect("native shutdown completes cleanly");
}

async fn assert_instance_shutdown(native: bool) {
    let profile = TempDir::new("browser-close-storage");
    fs::create_dir_all(&profile.path).expect("own storage directory");
    let (addr, task, state) = instance(Some(profile.path.clone())).await;
    let (other_addr, other_task, other_state) = instance(None).await;
    let mut root = browser(addr).await;
    let mut other_root = browser(other_addr).await;
    let first = create_dynamic_target(&mut root, 1).await;
    let _second = create_dynamic_target(&mut root, 2).await;
    let mut page = connect_dynamic_page(addr, &first).await;
    let cookie = send_cdp_command(&mut page, 3, "Network.setCookie", None,
        json!({"name": "owned-close", "value": "persisted", "url": "https://browser-close.invalid/"})).await;
    assert!(response_by_id(&cookie, 3)["error"].is_null());

    // Keep both independent BiDi and Classic-attached BiDi sockets open while
    // closing the instance; upgraded connections cannot be left detached.
    let (mut bidi, _) = connect_async(format!("ws://{addr}/session"))
        .await
        .expect("BiDi");
    let bidi_session = send_cdp_command(&mut bidi, 4, "session.new", None, json!({})).await;
    assert_eq!(response_by_id(&bidi_session, 4)["type"], "success");
    let classic = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("Classic session request"),
        )
        .await
        .expect("Classic response");
    assert_eq!(classic.status(), StatusCode::OK);
    let classic: serde_json::Value = serde_json::from_slice(
        &to_bytes(classic.into_body(), 65536)
            .await
            .expect("Classic body"),
    )
    .expect("Classic JSON");
    let classic_id = classic["value"]["sessionId"].as_str().expect("Classic ID");
    let (mut attached, _) = connect_async(format!("ws://{addr}/session/{classic_id}"))
        .await
        .expect("attached BiDi");
    let attached_ready =
        send_cdp_command(&mut attached, 5, "browser.getUserContexts", None, json!({})).await;
    assert_eq!(response_by_id(&attached_ready, 5)["type"], "success");
    assert!(state.classic_session_registry.has_session(classic_id));

    if native {
        let control = state.control.clone();
        std::thread::spawn(move || control.request(InstanceCommand::Shutdown))
            .join()
            .expect("native instance shutdown caller");
    } else {
        close_instance(&mut root, 100).await;
    }
    joined(task).await;
    closed(&mut root).await;
    closed(&mut page).await;
    closed(&mut bidi).await;
    closed(&mut attached).await;
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "closed listener must not accept"
    );
    assert_eq!(state.cdp_owner_registry.owner_count(), 0);
    assert!(!state.classic_session_registry.has_session(classic_id));
    assert_eq!(state.bidi_session_registry.lock().active_session_count(), 0);
    drop(state);
    let persisted = StoragePartitionState::open(Some(&profile.path))
        .expect("reopen closed profile")
        .cookies()
        .expect("read persisted cookies");
    assert!(
        persisted
            .iter()
            .any(|cookie| cookie.name == "owned-close" && cookie.value == "persisted")
    );

    let version =
        send_cdp_command(&mut other_root, 101, "Browser.getVersion", None, json!({})).await;
    assert!(response_by_id(&version, 101)["result"]["product"].is_string());
    assert!(!other_task.is_finished(), "another instance survives");
    assert!(other_state.control.connection().is_some());
    close_instance(&mut other_root, 102).await;
    joined(other_task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_session_and_frontend_disconnect_do_not_stop_instance() {
    let (addr, task, state) = instance(None).await;
    let mut root = browser(addr).await;
    let target = create_dynamic_target(&mut root, 1).await;
    let mut page = connect_dynamic_page(addr, &target).await;
    let attach = send_cdp_command(
        &mut root,
        2,
        "Target.attachToTarget",
        None,
        json!({"targetId": target, "flatten": true}),
    )
    .await;
    let session = response_by_id(&attach, 2)["result"]["sessionId"]
        .as_str()
        .expect("attached ID");
    let mut foreign = browser(addr).await;
    let foreign_close =
        send_cdp_command(&mut foreign, 3, "Browser.close", Some(session), json!({})).await;
    assert_eq!(response_by_id(&foreign_close, 3)["error"]["code"], -32001);
    let unknown = send_cdp_command(&mut root, 4, "Browser.close", Some("missing"), json!({})).await;
    assert_eq!(response_by_id(&unknown, 4)["error"]["code"], -32001);
    root.send(WsMessage::Text(
        json!({"id": 5, "method": "Browser.close", "sessionId": null})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let invalid = recv_until_id(&mut root, 5).await;
    assert_eq!(response_by_id(&invalid, 5)["error"]["code"], -32600);
    root.close(None).await.expect("disconnect viewer only");
    closed(&mut root).await;
    let alive = send_cdp_command(
        &mut page,
        6,
        "Runtime.evaluate",
        None,
        json!({"expression": "21 * 2"}),
    )
    .await;
    assert_eq!(response_by_id(&alive, 6)["result"]["result"]["value"], 42);
    assert!(!task.is_finished());
    assert!(state.control.connection().is_some());
    let mut replacement = browser(addr).await;
    close_instance(&mut replacement, 7).await;
    joined(task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_close_accepts_owned_browser_page_tab_and_nested_routes() {
    for scope in [
        "root",
        "browser-session",
        "nested-browser-session",
        "page-session",
        "page-socket",
        "tab-session",
    ] {
        let (addr, task, _state) = instance(None).await;
        let mut root = browser(addr).await;
        let target = create_dynamic_target(&mut root, 1).await;
        let mut session_id = None;
        if matches!(scope, "browser-session" | "nested-browser-session") {
            let attach = send_cdp_command(
                &mut root,
                2,
                "Target.attachToBrowserTarget",
                None,
                json!({}),
            )
            .await;
            session_id = Some(
                response_by_id(&attach, 2)["result"]["sessionId"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
            if scope == "nested-browser-session" {
                let attach = send_cdp_command(
                    &mut root,
                    3,
                    "Target.attachToBrowserTarget",
                    session_id.as_deref(),
                    json!({}),
                )
                .await;
                session_id = Some(
                    response_by_id(&attach, 3)["result"]["sessionId"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        } else if matches!(scope, "page-session" | "tab-session") {
            let target = if scope == "tab-session" {
                let targets = send_cdp_command(
                    &mut root,
                    4,
                    "Target.getTargets",
                    None,
                    json!({"filter": [{"type": "tab"}]}),
                )
                .await;
                response_by_id(&targets, 4)["result"]["targetInfos"][0]["targetId"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            } else {
                target.clone()
            };
            let attach = send_cdp_command(
                &mut root,
                5,
                "Target.attachToTarget",
                None,
                json!({"targetId": target, "flatten": true}),
            )
            .await;
            session_id = Some(
                response_by_id(&attach, 5)["result"]["sessionId"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        let mut selected = if scope == "page-socket" {
            connect_dynamic_page(addr, &target).await
        } else {
            root
        };
        let response = send_cdp_command(
            &mut selected,
            20,
            "Browser.close",
            session_id.as_deref(),
            json!({"ignored": true}),
        )
        .await;
        let mut expected = json!({"id": 20, "result": {}});
        if let Some(session_id) = session_id {
            expected["sessionId"] = json!(session_id);
        }
        assert_eq!(response_by_id(&response, 20), &expected, "scope={scope}");
        joined(task).await;
        closed(&mut selected).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_close_requests_finish_once_without_hanging() {
    let (addr, task, state) = instance(None).await;
    let mut first = browser(addr).await;
    let mut second = browser(addr).await;
    let (left, right) = tokio::join!(
        first.send(WsMessage::Text(
            json!({"id": 1, "method": "Browser.close"})
                .to_string()
                .into()
        )),
        second.send(WsMessage::Text(
            json!({"id": 2, "method": "Browser.close", "params": {}})
                .to_string()
                .into()
        ))
    );
    assert!(left.is_ok() || right.is_ok());
    async fn accepted(socket: &mut TestCdpSocket) -> bool {
        timeout(Duration::from_secs(2), async {
            while let Some(Ok(message)) = socket.next().await {
                if let WsMessage::Text(text) = message {
                    let response: serde_json::Value =
                        serde_json::from_str(&text).expect("close JSON");
                    if response["result"] == json!({}) {
                        return true;
                    }
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }
    let (left, right) = tokio::join!(accepted(&mut first), accepted(&mut second));
    assert!(
        left || right,
        "at least one close is acknowledged before exit"
    );
    joined(task).await;
    assert_eq!(state.cdp_owner_registry.owner_count(), 0);
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_close_does_not_wait_for_an_unresolved_runtime_command() {
    let (addr, task, _state) = instance(None).await;
    let mut root = browser(addr).await;
    let target = create_dynamic_target(&mut root, 1).await;
    let mut page = connect_dynamic_page(addr, &target).await;
    send_cdp_command(&mut page, 2, "Runtime.enable", None, json!({})).await;
    send_cdp_command_without_wait(
        &mut page,
        3,
        "Runtime.evaluate",
        None,
        json!({"expression": "console.log('close-pending'); new Promise(() => {})", "awaitPromise": true}),
    ).await;
    let started = recv_until_match(&mut page, |message| {
        message["method"] == "Runtime.consoleAPICalled"
            && message["params"]["args"][0]["value"] == "close-pending"
    })
    .await;
    assert!(!started.iter().any(|message| message["id"] == 3));
    close_instance(&mut root, 4).await;
    joined(task).await;
    closed(&mut page).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incomplete_worker_drain_is_bounded_and_not_reported_as_clean_shutdown() {
    let (addr, task, state) = instance(None).await;
    let held_worker = state
        .control
        .connection()
        .expect("controlled stalled worker");
    let mut root = browser(addr).await;
    close_instance(&mut root, 1).await;
    let error = timeout(Duration::from_secs(7), task)
        .await
        .expect("bounded shutdown")
        .expect("server task joins")
        .expect_err("stalled worker cannot be clean shutdown");
    assert!(error.to_string().contains("storage may be incomplete"));
    assert_eq!(state.cdp_owner_registry.owner_count(), 0);
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    drop(held_worker);
}
