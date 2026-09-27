use super::*;

async fn about_blank_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/sandbox",
                    axum::routing::get(|| async {
                        (
                            [(
                                "Content-Security-Policy",
                                "sandbox allow-scripts allow-top-navigation",
                            )],
                            axum::response::Html("<!doctype html><body>opaque source"),
                        )
                    }),
                )
                .route(
                    "/csp",
                    axum::routing::get(|| async {
                        (
                            [("Content-Security-Policy", "script-src 'none'")],
                            axum::response::Html("<!doctype html><body>protected source"),
                        )
                    }),
                )
                .route(
                    "/parent",
                    axum::routing::get(|| async {
                        axum::response::Html("<!doctype html><iframe id=f src='/child'></iframe>")
                    }),
                )
                .fallback(|| async { axum::response::Html("<!doctype html><body>source") }),
        )
        .await
        .unwrap();
    });
    (origin, task)
}

async fn about_blank_command(
    ctx: &mut TestContext,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    ctx.process_and_wait_for_response_async(json!({
        "id": 1, "sessionId": "SID-1", "method": method, "params": params,
    }))
    .await;
    let response = take_response_by_id(ctx, 1);
    assert!(response.get("error").is_none(), "{method}: {response:?}");
    response["result"].clone()
}

async fn about_blank_evaluate(ctx: &mut TestContext, expression: &str) -> serde_json::Value {
    let result = about_blank_command(
        ctx,
        "Runtime.evaluate",
        json!({
            "expression": expression, "returnByValue": true, "userGesture": true,
        }),
    )
    .await;
    assert!(
        result.get("exceptionDetails").is_none(),
        "{expression}: {result:?}"
    );
    result["result"]["value"].clone()
}

async fn about_blank_load(ctx: &mut TestContext, method: &str, params: serde_json::Value) {
    ctx.sent.clear();
    let result = about_blank_command(ctx, method, params).await;
    assert!(result.get("errorText").is_none(), "{result:?}");
    wait_until_frame_stopped_loading(ctx, "TID-1").await;
}

async fn about_blank_context(url: &str) -> TestContext {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    about_blank_load(&mut ctx, "Page.navigate", json!({"url": url})).await;
    ctx
}

