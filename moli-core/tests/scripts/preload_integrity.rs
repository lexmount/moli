use super::*;
use anyhow::Context;
use moli_core::page::Page;

const PROBE: &str = include_str!("../fixtures/preload-integrity.js");
const DESTINATIONS: [&str; 5] = ["script", "image", "font", "track", "fetch"];

fn preload_cases(origin: &str, cross: &str) -> serde_json::Value {
    let mut variants = network_cases(origin, cross).as_array().unwrap().clone();
    variants.extend([
        serde_json::json!({"name":"stronger-mismatch", "src":format!("{origin}/script.js"),
            "integrity":format!("{INTEGRITY} sha512-AAAA"), "expected":"error"}),
        serde_json::json!({"name":"weaker-mismatch", "src":format!("{origin}/script.js"),
            "integrity":format!("sha256-AAAA {INTEGRITY}"), "expected":"load"}),
        serde_json::json!({"name":"unknown-options", "src":format!("{origin}/script.js"),
            "integrity":format!("{INTEGRITY}?unknown=ignored"), "expected":"load"}),
    ]);
    let mut cases = Vec::new();
    for destination in DESTINATIONS {
        for variant in &variants {
            let mut case = variant.clone();
            let name = format!("{destination}/{}", case["name"].as_str().unwrap());
            let mut url = url::Url::parse(case["src"].as_str().unwrap()).unwrap();
            url.query_pairs_mut().append_pair("preload", &name);
            case["name"] = name.into();
            case["src"] = url.to_string().into();
            case["destination"] = destination.into();
            if matches!(
                variant["name"].as_str(),
                Some("same-origin" | "mismatch" | "stronger-mismatch")
            ) {
                case["expectedEncodedBodySize"] = SCRIPT.len().into();
            }
            cases.push(case);
        }
    }
    // Bytes 80 FF 00 FE cannot round-trip through a lossy UTF-8 string.
    for (name, integrity, expected) in [
        (
            "binary-match",
            "sha384-h6nJeIIKOHARZ5S2ZBO6T/lbOP/rJ7klGG3EO1mDu4cAiaxouqg58upbuCyCJ0mf",
            "load",
        ),
        ("binary-mismatch", "sha384-AAAA", "error"),
    ] {
        cases.push(serde_json::json!({"name":name, "destination":"fetch",
            "src":"data:application/octet-stream;base64,gP8A/g==",
            "integrity":integrity, "expected":expected}));
    }
    cases.into()
}

pub(super) fn parser_markup(origin: &str, cross: &str) -> String {
    let cases = preload_cases(origin, cross);
    let mut markup = format!(
        "<!doctype html><head><script>globalThis.preloadSriEvents=[];globalThis.preloadSriCases={cases};</script>"
    );
    for (index, case) in cases.as_array().unwrap().iter().enumerate() {
        markup.push_str(&format!(
            "<link data-preload-sri rel=preload as=\"{}\" href=\"{}\" onload=\"preloadSriEvents[{index}]=event.type\" onerror=\"preloadSriEvents[{index}]=event.type\"",
            case["destination"].as_str().unwrap(),
            case["src"].as_str().unwrap().replace('&', "&amp;")
        ));
        for attribute in ["integrity", "crossOrigin"] {
            if let Some(value) = case[attribute].as_str() {
                markup.push_str(&format!(" {attribute}=\"{value}\""));
            }
        }
        markup.push('>');
    }
    markup.push_str("</head><body>preload integrity");
    markup
}

async fn assert_preloads(page: &mut Page, expression: &str, count: usize) -> Result<()> {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            &format!("({expression}).then(JSON.stringify)"),
            true,
        ),
    )
    .await??;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("preload results")?)?;
    assert_eq!(result["observations"].as_array().unwrap().len(), count);
    assert_eq!(result["failures"], serde_json::json!([]));
    assert_eq!(result["executions"], 0, "preloads never execute scripts");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn preload_integrity_checks_bytes_and_cors_in_top_and_child_documents() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let cases = preload_cases(&servers.origin, &servers.cross_origin);
    let count = cases.as_array().unwrap().len();
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    assert_preloads(&mut page, &format!("{PROBE}({cases})"), count).await?;
    page = browser
        .fetch(&format!("{}/preload-parser.html", servers.origin))
        .await?;
    assert_preloads(&mut page, &format!("{PROBE}(preloadSriCases)"), count).await?;
    assert_preloads(
        &mut page,
        &format!(
            r#"(async () => {{
                const frame = document.createElement('iframe');
                frame.src = '/preload-parser.html?child';
                await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                const result = await frame.contentWindow.eval({probe});
                frame.remove();
                return result;
            }})()"#,
            probe = serde_json::to_string(&format!("{PROBE}(preloadSriCases)"))?
        ),
        count,
    ).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn preload_integrity_checks_service_worker_response_filter() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let mut cases = Vec::new();
    for destination in DESTINATIONS {
        for source in ["basic", "cors", "default", "opaque"] {
            for (name, integrity) in [
                ("match", INTEGRITY),
                ("mismatch", "sha384-AAAA"),
                ("empty", ""),
                ("unsupported", "unknown-AAAA"),
            ] {
                cases.push(serde_json::json!({
                    "name":format!("{destination}/{source}/{name}"), "destination":destination,
                    "src":format!("/sw-{source}.js?{destination}-{name}"), "integrity":integrity,
                    "expected":if name == "mismatch" || (name == "match" && source == "opaque") {"error"} else {"load"}
                }));
            }
        }
    }
    let count = cases.len();
    let cases = serde_json::to_string(&cases)?;
    assert_preloads(&mut page, &format!(r#"(async () => {{
        const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
            new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {{once:true}}));
        await navigator.serviceWorker.register('/worker.js', {{scope:'/'}});
        await navigator.serviceWorker.ready;
        await controlled;
        return {PROBE}({cases});
    }})()"#), count).await?;
    Ok(())
}
