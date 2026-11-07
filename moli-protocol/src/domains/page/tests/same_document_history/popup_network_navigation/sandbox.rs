use super::*;

const SANDBOX_DOCUMENT: &str = r#"<!doctype html><body><script>
if (opener) {
  addEventListener('load', async () => {
    const access = operation => {
      try { operation(); return 'allowed'; } catch (error) { return error.name; }
    };
    const fetchOrigin = await fetch('/sandbox-origin-check', {method:'POST'})
      .then(response => response.text());
    opener.postMessage({
      phase: location.pathname,
      origin,
      cookie: access(() => document.cookie),
      storage: access(() => localStorage.getItem('sandbox-test')),
      sessionStorage: access(() => sessionStorage.getItem('sandbox-test')),
      fetchOrigin,
    }, '*');
    if (location.pathname === '/sandbox-creator') {
      location.href = '/sandbox-successor';
    }
  }, {once:true});
} else {
  const kind = new URL(location).searchParams.get('kind');
  onmessage = event => {
    if (event.data === 'start') {
      if (kind === 'link') {
        const link = document.createElement('a');
        link.href = location.href;
        link.target = '_blank';
        link.rel = 'opener';
        document.body.append(link);
        link.click();
      } else if (kind === 'blank') {
        const popup = open();
        popup.location.href = location.href;
      }
    } else {
      parent.postMessage({frameOrigin:origin, messageOrigin:event.origin, facts:event.data}, '*');
    }
  };
  if (kind === 'direct') open(location.href);
  else parent.postMessage('sandbox-frame-ready', '*');
}
</script>"#;

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_windows_keep_inherited_sandbox_across_network_documents() {
    run_history_test(async {
        for kind in ["direct", "link", "blank"] {
            for (sandbox, opaque) in [
                ("allow-scripts allow-popups", true),
                (
                    "allow-scripts allow-popups allow-popups-to-escape-sandbox",
                    false,
                ),
                ("allow-scripts allow-popups allow-same-origin", false),
            ] {
                let routes = axum::Router::new()
                    .route(
                        "/sandbox-creator",
                        axum::routing::get(|| async { axum::response::Html(SANDBOX_DOCUMENT) }),
                    )
                    .route(
                        "/sandbox-successor",
                        axum::routing::get(|| async { axum::response::Html(SANDBOX_DOCUMENT) }),
                    )
                    .route(
                        "/sandbox-origin-check",
                        axum::routing::post(|headers: axum::http::HeaderMap| async move {
                            (
                                [(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
                                headers
                                    .get(axum::http::header::ORIGIN)
                                    .and_then(|value| value.to_str().ok())
                                    .unwrap_or("<missing>")
                                    .to_owned(),
                            )
                        }),
                    );
                let mut page = SameDocumentPage::with_routes(routes).await;
                let origin = url::Url::parse(&page.base_url)
                    .unwrap()
                    .origin()
                    .ascii_serialization();
                let result = page
                    .evaluate(&format!(
                        r#"(async () => {{
                    const messages = [];
                    let finish;
                    const complete = new Promise(resolve => finish = resolve);
                    const frame = document.createElement('iframe');
                    let ready;
                    const frameReady = new Promise(resolve => ready = resolve);
                    addEventListener('message', event => {{
                        if (event.data === 'sandbox-frame-ready' && event.source === frame.contentWindow) {{
                            ready();
                            return;
                        }}
                        if (!event.data?.facts) return;
                        messages.push({{outerOrigin:event.origin, ...event.data}});
                        if (messages.length === 2) finish(messages);
                    }});
                    frame.sandbox = {sandbox:?};
                    frame.src = '/sandbox-creator?kind={kind}';
                    document.body.append(frame);
                    if ({kind:?} !== 'direct') {{
                        await frameReady;
                        frame.contentWindow.postMessage('start', '*');
                    }}
                    return complete;
                }})()"#
                    ))
                    .await;
                let frame_origin = if sandbox.contains("allow-same-origin") {
                    origin.as_str()
                } else {
                    "null"
                };
                let popup_origin = if opaque { "null" } else { origin.as_str() };
                let access = if opaque { "SecurityError" } else { "allowed" };
                let expected = ["/sandbox-creator", "/sandbox-successor"].map(|phase| {
                    json!({
                        "outerOrigin":frame_origin,
                        "frameOrigin":frame_origin,
                        "messageOrigin":popup_origin,
                        "facts": {"phase":phase, "origin":popup_origin,
                            "cookie":access, "storage":access, "sessionStorage":access,
                            "fetchOrigin":popup_origin},
                    })
                });
                assert_eq!(result, json!(expected), "{kind}/{sandbox}");
                page.ctx
                    .process_and_wait_for_response_async(json!({
                        "id":1003, "method":"Target.getTargets", "params":{},
                    }))
                    .await;
                let targets = take_response_by_id(&mut page.ctx, 1003);
                assert!(targets["error"].is_null(), "{targets}");
                let target = targets["result"]["targetInfos"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|target| {
                        target["url"]
                            .as_str()
                            .is_some_and(|url| url.ends_with("/sandbox-successor"))
                    })
                    .expect("successor popup should remain open for its protocol identity check")
                    ["targetId"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                page.ctx
                    .process_and_wait_for_response_async(json!({
                        "id":1004, "method":"Target.attachToTarget",
                        "params":{"targetId":target,"flatten":true},
                    }))
                    .await;
                let attached = take_response_by_id(&mut page.ctx, 1004);
                assert!(attached["error"].is_null(), "{attached}");
                let session = attached["result"]["sessionId"].as_str().unwrap();
                let tree = page
                    .command_in_session(session, "Page.getFrameTree", json!({}))
                    .await;
                // Blink projects Page.Frame.securityOrigin from the loader URL.
                // The sandboxed origin is checked above through script and fetch.
                assert_eq!(
                    tree["frameTree"]["frame"]["securityOrigin"],
                    origin.as_str(),
                    "{kind}/{sandbox}: {tree}"
                );
                page.ctx
                    .process_and_wait_for_response_async(json!({
                        "id":1005, "method":"Target.closeTarget",
                        "params":{"targetId":target},
                    }))
                    .await;
                let closed = take_response_by_id(&mut page.ctx, 1005);
                assert!(closed["error"].is_null(), "{closed}");
            }
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_sandbox_script_restrictions_apply_before_parsing_network_documents() {
    run_history_test(async {
        for (sandbox, csp, scripts_allowed) in [
            ("allow-same-origin allow-popups", false, false),
            ("allow-same-origin allow-popups allow-scripts", false, true),
            (
                "allow-same-origin allow-popups allow-popups-to-escape-sandbox",
                false,
                true,
            ),
            ("allow-same-origin allow-popups allow-scripts", true, false),
        ] {
            let routes = axum::Router::new().route(
                "/sandbox-script-check",
                axum::routing::get(move || async move {
                    let mut headers = axum::http::HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("text/html"),
                    );
                    if csp {
                        headers.insert(
                            axum::http::header::CONTENT_SECURITY_POLICY,
                            axum::http::HeaderValue::from_static("sandbox allow-same-origin"),
                        );
                    }
                    (headers, "<!doctype html><body><noscript><p id=fallback>Fallback</p></noscript><script>globalThis.sandboxScriptDidRun = true;</script>")
                }),
            );
            let mut page = SameDocumentPage::with_routes(routes).await;
            page.evaluate(&format!(r#"(async () => {{
                const frame = document.createElement('iframe');
                frame.sandbox = {sandbox:?};
                frame.srcdoc = '<p>Creator</p>';
                const loaded = new Promise(resolve => frame.onload = () => {{
                    if (frame.contentDocument.URL === 'about:srcdoc') resolve();
                }});
                document.body.append(frame);
                await loaded;
                const link = frame.contentDocument.createElement('a');
                link.href = '/sandbox-script-check';
                link.target = '_blank';
                link.rel = 'opener';
                frame.contentDocument.body.append(link);
                link.click();
                return true;
            }})()"#)).await;
            let target = page.ctx.sent.iter().rev().find(|message| {
                message["method"] == "Target.targetCreated"
                    && message["params"]["targetInfo"]["url"].as_str()
                        .is_some_and(|url| url.ends_with("/sandbox-script-check"))
            }).expect("sandboxed hyperlink should create an auxiliary target")
                ["params"]["targetInfo"]["targetId"].as_str().unwrap().to_owned();
            page.ctx.process_and_wait_for_response_async(json!({
                "id":1003, "method":"Target.attachToTarget",
                "params":{"targetId":target,"flatten":true},
            })).await;
            let attached = take_response_by_id(&mut page.ctx, 1003);
            assert!(attached["error"].is_null(), "{attached}");
            let session = attached["result"]["sessionId"].as_str().unwrap().to_owned();
            let first_url = url::Url::parse(&page.base_url).unwrap()
                .join("/sandbox-script-check").unwrap().to_string();
            let first = sandbox_popup_snapshot(&mut page, &session, &first_url).await;
            let second_url = format!("{first_url}?successor");
            page.command_in_session(&session, "Page.navigate", json!({"url":second_url})).await;
            let second = sandbox_popup_snapshot(&mut page, &session, &second_url).await;
            let result = json!([first, second]);
            let expected = json!({"ran":scripts_allowed, "fallback":!scripts_allowed});
            assert_eq!(result, json!([expected, expected]), "{sandbox}/csp={csp}");
        }
    }).await;
}

async fn sandbox_popup_snapshot(
    page: &mut SameDocumentPage,
    session: &str,
    expected_url: &str,
) -> serde_json::Value {
    page.command_in_session(session, "Page.enable", json!({}))
        .await;
    let mut tree = page
        .command_in_session(session, "Page.getFrameTree", json!({}))
        .await;
    if tree["frameTree"]["frame"]["url"] != expected_url {
        crate::testing::wait_until_scheduler_message(
            &mut page.ctx,
            "sandbox popup document commit",
            |message| {
                message["sessionId"] == session
                    && message["method"] == "Page.frameNavigated"
                    && message["params"]["frame"]["url"] == expected_url
            },
        )
        .await;
        tree = page
            .command_in_session(session, "Page.getFrameTree", json!({}))
            .await;
    }
    let frame = &tree["frameTree"]["frame"];
    assert_eq!(frame["url"], expected_url, "{tree}");
    crate::testing::wait_until_renderer_document_load(
        &mut page.ctx,
        Some(session),
        frame["id"].as_str().unwrap(),
        frame["loaderId"].as_str().unwrap(),
    )
    .await;
    let result = page.command_in_session(session, "Runtime.evaluate", json!({
        "expression":"({url:document.URL, ready:document.readyState, ran:globalThis.sandboxScriptDidRun === true, fallback:!!document.getElementById('fallback')})",
        "returnByValue":true,
    })).await;
    assert!(result["exceptionDetails"].is_null(), "{result}");
    let value = &result["result"]["value"];
    assert_eq!(value["url"], expected_url);
    assert_eq!(value["ready"], "complete");
    json!({"ran":value["ran"],"fallback":value["fallback"]})
}
