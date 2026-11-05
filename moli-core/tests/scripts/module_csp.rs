use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn module_csp_redirect_completion_uses_admitted_snapshot_after_meta_policy_change()
-> Result<()> {
    let origin = TcpListener::bind("127.0.0.1:0").await?;
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let origin_url = format!("http://{}", origin.local_addr()?);
    let target_url = format!("http://{}/final.txt", target.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let origin_requests = Arc::clone(&requests);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let mut servers = JoinSet::new();
    servers.spawn(async move {
        loop {
            let Ok((mut stream, _)) = origin.accept().await else { break };
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") { head.push(stream.read_u8().await?); }
            let head = String::from_utf8(head)?;
            let path = head.split_whitespace().nth(1).unwrap();
            origin_requests.lock().push(path.to_owned());
            let (status, headers, body) = match path {
                "/page.html" => ("200 OK", "Content-Type: text/html\r\n".to_owned(), "<!doctype html><head></head><body>snapshot"),
                "/before.txt" => ("302 Found", format!("Location: {target_url}\r\n"), ""),
                _ => ("200 OK", "Content-Type: text/plain\r\n".to_owned(), "must not be fetched"),
            };
            stream.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
        }
        anyhow::Ok(())
    });
    servers.spawn(async move {
        let (mut stream, _) = target.accept().await?;
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") { head.push(stream.read_u8().await?); }
        started_tx.send(()).unwrap();
        release_rx.await?;
        let body = "admitted snapshot";
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
        anyhow::Ok(())
    });
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{origin_url}/page.html")).await?;
    page.evaluate_runtime_expression_async(
        r#"
      globalThis.moduleViolations = [];
      addEventListener('securitypolicyviolation', event => moduleViolations.push(event.blockedURI));
      globalThis.admittedImport = import('/before.txt', {with:{type:'text'}})
        .then(module => module.default, error => 'error: ' + error.message);
      'started';
    "#,
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(15), started_rx).await??;
    page.evaluate_runtime_expression_async(
        r#"
      const meta = document.createElement('meta');
      meta.httpEquiv = 'Content-Security-Policy'; meta.content = "connect-src 'none'";
      document.head.append(meta); 'policy installed';
    "#,
    )
    .await?;
    release_tx.send(()).unwrap();
    let admitted = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async("admittedImport", true),
    )
    .await??;
    assert_eq!(admitted["value"], "admitted snapshot", "{admitted}");
    let rejected = page
        .evaluate_runtime_expression_with_await_async(
            r#"
      import('/after.txt', {with:{type:'text'}}).then(() => 'loaded', error => error.name)
    "#,
            true,
        )
        .await?;
    assert_eq!(rejected["value"], "TypeError", "{rejected}");
    assert_eq!(*requests.lock(), ["/page.html", "/before.txt"]);
    while let Some(task) = servers.try_join_next() {
        task??;
    }
    Ok(())
}

