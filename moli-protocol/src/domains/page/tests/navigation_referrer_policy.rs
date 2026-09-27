use super::*;

async fn policy_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/protected/{policy}",
                    axum::routing::get(
                        |axum::extract::Path(policy): axum::extract::Path<String>| async move {
                            (
                                [("Referrer-Policy", policy)],
                                axum::response::Html("<!doctype html><body>source"),
                            )
                        },
                    ),
                )
                .fallback(|| async { axum::response::Html("<!doctype html><body>page") }),
        )
        .await
        .unwrap();
    });
    (origin, server)
}

async fn command(
    ctx: &mut TestContext,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    ctx.process_and_wait_for_response_async(
        json!({"id":1,"sessionId":"SID-1","method":method,"params":params}),
    )
    .await;
    let response = take_response_by_id(ctx, 1);
    assert!(response.get("error").is_none(), "{method}: {response}");
    response["result"].clone()
}

async fn evaluate(ctx: &mut TestContext, expression: &str) -> serde_json::Value {
    let result = command(
        ctx,
        "Runtime.evaluate",
        json!({"expression":expression,"returnByValue":true}),
    )
    .await;
    assert!(
        result.get("exceptionDetails").is_none(),
        "{expression}: {result}"
    );
    result["result"]["value"].clone()
}

async fn navigate(ctx: &mut TestContext, method: &str, params: serde_json::Value) {
    ctx.sent.clear();
    let result = command(ctx, method, params).await;
    assert!(result.get("errorText").is_none(), "{result}");
    wait_until_frame_stopped_loading(ctx, "TID-1").await;
}

async fn context(url: &str) -> TestContext {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    navigate(&mut ctx, "Page.navigate", json!({"url":url})).await;
    ctx
}

const SET_POLICY: &str = r#"function setPolicy(value) {
    let m = document.getElementById('policy');
    if (!m) { m=document.createElement('meta'); m.name='referrer'; m.id='policy'; document.head.append(m); }
    m.content=value;
}"#;
const SNAPSHOT: &str = r#"({
    urls:navigation.entries().map(e=>e.url),
    sameDocument:navigation.entries().map(e=>e.sameDocument),
    from:navigation.activation.from?.url ?? null
})"#;

#[tokio::test(flavor = "multi_thread")]
async fn top_level_history_censors_referrer_protected_documents() {
    let (origin, server) = policy_server().await;
    let destination = format!("{origin}/destination");
    for policy in ["no-referrer", "origin"] {
        for header in [false, true] {
            for method in ["location", "navigation", "anchor", "cdp"] {
                let source = if header {
                    format!("{origin}/protected/{policy}")
                } else {
                    format!("{origin}/source")
                };
                let mut ctx = context(&source).await;
                evaluate(
                    &mut ctx,
                    &format!("history.pushState(null,'','#state'); {SET_POLICY}; true"),
                )
                .await;
                if !header {
                    evaluate(&mut ctx, &format!("setPolicy({}); true", json!(policy))).await;
                }
                assert_eq!(
                    evaluate(&mut ctx, "navigation.entries().map(e=>e.url)").await,
                    json!([source, format!("{source}#state")])
                );
                ctx.sent.clear();
                let url = json!(destination);
                let script = match method {
                    "location" => format!("location.href={url}; true"),
                    "navigation" => format!("navigation.navigate({url}); true"),
                    "anchor" => format!(
                        "const a=document.createElement('a'); a.href={url}; document.body.append(a); a.click(); true"
                    ),
                    "cdp" => String::new(),
                    _ => unreachable!(),
                };
                if method == "cdp" {
                    command(&mut ctx, "Page.navigate", json!({"url":destination})).await;
                } else {
                    evaluate(&mut ctx, &script).await;
                }
                wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
                assert_eq!(
                    evaluate(&mut ctx, SNAPSHOT).await,
                    json!({"urls":[null,null,destination],"sameDocument":[false,false,true],"from":null}),
                    "{policy}, header={header}, {method}"
                );
            }
        }
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_history_preserves_policy_changes_across_traversal_and_reload() {
    let (origin, server) = policy_server().await;
    let source = format!("{origin}/source");
    let destination = format!("{origin}/destination");
    let mut ctx = context(&source).await;
    let history = command(&mut ctx, "Page.getNavigationHistory", json!({})).await;
    let source_id =
        history["entries"][history["currentIndex"].as_u64().unwrap() as usize]["id"].clone();
    navigate(&mut ctx, "Page.navigate", json!({"url":destination})).await;
    let history = command(&mut ctx, "Page.getNavigationHistory", json!({})).await;
    let destination_id =
        history["entries"][history["currentIndex"].as_u64().unwrap() as usize]["id"].clone();
    for policy in ["no-referrer", "same-origin", "origin", "unsafe-url"] {
        navigate(
            &mut ctx,
            "Page.navigateToHistoryEntry",
            json!({"entryId":source_id}),
        )
        .await;
        assert_eq!(
            evaluate(&mut ctx, "navigation.currentEntry.url").await,
            source
        );
        evaluate(
            &mut ctx,
            &format!("{SET_POLICY}; setPolicy({}); true", json!(policy)),
        )
        .await;
        navigate(
            &mut ctx,
            "Page.navigateToHistoryEntry",
            json!({"entryId":destination_id}),
        )
        .await;
        let expected_source = if matches!(policy, "no-referrer" | "origin") {
            serde_json::Value::Null
        } else {
            json!(source)
        };
        assert_eq!(
            evaluate(&mut ctx, SNAPSHOT).await,
            json!({"urls":[expected_source,destination],"sameDocument":[false,true],"from":expected_source}),
            "{policy}"
        );
        navigate(&mut ctx, "Page.reload", json!({})).await;
        assert_eq!(
            evaluate(&mut ctx, "navigation.entries().map(e=>e.url)").await,
            json!([expected_source, destination]),
            "reload: {policy}"
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_api_captures_history_policy_before_unload_callbacks() {
    let (origin, server) = policy_server().await;
    let source = format!("{origin}/source");
    let destination = format!("{origin}/destination");
    for event in ["beforeunload", "pagehide", "unload"] {
        let mut ctx = context(&source).await;
        evaluate(
            &mut ctx,
            &format!(
                "{SET_POLICY}; addEventListener({event:?},()=>{{ setPolicy('no-referrer'); localStorage.setItem('policy-event', document.getElementById('policy').content); }}); true"
            ),
        )
        .await;
        ctx.sent.clear();
        evaluate(
            &mut ctx,
            &format!("navigation.navigate({}); true", json!(destination)),
        )
        .await;
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
        assert_eq!(
            evaluate(&mut ctx, "localStorage.getItem('policy-event')").await,
            "no-referrer",
            "{event} must run and update the meta element"
        );
        let expected_source = if event == "beforeunload" {
            serde_json::Value::Null
        } else {
            json!(source)
        };
        assert_eq!(
            evaluate(&mut ctx, "navigation.entries().map(e=>e.url)").await,
            json!([expected_source, destination]),
            "{event}"
        );
    }
    server.abort();
}
