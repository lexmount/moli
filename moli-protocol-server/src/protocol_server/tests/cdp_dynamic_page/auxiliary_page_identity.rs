use super::*;

// The opener setters and closed-opener expectations also appear in Chromium's
// external/wpt/html/browsers/windows/auxiliary-browsing-contexts tests. These
// cases additionally observe the same browsing context through its CDP target.

pub(super) async fn open_auxiliary(
    addr: std::net::SocketAddr,
    opener: &mut TestCdpSocket,
    synchronous_script: &str,
) -> (String, TestCdpSocket) {
    open_auxiliary_with_url(addr, opener, "about:blank", synchronous_script).await
}

pub(super) async fn open_auxiliary_with_url(
    addr: std::net::SocketAddr,
    opener: &mut TestCdpSocket,
    url: &str,
    synchronous_script: &str,
) -> (String, TestCdpSocket) {
    let baseline = fetch_server_json(addr, "/json/list").await;
    let ids: Vec<_> = baseline
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].clone())
        .collect();
    assert_eq!(
        evaluate_window_name_probe(
            opener,
            90,
            &format!(
                "window.p = open({}, 'actual-page'); {synchronous_script}; p.opener === window",
                json!(url)
            )
        )
        .await,
        true
    );
    let targets = wait_for_target_list(addr, "the exact initial Page is adopted", |targets| {
        targets.len() == ids.len() + 1
    })
    .await;
    let id = targets.iter().find(|t| !ids.contains(&t["id"])).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut page = connect_dynamic_page(addr, &id).await;
    send_cdp_command(&mut page, 1, "Page.enable", None, json!({})).await;
    (id, page)
}

