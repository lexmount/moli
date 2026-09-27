use super::*;

pub(super) fn fixture_response(query: &str) -> (&'static str, String, String) {
    let params: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(query.as_bytes()).collect();
    let mut headers = "Content-Type: text/javascript\r\nCache-Control: no-store\r\n".to_owned();
    if let Some(policy) = params.get("response") {
        headers.push_str(&format!("Referrer-Policy: {policy}\r\n"));
    }
    (
        "200 OK",
        headers,
        dynamic_import_credentials::importer_source(params["name"].as_ref()),
    )
}

async fn check_dynamic_import_referrer_policy(child: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html?top", servers.origin))
        .await?;
    let mut cases = Vec::new();
    for kind in ["classic", "module"] {
        for external in [false, true] {
            let mut policies = vec![
                ("default", None, "strict-origin-when-cross-origin"),
                ("invalid", None, "strict-origin-when-cross-origin"),
                ("no-referrer", None, "no-referrer"),
                ("origin", None, "origin"),
                ("same-origin", None, "same-origin"),
                ("unsafe-url", None, "unsafe-url"),
            ];
            if external {
                for (header, effective) in [
                    ("no-referrer", "no-referrer"),
                    ("origin", "origin"),
                    ("invalid", "unsafe-url"),
                    ("no-referrer, origin", "origin"),
                ] {
                    policies.push((
                        "unsafe-url",
                        Some(header),
                        if kind == "module" {
                            effective
                        } else {
                            "unsafe-url"
                        },
                    ));
                }
            }
            for (attribute, response, effective) in policies {
                let name = format!("referrer-{}", cases.len());
                let mut src = url::Url::parse(&format!("{}/import-referrer.js", servers.origin))?;
                src.query_pairs_mut().append_pair("name", &name);
                if let Some(response) = response {
                    src.query_pairs_mut().append_pair("response", response);
                }
                cases.push(serde_json::json!({
                    "name":name, "kind":kind, "external":external, "attribute":attribute,
                    "effective":effective, "src":src.as_str(), "response":response,
                    "source":dynamic_import_credentials::importer_source(&name),
                }));
            }
        }
    }
    let probe = format!(
        r#"(async () => {{
            globalThis.importers = {{}};
            globalThis.importerReady = {{}};
            globalThis.timerImports = {{}};
            for (const test of {cases}) {{
                const script = document.createElement('script');
                if (test.kind === 'module') script.type = 'module';
                if (test.attribute !== 'default') script.referrerPolicy = test.attribute;
                if (test.external) script.src = test.src;
                else script.textContent = test.source;
                await new Promise((resolve, reject) => {{
                    const timer = setTimeout(() => reject(new Error('source did not run: ' + test.name)), 3000);
                    importerReady[test.name] = () => {{ clearTimeout(timer); resolve(); }};
                    script.onerror = () => reject(new Error('failed to load ' + test.name));
                    document.head.append(script);
                }});
                script.referrerPolicy = test.attribute === 'no-referrer' ? 'unsafe-url' : 'no-referrer';
                script.remove();
                for (const invocation of ['direct', 'timer'])
                for (const [label, origin] of [['same', location.origin], ['cross', {cross}]])
                    await importers[test.name](origin + '/echo-origin.js?' + test.name + '-' + invocation + '-' + label, invocation === 'timer');
            }}
            return true;
        }})()"#,
        cases = serde_json::to_string(&cases)?,
        cross = serde_json::to_string(&servers.cross_origin)?,
    );
    let expression = if child {
        format!(
            r#"(async () => {{
            const frame = document.createElement('iframe');
            frame.src = '/page.html?child';
            await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
            return frame.contentWindow.eval({});
        }})()"#,
            serde_json::to_string(&probe)?
        )
    } else {
        probe
    };
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    assert_eq!(result["value"], true, "child={child}: {result}");

    let requests = servers.requests.lock();
    let mut mismatches = Vec::new();
    for test in &cases {
        let source_url = if test["external"] == true {
            test["src"].as_str().unwrap().to_owned()
        } else {
            format!(
                "{}/page.html?{}",
                servers.origin,
                if child { "child" } else { "top" }
            )
        };
        for invocation in ["direct", "timer"] {
            for origin in ["same", "cross"] {
                let path = format!(
                    "/echo-origin.js?{}-{invocation}-{origin}",
                    test["name"].as_str().unwrap()
                );
                let matching: Vec<_> = requests
                    .iter()
                    .filter(|request| request.path == path)
                    .collect();
                assert_eq!(matching.len(), 1, "child={child}, {path}: {requests:?}");
                let expected = match (test["effective"].as_str().unwrap(), origin) {
                    ("no-referrer", _) | ("same-origin", "cross") => None,
                    ("origin", _) | ("strict-origin-when-cross-origin", "cross") => {
                        Some(format!("{}/", servers.origin))
                    }
                    _ => Some(source_url.clone()),
                };
                if matching[0].referer != expected {
                    mismatches.push(format!(
                        "{test}: {invocation}/{origin}: {:?}, expected {expected:?}",
                        matching[0].referer
                    ));
                }
            }
        }
    }
    assert!(mismatches.is_empty(), "child={child}: {mismatches:#?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_dynamic_imports_preserve_script_referrer_policy() -> Result<()> {
    check_dynamic_import_referrer_policy(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_dynamic_imports_preserve_script_referrer_policy() -> Result<()> {
    check_dynamic_import_referrer_policy(true).await
}
