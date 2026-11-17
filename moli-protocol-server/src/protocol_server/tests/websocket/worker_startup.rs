use super::*;

#[tokio::test]
async fn websocket_worker_accepts_debugger_resume_before_script_loading_finishes() {
    check_startup_resume(false).await;
}

#[tokio::test]
async fn websocket_worker_detach_cancels_resume_queued_before_startup() {
    check_startup_resume(true).await;
}

async fn check_startup_resume(detach_before_start: bool) {
    let requested = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let worker_requested = Arc::clone(&requested);
    let worker_release = Arc::clone(&release);
    let fixture = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/html")],
                    "<!doctype html><title>Worker startup</title>",
                )
            }),
        )
        .route(
            "/worker.js",
            get(move || {
                let requested = Arc::clone(&worker_requested);
                let release = Arc::clone(&worker_release);
                async move {
                    requested.notify_one();
                    release.notified().await;
                    (
                        [(header::CONTENT_TYPE, "text/javascript")],
                        "globalThis.startupProbe = 42; postMessage(startupProbe);",
                    )
                }
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture, "worker-inspector-startup");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let page = cdp_create_default_session_and_navigate(
        &mut socket,
        &format!("http://{fixture_addr}/page"),
    )
    .await;
    send_cdp_command(
        &mut socket,
        10,
        "Target.setDiscoverTargets",
        None,
        json!({"discover": true}),
    )
    .await;
    send_cdp_command(
        &mut socket,
        11,
        "Target.setAutoAttach",
        Some(&page),
        json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
    )
    .await;
    let mut observed = send_cdp_command(
        &mut socket, 12, "Runtime.evaluate", Some(&page),
        json!({
            "expression": "globalThis.startupWorker = new Worker('/worker.js'); globalThis.workerReady = new Promise(resolve => startupWorker.onmessage = event => resolve(event.data)); 'created'",
            "returnByValue": true,
        }),
    ).await;
    timeout(Duration::from_secs(30), requested.notified())
        .await
        .expect("Worker main script requested");
    let created = |message: &serde_json::Value| {
        message["method"] == "Target.targetCreated"
            && message["params"]["targetInfo"]["type"] == "worker"
    };
    if !observed.iter().any(created) {
        observed.extend(recv_until_match(&mut socket, created).await);
    }
    let target = observed.iter().find(|message| created(message)).unwrap()["params"]["targetInfo"]
        ["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    // Discovery precedes main-script completion. Explicitly attach now rather
    // than waiting for the automatic attachment published at ScriptLoaded.
    let attach = send_cdp_command(
        &mut socket,
        13,
        "Target.attachToTarget",
        None,
        json!({"targetId": target, "flatten": true}),
    )
    .await;
    let mut worker =
        attach.iter().find(|message| message["id"] == 13).unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
    let resume = send_cdp_command(
        &mut socket,
        14,
        "Runtime.runIfWaitingForDebugger",
        Some(&worker),
        json!({}),
    )
    .await;
    assert!(
        resume
            .iter()
            .any(|message| message["id"] == 14 && message.get("result").is_some()),
        "resume must succeed before releasing HTTP headers: {resume:?}"
    );

    if detach_before_start {
        let detached = send_cdp_command(
            &mut socket,
            15,
            "Target.detachFromTarget",
            None,
            json!({"sessionId": worker}),
        )
        .await;
        assert!(
            detached
                .iter()
                .any(|message| message["id"] == 15 && message.get("result").is_some()),
            "{detached:?}"
        );
        let attach = send_cdp_command(
            &mut socket,
            16,
            "Target.attachToTarget",
            None,
            json!({"targetId": target, "flatten": true}),
        )
        .await;
        let replacement =
            attach.iter().find(|message| message["id"] == 16).unwrap()["result"]["sessionId"]
                .as_str()
                .unwrap()
                .to_owned();
        assert_ne!(worker, replacement);
        worker = replacement;
    }
    release.notify_one();

    if detach_before_start {
        let probe = send_cdp_command(
            &mut socket,
            17,
            "Runtime.evaluate",
            Some(&worker),
            json!({"expression": "typeof startupProbe", "returnByValue": true}),
        )
        .await;
        assert!(
            probe.iter().any(|message| message["id"] == 17
                && message["result"]["result"]["value"] == "undefined"),
            "retired session must not resume the new Worker incarnation: {probe:?}"
        );
        let resume = send_cdp_command(
            &mut socket,
            18,
            "Runtime.runIfWaitingForDebugger",
            Some(&worker),
            json!({}),
        )
        .await;
        assert!(
            resume
                .iter()
                .any(|message| message["id"] == 18 && message.get("result").is_some()),
            "{resume:?}"
        );
    }
    let ready = send_cdp_command(
        &mut socket,
        19,
        "Runtime.evaluate",
        Some(&page),
        json!({"expression": "workerReady", "awaitPromise": true, "returnByValue": true}),
    )
    .await;
    assert!(
        ready
            .iter()
            .any(|message| message["id"] == 19 && message["result"]["result"]["value"] == 42),
        "Worker bootstrap must execute after the live session resumes: {ready:?}"
    );
    let _ = send_cdp_command(
        &mut socket,
        20,
        "Runtime.evaluate",
        Some(&page),
        json!({"expression": "startupWorker.terminate(); 1", "returnByValue": true}),
    )
    .await;
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}
