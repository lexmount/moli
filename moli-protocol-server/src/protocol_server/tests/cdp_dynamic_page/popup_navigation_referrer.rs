use super::*;

fn referrer_fixture() -> Router {
    Router::new().fallback(get(|headers: axum::http::HeaderMap| async move {
        let referrer = headers.get("referer").and_then(|value| value.to_str().ok());
        let site = headers
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok());
        axum::response::Html(format!(
            "<title>received</title><script>window.receivedReferrer={};window.receivedSite={};</script>",
            json!(referrer), json!(site)
        ))
    }))
}

async fn wait_for_http_document(page: &mut TestCdpSocket, url: &str) {
    timeout(Duration::from_secs(5), async {
        loop {
            if evaluate_window_name_probe(
                page,
                900,
                &format!(
                    "location.href==={} && typeof receivedReferrer!=='undefined'",
                    json!(url)
                ),
            )
            .await
                == true
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("HTTP popup did not load {url}"));
}

pub(super) async fn open_http_popup(
    addr: std::net::SocketAddr,
    parent: &mut TestCdpSocket,
    name: &str,
    url: &str,
    features: &str,
) -> (String, TestCdpSocket) {
    let before = fetch_server_json(addr, "/json/list").await;
    let ids = before
        .as_array()
        .unwrap()
        .iter()
        .map(|target| target["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        evaluate_window_name_probe(
            parent,
            90,
            &format!(
                "window.lastPopup=open({},{},{});true",
                json!(url),
                json!(name),
                json!(features)
            )
        )
        .await,
        true
    );
    let targets = wait_for_target_list(addr, "HTTP popup accepted", |targets| {
        targets.len() == ids.len() + 1
    })
    .await;
    let id = targets
        .iter()
        .find(|target| !ids.contains(&target["id"]))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let page = connect_dynamic_page(addr, &id).await;
    (id, page)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn popup_navigation_uses_source_referrer_policy_and_window_features() {
    let (fixture_addr, _fixture) =
        spawn_dedicated_fixture_server(referrer_fixture(), "popup-referrer");
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(
        &mut parent,
        2,
        &format!("{base}/parent?source=accepted#fragment"),
    )
    .await;
    for (index, (policy, features, expected)) in [
        (
            "strict-origin-when-cross-origin",
            "",
            Some(format!("{base}/parent?source=accepted")),
        ),
        ("origin", "", Some(format!("{base}/"))),
        ("no-referrer", "", None),
        (
            "strict-origin-when-cross-origin",
            "noopener",
            Some(format!("{base}/parent?source=accepted")),
        ),
        ("strict-origin-when-cross-origin", "noreferrer", None),
    ]
    .into_iter()
    .enumerate()
    {
        evaluate_window_name_probe(&mut parent, 3, &format!(
            "(()=>{{let m=document.querySelector('meta[name=referrer]');if(!m){{m=document.createElement('meta');m.name='referrer';document.head.append(m)}}m.content={};return true}})()", json!(policy)
        )).await;
        let url = format!("{base}/child-{index}");
        let name = if features.is_empty() {
            format!("child-{index}")
        } else {
            // Noopener/noreferrer must create a new context even when this
            // related name already belongs to the first ordinary popup.
            "child-0".to_owned()
        };
        let (_, mut child) = open_http_popup(addr, &mut parent, &name, &url, features).await;
        wait_for_http_document(&mut child, &url).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut child,
                4,
                "[document.referrer,receivedReferrer,receivedSite,opener===null]"
            )
            .await,
            json!([
                expected.as_deref().unwrap_or(""),
                expected,
                "same-origin",
                !features.is_empty()
            ])
        );
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_popup_navigation_uses_the_callers_referrer_policy() {
    let (fixture_addr, _fixture) =
        spawn_dedicated_fixture_server(referrer_fixture(), "named-popup-referrer");
    let (other_addr, _other) =
        spawn_dedicated_fixture_server(referrer_fixture(), "named-popup-cross-referrer");
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &format!("{base}/parent")).await;
    let (_, mut child) =
        open_http_popup(addr, &mut parent, "child", &format!("{base}/child"), "").await;
    wait_for_http_document(&mut child, &format!("{base}/child")).await;
    let (_, mut sibling) =
        open_http_popup(addr, &mut parent, "sibling", &format!("{base}/sibling"), "").await;
    wait_for_http_document(&mut sibling, &format!("{base}/sibling")).await;
    let destination = format!("{base}/reused");
    evaluate_window_name_probe(
        &mut sibling,
        3,
        &format!(
            "open({},'child');history.replaceState(null,'','/later');true",
            json!(destination)
        ),
    )
    .await;
    wait_for_http_document(&mut child, &destination).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 4, "[document.referrer,receivedReferrer]").await,
        json!([format!("{base}/sibling"), format!("{base}/sibling")])
    );
    let destination = format!("http://{other_addr}/cross-reused");
    evaluate_window_name_probe(&mut sibling, 5, &format!(
        "(()=>{{const m=document.createElement('meta');m.name='referrer';m.content='no-referrer';document.head.append(m);open({},'child');m.content='unsafe-url';return true}})()",json!(destination)
    )).await;
    wait_for_http_document(&mut child, &destination).await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut child,
            6,
            "[document.referrer,receivedReferrer,receivedSite]"
        )
        .await,
        json!(["", null, "same-site"])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paused_popup_navigation_retains_accepted_referrer_context() {
    let (fixture_addr, _fixture) =
        spawn_dedicated_fixture_server(referrer_fixture(), "paused-popup-referrer");
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &format!("{base}/parent")).await;
    send_cdp_command(
        &mut browser,
        2,
        "Target.setAutoAttach",
        None,
        json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}),
    )
    .await;
    let destination = format!("{base}/paused-child");
    let (child_id, mut child) =
        open_http_popup(addr, &mut parent, "paused-child", &destination, "").await;
    let attached = recv_until_match(&mut browser, |event| {
        event["method"] == "Target.attachedToTarget"
            && event["params"]["targetInfo"]["targetId"] == child_id
    })
    .await;
    let attached = attached.last().expect("popup attachment event");
    assert_eq!(attached["params"]["waitingForDebugger"], true);
    evaluate_window_name_probe(&mut parent, 3,
        "(()=>{const m=document.createElement('meta');m.name='referrer';m.content='no-referrer';document.head.append(m);history.replaceState(null,'','/late-parent');return true})()"
    ).await;
    send_cdp_command(
        &mut browser,
        4,
        "Runtime.runIfWaitingForDebugger",
        attached["params"]["sessionId"].as_str(),
        json!({}),
    )
    .await;
    wait_for_http_document(&mut child, &destination).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 5, "[document.referrer,receivedReferrer]").await,
        json!([format!("{base}/parent"), format!("{base}/parent")])
    );
    abort_test_cdp_server(server).await;
}
