use super::*;
use serde_json::Value;

#[tokio::test]
async fn native_navigation_admission_does_not_block_another_pages_inspection() {
    native_admission_does_not_block_inspection(
        "Page.navigate",
        json!({"url":"data:text/html,next"}),
    )
    .await;
}

#[tokio::test]
async fn native_reload_admission_does_not_block_another_pages_inspection() {
    native_admission_does_not_block_inspection("Page.reload", json!({})).await;
}

#[tokio::test]
async fn native_input_admission_does_not_block_another_pages_inspection() {
    native_admission_does_not_block_inspection("Input.insertText", json!({"text":"native"})).await;
}

#[tokio::test]
async fn native_policy_admission_does_not_block_another_pages_inspection() {
    native_admission_does_not_block_inspection(
        "Emulation.setTimezoneOverride",
        json!({"timezoneId":"Asia/Shanghai"}),
    )
    .await;
}

#[tokio::test]
async fn native_document_policy_admission_does_not_block_another_pages_inspection() {
    native_admission_does_not_block_inspection(
        "Emulation.setIdleOverride",
        json!({"isUserActive":true,"isScreenUnlocked":true}),
    )
    .await;
}

#[tokio::test]
async fn native_network_policy_admission_does_not_block_another_pages_inspection() {
    native_admission_does_not_block_inspection(
        "Network.setExtraHTTPHeaders",
        json!({"headers":{"x-policy":"native"}}),
    )
    .await;
}

