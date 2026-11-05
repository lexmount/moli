use super::*;

pub(super) fn fixture_response(
    path: &str,
    query: &str,
    origin: &str,
    cross: &str,
) -> (&'static str, String, String) {
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
