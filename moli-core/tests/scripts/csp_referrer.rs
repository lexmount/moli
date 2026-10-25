use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn iframe_navigation_referrer_policy_reaches_transport_document_and_csp() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let page_url = format!("{}/csp-response-status.html?200", servers.origin);
    let origin_referrer = format!("{}/", servers.origin);
    let mut page = browser.fetch(&page_url).await?;
    let expression = r#"(async () => {
        const rows = [];
        for (const kind of ['http', 'srcdoc']) {
          for (const policy of ['no-referrer', 'origin', 'unsafe-url', ' origin ', 'origin,no-referrer', '', 'same-origin', 'strict-origin-when-cross-origin', 'origin-when-cross-origin']) {
            const frame = document.createElement('iframe');
            frame.referrerPolicy = policy;
            if (kind === 'http') frame.src = '/csp-response-status.html?201';
            else frame.srcdoc = '<!doctype html><body>srcdoc</body>';
            await new Promise(resolve => {frame.onload = resolve; document.body.append(frame);});
            try {
                const view = frame.contentWindow;
                const reported = new Promise(resolve =>
                    view.document.addEventListener('securitypolicyviolation', resolve, {once:true}));
                await view.fetch('/blocked-csp-referrer-policy').then(
                    () => {throw new Error('CSP must block transport');},
                    error => {if (!(error instanceof view.TypeError)) throw error;});
                const event = await reported;
                rows.push([kind, policy, view.document.referrer, event.referrer,
                    event.statusCode, event.isTrusted, event.target === view.document]);
            } finally {frame.remove();}
          }
        }
        return rows;
    })().then(JSON.stringify)"#;
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        page.evaluate_runtime_expression_with_await_async(expression, true),
    )
    .await??;
    let observed: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .expect("iframe referrer observations"),
    )?;
    assert_eq!(
        observed,
        serde_json::json!([
            ["http", "no-referrer", "", "", 201, true, true],
            [
                "http",
                "origin",
                origin_referrer,
                origin_referrer,
                201,
                true,
                true
            ],
            ["http", "unsafe-url", page_url, page_url, 201, true, true],
            ["http", " origin ", page_url, page_url, 201, true, true],
            [
                "http",
                "origin,no-referrer",
                page_url,
                page_url,
                201,
                true,
                true
            ],
            ["http", "", page_url, page_url, 201, true, true],
            ["http", "same-origin", page_url, page_url, 201, true, true],
            [
                "http",
                "strict-origin-when-cross-origin",
                page_url,
                page_url,
                201,
                true,
                true
            ],
            [
                "http",
                "origin-when-cross-origin",
                page_url,
                page_url,
                201,
                true,
                true
            ],
            ["srcdoc", "no-referrer", "", "", 0, true, true],
            [
                "srcdoc",
                "origin",
                origin_referrer,
                origin_referrer,
                0,
                true,
                true
            ],
            ["srcdoc", "unsafe-url", page_url, page_url, 0, true, true],
            [
                "srcdoc",
                " origin ",
                origin_referrer,
                origin_referrer,
                0,
                true,
                true
            ],
            [
                "srcdoc",
                "origin,no-referrer",
                origin_referrer,
                origin_referrer,
                0,
                true,
                true
            ],
            [
                "srcdoc",
                "",
                origin_referrer,
                origin_referrer,
                0,
                true,
                true
            ],
            ["srcdoc", "same-origin", "", "", 0, true, true],
            [
                "srcdoc",
                "strict-origin-when-cross-origin",
                origin_referrer,
                origin_referrer,
                0,
                true,
                true
            ],
            [
                "srcdoc",
                "origin-when-cross-origin",
                origin_referrer,
                origin_referrer,
                0,
                true,
                true
            ]
        ])
    );
    let requests = servers.requests.lock();
    let referrers = requests
        .iter()
        .filter(|request| request.path == "/csp-response-status.html?201")
        .map(|request| request.referer.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        referrers,
        [
            None,
            Some(origin_referrer.as_str()),
            Some(page_url.as_str()),
            Some(page_url.as_str()),
            Some(page_url.as_str()),
            Some(page_url.as_str()),
            Some(page_url.as_str()),
            Some(page_url.as_str()),
            Some(page_url.as_str())
        ]
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.path.starts_with("/blocked-csp-referrer-policy"))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn csp_referrer_uses_native_document_metadata_without_reading_author_getters() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let page_url = format!("{}/csp-response-status.html?201", servers.origin);
    let origin_referrer = format!("{}/", servers.origin);
    let blocked_url = format!("{}/blocked-csp-referrer", servers.origin);
    let mut page = browser.fetch(&page_url).await?;
    let expression = format!(
        r#"(async () => {{
            const capture = async (view, label) => {{
                const doc = view.document;
                const nativeReferrer = Object.getOwnPropertyDescriptor(
                    view.Document.prototype, 'referrer').get.call(doc);
                let authorReads = 0;
                Object.defineProperty(doc, 'referrer', {{configurable:true, get() {{
                    authorReads++;
                    throw new Error('CSP must not read an author getter');
                }}}});
                try {{
                    const reported = new Promise(resolve =>
                        doc.addEventListener('securitypolicyviolation', resolve, {{once:true}}));
                    await view.fetch({blocked_url:?} + '?case=' + label).then(
                        () => {{throw new Error('CSP must block transport');}},
                        error => {{if (!(error instanceof view.TypeError)) throw error;}});
                    const event = await reported;
                    return [event.referrer, nativeReferrer, event.statusCode,
                        event.isTrusted, event.target === doc, authorReads];
                }} finally {{delete doc.referrer;}}
            }};
            const rows = [await capture(window, 'top')];
            for (const source of ['/csp-response-status.html?404', 'about:blank', 'srcdoc']) {{
                const frame = document.createElement('iframe');
                if (source === 'srcdoc') frame.srcdoc = '<!doctype html><body>srcdoc</body>';
                else frame.src = source;
                await new Promise(resolve => {{frame.onload = resolve; document.body.append(frame);}});
                try {{rows.push(await capture(frame.contentWindow, source));}}
                finally {{frame.remove();}}
            }}
            const popup = open();
            try {{rows.push(await capture(popup, 'popup-blank'));}}
            finally {{popup.close();}}
            return rows;
        }})().then(JSON.stringify)"#
    );
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let observed: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("CSP referrer observations"))?;
    assert_eq!(
        observed,
        serde_json::json!([
            ["", "", 201, true, true, 0],
            [page_url, page_url, 404, true, true, 0],
            [page_url, page_url, 0, true, true, 0],
            [origin_referrer, origin_referrer, 0, true, true, 0],
            [page_url, page_url, 0, true, true, 0]
        ])
    );
    assert!(
        servers
            .requests
            .lock()
            .iter()
            .all(|request| !request.path.starts_with("/blocked-csp-referrer"))
    );
    Ok(())
}