pub(super) fn fixture_response(
    path: &str,
    query: &str,
    origin: &str,
    cross: &str,
) -> (&'static str, String, String) {
    if matches!(path, "/module-csp-report-uri" | "/module-csp-report-to") {
        return ("204 No Content", String::new(), String::new());
    }
    if path == "/module-csp-privacy.html" {
        let child = query.starts_with("child-");
        let report_only = query.contains("report-only");
        let reporting = if query.ends_with("report-to") {
            "report-to csp"
        } else {
            "report-uri /module-csp-report-uri"
        };
        let restrictive = format!(
            "script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; connect-src 'self'; style-src-elem 'self'; {reporting}"
        );
        let enforce = if report_only {
            "script-src * 'unsafe-inline' 'wasm-unsafe-eval'; connect-src *; style-src-elem *"
        } else {
            &restrictive
        };
        let report = if report_only {
            format!("Content-Security-Policy-Report-Only: {restrictive}\r\n")
        } else {
            String::new()
        };
        let mut body = r#"<!doctype html><script>
          globalThis.privacyProbe = async () => {
            const modules = [['text','text'],['json','json'],['css','css'],['js',null]];
            /*WASM*/
            const events = [];
            let signal;
            const reported = new Promise(resolve => signal = resolve);
            addEventListener('securitypolicyviolation', event => {
              events.push({blockedURI:event.blockedURI, disposition:event.disposition, directive:event.effectiveDirective});
              if (events.length === modules.length) signal();
            });
            const results = [];
            for (const [suffix, type] of modules) {
              const url = '/module-csp.' + suffix + '?privacy-redirect';
              try { await import(url, type ? {with:{type}} : {}); results.push({status:'loaded'}); }
              catch (error) { results.push({status:error.name, message:error.message}); }
            }
            await reported;
            return JSON.stringify({events, results});
          };
        </script><body>Module redirect privacy"#.to_owned();
        // Child Wasm evaluation currently looks up records in the top-level
        // modulator. Window and Worker tests retain Wasm coverage.
        body = body.replace(
            "/*WASM*/",
            if child {
                ""
            } else {
                "modules.push(['wasm',null]);"
            },
        );
        if child {
            body.push_str(
                "<script>privacyProbe().then(result => parent.postMessage(result, '*'));</script>",
            );
        }
        return (
            "200 OK",
            format!(
                "Content-Type: text/html\r\nContent-Security-Policy: {enforce}\r\n{report}Reporting-Endpoints: csp=\"{origin}/module-csp-report-to\"\r\n"
            ),
            body,
        );
    }
    if query == "privacy-redirect" {
        return (
            "302 Found",
            format!(
                "Location: {cross}{path}?token=SECRET-redirect\r\nAccess-Control-Allow-Origin: *\r\n"
            ),
            String::new(),
        );
    }
    if path == "/module-csp.html" {
        let frame_case = query.strip_prefix("frame-");
        let preload_case = query.strip_prefix("preload-");
        let query = frame_case.or(preload_case).unwrap_or(query);
        let policy = match query {
            "connect-none" => "connect-src 'none'; style-src-elem 'self'",
            "default-none" => "default-src 'none'; style-src-elem 'self'",
            "style-none" => "connect-src 'self'; style-src-elem 'none'",
            "redirect" => "connect-src 'self'; style-src-elem 'self'",
            "redirect-report-only" | "roundtrip-report-only" => "connect-src *; style-src-elem *",
            _ => "connect-src 'self'; style-src-elem 'self'",
        };
        let script_policy = if query.starts_with("script-") {
            if query.ends_with("report-only") {
                "script-src * 'unsafe-inline' 'wasm-unsafe-eval'"
            } else {
                "script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'"
            }
        } else {
            "script-src 'nonce-setup'"
        };
        let report = if query.starts_with("script-") && query.ends_with("report-only") {
            "Content-Security-Policy-Report-Only: script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'\r\n"
        } else if query == "report-only" {
            "Content-Security-Policy-Report-Only: connect-src 'none'; style-src-elem 'none'\r\n"
        } else if query == "redirect-report-only" || query == "roundtrip-report-only" {
            "Content-Security-Policy-Report-Only: connect-src 'self'; style-src-elem 'self'\r\n"
        } else {
            ""
        };
        let mut response = (
            "200 OK",
            format!("Content-Type: text/html\r\nContent-Security-Policy: {script_policy}; {policy}\r\n{report}"),
            r#"<!doctype html><script nonce="setup">
              globalThis.moduleViolations = [];
              document.addEventListener('securitypolicyviolation', event => {
                moduleViolations.push([event.effectiveDirective, event.disposition, event.blockedURI]);
                document.dispatchEvent(new Event('module-violation-observed'));
              });
              globalThis.preloadResults = {};
              globalThis.preloadsReady = new Promise(resolve => {
                const observe = event => {
                  if (event.target.tagName !== 'LINK') return;
                  preloadResults[event.target.as] = event.type;
                  if (Object.keys(preloadResults).length === 3) resolve(preloadResults);
                };
                document.addEventListener('load', observe, true);
                document.addEventListener('error', observe, true);
              });
            </script>
            <link rel="modulepreload" as="text" href="/module-csp.text?preload">
            <link rel="modulepreload" as="json" href="/module-csp.json?preload">
            <link rel="modulepreload" as="style" href="/module-csp.css?preload">
            <body>Module CSP"#.to_owned() + if query == "allowed" {
                r#"<script type="module" nonce="setup">
                  import text from '/module-csp.text?static' with {type: 'text'};
                  import json from '/module-csp.json?static' with {type: 'json'};
                  import css from '/module-csp.css?static' with {type: 'css'};
                  globalThis.staticModules = [text, json, css.cssRules.length];
                </script>"#
            } else { "" },
        );
        if preload_case.is_some() {
            let target = if query == "redirect" {
                "redirect"
            } else {
                "roundtrip"
            };
            response.2 = response.2.replace("?preload", &format!("?{target}"));
        }
        if frame_case.is_some() {
            response.2.push_str(
                r#"<script nonce="setup">
              (async () => {
                const events = [];
                let signal;
                const reported = new Promise(resolve => signal = resolve);
                addEventListener('securitypolicyviolation', event => {
                  events.push([event.effectiveDirective, event.disposition, event.blockedURI]);
                  signal();
                });
                const status = await import('/module-csp.text?roundtrip', {with:{type:'text'}})
                    .then(() => 'loaded', error => error.name);
                await reported;
                parent.postMessage(JSON.stringify({status, events}), '*');
              })();
            </script>"#,
            );
        }
        return response;
    }
    if query == "redirect" || query == "roundtrip" || query == "return" {
        return (
            "302 Found",
            format!(
                "Location: {}{path}?{}\r\nAccess-Control-Allow-Origin: *\r\n",
                if query == "return" { origin } else { cross },
                if query == "roundtrip" {
                    "return"
                } else {
                    "final"
                }
            ),
            String::new(),
        );
    }
    let (mime, body) = match path {
        "/module-csp.text" => ("text/plain", "module text"),
        "/module-csp.js" => ("text/javascript", "export default 'module script';"),
        "/module-csp.wasm" => ("application/wasm", "\0asm\u{1}\0\0\0"),
        "/module-csp.json" => ("application/json", "\"module text\""),
        "/module-csp.css" => ("text/css", ".module-csp { color: green; }"),
        _ => unreachable!("unknown module CSP fixture {path}"),
    };
    (
        "200 OK",
        format!("Content-Type: {mime}\r\nAccess-Control-Allow-Origin: *\r\n"),
        body.to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn module_csp_redirect_privacy_events_reports_and_errors_use_original_request_url()
-> Result<()> {
    for child in [false, true] {
        for report_only in [false, true] {
            for report_to in [false, true] {
                let suffixes: &[&str] = if child {
                    &["text", "json", "css", "js"]
                } else {
                    &["text", "json", "css", "js", "wasm"]
                };
                let module_count = suffixes.len();
                let servers = IntegrityServers::spawn().await?;
                let browser = Browser::new(AppConfig::default())?;
                let case = format!(
                    "{}{}-{}",
                    if child { "child-" } else { "" },
                    if report_only {
                        "report-only"
                    } else {
                        "enforce"
                    },
                    if report_to { "report-to" } else { "report-uri" }
                );
                let page_url = if child {
                    format!("{}/page.html", servers.origin)
                } else {
                    format!("{}/module-csp-privacy.html?{case}", servers.origin)
                };
                let mut page = browser.fetch(&page_url).await?;
                let expression = if child {
                    format!(
                        r#"new Promise(resolve => {{
                      const frame = document.createElement('iframe');
                      addEventListener('message', function receive(event) {{
                        if (event.source !== frame.contentWindow) return;
                        removeEventListener('message', receive); resolve(event.data);
                      }});
                      frame.src = '/module-csp-privacy.html?{case}';
                      document.body.append(frame);
                    }})"#
                    )
                } else {
                    "privacyProbe()".to_owned()
                };
                let result = tokio::time::timeout(
                    Duration::from_secs(15),
                    page.evaluate_runtime_expression_with_await_async(&expression, true),
                )
                .await??;
                let serialized = result["value"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{case}: {result}"));
                assert!(
                    !serialized.contains("SECRET-redirect"),
                    "{case}: {serialized}"
                );
                assert!(
                    !serialized.contains(&servers.cross_origin),
                    "{case}: {serialized}"
                );
                let result: serde_json::Value = serde_json::from_str(serialized)?;
                assert_eq!(
                    result["events"].as_array().unwrap().len(),
                    module_count,
                    "{case}: {result}"
                );
                assert_eq!(
                    result["results"].as_array().unwrap().len(),
                    module_count,
                    "{case}: {result}"
                );
                for outcome in result["results"].as_array().unwrap() {
                    assert_eq!(
                        outcome["status"],
                        if report_only { "loaded" } else { "TypeError" },
                        "{case}: {result}"
                    );
                }
                let mut originals: Vec<_> = suffixes
                    .iter()
                    .map(|suffix| {
                        format!("{}/module-csp.{suffix}?privacy-redirect", servers.origin)
                    })
                    .collect();
                originals.sort();
                let mut event_urls: Vec<_> = result["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| {
                        assert_eq!(
                            event["disposition"],
                            if report_only { "report" } else { "enforce" }
                        );
                        event["blockedURI"].as_str().unwrap().to_owned()
                    })
                    .collect();
                event_urls.sort();
                assert_eq!(event_urls, originals, "{case}");
                let endpoint = if report_to {
                    "/module-csp-report-to"
                } else {
                    "/module-csp-report-uri"
                };
                tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let changed = servers.requests_changed.notified();
                        if servers
                            .requests
                            .lock()
                            .iter()
                            .filter(|r| r.path == endpoint)
                            .count()
                            >= module_count
                        {
                            break;
                        }
                        changed.await;
                    }
                })
                .await?;
                let requests = servers.requests.lock();
                assert_eq!(
                    requests
                        .iter()
                        .filter(|r| r.path.contains("token=SECRET-redirect"))
                        .count(),
                    if report_only { module_count } else { 0 },
                    "{case}: forbidden target request"
                );
                let reports: Vec<_> = requests.iter().filter(|r| r.path == endpoint).collect();
                assert_eq!(
                    reports.len(),
                    module_count,
                    "{case}: reports must be sent once per module"
                );
                let mut report_urls = Vec::new();
                for report in reports {
                    assert_eq!(report.method, "POST");
                    assert!(
                        !report.body.contains("SECRET-redirect"),
                        "{case}: {}",
                        report.body
                    );
                    assert!(
                        !report.body.contains(&servers.cross_origin),
                        "{case}: {}",
                        report.body
                    );
                    let body: serde_json::Value = serde_json::from_str(&report.body)?;
                    report_urls.push(
                        if report_to {
                            body[0]["body"]["blockedURL"].as_str().unwrap()
                        } else {
                            body["csp-report"]["blocked-uri"].as_str().unwrap()
                        }
                        .to_owned(),
                    );
                }
                report_urls.sort();
                assert_eq!(report_urls, originals, "{case}");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn synthetic_modules_apply_destination_csp_to_imports_and_modulepreloads() -> Result<()> {
    for case in [
        "connect-none",
        "default-none",
        "style-none",
        "allowed",
        "report-only",
        "redirect",
        "redirect-report-only",
        "roundtrip",
        "roundtrip-report-only",
    ] {
        let servers = IntegrityServers::spawn().await?;
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser
            .fetch(&format!("{}/module-csp.html?{case}", servers.origin))
            .await?;
        let expression = format!(
            r#"(async () => {{
          const preloads = await preloadsReady;
          const results = [];
          for (const [type, suffix] of [['text', 'text'], ['json', 'json'], ['css', 'css']]) {{
            results.push(await import('/module-csp.' + suffix + '?{query}', {{with: {{type}}}})
              .then(module => type === 'css' ? module.default.cssRules.length : module.default,
                error => error.name));
          }}
          if ({expected_violations} > 0) {{
            const ready = () => moduleViolations.length >= {expected_violations};
            if (!ready()) await new Promise(resolve => {{
              const observe = () => {{ if (ready()) {{
                document.removeEventListener('module-violation-observed', observe); resolve();
              }} }};
              document.addEventListener('module-violation-observed', observe);
            }});
          }}
          return JSON.stringify({{preloads, results, staticModules: globalThis.staticModules,
            violations: moduleViolations}});
        }})()"#,
            query = if case.starts_with("redirect") {
                "redirect"
            } else if case.starts_with("roundtrip") {
                "roundtrip"
            } else {
                "dynamic"
            },
            expected_violations = match case {
                "connect-none" | "default-none" => 4,
                "style-none" => 2,
                "report-only" => 6,
                "redirect" | "redirect-report-only" | "roundtrip" | "roundtrip-report-only" => 3,
                _ => 0,
            },
        );
        let observed = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let observed: serde_json::Value = serde_json::from_str(
            observed["value"]
                .as_str()
                .unwrap_or_else(|| panic!("{case}: {observed}")),
        )?;
        let connect_blocked = matches!(case, "connect-none" | "default-none");
        let style_blocked = case == "style-none";
        assert_eq!(
            observed["preloads"],
            serde_json::json!({
                "text": if connect_blocked { "error" } else { "load" },
                "json": if connect_blocked { "error" } else { "load" },
                "style": if style_blocked { "error" } else { "load" },
            }),
            "{case}: {observed}"
        );
        let redirected = case == "redirect" || case == "roundtrip";
        assert_eq!(
            observed["results"],
            serde_json::json!([
                if connect_blocked || redirected {
                    serde_json::json!("TypeError")
                } else {
                    serde_json::json!("module text")
                },
                if connect_blocked || redirected {
                    serde_json::json!("TypeError")
                } else {
                    serde_json::json!("module text")
                },
                if style_blocked || redirected {
                    serde_json::json!("TypeError")
                } else {
                    serde_json::json!(1)
                },
            ]),
            "{case}: {observed}"
        );
        let requests = servers.requests.lock();
        for extension in ["text", "json", "css"] {
            let blocked = if extension == "css" {
                style_blocked
            } else {
                connect_blocked
            };
            let count = requests
                .iter()
                .filter(|request| {
                    request
                        .path
                        .starts_with(&format!("/module-csp.{extension}"))
                })
                .count();
            assert_eq!(
                count,
                if blocked {
                    0
                } else if case == "roundtrip-report-only" {
                    4
                } else if case == "redirect-report-only" || case == "allowed" {
                    3
                } else {
                    2
                },
                "{case}/{extension}: {requests:?}"
            );
        }
        if redirected {
            assert!(
                requests
                    .iter()
                    .all(|request| !request.path.ends_with("?final")
                        && !request.path.ends_with("?return")),
                "blocked redirect contacted its destination: {requests:?}"
            );
        }
        let violations = observed["violations"].as_array().unwrap();
        if case == "allowed" {
            assert!(violations.is_empty(), "{observed}");
            assert_eq!(
                observed["staticModules"],
                serde_json::json!(["module text", "module text", 1])
            );
        }
        if case.ends_with("report-only") || redirected {
            for directive in ["connect-src", "style-src-elem"] {
                assert!(
                    violations.iter().any(|entry| entry[0] == directive
                        && entry[1] == if redirected { "enforce" } else { "report" }),
                    "{case}: {observed}"
                );
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_module_csp_redirects_keep_the_child_policy_and_reporting_owner() -> Result<()> {
    for report_only in [false, true] {
        let servers = IntegrityServers::spawn().await?;
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser
            .fetch(&format!("{}/page.html", servers.origin))
            .await?;
        let case = if report_only {
            "roundtrip-report-only"
        } else {
            "roundtrip"
        };
        let expression = format!(
            r#"new Promise(resolve => {{
          const frame = document.createElement('iframe');
          addEventListener('message', function receive(event) {{
            if (event.source !== frame.contentWindow) return;
            removeEventListener('message', receive);
            resolve(event.data);
          }});
          frame.src = '/module-csp.html?frame-{case}';
          document.body.append(frame);
        }})"#
        );
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let result: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .unwrap_or_else(|| panic!("child CSP: {result}")),
        )?;
        assert_eq!(
            result["status"],
            if report_only { "loaded" } else { "TypeError" },
            "{result}"
        );
        assert_eq!(result["events"].as_array().unwrap().len(), 1, "{result}");
        assert_eq!(result["events"][0][0], "connect-src");
        assert_eq!(
            result["events"][0][1],
            if report_only { "report" } else { "enforce" }
        );
        assert_eq!(
            servers
                .requests
                .lock()
                .iter()
                .filter(|request| request.path == "/module-csp.text?return")
                .count(),
            usize::from(report_only)
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn script_and_wasm_module_csp_redirects_block_before_network_dispatch() -> Result<()> {
    for case in ["redirect", "roundtrip", "roundtrip-report-only"] {
        let servers = IntegrityServers::spawn().await?;
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser
            .fetch(&format!("{}/module-csp.html?script-{case}", servers.origin))
            .await?;
        let query = if case == "redirect" {
            "redirect"
        } else {
            "roundtrip"
        };
        let expression = format!(
            r#"(async () => {{
          const results = [];
          for (const suffix of ['js', 'wasm']) {{
            results.push(await import('/module-csp.' + suffix + '?{query}').then(() => 'loaded', e => e.name));
          }}
          if (moduleViolations.length < 2) await new Promise(resolve => {{
            const observe = () => {{ if (moduleViolations.length >= 2) {{
              document.removeEventListener('module-violation-observed', observe); resolve();
            }} }};
            document.addEventListener('module-violation-observed', observe);
          }});
          return JSON.stringify({{results, events:moduleViolations}});
        }})()"#
        );
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let result: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .unwrap_or_else(|| panic!("script CSP: {result}")),
        )?;
        let report_only = case.ends_with("report-only");
        let status = if report_only { "loaded" } else { "TypeError" };
        assert_eq!(
            result["results"],
            serde_json::json!([status, status]),
            "{case}: {result}"
        );
        assert_eq!(
            result["events"].as_array().unwrap().len(),
            2,
            "{case}: {result}"
        );
        for event in result["events"].as_array().unwrap() {
            assert_eq!(event[0], "script-src-elem");
            assert_eq!(event[1], if report_only { "report" } else { "enforce" });
        }
        let requests = servers.requests.lock();
        for suffix in ["js", "wasm"] {
            let prefix = format!("/module-csp.{suffix}?");
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.path.starts_with(&prefix))
                    .count(),
                if report_only { 3 } else { 1 },
                "{case}: {requests:?}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn modulepreload_csp_redirects_block_before_dispatch_and_report_intermediate_hops()
