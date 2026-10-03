use super::*;

#[tokio::test]
async fn web_mcp_permissions_use_the_committed_origin_after_redirects_and_sandboxing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // Header, source, container allowlist, iframe sandbox, response sandbox,
    // nested initial about:blank, expected registration result.
    let cases = std::sync::Arc::new([
        (
            Some("tools=(self)"),
            "/redirect",
            Some("tools *"),
            None,
            false,
            false,
            "NotAllowedError",
        ),
        (
            Some("tools=(self)"),
            "/child",
            None,
            Some("allow-scripts"),
            false,
            false,
            "NotAllowedError",
        ),
        (
            None,
            "/child",
            None,
            Some("allow-scripts"),
            false,
            false,
            "NotAllowedError",
        ),
        (
            Some("tools=(self)"),
            "/child",
            Some("tools *"),
            Some("allow-scripts"),
            false,
            false,
            "NotAllowedError",
        ),
        (
            None,
            "/child",
            Some("tools *"),
            Some("allow-scripts"),
            false,
            false,
            "allowed",
        ),
        (
            Some("tools=*"),
            "/child",
            Some("tools *"),
            Some("allow-scripts"),
            false,
            false,
            "allowed",
        ),
        (
            Some("tools=(self)"),
            "/child",
            None,
            Some("allow-scripts allow-same-origin"),
            false,
            false,
            "allowed",
        ),
        (None, "/child", None, None, false, true, "allowed"),
        (
            Some("tools=(self)"),
            "/child",
            Some("tools *"),
            None,
            true,
            false,
            "NotAllowedError",
        ),
        (
            None,
            "/child",
            Some("tools *"),
            None,
            true,
            false,
            "allowed",
        ),
        (
            None,
            "/redirect",
            Some("tools 'src'"),
            None,
            false,
            false,
            "NotAllowedError",
        ),
        (
            None,
            "/redirect",
            Some("tools"),
            None,
            false,
            false,
            "NotAllowedError",
        ),
        (
            None,
            "/redirect",
            Some("tools *"),
            None,
            false,
            false,
            "allowed",
        ),
        (
            Some("tools=(self)"),
            "/return",
            None,
            None,
            false,
            false,
            "allowed",
        ),
    ]);
    let parent_cases = cases.clone();
    let child_cases = cases.clone();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route("/parent/{case}", axum::routing::get(move |axum::extract::Path(index): axum::extract::Path<usize>| {
                let cases = parent_cases.clone();
                async move {
                    let (header, source, allow, sandbox, _, nested, _) = cases[index];
                    let mut headers = axum::http::HeaderMap::new();
                    if let Some(header) = header { headers.insert("permissions-policy", header.parse().unwrap()); }
                    let source = if source == "/return" {
                        format!("http://{address}/return/{index}")
                    } else {
                        format!("{source}/{index}")
                    };
                    (headers, axum::response::Html(format!(r#"<!doctype html><body><script>
                        globalThis.childResult=new Promise(resolve=>addEventListener('message',event=>resolve(event.data),{{once:true}}));
                        const frame=document.createElement('iframe');
                        const allow={allow}; const sandbox={sandbox};
                        if (allow!==null) frame.allow=allow;
                        if (sandbox!==null) frame.setAttribute('sandbox',sandbox);
                        if ({nested}) {{
                            document.body.append(frame);
                            const grandchild=frame.contentDocument.createElement('iframe');
                            grandchild.src=new URL({source},location.href).href;
                            frame.contentDocument.body.append(grandchild);
                        }} else {{ frame.src={source}; document.body.append(frame); }}
                    </script>"#,
                        allow=json!(allow), sandbox=json!(sandbox), nested=nested, source=json!(source))))
                }
            }))
            .route("/redirect/{case}", axum::routing::get(move |axum::extract::Path(index): axum::extract::Path<usize>| async move {
                axum::response::Redirect::temporary(&format!("http://{address}/child/{index}"))
            }))
            .route("/return/{case}", axum::routing::get(move |axum::extract::Path(index): axum::extract::Path<usize>| async move {
                axum::response::Redirect::temporary(&format!("http://localhost:{}/child/{index}",address.port()))
            }))
            .route("/child/{case}", axum::routing::get(move |axum::extract::Path(index): axum::extract::Path<usize>| {
                let cases=child_cases.clone();
                async move {
                    let mut headers=axum::http::HeaderMap::new();
                    if cases[index].4 { headers.insert("content-security-policy","sandbox allow-scripts".parse().unwrap()); }
                    (headers, axum::response::Html(r#"<!doctype html><script>
                        (async()=>{
                            try {
                                await document.modelContext.registerTool({name:'child',description:'Child',execute:()=>42});
                                top.postMessage('allowed','*');
                            } catch (error) { top.postMessage(error.name,'*'); }
                        })();
                    </script>"#))
                }
            }));
        axum::serve(listener, app).await.unwrap();
    });
    for (index, case) in cases.iter().enumerate() {
        let mut ctx = page("").await;
        command(&mut ctx, 2, "Page.enable", json!({})).await;
        let reply = command(
            &mut ctx,
            3,
            "Page.navigate",
            json!({"url":format!("http://localhost:{}/parent/{index}",address.port())}),
        )
        .await;
        assert!(reply["result"].get("errorText").is_none(), "{reply}");
        event(&mut ctx, "Page.loadEventFired").await;
        let result = command(
            &mut ctx,
            4,
            "Runtime.evaluate",
            json!({"expression":"childResult","awaitPromise":true,"returnByValue":true}),
        )
        .await;
        assert_eq!(
            result["result"]["result"]["value"], case.6,
            "case {index}: {case:?}: {result}"
        );
    }
    server.abort();
}