async fn wait_for_value(page: &mut TestCdpSocket, expression: &str, expected: serde_json::Value) {
    timeout(Duration::from_secs(5), async {
        loop {
            if evaluate_window_name_probe(page, 900, expression).await == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expression} did not become {expected}"));
}

pub(super) async fn resume_auxiliary(browser: &mut TestCdpSocket, target_id: &str) {
    let events = recv_until_match(browser, |message| {
        message["method"] == "Target.attachedToTarget"
            && message["params"]["targetInfo"]["targetId"] == target_id
    })
    .await;
    let session = events
        .iter()
        .find(|message| {
            message["method"] == "Target.attachedToTarget"
                && message["params"]["targetInfo"]["targetId"] == target_id
        })
        .unwrap()["params"]["sessionId"]
        .as_str()
        .unwrap();
    send_cdp_command(
        browser,
        21,
        "Runtime.runIfWaitingForDebugger",
        Some(session),
        json!({}),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_page_borrowed_close_uses_the_window_receiver() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (b_id, _b) = open_auxiliary(addr, &mut opener, "p.name='b';window.b=p").await;
    let (c_id, mut c) = open_auxiliary(
        addr,
        &mut opener,
        "p.name='c';window.c=p;p.document.body.textContent='keep c'",
    )
    .await;

    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            2,
            "(() => {try {c.close.call({});return false} catch(e) {return e.name==='TypeError'}})()"
        )
        .await,
        true
    );
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            3,
            "c.close.call(window);c.close.call(b);[b.closed,c.closed]"
        )
        .await,
        json!([true, false])
    );
    wait_for_target_list(addr, "only the close receiver is removed", |targets| {
        !targets.iter().any(|t| t["id"] == b_id)
            && targets.iter().any(|t| t["id"] == c_id)
            && targets.iter().any(|t| t["id"] == opener_id)
    })
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut c, 2, "document.body.textContent").await,
        "keep c"
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_cross_page_timer_queues_notify_their_owners() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (_, mut child) = open_auxiliary(
        addr,
        &mut opener,
        "window.timerResult=[];window.canceled=false",
    )
    .await;
    // Wait on the caller Page. Polling the timer owner through CDP would itself
    // restore that Page and hide a missing queue-to-owner notification.
    let completed = send_cdp_command(
        &mut child,
        4,
        "Runtime.evaluate",
        None,
        json!({"awaitPromise": true, "returnByValue": true, "expression": r#"
new Promise(resolve => {
  const record = name => {
    opener.timerResult.push(name);
    if (opener.timerResult.length === 4) resolve(opener.timerResult.slice().sort());
  };
  opener.setTimeout(() => record('timeout'), 20);
  let ticks = 0;
  const interval = opener.setInterval(() => {
    if (++ticks === 2) { record('interval'); opener.clearInterval(interval); }
  }, 5);
  opener.requestAnimationFrame(() => record('animation'));
  opener.requestIdleCallback(() => record('idle'));
  const canceled = opener.setTimeout(() => opener.canceled = true, 0);
  opener.clearTimeout(canceled);
})
"#}),
    )
    .await;
    assert_eq!(
        response_by_id(&completed, 4)["result"]["result"]["value"],
        json!(["animation", "idle", "interval", "timeout"])
    );
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 5, "canceled").await,
        false
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_page_keeps_synchronous_state_and_both_window_proxies_across_navigation() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|uri: axum::http::Uri| async move {
            axum::response::Html(format!("<h1>{}</h1>", uri.path()))
        })),
        "auxiliary-page-identity",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/parent")).await;
    evaluate_window_name_probe(
        &mut opener,
        3,
        "sessionStorage.setItem('seed','creator'); true",
    )
    .await;
    let (_, mut child) = open_auxiliary(
        addr,
        &mut opener,
        r#"
        p.initial = 17;
        p.document.body.textContent = 'initial';
        window.savedDocument = p.document;
        window.savedFunction = p.Function('return document.body.textContent');
        p.received=[];
        p.onmessage=p.Function('event','received.push(event.data)');
        p.postMessage('before-adoption','*');
        p.setTimeout(p.Function('window.timerReady=true'),0);
        sessionStorage.setItem('seed', 'parent-only')
    "#,
    )
    .await;
    assert_eq!(evaluate_window_name_probe(&mut child, 2,
        "[initial, document.body.textContent, origin, document.baseURI, document.referrer, sessionStorage.getItem('seed'), isSecureContext]"
    ).await, json!([17, "initial", base, format!("{base}/parent"), format!("{base}/parent"), "creator", true]));
    wait_for_value(
        &mut child,
        "[received,window.timerReady]",
        json!([["before-adoption"], true]),
    )
    .await;
    evaluate_window_name_probe(
        &mut child,
        3,
        "document.body.textContent='child'; window.savedOpener=opener; true",
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            4,
            "[p.document===savedDocument, p.document.body.textContent, p.savedOpener===window]"
        )
        .await,
        json!([true, "child", true])
    );
    navigate_dynamic_page_and_wait_for_load(&mut child, 4, &format!("{base}/new-child")).await;
    // The first same-origin navigation retains the function's Window, whose
    // document binding now points to the new Document. Explicit old handles
    // still refer to the initial Document.
    assert_eq!(evaluate_window_name_probe(&mut opener, 5,
        "[savedDocument.body.textContent, savedFunction(), p.document.querySelector('h1').textContent, p.opener===window]"
    ).await, json!(["child", "/new-child", "/new-child", true]));
    evaluate_window_name_probe(&mut child, 5, "window.savedOpener=opener; true").await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 6, &format!("{base}/new-parent")).await;
    assert_eq!(evaluate_window_name_probe(&mut child, 6,
        "[savedOpener===opener, opener.location.pathname, opener.document.querySelector('h1').textContent]"
    ).await, json!([true, "/new-parent", "/new-parent"]));
    send_cdp_command(
        &mut browser,
        2,
        "Target.closeTarget",
        None,
        json!({"targetId":opener_id}),
    )
    .await;
    wait_for_target_list(addr, "opener removed", |targets| {
        !targets.iter().any(|t| t["id"] == opener_id)
    })
    .await;
    wait_for_value(
        &mut child,
        "[opener===null,savedOpener.closed]",
        json!([true, true]),
    )
    .await;
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_page_messages_and_rejections_reach_the_actual_page() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>page</p>") })),
        "auxiliary-page-messages",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/parent")).await;
    let (_, mut child) = open_auxiliary(addr, &mut opener, "").await;
    evaluate_window_name_probe(
        &mut child,
        2,
        r#"
        window.received=[];
        onmessage=e=>received.push([e.data,e.origin,e.source===opener]);
        onunhandledrejection=e=>{e.preventDefault();opener.rejected.push(e.reason)};
        window.rejectInChild=()=>Promise.reject('child-rejection'); true
    "#,
    )
    .await;
    evaluate_window_name_probe(
        &mut opener,
        3,
        r#"
        window.rejected=[];window.received=[];
        onmessage=e=>received.push([e.data,e.origin,e.source===p]);
        p.postMessage('default'); postMessage.call(p,'borrowed','*');
        p.rejectInChild(); true
    "#,
    )
    .await;
    wait_for_value(
        &mut child,
        "received",
        json!([["default", base, true], ["borrowed", base, true]]),
    )
    .await;
    wait_for_value(&mut opener, "rejected", json!(["child-rejection"])).await;
    evaluate_window_name_probe(&mut child, 3, "opener.postMessage('reply','*');true").await;
    wait_for_value(&mut opener, "received", json!([["reply", base, true]])).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_page_created_by_a_link_has_the_live_opener() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>page</p>") })),
        "auxiliary-page-link",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/parent")).await;
    for (index, from_child) in [false, true].into_iter().enumerate() {
        let baseline = fetch_server_json(addr, "/json/list").await;
        let ids: Vec<_> = baseline
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].clone())
            .collect();
        evaluate_window_name_probe(
            &mut opener,
            10 + index as u64,
            &format!(
                r#"
            (()=>{{ window.linkOpener = window;
            if ({from_child}) {{
              const f=document.createElement('iframe');document.body.append(f);
              window.linkOpener=f.contentWindow;
            }}
            linkOpener.marker=27;
            const a=linkOpener.document.createElement('a');
            a.rel='opener';a.target='_blank';a.href={};
            linkOpener.document.body.append(a);a.click();return true; }})()
        "#,
                json!(format!("{base}/child"))
            ),
        )
        .await;
        let targets = wait_for_target_list(addr, "link creates auxiliary Page", |t| {
            t.len() == ids.len() + 1
        })
        .await;
        let id = targets.iter().find(|t| !ids.contains(&t["id"])).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut child = connect_dynamic_page(addr, &id).await;
        send_cdp_command(&mut child, 1, "Page.enable", None, json!({})).await;
        wait_for_value(&mut child, "location.pathname", json!("/child")).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut child,
                2,
                "opener.marker===27 && (opener.childRef=window)===window"
            )
            .await,
            true
        );
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                20 + index as u64,
                "linkOpener.childRef.opener===linkOpener"
            )
            .await,
            true
        );
        navigate_dynamic_page_and_wait_for_load(&mut child, 3, &format!("{base}/after-link")).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                30 + index as u64,
                "linkOpener.childRef.location.pathname"
            )
            .await,
            "/after-link"
        );
        send_cdp_command(
            &mut browser,
            10 + index as u64,
            "Target.closeTarget",
            None,
            json!({"targetId":id}),
        )
        .await;
        wait_for_target_list(addr, "linked auxiliary closes", |t| {
            !t.iter().any(|t| t["id"] == id)
        })
        .await;
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_page_inherited_sandbox_keeps_opaque_origins_distinct() {
    // Chromium's sandbox-inherit-to-blank-document WPT cases distinguish a
    // newly sandboxed opaque origin from an escaped popup inheriting its creator.
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>page</p>") })),
        "auxiliary-page-sandbox",
    );
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let target = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target).await;
    send_cdp_command(&mut page, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut page, 2, &format!("http://{fixture_addr}/parent"))
        .await;
    for (index, (flags, expected)) in [
        ("allow-scripts allow-popups", "SecurityError"),
        (
            "allow-scripts allow-popups allow-same-origin",
            "about:blank",
        ),
        (
            "allow-scripts allow-popups allow-popups-to-escape-sandbox",
            "about:blank",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let html = "<script>const w=open('about:blank','_blank');let result;try{result=w.document.URL}catch(e){result=e.name};parent.postMessage(result,'*')</script>";
        evaluate_window_name_probe(
            &mut page,
            10 + index as u64,
            &format!(
                r#"
            (()=>{{window.results=[];onmessage=e=>results.push(e.data);
            const f=document.createElement('iframe');f.sandbox={};f.srcdoc={};
            document.body.append(f);return true;}})()
        "#,
                json!(flags),
                json!(html)
            ),
        )
        .await;
        wait_for_value(&mut page, "results", json!([expected])).await;
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_page_cross_origin_surface_delivers_messages_and_blocks_javascript_navigation() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>page</p>") })),
        "auxiliary-page-cross-origin",
    );
    let base = format!("http://{fixture_addr}");
    let cross = format!("http://localhost:{}", fixture_addr.port());
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/parent")).await;
    let (child_id, mut child) = open_auxiliary(addr, &mut opener, "").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{cross}/child")).await;
    evaluate_window_name_probe(
        &mut child,
        3,
        "window.received=[];onmessage=e=>received.push([e.data,e.origin,e.source===opener]);true",
    )
    .await;
    evaluate_window_name_probe(&mut opener, 3, "p.postMessage('cross','*');true").await;
    wait_for_value(&mut child, "received", json!([["cross", base, true]])).await;
    evaluate_window_name_probe(
        &mut child,
        30,
        "window.f=document.createElement('iframe');document.body.append(f);true",
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 30, "[p.length,p[0].parent===p,p[0].top===p]")
            .await,
        json!([1, true, true])
    );
    evaluate_window_name_probe(&mut child, 31, "f.remove();true").await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            31,
            "(()=>{try{return p[0]}catch(e){return e.name}})()"
        )
        .await,
        "SecurityError"
    );
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            4,
            r#"
        (()=>{
          const attempt=f=>{try{f();return 'allowed'}catch(e){return e.name}};
          return [p.closed,p.top===p,p.parent===p,p.opener===window,p.length,
            attempt(()=>p.document),
            attempt(()=>Object.getOwnPropertyDescriptor(window,'document').get.call(p)),
            attempt(()=>p.location.href),
            attempt(()=>p.location='javascript:window.crossOriginScriptRan=true'),
            attempt(()=>p.location.href=' \nJaVaScRiPt:window.crossOriginScriptRan=true'),
            attempt(()=>p.location.replace('java\tscript:window.crossOriginScriptRan=true'))];
        })()
    "#
        )
        .await,
        json!([
            false,
            true,
            true,
            true,
            0,
            "SecurityError",
            "SecurityError",
            "SecurityError",
            "SecurityError",
            "SecurityError",
            "SecurityError"
        ])
    );
    assert_eq!(
        evaluate_window_name_probe(&mut child, 4, "window.crossOriginScriptRan===true").await,
        false
    );
    evaluate_window_name_probe(
        &mut opener,
        5,
        &format!("p.location.replace({});true", json!(format!("{base}/back"))),
    )
    .await;
    recv_until_match(&mut child, |m| m["method"] == "Page.loadEventFired").await;
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 6, "p.location.pathname").await,
        "/back"
    );
    navigate_dynamic_page_and_wait_for_load(&mut child, 7, &format!("{cross}/child")).await;
    send_cdp_command(
        &mut browser,
        2,
        "Target.closeTarget",
        None,
        json!({"targetId":child_id}),
    )
    .await;
    wait_for_target_list(addr, "cross-origin child removed", |targets| {
        !targets.iter().any(|t| t["id"] == child_id)
    })
    .await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            7,
            "[p.closed,p.parent===null,p.top===null,p.length]"
        )
        .await,
        json!([true, true, true, 0])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_initial_history_stays_provisional_until_document_navigation() {
    // Blink NavigationApi::HasEntriesAndEventsDisabled and FrameLoader's
    // initial-empty-document state are independent from the CDP placeholder.
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async {
            axum::response::Html("<p>real document</p>")
        })),
        "auxiliary-initial-history",
    );
    let parent_url = format!("http://{fixture_addr}/parent");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &parent_url).await;
    for (index, operation) in [
        "location.hash='one'",
        "history.pushState({value:1},'', 'about:blank#one')",
        "history.replaceState({value:1},'', 'about:blank#one')",
        "history.pushState({value:1},'', '')",
        "history.pushState({value:1},'', null)",
        "document.open();document.write('<p>written</p>');document.close()",
    ]
    .into_iter()
    .enumerate()
    {
        let (child_id, mut child) = open_auxiliary(addr, &mut parent,
            "window.initialHistory=[p.history.length,p.navigation.entries().length,p.navigation.currentEntry]").await;
        assert_eq!(
            evaluate_window_name_probe(&mut parent, 100, "initialHistory").await,
            json!([1, 0, null])
        );
        let snapshot = "[history.length,navigation.entries().length,navigation.currentEntry,navigation.canGoBack,navigation.canGoForward,document.baseURI]";
        let initial = json!([1, 0, null, false, false, parent_url]);
        assert_eq!(
            evaluate_window_name_probe(&mut child, 3, snapshot).await,
            initial
        );
        assert_eq!(evaluate_window_name_probe(&mut child, 4, r#"(()=>{
            const check=url=>{try{history.pushState(null,'',url);return 'allowed'}catch(e){return e.name}};
            return [check('#forbidden'),check('/forbidden')];
        })()"#).await, json!(["SecurityError","SecurityError"]));
        evaluate_window_name_probe(&mut child, 5,
            "window.events=[];for(const type of ['navigate','currententrychange','navigatesuccess'])navigation.addEventListener(type,()=>events.push(type));true").await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut child,
                6,
                &format!("(()=>{{{operation};return true}})()")
            )
            .await,
            true
        );
        assert_eq!(
            evaluate_window_name_probe(&mut child, 7, snapshot).await,
            initial,
            "{operation}"
        );
        assert_eq!(
            evaluate_window_name_probe(&mut child, 8, "events").await,
            json!([]),
            "{operation}"
        );
        let before =
            send_cdp_command(&mut child, 9, "Page.getNavigationHistory", None, json!({})).await;
        let before = before.iter().find(|message| message["id"] == 9).unwrap();
        assert_eq!(
            before["result"]["entries"].as_array().unwrap().len(),
            1,
            "{operation}: {before}"
        );
        assert_eq!(before["result"]["entries"][0]["userTypedURL"], "");
        assert_eq!(before["result"]["entries"][0]["transitionType"], "link");
        let destination = format!("http://{fixture_addr}/real-{index}");
        navigate_dynamic_page_and_wait_for_load(&mut child, 10, &destination).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut child,
                11,
                "[history.length,navigation.entries().length,navigation.currentEntry.url]"
            )
            .await,
            json!([1, 1, destination])
        );
        let after =
            send_cdp_command(&mut child, 12, "Page.getNavigationHistory", None, json!({})).await;
        let after = after.iter().find(|message| message["id"] == 12).unwrap();
        assert_eq!(
            after["result"]["entries"].as_array().unwrap().len(),
            1,
            "{operation}: {after}"
        );
        send_cdp_command(
            &mut browser,
            10 + index as u64,
            "Target.closeTarget",
            None,
            json!({"targetId":child_id}),
        )
        .await;
        wait_for_target_list(addr, "initial-history child closed", |targets| {
            !targets.iter().any(|t| t["id"] == child_id)
        })
        .await;
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_initial_document_adoption_keeps_closed_window_name() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (_, _child) = open_auxiliary(addr, &mut opener, "").await;
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 4, "p.close();p.name").await,
        json!("actual-page"),
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_auxiliary_name_is_retained_and_ignores_writes_after_conversion() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>parent</p>") })),
        "auxiliary-closed-name",
    );
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
        &format!("http://{fixture_addr}/parent"),
    )
    .await;
    for by_protocol in [false, true] {
        let (child_id, mut child) = open_auxiliary(addr, &mut parent, "p.name='kept'").await;
        assert_eq!(
            evaluate_window_name_probe(&mut child, 2, "name").await,
            "kept"
        );
        if by_protocol {
            send_cdp_command(
                &mut browser,
                10,
                "Target.closeTarget",
                None,
                json!({"targetId":child_id}),
            )
            .await;
        } else {
            evaluate_window_name_probe(&mut parent, 10, "p.close();true").await;
        }
        wait_for_target_list(addr, "the named popup is closed", |targets| {
            !targets.iter().any(|target| target["id"] == child_id)
        })
        .await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut parent,
                11,
                r#"(()=>{
            const before=p.name; let converted=0;
            p.name={toString(){converted++;return 'ignored'}};
            return [p.closed,before,p.name,converted];
        })()"#
            )
            .await,
            json!([true, "kept", "kept", 1])
        );
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_browser_and_noopener_blank_documents_keep_their_committed_history() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>loaded</p>") })),
        "explicit-blank-history",
    );
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    assert_eq!(
        evaluate_window_name_probe(&mut parent, 2, "history.length").await,
        1
    );
    navigate_dynamic_page_and_wait_for_load(
        &mut parent,
        3,
        &format!("http://{fixture_addr}/parent"),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut parent, 4, "history.length").await,
        2
    );
    let before = fetch_server_json(addr, "/json/list")
        .await
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        evaluate_window_name_probe(
            &mut parent,
            5,
            "window.open('about:blank','independent','noopener')===null"
        )
        .await,
        true
    );
    let targets = wait_for_target_list(addr, "a noopener page is created", |targets| {
        targets.len() == before.len() + 1
    })
    .await;
    let child_id = targets
        .iter()
        .find(|target| !before.iter().any(|old| old["id"] == target["id"]))
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let mut child = connect_dynamic_page(addr, child_id).await;
    send_cdp_command(&mut child, 1, "Page.enable", None, json!({})).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 2, "history.length").await,
        1
    );
    assert_eq!(
        evaluate_window_name_probe(&mut child, 20, "navigation.entries().map(entry=>entry.url)")
            .await,
        json!(["about:blank"])
    );
    let history =
        send_cdp_command(&mut child, 3, "Page.getNavigationHistory", None, json!({})).await;
    let entry = &response_by_id(&history, 3)["result"]["entries"][0];
    assert_eq!(entry["userTypedURL"], "about:blank");
    assert_eq!(entry["transitionType"], "link");
    let destination = format!("http://{fixture_addr}/child");
    navigate_dynamic_page_and_wait_for_load(&mut child, 4, &destination).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 5, "history.length").await,
        2
    );
    let history =
        send_cdp_command(&mut child, 6, "Page.getNavigationHistory", None, json!({})).await;
    let entries = response_by_id(&history, 6)["result"]["entries"]
        .as_array()
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["url"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["about:blank", destination.as_str()]
    );
    assert_eq!(
        evaluate_window_name_probe(&mut child, 21, "navigation.entries().map(entry=>entry.url)")
            .await,
        json!(["about:blank", destination])
    );
    let cross_origin_destination = format!("http://localhost:{}/foreign", fixture_addr.port());
    navigate_dynamic_page_and_wait_for_load(&mut child, 22, &cross_origin_destination).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 23, "navigation.entries().map(entry=>entry.url)")
            .await,
        json!([cross_origin_destination])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_blank_reload_preserves_origin_storage_and_fallback_base() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>parent</p>") })),
        "auxiliary-blank-reload",
    );
    let base = format!("http://{fixture_addr}");
    let parent_url = format!("{base}/parent");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut parent, 2, &parent_url).await;
    evaluate_window_name_probe(
        &mut parent,
        3,
        "sessionStorage.setItem('seed','creator');true",
    )
    .await;
    for browser_reload in [false, true] {
        let (child_id, mut child) = open_auxiliary(
            addr,
            &mut parent,
            "p.document.body.textContent='old';window.oldDocument=p.document",
        )
        .await;
        evaluate_window_name_probe(
            &mut child,
            2,
            "document.head.innerHTML='<base href=\"https://different.test/base/\">';true",
        )
        .await;
        if browser_reload {
            send_cdp_command(&mut child, 3, "Page.reload", None, json!({})).await;
        } else {
            evaluate_window_name_probe(&mut child, 3, "location.reload();true").await;
        }
        wait_for_value(&mut child, "document.body.textContent", json!("")).await;
        let referrer = if browser_reload {
            format!("{base}/")
        } else {
            String::new()
        };
        let document_base = if browser_reload {
            parent_url.as_str()
        } else {
            "https://different.test/base/"
        };
        assert_eq!(evaluate_window_name_probe(&mut child, 4,
            "[origin,document.baseURI,document.referrer,sessionStorage.getItem('seed'),history.length,navigation.entries().map(e=>e.url)]").await,
            json!([base,document_base,referrer,"creator",1,["about:blank"]]));
        assert_eq!(evaluate_window_name_probe(&mut parent, 4,
            "[p.document!==oldDocument,oldDocument.body.textContent,p.document.body.textContent,p.opener===window]").await,
            json!([true,"old","",true]));
        // A later explicit browser navigation creates a new opaque origin.
        navigate_dynamic_page_and_wait_for_load(&mut child, 5, "about:blank").await;
        assert_eq!(evaluate_window_name_probe(&mut child, 6,
            "[origin,document.baseURI,(()=>{try{return sessionStorage.getItem('seed')}catch(e){return e.name}})()]").await,
            json!(["null","about:blank","SecurityError"]));
        send_cdp_command(
            &mut browser,
            20,
            "Target.closeTarget",
            None,
            json!({"targetId":child_id}),
        )
        .await;
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_cross_origin_functions_and_descriptors_use_each_accessing_realm() {
    // Chromium's cross-origin-objects.html WPT checks local Function.prototype,
    // distinct per-observer functions, and shared Window/Location identity.
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>page</p>") })),
        "auxiliary-cross-origin-realms",
    );
    let base = format!("http://{fixture_addr}");
    let cross = format!("http://localhost:{}", fixture_addr.port());
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/parent")).await;
    let (child_id, mut child) =
        open_auxiliary(addr, &mut opener, "p.name='target';window.w=p").await;
    let (_, mut observer) = open_auxiliary(addr, &mut opener, "p.name='observer'").await;
    evaluate_window_name_probe(&mut observer, 2, "window.w=opener.w;true").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{cross}/child")).await;
    evaluate_window_name_probe(
        &mut child,
        3,
        "window.received=[];onmessage=e=>received.push([e.data,e.origin,e.source===opener]);true",
    )
    .await;
    let realm_probe = r#"
        (()=>{
          window.remoteClose=w.close;
          window.remoteLocation=w.location;
          window.remoteParentGetter=Object.getOwnPropertyDescriptor(w,'parent').get;
          window.remoteHrefSetter=Object.getOwnPropertyDescriptor(w.location,'href').set;
          const functions=[w.postMessage,w.close,w.focus,w.blur,w.location.replace,
            remoteParentGetter,remoteHrefSetter,
            Object.getOwnPropertyDescriptor(w,'location').get,
            Object.getOwnPropertyDescriptor(w,'location').set];
          return [functions.every(f=>Object.getPrototypeOf(f)===Function.prototype),
            Object.getPrototypeOf(Object.getOwnPropertyDescriptor(w,'parent'))===Object.prototype,
            Object.getPrototypeOf(Object.getOwnPropertyDescriptor(w.location,'href'))===Object.prototype,
            remoteClose===w.close,
            remoteParentGetter===Object.getOwnPropertyDescriptor(w,'parent').get,
            remoteHrefSetter===Object.getOwnPropertyDescriptor(w.location,'href').set,
            Reflect.ownKeys(w.location).includes('href'),
            Reflect.ownKeys(w.location).includes('replace')];
        })()
    "#;
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 3, realm_probe).await,
        json!([true, true, true, true, true, true, true, true])
    );
    assert_eq!(
        evaluate_window_name_probe(&mut observer, 3, realm_probe).await,
        json!([true, true, true, true, true, true, true, true])
    );
    assert_eq!(evaluate_window_name_probe(&mut observer, 4,
        "[w===opener.w,remoteLocation===opener.remoteLocation,remoteClose!==opener.remoteClose,remoteParentGetter!==opener.remoteParentGetter,remoteHrefSetter!==opener.remoteHrefSetter]"
    ).await, json!([true,true,true,true,true]));
    evaluate_window_name_probe(
        &mut opener,
        4,
        r#"
        Object.getPrototypeOf(w.postMessage).__moliRealmProbe=73;
        Object.getPrototypeOf(Object.getPrototypeOf(w.postMessage)).__moliObjectRealmProbe=91;
        w.postMessage('opener','*');true
    "#,
    )
    .await;
    assert_eq!(evaluate_window_name_probe(&mut child, 4,
        "[Function.prototype.__moliRealmProbe===undefined,Object.prototype.__moliObjectRealmProbe===undefined]"
    ).await, json!([true,true]));
    assert_eq!(evaluate_window_name_probe(&mut observer, 5,
        "[Function.prototype.__moliRealmProbe===undefined,Object.prototype.__moliObjectRealmProbe===undefined]"
    ).await, json!([true,true]));
    wait_for_value(&mut child, "received", json!([["opener", base, true]])).await;
    evaluate_window_name_probe(&mut observer, 6, "w.postMessage('observer','*');true").await;
    wait_for_value(
        &mut child,
        "received",
        json!([["opener", base, true], ["observer", base, false]]),
    )
    .await;
    evaluate_window_name_probe(&mut observer, 7, "w.close();true").await;
    wait_for_target_list(addr, "cross-origin close retains its target", |targets| {
        !targets.iter().any(|t| t["id"] == child_id)
    })
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut observer, 8, "[w.closed,w.parent===null,w.length]").await,
        json!([true, true, 0])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_page_adoption_preserves_native_node_and_owner_document_identity() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (_, _child) = open_auxiliary(addr, &mut opener, "").await;
    assert_eq!(evaluate_window_name_probe(&mut opener, 2, r#"(()=>{
        window.div=document.createElement('div');
        window.span=document.createElement('span');div.append(span);
        p.document.body.append(div);
        return [div.ownerDocument===p.document,span.ownerDocument===p.document,p.document.body.firstChild===div,div.ownerDocument===document,div.ownerDocument.defaultView===p,p.document.defaultView===p];
    })()"#).await, json!([true,true,true,false,true,true]));
    assert_eq!(evaluate_window_name_probe(&mut opener, 3, r#"(()=>{
        document.body.append(div);
        return [div.ownerDocument===document,span.ownerDocument===document,document.body.lastChild===div];
    })()"#).await, json!([true,true,true]));
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_same_origin_popup_commit_preserves_window_and_replaces_document() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|uri: axum::http::Uri| async move {
            if uri.path() == "/child/dir" {
                tokio::time::sleep(Duration::from_millis(200)).await;
                axum::response::Html(
                    "<script>window.serverMarker=typeof marker;</script><p>child</p>",
                )
            } else {
                axum::response::Html("<p>parent</p>")
            }
        })),
        "review-initial-popup-state",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/source/start")).await;
    let baseline = fetch_server_json(addr, "/json/list").await;
    let ids: Vec<_> = baseline
        .as_array()
        .unwrap()
        .iter()
        .map(|target| target["id"].clone())
        .collect();
    assert_eq!(evaluate_window_name_probe(&mut opener, 3,
        "window.p=open('/child/dir','initial-state');p.marker=73;p.onload=()=>p.onloadMarker=91;p.addEventListener('load',()=>p.listenerMarker=92);window.oldArray=p.Array;window.oldDocument=p.document;oldDocument.body.innerHTML='<span id=old>initial</span>';oldDocument.addEventListener('retained',()=>p.oldDocumentMarker=93);window.oldFilter=()=>1;window.oldWalker=oldDocument.createTreeWalker(oldDocument.body,1,oldFilter);true").await, true);
    let targets = wait_for_target_list(addr, "review popup published", |targets| {
        targets.len() == ids.len() + 1
    })
    .await;
    let child_id = targets
        .iter()
        .find(|target| !ids.contains(&target["id"]))
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let mut child = connect_dynamic_page(addr, child_id).await;
    send_cdp_command(&mut child, 1, "Page.enable", None, json!({})).await;
    wait_for_value(
        &mut child,
        "typeof window.serverMarker === 'string'",
        json!(true),
    )
    .await;
    wait_for_value(&mut child, "document.readyState", json!("complete")).await;
    let observed = evaluate_window_name_probe(&mut opener, 4,
        "oldDocument.dispatchEvent(new p.Event('retained'));[p.marker??null,p.serverMarker,p.Array===oldArray,p.document!==oldDocument,oldDocument.URL,oldDocument.getElementById('old').textContent,p.onloadMarker,oldWalker.filter===oldFilter,oldWalker.nextNode().textContent,p.listenerMarker,p.oldDocumentMarker]").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{base}/replacement")).await;
    let subsequent = evaluate_window_name_probe(
        &mut opener,
        5,
        "[p.marker??null,p.Array===oldArray,p.document!==oldDocument]",
    )
    .await;
    abort_test_cdp_server(server).await;
    assert_eq!(
        observed,
        json!([
            73,
            "number",
            true,
            true,
            "about:blank",
            "initial",
            91,
            true,
            "initial",
            92,
            93
        ]),
        "first same-origin commit must keep the original Window and intrinsics"
    );
    assert_eq!(
        subsequent,
        json!([null, false, true]),
        "later navigation must replace the Window's realm"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_popup_commit_recreates_isolated_worlds_for_all_frontends() {
    let release = Arc::new(tokio::sync::Notify::new());
    let child_response = Arc::clone(&release);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let child_response = Arc::clone(&child_response);
            async move {
                if uri.path() == "/child" {
                    child_response.notified().await;
                    axum::response::Html("<main>popup document</main>")
                } else {
                    axum::response::Html("<main>opener document</main>")
                }
            }
        })),
        "initial-popup-isolated-worlds",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    send_cdp_command(
        &mut browser,
        20,
        "Target.setAutoAttach",
        None,
        json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}),
    )
    .await;
    let (child_id, mut child) = open_auxiliary_with_url(
        addr,
        &mut opener,
        "/child",
        "p.marker=73;window.oldArray=p.Array",
    )
    .await;
    send_cdp_command(&mut child, 2, "Runtime.enable", None, json!({})).await;
    let tree = send_cdp_command(&mut child, 3, "Page.getFrameTree", None, json!({})).await;
    let frame_id = response_by_id(&tree, 3)["result"]["frameTree"]["frame"]["id"]
        .as_str()
        .unwrap();
    let world_params = json!({
        "frameId": frame_id,
        "worldName": "popup-utility-world",
        "grantUniveralAccess": true,
    });
    let world = send_cdp_command(
        &mut child,
        4,
        "Page.createIsolatedWorld",
        None,
        world_params.clone(),
    )
    .await;
    let initial_context_id = response_by_id(&world, 4)["result"]["executionContextId"]
        .as_i64()
        .unwrap();
    let initial = send_cdp_command(
        &mut child,
        5,
        "Runtime.evaluate",
        None,
        json!({
            "contextId": initial_context_id,
            "expression": "window.initialUtilityMarker=91;document.URL",
            "returnByValue": true,
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&initial, 5)["result"]["result"]["value"],
        "about:blank"
    );

    let mut peer = connect_dynamic_page(addr, &child_id).await;
    send_cdp_command(&mut peer, 1, "Runtime.enable", None, json!({})).await;
    let peer_world = send_cdp_command(
        &mut peer,
        2,
        "Page.createIsolatedWorld",
        None,
        world_params.clone(),
    )
    .await;
    let peer_context_id = response_by_id(&peer_world, 2)["result"]["executionContextId"]
        .as_i64()
        .unwrap();
    assert_ne!(peer_context_id, initial_context_id);

    assert_eq!(evaluate_window_name_probe(&mut child, 9,
        "document.body.innerHTML='<iframe id=nested></iframe>';void document.querySelector('iframe').contentWindow;true"
    ).await, true);
    let nested_tree = send_cdp_command(&mut child, 10, "Page.getFrameTree", None, json!({})).await;
    let nested_frame_id = response_by_id(&nested_tree, 10)["result"]["frameTree"]["childFrames"][0]
        ["frame"]["id"]
        .as_str()
        .unwrap();
    let nested_world = send_cdp_command(
        &mut child,
        11,
        "Page.createIsolatedWorld",
        None,
        json!({
            "frameId": nested_frame_id, "worldName": "nested-popup-utility",
        }),
    )
    .await;
    let nested_context_id = response_by_id(&nested_world, 11)["result"]["executionContextId"]
        .as_i64()
        .unwrap();

    release.notify_one();
    resume_auxiliary(&mut browser, &child_id).await;
    wait_for_value(&mut child, "document.URL", json!(format!("{base}/child"))).await;
    wait_for_value(&mut child, "document.readyState", json!("complete")).await;
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 4, "[p.marker,p.Array===oldArray]").await,
        json!([73, true]),
        "retiring utility worlds must preserve the main Window and its intrinsics"
    );

    // A late Runtime frontend must not try to transfer registrations that
    // were destroyed along with the initial Document's inspector agent.
    let mut observer = connect_dynamic_page(addr, &child_id).await;
    let enabled = send_cdp_command(&mut observer, 1, "Runtime.enable", None, json!({})).await;
    assert_eq!(response_by_id(&enabled, 1)["result"], json!({}));
    for frontend in [&mut child, &mut peer, &mut observer] {
        for retired_id in [initial_context_id, peer_context_id, nested_context_id] {
            let retired = send_cdp_command(
                frontend,
                8,
                "Runtime.evaluate",
                None,
                json!({
                    "contextId": retired_id,
                    "expression": "document.URL",
                    "returnByValue": true,
                }),
            )
            .await;
            assert!(
                response_by_id(&retired, 8).get("error").is_some(),
                "the replacement Inspector must reject retired execution context IDs"
            );
        }
        let world = send_cdp_command(
            frontend,
            6,
            "Page.createIsolatedWorld",
            None,
            world_params.clone(),
        )
        .await;
        let context_id = response_by_id(&world, 6)["result"]["executionContextId"]
            .as_i64()
            .unwrap();
        assert_ne!(context_id, initial_context_id);
        assert_ne!(context_id, peer_context_id);
        let evaluated = send_cdp_command(
            frontend,
            7,
            "Runtime.evaluate",
            None,
            json!({
                "contextId": context_id,
                "expression": "[document.URL,document.querySelector('main').textContent,typeof initialUtilityMarker]",
                "returnByValue": true,
            }),
        )
        .await;
        assert_eq!(
            response_by_id(&evaluated, 7)["result"]["result"]["value"],
            json!([format!("{base}/child"), "popup document", "undefined"]),
            "each frontend must evaluate against the committed Document"
        );
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn document_open_preserves_isolated_worlds_across_frontends() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;
    send_cdp_command(&mut page, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(
        &mut page,
        2,
        "data:text/html,<main>original document</main>",
    )
    .await;
    let mut peer = connect_dynamic_page(addr, &target_id).await;
    let tree = send_cdp_command(&mut page, 3, "Page.getFrameTree", None, json!({})).await;
    let params = json!({
        "frameId": response_by_id(&tree, 3)["result"]["frameTree"]["frame"]["id"],
        "worldName": "document-open-utility",
    });
    let mut context_ids = Vec::new();
    for frontend in [&mut page, &mut peer] {
        send_cdp_command(frontend, 4, "Runtime.enable", None, json!({})).await;
        let world = send_cdp_command(
            frontend,
            5,
            "Page.createIsolatedWorld",
            None,
            params.clone(),
        )
        .await;
        let context_id = response_by_id(&world, 5)["result"]["executionContextId"]
            .as_i64()
            .unwrap();
        context_ids.push(context_id);
        send_cdp_command(
            frontend,
            6,
            "Runtime.evaluate",
            None,
            json!({
                "contextId": context_id,
                "expression": "window.utilityMarker=91;true",
                "returnByValue": true,
            }),
        )
        .await;
    }
    assert_ne!(context_ids[0], context_ids[1]);
    assert_eq!(
        evaluate_window_name_probe(
            &mut page,
            7,
            "document.open();document.write('<main>opened document</main>');document.close();true"
        )
        .await,
        true
    );

    let mut observer = connect_dynamic_page(addr, &target_id).await;
    send_cdp_command(&mut observer, 1, "Runtime.enable", None, json!({})).await;
    for (frontend, context_id) in [&mut page, &mut peer].into_iter().zip(context_ids) {
        let cached = send_cdp_command(
            frontend,
            8,
            "Page.createIsolatedWorld",
            None,
            params.clone(),
        )
        .await;
        assert_eq!(
            response_by_id(&cached, 8)["result"]["executionContextId"],
            context_id
        );
        let evaluated = send_cdp_command(
            frontend,
            9,
            "Runtime.evaluate",
            None,
            json!({
                "contextId": context_id,
                "expression": "[utilityMarker,document.querySelector('main').textContent]",
                "returnByValue": true,
            }),
        )
        .await;
        assert_eq!(
            response_by_id(&evaluated, 9)["result"]["result"]["value"],
            json!([91, "opened document"])
        );
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_same_origin_popup_commit_rebinds_xml_and_text_parsers() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|uri: axum::http::Uri| async move {
            let (content_type, body) = match uri.path() {
                "/text" => ("text/plain", "plain <node>"),
                "/xml" => ("application/xml", "<root>xml</root>"),
                _ => ("text/html", "<p>parent</p>"),
            };
            ([("content-type", content_type)], body)
        })),
        "initial-popup-parser-modes",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/parent")).await;
    for (index, (path, content_type, root_name, text)) in [
        ("/text", "text/plain", "html", "plain <node>"),
        ("/xml", "application/xml", "root", "xml"),
    ]
    .into_iter()
    .enumerate()
    {
        let (child_id, mut child) = open_auxiliary_with_url(
            addr,
            &mut opener,
            path,
            "p.marker=73;window.oldDocument=p.document;window.oldArray=p.Array;",
        )
        .await;
        wait_for_value(&mut child, "document.URL", json!(format!("{base}{path}"))).await;
        wait_for_value(&mut child, "document.readyState", json!("complete")).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                10 + index as u64,
                r#"[p.marker, p.Array===oldArray, p.document!==oldDocument,
                    p.document.contentType, p.document.documentElement.localName,
                    (p.document.body||p.document.documentElement).textContent]"#,
            )
            .await,
            json!([73, true, true, content_type, root_name, text]),
            "the response parser must target the new {content_type} Document"
        );
        send_cdp_command(
            &mut browser,
            10 + index as u64,
            "Target.closeTarget",
            None,
            json!({"targetId":child_id}),
        )
        .await;
        wait_for_target_list(addr, "parser-mode popup removed", |targets| {
            !targets.iter().any(|target| target["id"] == child_id)
        })
        .await;
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_window_location_uses_source_base_and_referrer() {
    let requests = Arc::new(parking_lot::Mutex::new(Vec::<(String, String)>::new()));
    let captured = Arc::clone(&requests);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(
            move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
                let captured = Arc::clone(&captured);
                async move {
                    captured.lock().push((
                        uri.path().to_owned(),
                        headers
                            .get("referer")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("")
                            .to_owned(),
                    ));
                    axum::response::Html("<p>page</p>")
                }
            },
        )),
        "review-popup-source-environment",
    );
    let source = format!("http://{fixture_addr}");
    let target = format!("http://localhost:{}", fixture_addr.port());
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{source}/source/start"))
        .await;
    let (_, mut child) = open_auxiliary(addr, &mut opener, "").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{target}/target/start")).await;
    evaluate_window_name_probe(&mut opener, 3, "p.location.href='next';true").await;
    recv_until_match(&mut child, |message| {
        message["method"] == "Page.loadEventFired"
    })
    .await;
    let actual_url = evaluate_window_name_probe(&mut child, 4, "location.href").await;
    let request = requests
        .lock()
        .iter()
        .find(|(path, _)| path.ends_with("/next"))
        .cloned()
        .unwrap();
    abort_test_cdp_server(server).await;
    assert_eq!(
        (actual_url, request),
        (
            json!(format!("{source}/source/next")),
            ("/source/next".to_owned(), format!("{source}/source/start"))
        ),
        "Location navigation must preserve the entry base URL and incumbent outgoing referrer"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_window_blank_location_inherits_source_environment() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|uri: axum::http::Uri| async move {
            (
                [(
                    "content-security-policy",
                    if uri.path().starts_with("/target/") {
                        "sandbox allow-scripts"
                    } else {
                        ""
                    },
                )],
                axum::response::Html("<p>page</p>"),
            )
        })),
        "review-popup-blank-environment",
    );
    let source = format!("http://{fixture_addr}");
    let target = format!("http://localhost:{}", fixture_addr.port());
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{source}/source/start"))
        .await;
    let (_, mut child) = open_auxiliary(addr, &mut opener, "").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{target}/target/start")).await;
    evaluate_window_name_probe(&mut opener, 3, "p.location.href='about:blank';true").await;
    recv_until_match(&mut child, |message| {
        message["method"] == "Page.loadEventFired"
    })
    .await;
    let facts = evaluate_window_name_probe(&mut child, 4, "[origin, document.baseURI]").await;
    let access = evaluate_window_name_probe(
        &mut opener,
        4,
        "(()=>{try{return p.document.URL}catch(error){return error.name}})()",
    )
    .await;
    abort_test_cdp_server(server).await;
    assert_eq!(
        (facts, access),
        (
            json!([source, format!("{source}/source/start")]),
            json!("about:blank")
        ),
        "a referenced about:blank navigation inherits its source rather than target environment"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_initial_document_adoption_keeps_its_history_entry() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (_, _child) = open_auxiliary(
        addr,
        &mut opener,
        "window.initialHistoryLength=p.history.length",
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 2, "[initialHistoryLength,p.history.length]",)
            .await,
        json!([1, 1]),
    );
    abort_test_cdp_server(server).await;
}
