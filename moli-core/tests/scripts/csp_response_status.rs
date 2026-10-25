use super::*;

pub(super) fn fixture_response(query: &str) -> (&'static str, String, String) {
    let status = match query {
        "201" => "201 Created",
        "404" => "404 Not Found",
        _ => "200 OK",
    };
    (
        status,
        "Content-Type: text/html\r\nContent-Security-Policy: connect-src 'none'\r\nCache-Control: no-store\r\n".into(),
        "<!doctype html><body>CSP response metadata</body>".into(),
    )
}

const PROBE: &str = r#"async function probe(view) {
    const reported = new Promise(resolve => {
        view.document.addEventListener('securitypolicyviolation', resolve, {once:true});
    });
    await view.fetch('/blocked-csp-response-status').then(
        () => {throw new Error('CSP must prevent transport');},
        error => {if (!(error instanceof view.TypeError)) throw error;}
    );
    const event = await reported;
    return [event.statusCode, event instanceof view.SecurityPolicyViolationEvent,
        event.isTrusted, event.target === view.document, event.effectiveDirective];
}"#;

#[tokio::test(flavor = "multi_thread")]
async fn document_csp_response_status_retains_navigation_metadata_through_open() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for status in [201, 404] {
        let mut page = browser
            .fetch_allow_http_error(&format!(
                "{}/csp-response-status.html?{status}",
                servers.origin
            ))
            .await?;
        let expression = format!(
            r#"(async () => {{
                const probe = {PROBE};
                const before = await probe(window);
                document.open();
                document.write('<!doctype html><body>replaced</body>');
                document.close();
                return [before, await probe(window)];
            }})().then(JSON.stringify)"#
        );
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let observed: serde_json::Value =
            serde_json::from_str(result["value"].as_str().expect("CSP observations"))?;
        assert_eq!(
            observed,
            serde_json::json!([
                [status, true, true, true, "connect-src"],
                [status, true, true, true, "connect-src"]
            ])
        );
    }
    assert!(
        servers
            .requests
            .lock()
            .iter()
            .all(|request| { !request.path.starts_with("/blocked-csp-response-status") })
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_csp_response_status_uses_its_response_without_inheriting_parent_status() -> Result<()>
{
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/csp-response-status.html?201", servers.origin))
        .await?;
    let expression = format!(
        r#"(async () => {{
            const probe = {PROBE};
            const rows = [];
            for (const source of ['/csp-response-status.html?404', 'about:blank', 'srcdoc']) {{
                const frame = document.createElement('iframe');
                if (source === 'srcdoc') frame.srcdoc = '<!doctype html><body>srcdoc</body>';
                else frame.src = source;
                await new Promise(resolve => {{frame.onload = resolve; document.body.append(frame);}});
                try {{rows.push(await probe(frame.contentWindow));}}
                finally {{frame.remove();}}
            }}
            const popup = open();
            try {{rows.push(await probe(popup));}}
            finally {{popup.close();}}
            rows.push(await probe(window));
            return rows;
        }})().then(JSON.stringify)"#
    );
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let observed: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("child CSP observations"))?;
    assert_eq!(
        observed,
        serde_json::json!([
            [404, true, true, true, "connect-src"],
            [0, true, true, true, "connect-src"],
            [0, true, true, true, "connect-src"],
            [0, true, true, true, "connect-src"],
            [201, true, true, true, "connect-src"]
        ])
    );
    assert!(
        servers
            .requests
            .lock()
            .iter()
            .all(|request| { !request.path.starts_with("/blocked-csp-response-status") })
    );
    Ok(())
}