async fn about_blank_state(ctx: &mut TestContext) -> serde_json::Value {
    about_blank_evaluate(
        ctx,
        r#"(() => {
        const attempt = f => { try { return f(); } catch (e) { return e.name; } };
        return {url:location.href, origin, locationOrigin:location.origin,
            base:document.baseURI, domain:document.domain, secure:isSecureContext,
            referrer:document.referrer,
            storage:attempt(() => localStorage.getItem('about-blank-origin')),
            entries:navigation.entries().map(e => e.url),
            current:navigation.currentEntry?.url ?? null,
            from:navigation.activation?.from?.url ?? null};
    })()"#,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_about_blank_inherits_native_state_through_reload_and_traversal() {
    let (origin, server) = about_blank_server().await;
    let source = format!("{origin}/source");
    let destination = format!("{origin}/destination");
    for script in [
        "location.assign('about:blank'); true",
        "navigation.navigate('about:blank'); true",
        "const a=document.createElement('a'); a.href='about:blank'; document.body.append(a); a.click(); true",
    ] {
        let mut ctx = about_blank_context(&source).await;
        about_blank_evaluate(
            &mut ctx,
            r#"
            localStorage.setItem('about-blank-origin', 'source');
            const base=document.createElement('base');
            base.href='https://base.example/first/'; document.head.append(base);
            addEventListener('beforeunload', () => base.href='https://base.example/second/');
            Object.defineProperty(globalThis, 'origin', {value:'https://spoof.example'});
            Object.defineProperty(document, 'baseURI', {value:'https://spoof.example/'});
            true;
        "#,
        )
        .await;
        ctx.sent.clear();
        about_blank_evaluate(&mut ctx, script).await;
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
        let mut expected = json!({
            "url":"about:blank", "origin":origin, "locationOrigin":"null",
            "base":"https://base.example/first/", "domain":"127.0.0.1", "secure":true,
            "referrer":format!("{origin}/"), "storage":"source",
            "entries":[source, "about:blank"], "current":"about:blank", "from":source,
        });
        assert_eq!(about_blank_state(&mut ctx).await, expected, "{script}");

        about_blank_evaluate(&mut ctx, "globalThis.oldRealm=true; true").await;
        about_blank_load(&mut ctx, "Page.reload", json!({})).await;
        assert_eq!(
            about_blank_evaluate(&mut ctx, "typeof oldRealm").await,
            "undefined"
        );
        expected["from"] = json!("about:blank");
        assert_eq!(
            about_blank_state(&mut ctx).await,
            expected,
            "reload: {script}"
        );
        let history = about_blank_command(&mut ctx, "Page.getNavigationHistory", json!({})).await;
        let blank_id =
            history["entries"][history["currentIndex"].as_u64().unwrap() as usize]["id"].clone();

        about_blank_load(&mut ctx, "Page.navigate", json!({"url": destination})).await;
        let departed = about_blank_state(&mut ctx).await;
        assert_eq!(departed["base"], destination);
        assert_eq!(
            departed["entries"],
            json!([source, "about:blank", destination])
        );
        about_blank_load(
            &mut ctx,
            "Page.navigateToHistoryEntry",
            json!({"entryId":blank_id}),
        )
        .await;
        expected["entries"] = json!([source, "about:blank", destination]);
        expected["from"] = json!(destination);
        assert_eq!(
            about_blank_state(&mut ctx).await,
            expected,
            "traversal: {script}"
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_about_blank_without_author_initiator_stays_opaque() {
    let (origin, server) = about_blank_server().await;
    for source in ["about:blank".to_owned(), format!("{origin}/source")] {
        let mut ctx = about_blank_context(&source).await;
        about_blank_load(&mut ctx, "Page.navigate", json!({"url":"about:blank"})).await;
        for reload in [false, true] {
            if reload {
                about_blank_load(&mut ctx, "Page.reload", json!({})).await;
            }
            assert_eq!(
                about_blank_state(&mut ctx).await,
                json!({
                    "url":"about:blank", "origin":"null", "locationOrigin":"null",
                    "base":"about:blank", "domain":"", "secure":false, "referrer":"",
                    "storage":"SecurityError", "entries":[], "current":null, "from":null,
                }),
                "{source}, reload={reload}"
            );
        }
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_about_blank_inherits_opaque_source_base_without_enabling_navigation() {
    let (origin, server) = about_blank_server().await;
    for source in [
        format!("{origin}/sandbox"),
        "data:text/html,source".to_owned(),
    ] {
        let mut ctx = about_blank_context(&source).await;
        ctx.sent.clear();
        about_blank_evaluate(&mut ctx, "location.href='about:blank'; true").await;
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
        for reload in [false, true] {
            if reload {
                about_blank_load(&mut ctx, "Page.reload", json!({})).await;
            }
            assert_eq!(
                about_blank_state(&mut ctx).await,
                json!({
                    "url":"about:blank", "origin":"null", "locationOrigin":"null",
                    "base":source, "domain":"", "secure":false, "referrer":"",
                    "storage":"SecurityError", "entries":[], "current":null, "from":null,
                }),
                "{source}, reload={reload}"
            );
        }
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_about_blank_inherits_response_and_meta_csp() {
    let (origin, server) = about_blank_server().await;
    for source in ["/csp", "/meta-csp"] {
        let mut ctx = about_blank_context(&format!("{origin}{source}")).await;
        if source == "/meta-csp" {
            about_blank_evaluate(
                &mut ctx,
                r#"
                const m=document.createElement('meta'); m.httpEquiv='Content-Security-Policy';
                m.content="script-src 'none'"; document.head.append(m); true;
            "#,
            )
            .await;
        }
        ctx.sent.clear();
        about_blank_evaluate(&mut ctx, "location.href='about:blank'; true").await;
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
        for reload in [false, true] {
            if reload {
                about_blank_load(&mut ctx, "Page.reload", json!({})).await;
            }
            assert_eq!(
                about_blank_evaluate(
                    &mut ctx,
                    r#"(() => {
                const s=document.createElement('script'); s.textContent='globalThis.inlineRan=true';
                document.head.append(s); return globalThis.inlineRan === true;
            })()"#
                )
                .await,
                false,
                "{source}, reload={reload}"
            );
        }
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_about_blank_uses_the_navigation_initiator_document_base() {
    let (origin, server) = about_blank_server().await;
    for (script, base_path) in [
        ("top.location.href='about:blank'", "/child"),
        ("top.navigation.navigate('about:blank')", "/parent"),
        (
            "const a=document.createElement('a'); a.href='about:blank'; a.target='_top'; document.body.append(a); a.click()",
            "/child",
        ),
    ] {
        let mut ctx = about_blank_context(&format!("{origin}/parent")).await;
        ctx.sent.clear();
        about_blank_evaluate(
            &mut ctx,
            &format!("f.contentWindow.eval({}); true", json!(script)),
        )
        .await;
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
        let state = about_blank_state(&mut ctx).await;
        assert_eq!(state["origin"], origin, "{script}");
        assert_eq!(state["base"], format!("{origin}{base_path}"), "{script}");
        assert_eq!(state["url"], "about:blank");
    }
    // The caller can be in the parent realm while activating a child link.
    // The link's node Document, rather than the incumbent script, is the source.
    let mut ctx = about_blank_context(&format!("{origin}/parent")).await;
    ctx.sent.clear();
    about_blank_evaluate(&mut ctx, "const a=f.contentDocument.createElement('a'); a.href='about:blank'; a.target='_top'; f.contentDocument.body.append(a); a.click(); true").await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    assert_eq!(
        about_blank_state(&mut ctx).await["base"],
        format!("{origin}/child")
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_about_blank_preserves_the_selected_referrer() {
    let (origin, server) = about_blank_server().await;
    let source = format!("{origin}/source");
    for (script, expected) in [
        (
            "const m=document.createElement('meta'); m.name='referrer'; m.content='no-referrer'; document.head.append(m); location.href='about:blank'; true",
            "",
        ),
        (
            "const a=document.createElement('a'); a.href='about:blank'; a.rel='noreferrer'; document.body.append(a); a.click(); true",
            "",
        ),
        (
            "const a=document.createElement('a'); a.href='about:blank'; a.referrerPolicy='no-referrer'; document.body.append(a); a.click(); true",
            "",
        ),
        (
            "const a=document.createElement('a'); a.href='about:blank'; a.referrerPolicy='unsafe-url'; document.body.append(a); a.click(); true",
            source.as_str(),
        ),
    ] {
        let mut ctx = about_blank_context(&source).await;
        ctx.sent.clear();
        about_blank_evaluate(&mut ctx, script).await;
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
        assert_eq!(
            about_blank_state(&mut ctx).await["referrer"],
            expected,
            "{script}"
        );
        about_blank_load(&mut ctx, "Page.reload", json!({})).await;
        assert_eq!(
            about_blank_state(&mut ctx).await["referrer"],
            expected,
            "reload: {script}"
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_about_blank_shares_its_storage_partition_with_same_origin_http_children() {
    let (origin, server) = about_blank_server().await;
    let mut ctx = about_blank_context(&format!("{origin}/source")).await;
    ctx.sent.clear();
    about_blank_evaluate(&mut ctx, "localStorage.setItem('blank-child','local'); sessionStorage.setItem('blank-child','session'); location.href='about:blank'; true").await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let child_url = json!(format!("{origin}/child"));
    let result = about_blank_command(
        &mut ctx,
        "Runtime.evaluate",
        json!({
            "expression": format!(r#"new Promise(resolve => {{
            const f=document.createElement('iframe');
            f.onload=() => {{
                resolve([f.contentWindow.location.href, f.contentWindow.origin,
                    f.contentWindow.isSecureContext,
                    f.contentWindow.localStorage.getItem('blank-child'),
                    f.contentWindow.sessionStorage.getItem('blank-child')]);
                f.remove();
            }};
            f.src={child_url}; document.body.append(f);
        }})"#),
            "awaitPromise": true, "returnByValue": true,
        }),
    )
    .await;
    assert!(result.get("exceptionDetails").is_none(), "{result}");
    assert_eq!(
        result["result"]["value"],
        json!([child_url, origin, true, "local", "session"])
    );
    server.abort();
}
