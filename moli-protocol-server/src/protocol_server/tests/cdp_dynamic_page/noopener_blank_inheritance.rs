use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn noopener_blank_documents_keep_creator_security_and_storage_on_reload() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async {
            axum::response::Html("<body>creator</body>")
        })),
        "noopener-blank-inheritance",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &format!("{base}/creator")).await;
    evaluate_window_name_probe(
        &mut parent,
        3,
        "localStorage.setItem('shared','creator');sessionStorage.setItem('private','creator');true",
    )
    .await;
    for (index, (url, features)) in [
        ("", "noopener"),
        ("about:blank", "noopener"),
        ("", "noreferrer"),
        ("about:blank", "noreferrer"),
    ]
    .into_iter()
    .enumerate()
    {
        let (child_id, mut child) = popup_navigation_referrer::open_http_popup(
            addr,
            &mut parent,
            &format!("detached-{index}"),
            url,
            features,
        )
        .await;
        send_cdp_command(&mut child, 1, "Page.enable", None, json!({})).await;
        let target = send_cdp_command(
            &mut browser,
            10,
            "Target.getTargetInfo",
            None,
            json!({"targetId": child_id}),
        )
        .await;
        assert_eq!(
            response_by_id(&target, 10)["result"]["targetInfo"]["openerId"],
            parent_id
        );
        assert_eq!(
            response_by_id(&target, 10)["result"]["targetInfo"]["canAccessOpener"],
            false
        );
        for reload in [false, true] {
            if reload {
                let events = send_cdp_command(&mut child, 3, "Page.reload", None, json!({})).await;
                if !events
                    .iter()
                    .any(|event| event["method"] == "Page.loadEventFired")
                {
                    recv_until_match(&mut child, |event| event["method"] == "Page.loadEventFired")
                        .await;
                }
            }
            assert_eq!(evaluate_window_name_probe(&mut child, 4,
                "[location.href,origin,isSecureContext,opener===null,document.baseURI,localStorage.getItem('shared'),sessionStorage.getItem('private'),document.referrer]"
            ).await, json!(["about:blank", base, true, true, "about:blank", "creator", null, if features == "noreferrer" { String::new() } else { format!("{base}/") }]));
            let tree = send_cdp_command(&mut child, 5, "Page.getFrameTree", None, json!({})).await;
            let frame = &response_by_id(&tree, 5)["result"]["frameTree"]["frame"];
            assert_eq!(frame["securityOrigin"], base);
            assert_eq!(frame["secureContextType"], "Secure");
        }
    }
    abort_test_cdp_server(server).await;
}
