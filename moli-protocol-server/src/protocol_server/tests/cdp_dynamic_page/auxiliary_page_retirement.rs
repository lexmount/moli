use super::auxiliary_page_identity::{open_auxiliary, open_auxiliary_with_url, resume_auxiliary};
use super::*;

async fn wait_for_value(page: &mut TestCdpSocket, expression: &str, expected: serde_json::Value) {
    let mut actual = serde_json::Value::Null;
    timeout(Duration::from_secs(5), async {
        loop {
            actual = evaluate_window_name_probe(page, 900, expression).await;
            if actual == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expression} did not become {expected}; last value was {actual}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_popup_commit_keeps_window_timers_and_service_worker_completion() {
    let release = Arc::new(tokio::sync::Notify::new());
    let response = Arc::clone(&release);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let response = Arc::clone(&response);
            async move {
                if uri.path() == "/worker.js" {
                    return ([("content-type", "text/javascript")], "self.addEventListener('install',e=>e.waitUntil(self.skipWaiting()));self.addEventListener('activate',e=>e.waitUntil(self.clients.claim()));");
                }
                if uri.path() == "/child" {
                    response.notified().await;
                }
                ([("content-type", "text/html")], "<p>loaded</p>")
            }
        })),
        "initial-popup-window-tasks",
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
    let (child_id, mut child) = open_auxiliary_with_url(addr, &mut opener, "/child", "").await;
    evaluate_window_name_probe(&mut child, 2, "window.ticks=0;window.timerFired=false;window.timeoutId=setTimeout(()=>timerFired=true,900);window.intervalId=setInterval(()=>ticks++,200);true").await;
    wait_for_value(&mut child, "ticks>0", json!(true)).await;
    let ticks = evaluate_window_name_probe(&mut child, 3, "ticks")
        .await
        .as_u64()
        .unwrap();
    release.notify_one();
    resume_auxiliary(&mut browser, &child_id).await;
    wait_for_value(&mut child, "document.URL", json!(format!("{base}/child"))).await;
    wait_for_value(&mut child, "timerFired", json!(true)).await;
    wait_for_value(&mut child, &format!("ticks>{ticks}"), json!(true)).await;
    send_cdp_command(
        &mut browser,
        22,
        "Target.setAutoAttach",
        None,
        json!({"autoAttach":false,"waitForDebuggerOnStart":false,"flatten":true}),
    )
    .await;
    evaluate_window_name_probe(&mut child, 4, "window.registered='pending';navigator.serviceWorker.register('/worker.js').then(()=>registered='success',e=>registered=e.name);true").await;
    wait_for_value(&mut child, "registered", json!("success")).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 5, "clearInterval(intervalId);typeof timeoutId")
            .await,
        "number"
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn isolated_popup_navigation_releases_indexed_db_connections() {
    let release = Arc::new(tokio::sync::Notify::new());
    let response = Arc::clone(&release);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let response = Arc::clone(&response);
            async move {
                if uri.path() == "/child" {
                    response.notified().await;
                }
                axum::response::Html("<p>loaded</p>")
            }
        })),
        "isolated-popup-idb-retirement",
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
    let (child_id, mut child) = open_auxiliary_with_url(addr, &mut opener, "/child", "").await;
    let tree = send_cdp_command(&mut child, 2, "Page.getFrameTree", None, json!({})).await;
    let world = send_cdp_command(&mut child, 3, "Page.createIsolatedWorld", None, json!({"frameId":response_by_id(&tree,2)["result"]["frameTree"]["frame"]["id"],"worldName":"idb-retirement"})).await;
    let context_id = response_by_id(&world, 3)["result"]["executionContextId"].clone();
    let opened = send_cdp_command(&mut child, 4, "Runtime.evaluate", None, json!({"contextId":context_id,"expression":"new Promise((resolve,reject)=>{window.retained=new Array(1024*1024).fill(3);const r=indexedDB.open('isolated-retirement',1);r.onsuccess=()=>{window.db=r.result;resolve('opened')};r.onerror=()=>reject(r.error)})","awaitPromise":true,"returnByValue":true})).await;
    assert_eq!(
        response_by_id(&opened, 4)["result"]["result"]["value"],
        "opened"
    );
    release.notify_one();
    resume_auxiliary(&mut browser, &child_id).await;
    wait_for_value(&mut child, "document.URL", json!(format!("{base}/child"))).await;
    evaluate_window_name_probe(&mut child, 5, "window.upgrade='pending';const r=indexedDB.open('isolated-retirement',2);r.onblocked=()=>upgrade='blocked';r.onerror=()=>upgrade=r.error.name;r.onsuccess=()=>{upgrade='success';r.result.close()};true").await;
    wait_for_value(&mut child, "upgrade", json!("success")).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iframe_popup_location_navigation_uses_incumbent_document_referrer() {
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
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_owned(),
                    ));
                    axum::response::Html(if uri.path() == "/frame/inner" {
                        "<script>onmessage=()=>parent.p.location.href='next'</script>"
                    } else {
                        "<p>loaded</p>"
                    })
                }
            },
        )),
        "iframe-popup-location-referrer",
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
    let (_, mut child) = open_auxiliary(addr, &mut opener, "").await;
    evaluate_window_name_probe(
        &mut opener,
        3,
        "document.body.innerHTML='<iframe src=/frame/inner></iframe>';true",
    )
    .await;
    wait_for_value(&mut opener,"document.querySelector('iframe').contentDocument.URL.endsWith('/frame/inner') && document.querySelector('iframe').contentDocument.readyState==='complete'",json!(true)).await;
    evaluate_window_name_probe(
        &mut opener,
        4,
        "document.querySelector('iframe').contentWindow.postMessage('navigate','*');true",
    )
    .await;
    wait_for_value(
        &mut child,
        "document.URL",
        json!(format!("{base}/frame/next")),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "document.referrer").await,
        format!("{base}/frame/inner")
    );
    assert_eq!(
        requests
            .lock()
            .iter()
            .find(|(path, _)| path == "/frame/next")
            .unwrap()
            .1,
        format!("{base}/frame/inner")
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_window_blank_reload_keeps_target_base_and_referrer() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>loaded</p>") })),
        "cross-window-blank-reload-environment",
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
    let (_,mut child) = open_auxiliary(addr,&mut opener, "p.document.head.innerHTML='<base href=\"https://popup-base.test/dir/\">';p.document.body.textContent='old'").await;
    evaluate_window_name_probe(&mut opener, 3, "p.location.reload();true").await;
    wait_for_value(&mut child, "document.body.textContent", json!("")).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "[document.baseURI,document.referrer,origin]")
            .await,
        json!(["https://popup-base.test/dir/", "", base])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_origin_location_functions_accept_other_location_receivers() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>loaded</p>") })),
        "borrowed-cross-origin-location-functions",
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
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    let (_, mut child) = open_auxiliary(addr, &mut opener, "").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{cross}/child")).await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            3,
            r#"
        (()=>{
          const setter=Object.getOwnPropertyDescriptor(p.location,'href').set;
          const replace=p.location.replace;
          const attempt=f=>{try{f();return 'allowed'}catch(e){return e.name}};
          const invalid=[attempt(()=>setter.call({},'#bad')),attempt(()=>replace.call({},'#bad'))];
          setter.call(location,'#setter');
          const first=location.hash;
          replace.call(location,'#replace');
          return [first,location.hash,...invalid];
        })()
    "#
        )
        .await,
        json!(["#setter", "#replace", "TypeError", "TypeError"])
    );
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "location.href").await,
        format!("{cross}/child")
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_popup_locations_cannot_navigate_a_replacement_document() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>loaded</p>") })),
        "saved-popup-location-retirement",
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
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    for target in [&base, &cross] {
        let (child_id, mut child) = open_auxiliary(addr, &mut opener, "").await;
        navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{target}/one")).await;
        evaluate_window_name_probe(&mut opener, 3, "window.savedLocation=p.location;true").await;
        navigate_dynamic_page_and_wait_for_load(&mut child, 3, &format!("{target}/two")).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                4,
                &format!(
                    "savedLocation.replace({});true",
                    json!(format!("{target}/stale"))
                )
            )
            .await,
            true
        );
        // A subsequent command observes any synchronously admitted navigation.
        evaluate_window_name_probe(&mut child, 4, "true").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            evaluate_window_name_probe(&mut child, 5, "location.href").await,
            format!("{target}/two")
        );
        evaluate_window_name_probe(&mut child, 6, "window.close();true").await;
        wait_for_target_list(addr, "closed popup removed", |targets| {
            !targets.iter().any(|target| target["id"] == child_id)
        })
        .await;
    }
    abort_test_cdp_server(server).await;
}
