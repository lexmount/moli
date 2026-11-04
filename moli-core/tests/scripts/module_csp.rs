use super::*;

pub(super) fn fixture_response(
    path: &str,
    query: &str,
    cross: &str,
) -> (&'static str, String, String) {
    if path == "/module-csp.html" {
        let policy = match query {
            "connect-none" => "connect-src 'none'; style-src-elem 'self'",
            "default-none" => "default-src 'none'; style-src-elem 'self'",
            "style-none" => "connect-src 'self'; style-src-elem 'none'",
            "redirect" => "connect-src 'self'; style-src-elem 'self'",
            "redirect-report-only" => "connect-src *; style-src-elem *",
            _ => "connect-src 'self'; style-src-elem 'self'",
        };
        let report = if query == "report-only" {
            "Content-Security-Policy-Report-Only: connect-src 'none'; style-src-elem 'none'\r\n"
        } else if query == "redirect-report-only" {
            "Content-Security-Policy-Report-Only: connect-src 'self'; style-src-elem 'self'\r\n"
        } else {
            ""
        };
        return (
            "200 OK",
            format!("Content-Type: text/html\r\nContent-Security-Policy: script-src 'nonce-setup'; {policy}\r\n{report}"),
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
    }
    if query == "redirect" {
        return (
            "302 Found",
            format!("Location: {cross}{path}?final\r\n"),
            String::new(),
        );
    }
    let (mime, body) = match path {
        "/module-csp.text" => ("text/plain", "module text"),
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
            } else {
                "dynamic"
            },
            expected_violations = match case {
                "connect-none" | "default-none" => 4,
                "style-none" => 2,
                "report-only" => 6,
                "redirect" | "redirect-report-only" => 3,
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
        let redirected = case == "redirect";
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
                } else if case.starts_with("redirect") || case == "allowed" {
                    3
                } else {
                    2
                },
                "{case}/{extension}: {requests:?}"
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
