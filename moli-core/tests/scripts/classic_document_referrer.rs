use super::*;

fn parser_cases(origin: &str, cross: &str) -> Vec<serde_json::Value> {
    let mut cases = Vec::new();
    for mode in ["preload", "blocking", "async", "defer"] {
        for policy in ["default", "invalid", "unsafe-url"] {
            for (label, target) in [("same", origin), ("cross", cross)] {
                cases.push(serde_json::json!({
                    "mode":mode, "policy":policy,
                    "url":format!("{target}/classic-policy.js?parser-{mode}-{policy}-{label}"),
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
            "Content-Type: text/javascript\r\nCache-Control: no-store\r\n".into(),
            "globalThis.executions = (globalThis.executions || 0) + 1;".into(),
        );
    }
    let mut body = "<!doctype html><head>".to_owned();
    if query.contains("delivery=meta") {
        body.push_str("<meta name=referrer content=origin>");
    }
    let cold = query.contains("delivery=late-meta-cold");
    let initialize = "<script>globalThis.initialLoads=globalThis.initialLoads||0;globalThis.initialErrors=globalThis.initialErrors||0;</script>";
    if !cold {
        body.push_str(initialize);
    }
    for (index, case) in parser_cases(origin, cross).iter().enumerate() {
        if index == 6 && query.contains("delivery=late-meta") {
            body.push_str("<meta name=referrer content=origin>");
            if cold {
                // Preloads discovered before any script also wait for the
                // child realm. Its creation must not recapture the later meta.
                body.push_str(initialize);
            }
        }
        let policy = case["policy"].as_str().unwrap();
        let attr = if policy == "default" {
            String::new()
        } else {
            format!(" referrerpolicy='{policy}'")
        };
        let mode = case["mode"].as_str().unwrap();
        let url = case["url"].as_str().unwrap();
        let events = "onload='globalThis.initialLoads=(globalThis.initialLoads||0)+1' onerror='globalThis.initialErrors=(globalThis.initialErrors||0)+1'";
        body.push_str(&if mode == "preload" {
            format!("<link rel=preload as=script href='{url}'{attr} {events}>")
        } else {
            let mode = if mode == "blocking" { "" } else { mode };
            format!("<script {mode} src='{url}'{attr} {events}></script>")
        });
    }
    body.push_str("</head><body>Classic script fetch policy</body>");
    (
        "200 OK",
        "Content-Type: text/html\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\n"
            .into(),
        body,
    )
}

async fn check_classic_document_policy(child: bool) -> Result<()> {
    for delivery in ["header", "meta", "late-meta", "late-meta-cold"] {
        let servers = IntegrityServers::spawn().await?;
        let browser = Browser::new(AppConfig::default())?;
        let document_url = format!(
            "{}/classic-policy.html?delivery={delivery}&child={child}",
            servers.origin,
        );
        // The child's policy must be independent of its parent's unsafe-url.
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
        let mut cases = parser_cases(&servers.origin, &servers.cross_origin);
        for policy in ["no-referrer", "origin", "unsafe-url"] {
            for mode in ["preload", "async", "in-order"] {
                for explicit in ["default", "invalid", "unsafe-url"] {
                    for (label, origin) in
                        [("same", &servers.origin), ("cross", &servers.cross_origin)]
                    {
                        cases.push(serde_json::json!({
                            "url":format!("{origin}/classic-policy.js?runtime-{policy}-{mode}-{explicit}-{label}"),
                            "mode":mode, "policy":explicit, "documentPolicy":policy,
                        }));
                    }
                }
            }
        }
        let probe = format!(
            r#"(async () => {{
                while (initialLoads + initialErrors < 24)
                    await new Promise(resolve => setTimeout(resolve, 1));
                if (initialErrors || executions !== 18)
                    throw new Error('parser fetch results: ' + initialLoads + '/' + initialErrors + '/' + executions);
                let meta = document.querySelector('meta[name=referrer]');
                if (!meta) {{ meta = document.createElement('meta'); meta.name = 'referrer'; document.head.append(meta); }}
                for (const test of {cases}) {{
                    if (!test.documentPolicy) continue;
                    meta.content = test.documentPolicy;
                    const element = document.createElement(test.mode === 'preload' ? 'link' : 'script');
                    if (test.mode === 'preload') {{ element.rel = 'preload'; element.as = 'script'; element.href = test.url; }}
                    else {{ element.src = test.url; element.async = test.mode !== 'in-order'; }}
                    if (test.policy !== 'default') element.referrerPolicy = test.policy;
                    await new Promise((resolve, reject) => {{
                        element.onload = resolve; element.onerror = () => reject(new Error('fetch failed: ' + test.url));
                        document.head.append(element);
                    }});
                    element.remove();
                }}
                if (executions !== 54) throw new Error('execution count: ' + executions);
                return true;
            }})()"#,
            cases = serde_json::to_string(&cases)?,
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
        let requests = servers.requests.lock();
        let mut mismatches = Vec::new();
        for case in &cases {
            let url = url::Url::parse(case["url"].as_str().unwrap())?;
            let path = format!("{}?{}", url.path(), url.query().unwrap());
            let policy = if case["policy"] == "unsafe-url" {
                "unsafe-url"
            } else {
                case["documentPolicy"].as_str().unwrap_or(
                    if delivery == "header"
                        || (delivery.starts_with("late-meta") && case["mode"] == "preload")
                    {
                        "no-referrer"
                    } else {
                        "origin"
                    },
                )
            };
            let expected = match policy {
                "no-referrer" => None,
                "origin" => Some(format!("{}/", servers.origin)),
                "unsafe-url" => Some(document_url.clone()),
                _ => unreachable!(),
            };
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
        }
        assert!(
            mismatches.is_empty(),
            "child={child}, delivery={delivery}: {mismatches:#?}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_classic_document_referrer_covers_parser_runtime_and_preloads() -> Result<()> {
    check_classic_document_policy(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_classic_document_referrer_covers_parser_runtime_and_preloads() -> Result<()> {
    check_classic_document_policy(true).await
}
