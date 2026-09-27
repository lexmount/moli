use super::*;

pub(super) fn fixture_response(
    path: &str,
    query: &str,
    referer: Option<&str>,
) -> (&'static str, String, String) {
    let (content_type, body, policy) = match path {
        "/delivered-document-referrer.html" => {
            let params: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(query.as_bytes()).collect();
            (
                "text/html",
                "<!doctype html><body>Policy document".to_owned(),
                params.get("response").map_or("unsafe-url", |value| value.as_ref()).to_owned(),
            )
        }
        "/delivered-meta-policy.html" => (
            "text/html",
            r#"<!doctype html><head><meta name=referrer content=no-referrer id=first></head><body>
            <script>
            addEventListener('message', async event => {
                if (!event.data.metaPolicyProbe) return;
                try {
                    opener.postMessage({metaPolicyResult: await eval(event.data.metaPolicyProbe)}, '*');
                } catch (error) {
                    opener.postMessage({metaPolicyError: String(error)}, '*');
                }
            });
            </script>"#.to_owned(),
            "unsafe-url".to_owned(),
        ),
        _ => ("application/json", serde_json::to_string(&referer).unwrap(), "unsafe-url".to_owned()),
    };
    (
        "200 OK",
        format!(
            "Content-Type: {content_type}\r\nCache-Control: no-store\r\nReferrer-Policy: {policy}\r\nAccess-Control-Allow-Origin: *\r\n"
        ),
        body,
    )
}

async fn check_document_referrer_dispatch(owner: &str) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let document_url = format!("{}/delivered-meta-policy.html?owner={owner}", servers.origin);
    let parent_url = format!(
        "{}/delivered-document-referrer.html?response=unsafe-url",
        servers.origin
    );
    let mut page = browser
        .fetch(if owner == "main" {
            &document_url
        } else {
            &parent_url
        })
        .await?;
    let probe = format!(
        r#"(async () => {{
        let checks = 0;
        async function check(label, expected) {{
            for (const origin of {origins}) {{
                const resource = origin + '/delivered-meta-policy-fetch?dispatch-' + ++checks;
                const actual = await (await fetch(resource)).json();
                if (actual !== expected) throw new Error(label + ': fetch ' + actual + ' != ' + expected);
                const explicit = await (await fetch(resource + '-explicit', {{referrerPolicy:'unsafe-url'}})).json();
                if (explicit !== location.href) throw new Error(label + ': explicit ' + explicit);
                const xhr = new XMLHttpRequest();
                const value = await new Promise((resolve,reject) => {{
                    xhr.open('GET', resource + '-xhr');
                    xhr.onload = () => resolve(JSON.parse(xhr.responseText)); xhr.onerror = reject; xhr.send();
                }});
                if (value !== expected) throw new Error(label + ': XHR ' + value + ' != ' + expected);
                const sync = new XMLHttpRequest();
                sync.open('GET', resource + '-sync-xhr', false); sync.send();
                if (JSON.parse(sync.responseText) !== expected) throw new Error(label + ': sync XHR');
            }}
        }}
        const first = document.getElementById('first');
        await check('parser meta overrides response', null);
        const second = document.createElement('meta'); second.name='referrer'; second.content='origin';
        document.head.insertBefore(second, first);
        await check('new delivery before older element', location.origin + '/');
        first.content='no-referrer';
        await check('same-value delivery', null);
        first.remove();
        await check('removal retains policy', null);
        first.content='unsafe-url';
        await check('detached mutation ignored', null);
        document.body.append(first);
        await check('reinsertion delivers policy', location.href);
        first.content='origin, no-referrer';
        await check('header-list grammar rejected for meta', location.href);
        first.content='never';
        await check('legacy alias', null);
        first.content='origin';
        await check('last valid mutation', location.origin + '/');
        return checks;
    }})()"#,
        origins = serde_json::to_string(&[&servers.origin, &servers.cross_origin])?
    );
    let expression = match owner {
        "child" => format!(
            r#"(async () => {{
            const frame=document.createElement('iframe');frame.src={url};
            await new Promise(resolve => {{frame.onload=resolve;document.body.append(frame);}});
            return frame.contentWindow.eval({probe});
        }})()"#,
            url = serde_json::to_string(&document_url)?,
            probe = serde_json::to_string(&probe)?
        ),
        "popup" => format!(
            r#"(async () => {{
            const popup=window.open({url});
            while (!popup.document || popup.document.readyState !== 'complete' || !popup.document.getElementById('first'))
                await new Promise(resolve => setTimeout(resolve,1));
            try {{
                return await new Promise((resolve,reject) => {{
                    function onResult(event) {{
                        if ('metaPolicyResult' in event.data) {{
                            removeEventListener('message',onResult); resolve(event.data.metaPolicyResult);
                        }} else if ('metaPolicyError' in event.data) {{
                            removeEventListener('message',onResult); reject(new Error(event.data.metaPolicyError));
                        }}
                    }}
                    addEventListener('message',onResult);
                    popup.postMessage({{metaPolicyProbe:{probe}}}, '*');
                }});
            }} finally {{popup.close();}}
        }})()"#,
            url = serde_json::to_string(&document_url)?,
            probe = serde_json::to_string(&probe)?
        ),
        _ => probe,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    assert_eq!(result["value"], 18, "{owner}: {result}");
    let requests = servers.requests.lock();
    let requests: Vec<_> = requests
        .iter()
        .filter(|request| request.path.starts_with("/delivered-meta-policy-fetch?dispatch-"))
        .collect();
    assert_eq!(requests.len(), 72, "{owner}: {requests:?}");
    for request in requests {
        assert_eq!(
            request.sec_fetch_dest.as_deref(),
            Some("empty"),
            "{owner}: {request:?}"
        );
        assert_eq!(
            request.sec_fetch_mode.as_deref(),
            Some("cors"),
            "{owner}: {request:?}"
        );
        assert!(
            matches!(
                request.sec_fetch_site.as_deref(),
                Some("same-origin" | "same-site")
            ),
            "{owner}: {request:?}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_document_referrer_policy_reaches_fetch_and_xhr() -> Result<()> {
    check_document_referrer_dispatch("main").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_referrer_policy_reaches_fetch_and_xhr() -> Result<()> {
    check_document_referrer_dispatch("child").await
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_document_referrer_policy_reaches_fetch_and_xhr() -> Result<()> {
    check_document_referrer_dispatch("popup").await
}
