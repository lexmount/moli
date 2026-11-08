use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_window_client_focus_activates_background_page_and_nested_frame() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><body><button id=root>root</button><iframe id=child src='/frame'></iframe><script>
        globalThis.focusResult=null;
        navigator.serviceWorker.addEventListener('message',event=>{
          focusResult=event.data;
          if(globalThis.focusResolve) focusResolve(event.data);
        });
        </script>"#,
        )
    }
    async fn frame() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body><input id=control value=abcdef>",
        )
    }
    async fn service_worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
        self.addEventListener('install',event=>event.waitUntil(self.skipWaiting()));
        self.addEventListener('activate',event=>event.waitUntil(clients.claim()));
        self.addEventListener('notificationclick',event=>event.waitUntil((async()=>{
          const windows=await clients.matchAll({type:'window',includeUncontrolled:true});
          const target=windows.find(client=>new URL(client.url).pathname===(event.action==='frame'?'/frame':'/one'));
          let result;
          try {
            const focused=await target.focus();
            result={focused:focused.focused,visibility:focused.visibilityState,frameType:focused.frameType};
          } catch(error) { result={error:error.name}; }
          for(const client of windows) client.postMessage(result);
        })()));
        "#,
        )
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/one", get(page))
                .route("/two", get(page))
                .route("/frame", get(frame))
                .route("/worker.js", get(service_worker)),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    let mut browser = BrowserContext::new("BID-native-focus".to_owned());
    browser.set_active_target_id("TID-native-focus-one");
    browser.attach_active_session("SID-native-focus-one");
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
    ctx.process_async(json!({"id":80999,"method":"Browser.grantPermissions","params":{"browserContextId":"BID-native-focus","origin":format!("http://{addr}"),"permissions":["notifications"]}})).await;
    ctx.expect_result(80999, json!({}), None);
    ctx.process_async(json!({"id":81000,"method":"Page.navigate","sessionId":"SID-native-focus-one","params":{"url":format!("http://{addr}/one")}})).await;
    let _ = ctx.take_response_by_id(81000);
    ctx.process_async(json!({"id":81001,"method":"Runtime.evaluate","sessionId":"SID-native-focus-one","params":{"expression":r#"
    (async()=>{
      await navigator.serviceWorker.register('/worker.js',{scope:'/'});
      const registration=await navigator.serviceWorker.ready;
      if(registration.active.state!=='activated') await new Promise(resolve=>registration.active.addEventListener('statechange',()=>{if(registration.active.state==='activated')resolve();}));
      if(!child.contentDocument || !child.contentDocument.getElementById('control')) await new Promise(resolve=>child.addEventListener('load',resolve,{once:true}));
      await registration.showNotification('focus');
      const control=child.contentDocument.getElementById('control');
      control.focus(); control.setSelectionRange(2,4); root.focus();
      Window.prototype.focus=()=>{throw Error('author Window.focus must not run');};
      return 'ready';
    })()
    "#,"awaitPromise":true,"returnByValue":true}})).await;
    wait_until_messages(
        &mut ctx,
        Some("SID-native-focus-one"),
        "focus worker activation",
        |m| m.iter().any(|m| m["id"] == json!(81001)),
    )
    .await;
    assert_eq!(
        ctx.take_response_by_id(81001)["result"]["result"]["value"],
        json!("ready")
    );
    ctx.process_async(json!({"id":81002,"method":"Target.createTarget","params":{"browserContextId":"BID-native-focus","url":format!("http://{addr}/two")}})).await;
    let second = ctx.take_response_by_id(81002)["result"]["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    // Keep the second target without a child so /frame identifies the first
    // Page's exact nested Window rather than depending on Clients ordering.
    ctx.process_async(json!({"id":81003,"method":"Target.attachToTarget","params":{"targetId":second,"flatten":true}})).await;
    let second_session = ctx.take_response_by_id(81003)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    ctx.process_async(json!({"id":81004,"method":"Runtime.evaluate","sessionId":second_session,"params":{"expression":"child.remove(); 'ready'","returnByValue":true}})).await;
    assert_eq!(
        ctx.take_response_by_id(81004)["result"]["result"]["value"],
        json!("ready")
    );
    let runtime = ctx.with_fixture(|conn| {
        conn.browser_context_by_id("BID-native-focus")
            .unwrap()
            .renderer_runtime()
    });
    for (phase, action, frame_type, expected_element) in [
        (0, "page", "top-level", "root"),
        (1, "frame", "nested", "child"),
        (2, "frame", "nested", "child"),
        (3, "page", "top-level", ""),
    ] {
        let id = 81100 + phase * 10;
        if phase >= 2 {
            let expression = if phase == 2 {
                "child.contentDocument.body.innerHTML=''; root.focus(); 'ready'"
            } else {
                "root.remove(); child.remove(); 'ready'"
            };
            ctx.process_async(json!({"id":id+5,"method":"Runtime.evaluate","sessionId":"SID-native-focus-one","params":{"expression":expression,"returnByValue":true}})).await;
            assert_eq!(
                ctx.take_response_by_id(id + 5)["result"]["result"]["value"],
                json!("ready")
            );
        }
        ctx.process_async(
            json!({"id":id,"method":"Target.activateTarget","params":{"targetId":second}}),
        )
        .await;
        ctx.expect_result(id, json!({}), None);
        ctx.process_async(json!({"id":id+1,"method":"Runtime.evaluate","sessionId":"SID-native-focus-one","params":{"expression":"document.hasFocus()","returnByValue":true}})).await;
        assert_eq!(
            ctx.take_response_by_id(id + 1)["result"]["result"]["value"],
            json!(false)
        );
        ctx.process_async(json!({"id":id+2,"method":"Runtime.evaluate","sessionId":"SID-native-focus-one","params":{"expression":"focusResult=null; new Promise(resolve=>globalThis.focusResolve=resolve)","awaitPromise":true,"returnByValue":true}})).await;
        assert!(runtime.dispatch_service_worker_notification_click(
            &url::Url::parse(&format!("http://{addr}/")).unwrap(),
            "focus",
            action
        ));
        wait_until_messages(
            &mut ctx,
            Some("SID-native-focus-one"),
            "native WindowClient.focus completion",
            |m| m.iter().any(|m| m["id"] == json!(id + 2)),
        )
        .await;
        assert_eq!(
            ctx.take_response_by_id(id + 2)["result"]["result"]["value"],
            json!({"focused":true,"visibility":"visible","frameType":frame_type})
        );
        ctx.with_fixture(|conn| {
            assert_eq!(
                conn.browser_context_by_id("BID-native-focus")
                    .unwrap()
                    .active_target_id(),
                Some("TID-native-focus-one")
            )
        });
        ctx.process_async(json!({"id":id+3,"method":"Runtime.evaluate","sessionId":"SID-native-focus-one","params":{"expression":"JSON.stringify([document.hasFocus(),document.activeElement.id,globalThis.child?.contentDocument.hasFocus()??false,globalThis.child?.contentDocument.getElementById('control')?.selectionStart??null,globalThis.child?.contentDocument.getElementById('control')?.selectionEnd??null])","returnByValue":true}})).await;
        let state = ctx.take_response_by_id(id + 3);
        let selection = if phase < 2 { "2,4" } else { "null,null" };
        assert_eq!(
            state["result"]["result"]["value"],
            json!(format!(
                "[true,\"{expected_element}\",{},{selection}]",
                phase == 1 || phase == 2
            ))
        );
        ctx.process_async(json!({"id":id+4,"method":"Runtime.evaluate","sessionId":second_session,"params":{"expression":"document.hasFocus()","returnByValue":true}})).await;
        assert_eq!(
            ctx.take_response_by_id(id + 4)["result"]["result"]["value"],
            json!(false)
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_service_worker_updates_live_page_fetches() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>service worker bypass</body>",
        )
    }

    async fn service_worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
self.addEventListener("install", event => {
  event.waitUntil(self.skipWaiting());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});
self.addEventListener("fetch", event => {
  const path = new URL(event.request.url).pathname;
  event.respondWith(new Response("worker:" + path));
});
"#,
        )
    }

    async fn probe() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "network:/probe")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(service_worker))
                .route("/probe", get(probe)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));

    ctx.process_async(json!({
        "id": 80_000,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(80_000, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 80_001,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = ctx.take_response_by_id(80_001);

    ctx.process_async(json!({
        "id": 80_002,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(async () => {
  await navigator.serviceWorker.register("/worker.js", { scope: "/" });
  await navigator.serviceWorker.ready;
  {
    // ready may resolve while activate (and its claim) is still pending.
    const active = (await navigator.serviceWorker.ready).active;
    if (active.state !== "activated") {
      await new Promise(resolve => active.addEventListener("statechange", () => {
        if (active.state === "activated") resolve();
      }));
    }
  }
  const response = await fetch("/probe");
  return JSON.stringify({
    body: await response.text(),
    controlled: navigator.serviceWorker.controller !== null
  });
})()
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "service worker fetch before bypass",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(80_002))
        },
    )
    .await;
    let before = ctx.take_response_by_id(80_002);
    assert_eq!(
        before["result"]["result"]["value"],
        json!(r#"{"body":"worker:/probe","controlled":true}"#)
    );

    ctx.process_async(json!({
        "id": 80_003,
        "method": "Network.setBypassServiceWorker",
        "sessionId": "SID-1",
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_result(80_003, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 80_004,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(async () => {
  const response = await fetch("/probe");
  return JSON.stringify({
    body: await response.text(),
    controlled: navigator.serviceWorker.controller !== null
  });
})()
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "service worker fetch after bypass",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(80_004))
        },
    )
    .await;
    let after = ctx.take_response_by_id(80_004);
    assert_eq!(
        after["result"]["result"]["value"],
        json!(r#"{"body":"network:/probe","controlled":true}"#)
    );

    ctx.process_async(json!({
        "id": 80_005,
        "method": "Network.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": { "userAgent": "Moli/ServiceWorkerBypassTest" }
    }))
    .await;
    ctx.expect_result(80_005, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 80_006,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(async () => {
  const response = await fetch("/probe");
  return JSON.stringify({
    body: await response.text(),
    controlled: navigator.serviceWorker.controller !== null
  });
})()
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "service worker fetch after loader rebuild",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(80_006))
        },
    )
    .await;
    let after_rebuild = ctx.take_response_by_id(80_006);
    assert_eq!(
        after_rebuild["result"]["result"]["value"],
        json!(r#"{"body":"network:/probe","controlled":true}"#)
    );

    server.abort();
}
