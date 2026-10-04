use super::*;

async fn auto_attach(socket: &mut TestCdpSocket) {
    send_cdp_command(
        socket,
        1,
        "Target.setAutoAttach",
        None,
        json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}),
    )
    .await;
}

fn attached_session(messages: &[serde_json::Value], url: &str) -> String {
    let event = messages
        .iter()
        .find(|message| {
            message["method"] == "Target.attachedToTarget"
                && message["params"]["targetInfo"]["url"] == url
        })
        .expect("page attachment");
    assert_eq!(event["params"]["waitingForDebugger"], true);
    event["params"]["sessionId"].as_str().unwrap().to_owned()
}

async fn next_attached_session(socket: &mut TestCdpSocket, url: &str) -> String {
    let events = recv_until_match(socket, |message| {
        message["method"] == "Target.attachedToTarget"
            && message["params"]["targetInfo"]["url"] == url
    })
    .await;
    attached_session(&events, url)
}

async fn evaluate(socket: &mut TestCdpSocket, session: &str) -> serde_json::Value {
    let messages = send_cdp_command(
        socket,
        100,
        "Runtime.evaluate",
        Some(session),
        json!({"expression":"[location.href,window.started===true]","returnByValue":true}),
    )
    .await;
    response_by_id(&messages, 100)["result"]["result"]["value"].clone()
}

async fn wait_loaded(socket: &mut TestCdpSocket, session: &str, url: &str) {
    timeout(Duration::from_secs(5), async {
        loop {
            if evaluate(socket, session).await == json!([url, true]) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("initial page navigation completes without another Inspector resume");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_created_page_loads_with_two_waiting_auto_attach_observers() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async {
            axum::response::Html("<script>window.started=true</script>")
        })),
        "direct-auto-attach-startup",
    );
    let (addr, server) = spawn_test_protocol_server().await;
    let endpoint = format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}");
    let (mut first, _) = connect_async(&endpoint).await.unwrap();
    let (mut second, _) = connect_async(&endpoint).await.unwrap();
    auto_attach(&mut first).await;
    auto_attach(&mut second).await;
    let url = format!("http://{fixture_addr}/direct");
    let messages = send_cdp_command(
        &mut first,
        2,
        "Target.createTarget",
        None,
        json!({"url":url}),
    )
    .await;
    let first_session = attached_session(&messages, &url);
    let second_session = next_attached_session(&mut second, &url).await;
    wait_loaded(&mut first, &first_session, &url).await;
    assert_eq!(
        evaluate(&mut second, &second_session).await,
        json!([url, true])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn popup_startup_pause_is_shared_but_detaching_one_observer_does_not_resume_it() {
    let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fixture_requests = requests.clone();
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let requests = fixture_requests.clone();
            async move {
                if uri.path().starts_with("/child-") {
                    requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                axum::response::Html("<script>window.started=true</script>")
            }
        })),
        "popup-auto-attach-startup",
    );
    let (addr, server) = spawn_test_protocol_server().await;
    let endpoint = format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}");
    let (mut first, _) = connect_async(&endpoint).await.unwrap();
    let (mut second, _) = connect_async(&endpoint).await.unwrap();
    let parent_id = create_dynamic_target(&mut first, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(
        &mut parent,
        2,
        &format!("http://{fixture_addr}/parent"),
    )
    .await;
    auto_attach(&mut first).await;
    auto_attach(&mut second).await;
    for (index, (reverse, detach)) in [(false, false), (true, false), (false, true), (true, true)]
        .into_iter()
        .enumerate()
    {
        let url = format!("http://{fixture_addr}/child-{index}");
        evaluate_window_name_probe(
            &mut parent,
            10,
            &format!("window.open({},'_blank')!==null", json!(url)),
        )
        .await;
        let first_session = next_attached_session(&mut first, &url).await;
        let second_session = next_attached_session(&mut second, &url).await;
        assert_eq!(
            evaluate(&mut first, &first_session).await,
            json!(["about:blank", false])
        );
        assert_eq!(
            evaluate(&mut second, &second_session).await,
            json!(["about:blank", false])
        );
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), index);
        let (resumer, session, observer, other_session) = if reverse {
            (&mut second, &second_session, &mut first, &first_session)
        } else {
            (&mut first, &first_session, &mut second, &second_session)
        };
        if detach {
            send_cdp_command(
                observer,
                11,
                "Target.detachFromTarget",
                None,
                json!({"sessionId":other_session}),
            )
            .await;
            assert_eq!(
                evaluate(resumer, session).await,
                json!(["about:blank", false])
            );
            assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), index);
        }
        send_cdp_command(
            resumer,
            12,
            "Runtime.runIfWaitingForDebugger",
            Some(session),
            json!({}),
        )
        .await;
        wait_loaded(resumer, session, &url).await;
        if !detach {
            assert_eq!(evaluate(observer, other_session).await, json!([url, true]));
        }
        assert_eq!(
            requests.load(std::sync::atomic::Ordering::SeqCst),
            index + 1
        );
    }
    abort_test_cdp_server(server).await;
}
