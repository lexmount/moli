use super::*;

pub(super) fn importer_source(name: &str) -> String {
    format!(
        r#"globalThis.importers[{0}] = (url, timer) => {{
            if (!timer) return import(url);
            return new Promise((resolve, reject) => {{
                globalThis.timerImports[url] = {{resolve, reject}};
                const key = JSON.stringify(url);
                setTimeout('import(' + key + ').then(timerImports[' + key + '].resolve, timerImports[' + key + '].reject)', 0);
            }});
        }};
        globalThis.importerReady[{0}]();"#,
        serde_json::to_string(name).unwrap()
    )
}

async fn check_dynamic_import_credentials(child: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/cookie-page.html", servers.origin))
        .await?;
    let mut cases = Vec::new();
    for kind in ["classic", "module"] {
        for external in [false, true] {
            for credentials in ["default", "anonymous", "use-credentials"] {
                let name = format!("{kind}-{external}-{credentials}");
                cases.push(serde_json::json!({
                    "name":name, "kind":kind, "external":external,
                    "credentials":credentials, "source":importer_source(&name),
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
                if (test.credentials !== 'default') script.crossOrigin = test.credentials;
                if (test.external) script.src = '/import-credentials.js?' + test.name;
                else script.textContent = test.source;
                await new Promise((resolve, reject) => {{
                    const timer = setTimeout(() => reject(new Error('source did not run: ' + test.name)), 3000);
                    importerReady[test.name] = () => {{ clearTimeout(timer); resolve(); }};
                    script.onerror = () => reject(new Error('failed to load ' + test.name));
                    document.head.append(script);
                }});
                // Later mutation/removal must not change the compiled script's fetch options.
                script.crossOrigin = test.credentials === 'use-credentials' ? 'anonymous' : 'use-credentials';
                script.remove();
                for (const invocation of ['direct', 'timer'])
                for (const [label, origin] of [['same', location.origin], ['cross', {cross}]])
                    await importers[test.name](origin + '/echo-origin.js?credentials-' + test.name + '-' + invocation + '-' + label, invocation === 'timer');
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
    for test in cases {
        for invocation in ["direct", "timer"] {
            for origin in ["same", "cross"] {
                let path = format!(
                    "/echo-origin.js?credentials-{}-{invocation}-{origin}",
                    test["name"].as_str().unwrap()
                );
                let matching: Vec<_> = requests
                    .iter()
                    .filter(|request| request.path == path)
                    .collect();
                assert_eq!(matching.len(), 1, "child={child}, {path}: {requests:?}");
                let request = matching[0];
                assert_eq!(request.sec_fetch_mode.as_deref(), Some("cors"), "{path}");
                assert_eq!(request.sec_fetch_dest.as_deref(), Some("script"), "{path}");
                let expected = (origin == "same" || test["credentials"] == "use-credentials")
                    .then_some("sriSession=present");
                if request.cookie.as_deref() != expected {
                    mismatches.push(format!(
                        "{path}: {:?}, expected {expected:?}",
                        request.cookie
                    ));
                }
            }
        }
    }
    assert!(mismatches.is_empty(), "child={child}: {mismatches:#?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_dynamic_imports_preserve_script_credentials() -> Result<()> {
    check_dynamic_import_credentials(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_dynamic_imports_preserve_script_credentials() -> Result<()> {
    check_dynamic_import_credentials(true).await
}
