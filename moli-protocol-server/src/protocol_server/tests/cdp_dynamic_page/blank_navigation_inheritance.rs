use super::*;

async fn assert_named_blank_navigation_inherits_accepted_initiator(cross_origin: bool) {
    let fixture = || {
        Router::new().fallback(get(|| async {
            axum::response::Html("<title>fixture</title><body>document</body>")
        }))
    };
    let (first_addr, _first) = spawn_dedicated_fixture_server(fixture(), "blank-source-first");
    let (second_addr, _second) = spawn_dedicated_fixture_server(fixture(), "blank-source-second");
    let first = format!("http://{first_addr}");
    let second = format!("http://{second_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &format!("{first}/parent")).await;
    let (_, mut child) = auxiliary_page_identity::open_auxiliary(addr, &mut parent, "").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{first}/child")).await;
    evaluate_window_name_probe(
        &mut child,
        3,
        "window.marker=73;sessionStorage.setItem('kept','child-namespace');true",
    )
    .await;

    // The source is either the original parent or a related sibling which has
    // navigated to another origin. In both cases the destination is the same C.
    let mut sibling = if cross_origin {
        evaluate_window_name_probe(
            &mut parent,
            10,
            "window.savedChild=p;p.name='destination';true",
        )
        .await;
        let (_, mut page) = auxiliary_page_identity::open_auxiliary(addr, &mut parent, "").await;
        evaluate_window_name_probe(&mut page, 10, "name='sibling';true").await;
        evaluate_window_name_probe(&mut child, 10, "name='actual-page';true").await;
        evaluate_window_name_probe(&mut page, 11, "open('', 'actual-page');true").await;
        Some(page)
    } else {
        None
    };
    let (source, origin) = if let Some(sibling) = sibling.as_mut() {
        navigate_dynamic_page_and_wait_for_load(sibling, 2, &format!("{second}/sibling")).await;
        (sibling, &second)
    } else {
        (&mut parent, &first)
    };
    let accepted_base = format!("{origin}/accepted/");
    assert_eq!(
        evaluate_window_name_probe(
            source,
            4,
            &format!(
                r#"(() => {{
        const base=document.createElement('base');base.href={};document.head.append(base);
        window.reused=open('about:blank','actual-page');
        base.href={};return true;
    }})()"#,
                json!(accepted_base),
                json!(format!("{origin}/late/"))
            )
        )
        .await,
        true
    );
    recv_until_match(&mut child, |event| event["method"] == "Page.loadEventFired").await;
    assert_eq!(evaluate_window_name_probe(&mut child, 5,
        "[location.href,origin,document.baseURI,document.referrer,typeof marker,sessionStorage.getItem('kept'),opener===null]"
    ).await, json!([
        "about:blank", origin, accepted_base, format!("{origin}/"), "undefined",
        if cross_origin { None } else { Some("child-namespace") }, false,
    ]));
    assert_eq!(
        evaluate_window_name_probe(
            source,
            5,
            "reused.document.baseURI===location.origin+'/accepted/' && reused.opener===window"
        )
        .await,
        true
    );
    let tree = send_cdp_command(&mut child, 6, "Page.getFrameTree", None, json!({})).await;
    assert_eq!(
        response_by_id(&tree, 6)["result"]["frameTree"]["frame"]["securityOrigin"],
        *origin
    );
    assert_eq!(evaluate_window_name_probe(&mut child, 9,
        "(()=>{const before=document.domain;document.domain='127.0.0.1';return [before,document.domain]})()"
    ).await, json!(["127.0.0.1", "127.0.0.1"]));

    if cross_origin {
        assert_eq!(
            evaluate_window_name_probe(
                &mut parent,
                6,
                "(()=>{try{return savedChild.document.baseURI}catch(e){return e.name}})()"
            )
            .await,
            "SecurityError"
        );
    }
    let reloaded = send_cdp_command(&mut child, 7, "Page.reload", None, json!({})).await;
    if !reloaded
        .iter()
        .any(|event| event["method"] == "Page.loadEventFired")
    {
        recv_until_match(&mut child, |event| event["method"] == "Page.loadEventFired").await;
    }
    assert_eq!(
        evaluate_window_name_probe(&mut child, 8, "[origin,document.baseURI,document.referrer]")
            .await,
        json!([origin, accepted_base, format!("{origin}/")])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_blank_navigation_freezes_initiator_base_and_preserves_target_storage() {
    assert_named_blank_navigation_inherits_accepted_initiator(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_blank_navigation_inherits_cross_origin_sibling_environment() {
    assert_named_blank_navigation_inherits_accepted_initiator(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_blank_navigation_discards_the_targets_old_response_sandbox() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new()
            .route(
                "/child",
                get(|| async {
                    (
                        [("content-security-policy", "sandbox allow-scripts")],
                        axum::response::Html("<body>sandboxed response</body>"),
                    )
                }),
            )
            .fallback(get(|| async {
                axum::response::Html("<body>initiator</body>")
            })),
        "blank-old-response-sandbox",
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
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &format!("{base}/parent")).await;
    let (_, mut child) = auxiliary_page_identity::open_auxiliary(addr, &mut parent, "").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{base}/child")).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "document.body.textContent").await,
        "sandboxed response"
    );

    assert_eq!(
        evaluate_window_name_probe(&mut parent, 3, "open('about:blank','actual-page')===p").await,
        true
    );
    recv_until_match(&mut child, |event| event["method"] == "Page.loadEventFired").await;
    assert_eq!(evaluate_window_name_probe(&mut child, 4,
        "sessionStorage.setItem('test','new');[origin,sessionStorage.getItem('test'),isSecureContext]"
    ).await, json!([base, "new", true]));
    assert_eq!(
        evaluate_window_name_probe(
            &mut parent,
            4,
            "p.document.URL==='about:blank' && p.document.body!==null"
        )
        .await,
        true
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_blank_navigation_keeps_fixed_auxiliary_sandbox_from_its_creator() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new()
            .route(
                "/sandboxed-source",
                get(|| async {
                    (
                        [(
                            "content-security-policy",
                            "sandbox allow-scripts allow-popups allow-same-origin",
                        )],
                        axum::response::Html("<body>sandboxed creator</body>"),
                    )
                }),
            )
            .fallback(get(|| async {
                axum::response::Html("<body>parent</body>")
            })),
        "blank-fixed-frame-sandbox",
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
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &format!("{base}/sandboxed-source"))
        .await;
    let (_, mut child) = auxiliary_page_identity::open_auxiliary(addr, &mut parent, "").await;
    assert_fixed_sandbox_suppresses_modal(&mut child, 2).await;

    navigate_dynamic_page_and_wait_for_load(&mut parent, 3, &format!("{base}/parent")).await;
    // Prove that this client handles ordinary dialogs, so a default headless
    // response cannot mask the sandbox checks on the auxiliary Page below.
    send_cdp_command_without_wait(
        &mut parent,
        90,
        "Runtime.evaluate",
        None,
        json!({"expression":"confirm('unsandboxed control')","returnByValue":true}),
    )
    .await;
    recv_until_match(&mut parent, |event| {
        event["method"] == "Page.javascriptDialogOpening"
    })
    .await;
    send_cdp_command(
        &mut parent,
        91,
        "Page.handleJavaScriptDialog",
        None,
        json!({"accept":false}),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut parent, 6, "open('about:blank','actual-page')!==null")
            .await,
        true
    );
    recv_until_match(&mut child, |event| event["method"] == "Page.loadEventFired").await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "origin").await,
        base
    );
    assert_fixed_sandbox_suppresses_modal(&mut child, 4).await;
    abort_test_cdp_server(server).await;
}

