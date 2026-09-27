use super::*;

fn parser_cases(origin: &str, cross: &str) -> Vec<serde_json::Value> {
    let mut cases = Vec::new();
    for kind in ["preload", "script"] {
        for policy in ["default", "invalid", "unsafe-url"] {
            for (label, target) in [("same", origin), ("cross", cross)] {
                cases.push(serde_json::json!({
                    "kind":kind, "policy":policy,
                    "url":format!("{target}/preload-policy.js?parser-{kind}-{policy}-{label}"),
                }));
            }
        }
    }
    cases
}

pub(super) fn fixture_response(
    path: &str,
    query: &str,
    origin: &str,
    cross: &str,
) -> (&'static str, String, String) {
    if path.ends_with(".js") {
        return (
            "200 OK",
            "Content-Type: text/javascript\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\n".into(),
            "export const load = url => import(url);".into(),
        );
    }
    let mut body = "<!doctype html><head>".to_owned();
    if query.contains("delivery=meta") {
        body.push_str("<meta name=referrer content=origin>");
    }
    body.push_str("<script>globalThis.initialLoads=0;globalThis.initialErrors=0;</script>");
    for (index, case) in parser_cases(origin, cross).into_iter().enumerate() {
        if index == 6 && query.contains("delivery=late-meta") {
            body.push_str("<meta name=referrer content=origin>");
        }
        let policy = case["policy"].as_str().unwrap();
        let attr = if policy == "default" {
            String::new()
        } else {
            format!(" referrerpolicy='{policy}'")
        };
        let url = case["url"].as_str().unwrap();
        let events = "onload='initialLoads++' onerror='initialErrors++'";
        body.push_str(&if case["kind"] == "preload" {
            format!("<link rel=modulepreload href='{url}'{attr} {events}>")
        } else {
            format!("<script type=module src='{url}'{attr} {events}></script>")
        });
    }
    body.push_str("</head><body>Module fetch policy</body>");
    (
        "200 OK",
        "Content-Type: text/html\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\n"
            .into(),
        body,
    )
}

async fn check_modulepreload_document_policy(child: bool) -> Result<()> {
    for delivery in ["header", "meta", "late-meta"] {
        let servers = IntegrityServers::spawn().await?;
        let browser = Browser::new(AppConfig::default())?;
        let document_url = format!(
            "{}/preload-policy.html?delivery={delivery}&child={child}",
            servers.origin,
        );
        let mut page = browser
            .fetch(&if child {
                format!(
                    "{}/document-referrer.html?response=unsafe-url",
                    servers.origin
                )
            } else {
                document_url.clone()
            })
            .await?;
        let cases = parser_cases(&servers.origin, &servers.cross_origin);
        let probe = format!(
            r#"(async () => {{
                while (initialLoads + initialErrors < 12)
                    await new Promise(resolve => setTimeout(resolve, 1));
                if (initialErrors) throw new Error('parser fetch errors: ' + initialErrors);
                let meta = document.querySelector('meta[name=referrer]');
                if (!meta) {{ meta = document.createElement('meta'); meta.name = 'referrer'; document.head.append(meta); }}
                const cases = {cases};
                for (const policy of ['no-referrer', 'origin', 'unsafe-url']) {{
                    meta.content = policy;
                    for (const explicit of ['default', 'invalid', 'unsafe-url'])
                    for (const [label, origin] of [['same', location.origin], ['cross', {cross}]]) {{
                        const url = origin + '/preload-policy.js?runtime-' + policy + '-' + explicit + '-' + label;
                        const link = document.createElement('link'); link.rel = 'modulepreload'; link.href = url;
                        if (explicit !== 'default') link.referrerPolicy = explicit;
                        await new Promise((resolve, reject) => {{
                            link.onload = resolve; link.onerror = () => reject(new Error('preload failed: ' + url));
                            document.head.append(link);
                        }});
                        cases.push({{url, policy:explicit}});
                        link.remove();
                    }}
                }}
                // Preloading captures a policy for that request, not as an
                // explicit policy on the module's later dynamic imports.
                meta.content = 'origin';
                for (let i = 0; i < cases.length; i++) {{
                    const module = await import(cases[i].url);
                    await module.load(location.origin + '/echo-origin.js?preloaded-import-' + i);
                }}
                if (initialLoads !== 12 || initialErrors)
                    throw new Error('parser link/script terminals: ' + initialLoads + '/' + initialErrors);
                return true;
            }})()"#,
            cases = serde_json::to_string(&cases)?,
            cross = serde_json::to_string(&servers.cross_origin)?,
        );
        let expression = if child {
            format!(
                r#"(async () => {{
                    const frame = document.createElement('iframe'); frame.src = {url};
                    await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                    return frame.contentWindow.eval({probe});
                }})()"#,
                url = serde_json::to_string(&document_url)?,
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
        assert_eq!(
            result["value"], true,
            "child={child}, delivery={delivery}: {result}"
        );

        let mut all_cases = cases;
        for policy in ["no-referrer", "origin", "unsafe-url"] {
            for explicit in ["default", "invalid", "unsafe-url"] {
                for (label, origin) in [("same", &servers.origin), ("cross", &servers.cross_origin)]
                {
                    all_cases.push(serde_json::json!({
                        "url":format!("{origin}/preload-policy.js?runtime-{policy}-{explicit}-{label}"),
                        "policy":explicit, "documentPolicy":policy,
                    }));
                }
            }
        }
        let requests = servers.requests.lock();
        let mut mismatches = Vec::new();
        let mut check = |path: &str, expected: Option<String>| {
            let matches: Vec<_> = requests.iter().filter(|r| r.path == path).collect();
            assert_eq!(
                matches.len(),
                1,
                "child={child}, delivery={delivery}, {path}: {requests:?}"
            );
            if matches[0].referer != expected {
                mismatches.push(format!(
                    "{path}: {:?}, expected {expected:?}",
                    matches[0].referer
                ));
            }
        };
        for (index, case) in all_cases.iter().enumerate() {
            let source = case["url"].as_str().unwrap();
            let url = url::Url::parse(source)?;
            let explicit = case["policy"] == "unsafe-url";
            let policy = if explicit {
                "unsafe-url"
            } else {
                case["documentPolicy"].as_str().unwrap_or(
                    if delivery == "header"
                        || (delivery == "late-meta" && case["kind"] == "preload")
                    {
                        "no-referrer"
                    } else {
                        "origin"
                    },
                )
            };
            check(
                &format!("{}?{}", url.path(), url.query().unwrap()),
                match policy {
                    "no-referrer" => None,
                    "origin" => Some(format!("{}/", servers.origin)),
                    "unsafe-url" => Some(document_url.clone()),
                    _ => unreachable!(),
                },
            );
            check(
                &format!("/echo-origin.js?preloaded-import-{index}"),
                Some(if explicit {
                    source.to_owned()
                } else {
                    format!("{}/", url.origin().ascii_serialization())
                }),
            );
        }
        assert!(
            mismatches.is_empty(),
            "child={child}, delivery={delivery}: {mismatches:#?}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_modulepreload_document_policy_is_request_local() -> Result<()> {
    check_modulepreload_document_policy(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_modulepreload_document_policy_is_request_local() -> Result<()> {
    check_modulepreload_document_policy(true).await
}
