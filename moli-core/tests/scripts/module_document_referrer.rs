use super::*;

fn importer_source(name: &str, module: bool, origin: &str, cross: &str) -> String {
    let mut source = dynamic_import_credentials::importer_source(name);
    if module {
        for (label, origin) in [("same", origin), ("cross", cross)] {
            source.push_str(&format!(
                "\nimport '{origin}/echo-origin.js?{name}-static-{label}';"
            ));
        }
    }
    source
}

pub(super) fn fixture_response(
    path: &str,
    query: &str,
    origin: &str,
    cross: &str,
) -> (&'static str, String, String) {
    let params: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(query.as_bytes()).collect();
    let mut headers = "Cache-Control: no-store\r\n".to_owned();
    if let Some(policy) = params.get("response") {
        headers.push_str(&format!("Referrer-Policy: {policy}\r\n"));
    }
    let body = if path.ends_with(".html") {
        headers.push_str("Content-Type: text/html\r\n");
        "<!doctype html><body>Document referrer policy".to_owned()
    } else {
        headers.push_str("Content-Type: text/javascript\r\n");
        importer_source(
            params["name"].as_ref(),
            params["kind"] == "module",
            origin,
            cross,
        )
    };
    ("200 OK", headers, body)
}

async fn check_module_document_referrer(child: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    // A child must use its own policy even when its parent permits a full URL.
    let mut page = browser
        .fetch(&format!(
            "{}/document-referrer.html?response={}",
            servers.origin,
            if child { "unsafe-url" } else { "no-referrer" },
        ))
        .await?;
    let document_url = format!(
        "{}/document-referrer.html?response=no-referrer{}",
        servers.origin,
        if child { "&child" } else { "" },
    );
    let mut cases = Vec::new();
    for kind in ["classic", "module"] {
        for external in [false, true] {
            let mut policies = vec![("default", None), ("invalid", None), ("unsafe-url", None)];
            if external && kind == "module" {
                policies.extend([
                    ("default", Some("invalid")),
                    ("default", Some("origin")),
                    ("unsafe-url", Some("no-referrer")),
                ]);
            }
            for (attribute, response) in policies {
                let name = format!("document-referrer-{}", cases.len());
                let mut src = url::Url::parse(&format!("{}/document-referrer.js", servers.origin))?;
                src.query_pairs_mut()
                    .append_pair("name", &name)
                    .append_pair("kind", kind);
                if let Some(response) = response {
                    src.query_pairs_mut().append_pair("response", response);
                }
                cases.push(serde_json::json!({
                    "name":name, "kind":kind, "external":external, "attribute":attribute,
                    "response":response, "src":src.as_str(),
                    "source":importer_source(&name, kind == "module", &servers.origin, &servers.cross_origin),
                }));
            }
        }
    }
    let probe = format!(
        r#"(async () => {{
            globalThis.importers = {{}};
            globalThis.importerReady = {{}};
            globalThis.timerImports = {{}};
            const cases = {cases};
            for (const test of cases) {{
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
                script.remove();
            }}
            const meta = document.createElement('meta');
            meta.name = 'referrer';
            for (const policy of ['no-referrer', 'origin', 'unsafe-url']) {{
                if (policy !== 'no-referrer') {{ meta.content = policy; document.head.append(meta); }}
                for (const test of cases)
                for (const invocation of ['direct', 'timer'])
                for (const [label, origin] of [['same', location.origin], ['cross', {cross}]])
                    await importers[test.name](origin + '/echo-origin.js?' + test.name + '-' + policy + '-' + invocation + '-' + label, invocation === 'timer');
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
                frame.src = {document_url};
                await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                return frame.contentWindow.eval({probe});
            }})()"#,
            document_url = serde_json::to_string(&document_url)?,
            probe = serde_json::to_string(&probe)?,
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
    let mut check = |path: &str, policy: &str, source: &str| {
        let matching: Vec<_> = requests.iter().filter(|r| r.path == path).collect();
        assert_eq!(matching.len(), 1, "child={child}, {path}: {requests:?}");
        let expected = match policy {
            "no-referrer" => None,
            "origin" => Some(format!("{}/", servers.origin)),
            "unsafe-url" => Some(source.to_owned()),
            other => panic!("unexpected policy {other}"),
        };
        if matching[0].referer != expected {
            mismatches.push(format!(
                "{path}: {:?}, expected {expected:?}",
                matching[0].referer
            ));
        }
    };
    for test in &cases {
        let name = test["name"].as_str().unwrap();
        let source = if test["external"] == true {
            test["src"].as_str().unwrap()
        } else {
            &document_url
        };
        let attribute = test["attribute"].as_str().unwrap();
        let response = test["response"].as_str();
        let explicit = match response {
            Some("origin") => Some("origin"),
            Some("no-referrer") => Some("no-referrer"),
            _ if attribute == "unsafe-url" => Some("unsafe-url"),
            _ => None,
        };
        if test["kind"] == "module" {
            for label in ["same", "cross"] {
                check(
                    &format!("/echo-origin.js?{name}-static-{label}"),
                    explicit.unwrap_or("no-referrer"),
                    source,
                );
            }
            if test["external"] == true {
                let url = url::Url::parse(source)?;
                check(
                    &format!("{}?{}", url.path(), url.query().unwrap()),
                    if attribute == "unsafe-url" {
                        "unsafe-url"
                    } else {
                        "no-referrer"
                    },
                    &document_url,
                );
            }
        }
        for policy in ["no-referrer", "origin", "unsafe-url"] {
            for invocation in ["direct", "timer"] {
                for label in ["same", "cross"] {
                    check(
                        &format!("/echo-origin.js?{name}-{policy}-{invocation}-{label}"),
                        explicit.unwrap_or(policy),
                        source,
                    );
                }
            }
        }
    }
    assert!(mismatches.is_empty(), "child={child}: {mismatches:#?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_module_imports_use_current_document_referrer_policy() -> Result<()> {
    check_module_document_referrer(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_module_imports_use_current_document_referrer_policy() -> Result<()> {
    check_module_document_referrer(true).await
}