-> Result<()> {
    for case in ["redirect", "roundtrip", "roundtrip-report-only"] {
        let servers = IntegrityServers::spawn().await?;
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser
            .fetch(&format!(
                "{}/module-csp.html?preload-{case}",
                servers.origin
            ))
            .await?;
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(
                r#"(async () => {
          const preloads = await preloadsReady;
          const imports = [];
          for (const [as, type] of [['text','text'], ['json','json'], ['style','css']]) {
            const link = document.querySelector('link[as="' + as + '"]');
            imports.push(await import(link.href, {with:{type}}).then(() => 'loaded', e => e.name));
          }
          if (moduleViolations.length < 3) await new Promise(resolve => {
            const observe = () => { if (moduleViolations.length >= 3) {
              document.removeEventListener('module-violation-observed', observe); resolve();
            } };
            document.addEventListener('module-violation-observed', observe);
          });
          return JSON.stringify({preloads, imports, events:moduleViolations});
        })()"#,
                true,
            ),
        )
        .await??;
        let result: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .unwrap_or_else(|| panic!("preload CSP: {result}")),
        )?;
        let report_only = case.ends_with("report-only");
        assert_eq!(
            result["preloads"],
            serde_json::json!({
                "text": if report_only { "load" } else { "error" },
                "json": if report_only { "load" } else { "error" },
                "style": if report_only { "load" } else { "error" },
            }),
            "{case}: {result}"
        );
        let status = if report_only { "loaded" } else { "TypeError" };
        assert_eq!(
            result["imports"],
            serde_json::json!([status, status, status]),
            "{case}: {result}"
        );
        assert_eq!(
            result["events"].as_array().unwrap().len(),
            3,
            "{case}: {result}"
        );
        let requests = servers.requests.lock();
        for suffix in ["text", "json", "css"] {
            let prefix = format!("/module-csp.{suffix}");
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.path.starts_with(&prefix))
                    .count(),
                if report_only { 3 } else { 1 },
                "{case}: {requests:?}"
            );
        }
    }
    Ok(())
}
