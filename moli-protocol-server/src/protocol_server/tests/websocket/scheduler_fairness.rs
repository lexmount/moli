use super::*;

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
