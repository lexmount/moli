use super::*;

pub(super) fn fixture_response(
    path: &str,
    referrer: Option<&str>,
) -> (&'static str, String, String) {
    let (content_type, body) = match path {
        "/meta-policy.html" => (
            "text/html",
            r#"<!doctype html><head><meta name=referrer content=no-referrer id=first></head><body>Policy document
            <script>
            addEventListener('message', async event => {
                if (!event.data.metaPolicyProbe) return;
                try {
                    opener.postMessage({metaPolicyResult: await eval(event.data.metaPolicyProbe)}, '*');
                } catch (error) {
                    opener.postMessage({metaPolicyError: String(error)}, '*');
                }
            });
            </script></body>"#.to_owned(),
        ),
        "/meta-policy.js" => ("text/javascript", "/* policy probe */".to_owned()),
        _ => ("application/json", serde_json::to_string(&referrer).unwrap()),
    };
    (
        "200 OK",
        format!(
            "Content-Type: {content_type}\r\nCache-Control: no-store\r\nReferrer-Policy: unsafe-url\r\nAccess-Control-Allow-Origin: *\r\n"
        ),
        body,
    )
}

async fn check_meta_policy(owner: &str) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let document_url = format!("{}/meta-policy.html?owner={owner}", servers.origin);
    let parent_url = format!(
        "{}/document-referrer.html?response=no-referrer",
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
            const observations = [];
            let index = 0;
            async function check(label, expected) {{
                for (const origin of {origins}) {{
                    const id = ++index;
                    const resource = origin + '/meta-policy-fetch?' + id;
                    const actual = await (await fetch(resource)).json();
                    if (actual !== expected) throw new Error(label + ': fetch ' + actual + ' != ' + expected);
                    const explicit = await (await fetch(resource + '-explicit', {{referrerPolicy:'unsafe-url'}})).json();
                    if (explicit !== location.href) throw new Error(label + ': explicit policy ' + explicit);
                    const xhr = new XMLHttpRequest();
                    const value = await new Promise((resolve,reject) => {{
                        xhr.open('GET', resource + '-xhr');
                        xhr.onload = () => resolve(JSON.parse(xhr.responseText)); xhr.onerror = reject; xhr.send();
                    }});
                    if (value !== expected) throw new Error(label + ': XHR ' + value + ' != ' + expected);
                    const sync = new XMLHttpRequest();
                    sync.open('GET', resource + '-sync-xhr', false); sync.send();
                    const syncValue = JSON.parse(sync.responseText);
                    if (syncValue !== expected) throw new Error(label + ': sync XHR ' + syncValue + ' != ' + expected);
                    const observation = {{expected}};
                    if ({check_scripts}) {{
                        const url = origin + '/meta-policy.js?' + id;
                        await new Promise((resolve,reject) => {{
                            const script = document.createElement('script'); script.src=url;
                            script.onload=resolve; script.onerror=reject; document.body.append(script);
                        }});
                        observation.url = url;
                    }}
                    observations.push(observation);
                }}
            }}
            const origin = location.origin + '/';
            const first = document.getElementById('first');
            await check('parser meta overrides response', null);
            const second = document.createElement('meta'); second.name='referrer'; second.content='origin';
            document.head.insertBefore(second,first);
            await check('most recent insertion before older meta', origin);
            first.content='no-referrer';
            await check('same-value assignment delivers again', null);
            first.remove();
            await check('removal does not undo delivery', null);
            first.content='unsafe-url';
            await check('detached mutation has no effect', null);
            document.body.append(first);
            await check('body insertion delivers policy', location.href);
            first.content='never';
            await check('legacy never', null);
            first.content='always';
            await check('legacy always', location.href);
            first.content='origin';
            for (const value of ['', ' origin', 'origin, no-referrer', 'future-policy']) first.content=value;
            await check('invalid values retain current policy', origin);
            first.name='other'; first.content='no-referrer';
            await check('other metadata ignored', origin);
            first.name='REFERRER';
            await check('name change delivers content', null);
            const host=document.createElement('div');document.body.append(host);
            const shadow=host.attachShadow({{mode:'open'}});
            const hidden=document.createElement('meta'); hidden.name='referrer'; hidden.content='unsafe-url';
            shadow.append(hidden); hidden.content='origin';
            await check('shadow metadata ignored', null);
            first.setAttributeNS('urn:other','content','unsafe-url');
            await check('namespaced content ignored', null);
            first.setAttributeNS(null,'content','origin');
            await check('null namespace content applies', origin);
            first.remove(); second.remove();
            await check('all metadata removed', origin);
            return JSON.stringify(observations);
        }})()"#,
        origins = serde_json::to_string(&[&servers.origin, &servers.cross_origin])?,
        // Lightweight popups do not yet schedule dynamically inserted scripts.
        // Keep their Fetch and synchronous/asynchronous XHR coverage independent.
        check_scripts = owner != "popup",
    );
    let expression = match owner {
        "child" => format!(
            r#"(async () => {{
                const frame=document.createElement('iframe');frame.src={url};
                await new Promise(resolve => {{frame.onload=resolve;document.body.append(frame);}});
                return frame.contentWindow.eval({probe});
            }})()"#,
            url = serde_json::to_string(&document_url)?,
            probe = serde_json::to_string(&probe)?,
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
                }} finally {{ popup.close(); }}
            }})()"#,
            url = serde_json::to_string(&document_url)?,
            probe = serde_json::to_string(&probe)?,
        ),
        _ => probe,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let observations: Vec<serde_json::Value> = serde_json::from_str(
        result["value"]
            .as_str()
            .unwrap_or_else(|| panic!("{owner}: {result}")),
    )?;
    assert_eq!(observations.len(), 30, "{owner}: {result}");
    assert_eq!(
        observations
            .iter()
            .filter(|entry| entry["url"].is_string())
            .count(),
        if owner == "popup" { 0 } else { 30 },
        "{owner}: script observations"
    );
    let requests = servers.requests.lock();
    for observation in &observations {
        let Some(script_url) = observation["url"].as_str() else {
            continue;
        };
        let url = url::Url::parse(script_url)?;
        let path = format!("{}?{}", url.path(), url.query().unwrap());
        let found = requests
            .iter()
            .filter(|request| request.path == path)
            .collect::<Vec<_>>();
        assert_eq!(found.len(), 1, "{owner}, {path}: {requests:?}");
        assert_eq!(
            found[0].referer.as_deref(),
            observation["expected"].as_str(),
            "{owner}, {path}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_document_meta_referrer_policy_tracks_deliveries() -> Result<()> {
    check_meta_policy("main").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_meta_referrer_policy_tracks_deliveries() -> Result<()> {
    check_meta_policy("child").await
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_document_meta_referrer_policy_tracks_deliveries() -> Result<()> {
    check_meta_policy("popup").await
}