async fn native_admission_does_not_block_inspection(method: &str, params: Value) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = protocol_server_test_state(
        addr,
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
    );
    let browser = state.browser_service.handle();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let mut sessions = Vec::new();
    for id in [1, 4] {
        let created = send_cdp_command(
            &mut socket,
            id,
            "Target.createTarget",
            None,
            json!({"url":"about:blank"}),
        )
        .await;
        let target =
            created.iter().find(|message| message["id"] == id).unwrap()["result"]["targetId"]
                .as_str()
                .unwrap();
        let attached = send_cdp_command(
            &mut socket,
            id + 1,
            "Target.attachToTarget",
            None,
            json!({"targetId":target,"flatten":true}),
        )
        .await;
        let session = attached
            .iter()
            .find(|message| message["id"] == id + 1)
            .unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        let ready = send_cdp_command(
            &mut socket,
            id + 2,
            "Runtime.evaluate",
            Some(&session),
            json!({"expression":"6 * 7","returnByValue":true}),
        )
        .await;
        assert_eq!(
            ready
                .iter()
                .find(|message| message["id"] == id + 2)
                .unwrap()["result"]["result"]["value"],
            42
        );
        sessions.push(session);
    }
    let release = browser.block_owner_for_test().unwrap();
    let (finished, completion) = std::sync::mpsc::channel();
    // The owner gate is released even if the protocol actor blocks its own
    // executor. The deadline detects failure; it never schedules production.
    let watchdog = std::thread::spawn(move || {
        let progressed = completion.recv_timeout(Duration::from_secs(5)).is_ok();
        drop(release);
        progressed
    });
    socket
        .send(WsMessage::Text(
            json!({"id":10,"method":method,"sessionId":sessions[0],"params":params})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket.send(WsMessage::Text(json!({"id":11,"method":"Runtime.evaluate","sessionId":sessions[1],"params":{"expression":"6 * 7","returnByValue":true}}).to_string().into())).await.unwrap();
    let inspection = recv_until_match(&mut socket, |message| message["id"] == 11).await;
    let _ = finished.send(());
    let progressed = watchdog.join().unwrap();
    let reply = inspection
        .iter()
        .find(|message| message["id"] == 11)
        .unwrap();
    assert_eq!(reply["result"]["result"]["value"], 42);
    let native = if let Some(reply) = inspection.iter().find(|message| message["id"] == 10) {
        reply.clone()
    } else {
        recv_until_match(&mut socket, |message| message["id"] == 10)
            .await
            .into_iter()
            .find(|message| message["id"] == 10)
            .unwrap()
    };
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
    assert!(native["error"].is_null(), "{native}");
    assert!(
        progressed,
        "{method} blocked independent inspection until BrowserOwner was released"
    );
}

#[tokio::test]
async fn commands_and_renderer_replies_advance_during_continuous_browser_activity() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let completed = timeout(Duration::from_secs(5), async {
        let created = send_cdp_command(
            &mut socket,
            1,
            "Target.createTarget",
            None,
            json!({"url":"about:blank"}),
        )
        .await;
        let target =
            created.iter().find(|message| message["id"] == 1).unwrap()["result"]["targetId"]
                .as_str()
                .unwrap();
        let attached = send_cdp_command(
            &mut socket,
            2,
            "Target.attachToTarget",
            None,
            json!({"targetId":target, "flatten":true}),
        )
        .await;
        let session =
            attached.iter().find(|message| message["id"] == 2).unwrap()["result"]["sessionId"]
                .as_str()
                .unwrap()
                .to_owned();
        for (id, method) in [(3, "Runtime.enable"), (4, "Network.enable")] {
            let messages =
                send_cdp_command(&mut socket, id, method, Some(&session), json!({})).await;
            assert!(
                messages.iter().find(|message| message["id"] == id).unwrap()["error"].is_null()
            );
        }
        // Each iteration publishes native Browser title/network facts and
        // renderer console output. Only this test can stop the producer.
        let mut messages = send_cdp_command(
            &mut socket,
            5,
            "Runtime.evaluate",
            Some(&session),
            json!({"expression":r#"
                globalThis.producing = true;
                globalThis.produced = 0;
                globalThis.producer = (async () => {
                    while (producing) {
                        document.title = 'fairness-' + produced;
                        const body = await fetch('data:text/plain,fairness').then(r => r.text());
                        if (body !== 'fairness') throw new Error('unexpected body');
                        console.log('fairness', ++produced);
                        await new Promise(resolve => setTimeout(resolve, 0));
                    }
                })();
                undefined;
            "#}),
        )
        .await;
        if !messages
            .iter()
            .any(|message| message["method"] == "Runtime.consoleAPICalled")
        {
            messages.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Runtime.consoleAPICalled"
                        && message["params"]["args"][0]["value"] == "fairness"
                })
                .await,
            );
        }
        for id in 10..26 {
            let replies = send_cdp_command(
                &mut socket,
                id,
                "Runtime.evaluate",
                Some(&session),
                json!({"expression":"Promise.resolve([producing, produced, 6 * 7])",
                    "awaitPromise":true, "returnByValue":true}),
            )
            .await;
            let reply = replies.iter().find(|message| message["id"] == id).unwrap();
            assert!(reply["error"].is_null(), "{reply}");
            let value = &reply["result"]["result"]["value"];
            assert_eq!(
                value[0], true,
                "the producer must remain active until all replies arrive"
            );
            assert!(value[1].as_u64().unwrap() > 0);
            assert_eq!(value[2], 42);
            messages.extend(replies);
        }
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == "Network.loadingFinished")
        );
        assert!(
            messages
                .iter()
                .filter(|message| message["method"] == "Runtime.consoleAPICalled")
                .count()
                > 1
        );
        let stopped = send_cdp_command(
            &mut socket,
            30,
            "Runtime.evaluate",
            Some(&session),
            json!({"expression":"producing = false; producer", "awaitPromise":true}),
        )
        .await;
        let reply = stopped.iter().find(|message| message["id"] == 30).unwrap();
        assert!(reply["error"].is_null(), "{reply}");
        assert!(reply["result"]["exceptionDetails"].is_null(), "{reply}");
    })
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
    completed
        .expect("commands, completions and renderer output must advance while the producer runs");
}
