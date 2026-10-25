use super::*;

pub(super) fn fixture_response(path: &str, mode: &str) -> (&'static str, String, String) {
    if path == "/csp-connect-resource" {
        return (
            "200 OK",
            "Content-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\n".into(),
            "connected".into(),
        );
    }
    let policy = if mode == "report" {
        "Content-Security-Policy-Report-Only: connect-src 'none'"
    } else {
        "Content-Security-Policy: connect-src 'self'"
    };
    (
        "200 OK",
        format!("Content-Type: text/html\r\n{policy}\r\nCache-Control: no-store\r\n"),
        "<!doctype html><body>Inherited connect policy</body>".into(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn local_documents_use_inherited_connect_policy_for_fetch_admission() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for mode in ["enforce", "report"] {
        let mut page = browser
            .fetch(&format!(
                "{}/csp-connect-policy.html?{mode}",
                servers.origin
            ))
            .await?;
        let expression = format!(
            r#"(async () => {{
                const rows = [];
                const mode = {mode:?};
                const probe = async (view, label) => {{
                    for (const target of ['same', 'cross']) {{
                        const violationExpected = mode === 'report' || target === 'cross';
                        const reported = violationExpected
                            ? new Promise(resolve => view.document.addEventListener(
                                'securitypolicyviolation', resolve, {{once:true}}))
                            : Promise.resolve(null);
                        const origin = target === 'same' ? {origin:?} : {cross:?};
                        const url = origin + '/csp-connect-resource?mode=' + mode +
                            '&scope=' + label + '&target=' + target;
                        const outcome = await view.fetch(url).then(
                            response => response.text(), error => error.name);
                        const event = await reported;
                        rows.push([label, target, outcome, event?.disposition ?? null,
                            event === null ? null : event.isTrusted && event.target === view.document]);
                    }}
                }};
                await probe(window, 'top');
                for (const source of ['about:blank', 'srcdoc']) {{
                    const frame = document.createElement('iframe');
                    if (source === 'srcdoc') frame.srcdoc = '<!doctype html><body>child</body>';
                    else frame.src = source;
                    await new Promise(resolve => {{frame.onload = resolve; document.body.append(frame);}});
                    try {{await probe(frame.contentWindow, source);}}
                    finally {{frame.remove();}}
                }}
                const popup = open();
                try {{await probe(popup, 'popup');}}
                finally {{popup.close();}}
                return rows;
            }})().then(JSON.stringify)"#,
            origin = servers.origin,
            cross = servers.cross_origin,
        );
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let observed: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .expect("connect policy observations"),
        )?;
        let mut expected = Vec::new();
        for scope in ["top", "about:blank", "srcdoc", "popup"] {
            for target in ["same", "cross"] {
                let blocked = mode == "enforce" && target == "cross";
                let reported = mode == "report" || blocked;
                expected.push(serde_json::json!([
                    scope,
                    target,
                    if blocked { "TypeError" } else { "connected" },
                    if reported { Some(mode) } else { None },
                    if reported { Some(true) } else { None }
                ]));
            }
        }
        assert_eq!(observed, serde_json::Value::Array(expected));
    }
    let requests = servers.requests.lock();
    let mut resource_count = 0;
    for request in requests.iter() {
        if request.path.starts_with("/csp-connect-resource?") {
            resource_count += 1;
            assert!(
                !request.path.contains("mode=enforce") || !request.path.contains("target=cross")
            );
        }
    }
    assert_eq!(resource_count, 12);
    Ok(())
}