async fn assert_fixed_sandbox_suppresses_modal(page: &mut TestCdpSocket, id: u64) {
    send_cdp_command_without_wait(
        page,
        id,
        "Runtime.evaluate",
        None,
        json!({"expression":"confirm('fixed sandbox')","returnByValue":true}),
    )
    .await;
    let messages = recv_until_match(page, |event| {
        event["id"] == id || event["method"] == "Page.javascriptDialogOpening"
    })
    .await;
    assert!(
        messages
            .iter()
            .all(|event| event["method"] != "Page.javascriptDialogOpening"),
        "fixed auxiliary-frame restrictions must suppress the dialog: {messages:?}"
    );
    assert_eq!(
        response_by_id(&messages, id)["result"]["result"]["value"],
        false
    );
}

async fn assert_repeated_named_blank_history(
    opaque_sandbox: bool,
    inherited_opaque: bool,
    relaxed_domain: bool,
) {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move || async move {
            (
                [(
                    "content-security-policy",
                    if opaque_sandbox {
                        "sandbox allow-scripts allow-popups"
                    } else {
                        ""
                    },
                )],
                axum::response::Html("<p>initiator</p>"),
            )
        })),
        "repeated-named-blank-history",
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
    let source_url = if inherited_opaque {
        "data:text/html,<p>opaque initiator</p>".to_owned()
    } else {
        format!("{base}/parent")
    };
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &source_url).await;
    if relaxed_domain {
        evaluate_window_name_probe(&mut parent, 3, "document.domain='127.0.0.1';true").await;
    }
    let (_, mut child) = auxiliary_page_identity::open_auxiliary(addr, &mut parent, "").await;
    for navigation in 1..=3 {
        evaluate_window_name_probe(&mut child, 10 + navigation, "window.marker=73;true").await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut parent,
                10 + navigation,
                "open('about:blank','actual-page')===p"
            )
            .await,
            true
        );
        recv_until_match(&mut child, |event| event["method"] == "Page.loadEventFired").await;
        let entries = if opaque_sandbox { navigation } else { 1 };
        assert_eq!(
            evaluate_window_name_probe(
                &mut child,
                20 + navigation,
                "[location.href,origin,history.length,typeof marker]"
            )
            .await,
            json!([
                "about:blank",
                if opaque_sandbox || inherited_opaque {
                    "null"
                } else {
                    &base
                },
                entries,
                "undefined"
            ])
        );
        let history = send_cdp_command(
            &mut child,
            30 + navigation,
            "Page.getNavigationHistory",
            None,
            json!({}),
        )
        .await;
        let result = &response_by_id(&history, 30 + navigation)["result"];
        assert_eq!(result["currentIndex"], entries - 1);
        assert_eq!(
            result["entries"].as_array().unwrap().len(),
            entries as usize
        );
        assert!(
            result["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["url"] == "about:blank")
        );
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_named_blank_navigation_replaces_history_using_inherited_document_origin() {
    assert_repeated_named_blank_history(false, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_named_blank_navigation_does_not_equate_sandboxed_null_origins() {
    assert_repeated_named_blank_history(true, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_named_blank_navigation_preserves_inherited_opaque_origin_identity() {
    assert_repeated_named_blank_history(false, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_named_blank_navigation_uses_document_origin_after_document_domain_mutation() {
    assert_repeated_named_blank_history(false, false, true).await;
}
