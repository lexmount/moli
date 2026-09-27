use super::*;

const CONSUMER_PROBE: &str = include_str!("../fixtures/preload-consumers.js");

#[tokio::test(flavor = "multi_thread")]
async fn preload_consumers_reuse_service_worker_results_and_response_filters() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let mut cases = vec![
        serde_json::json!({"name":"basic-good", "integrity":INTEGRITY, "preload":"load", "consumer":"load"}),
        serde_json::json!({"name":"basic-bad", "integrity":"sha384-AAAA", "preload":"error", "consumer":"error"}),
        serde_json::json!({"name":"opaque-integrity", "integrity":INTEGRITY, "preload":"error", "consumer":"error"}),
        serde_json::json!({"name":"opaque-empty", "integrity":"", "preload":"load", "consumer":"load"}),
        serde_json::json!({"name":"custom-status", "destination":"fetch", "integrity":INTEGRITY,
            "preload":"load", "consumer":"load", "statusText":"Preloaded reply 1"}),
    ];
    for case in &mut cases {
        if case.get("destination").is_none() {
            case["destination"] = "script".into();
        }
        case["src"] = format!(
            "{}/sw-preload-consumer.js?consumer={}",
            servers.origin,
            case["name"].as_str().unwrap()
        )
        .into();
    }
    let expression = format!(
        r#"(async () => {{
        const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
            new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {{once:true}}));
        await navigator.serviceWorker.register('/worker.js', {{scope:'/'}});
        await navigator.serviceWorker.ready;
        await controlled;
        return {CONSUMER_PROBE}({});
    }})()"#,
        serde_json::to_string(&cases)?
    );
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let actual: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("SW observations")?)?;
    assert_eq!(actual.as_array().unwrap().len(), cases.len());
    for (expected, actual) in cases.iter().zip(actual.as_array().unwrap()) {
        assert_eq!(actual["preload"], expected["preload"], "{actual}");
        assert_eq!(actual["consumer"], expected["consumer"], "{actual}");
        assert_eq!(
            actual["executions"],
            usize::from(expected["destination"] == "script" && expected["consumer"] == "load"),
            "{actual}"
        );
        if expected.get("statusText").is_some() {
            assert_eq!(actual["statusText"], expected["statusText"], "{actual}");
            continue;
        }
        let path = format!("/script.js?consumer={}", expected["name"].as_str().unwrap());
        assert_eq!(
            servers
                .requests
                .lock()
                .iter()
                .filter(|request| request.path == path)
                .count(),
            1,
            "{actual}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn preload_consumers_share_success_and_failure_without_another_request() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let mut cases = vec![
        serde_json::json!({"name":"script-bad", "destination":"script", "integrity":"sha384-AAAA", "preload":"error", "consumer":"error", "requests":1}),
        serde_json::json!({"name":"script-good", "destination":"script", "integrity":INTEGRITY, "preload":"load", "consumer":"load", "requests":1}),
        serde_json::json!({"name":"script-pending-bad", "destination":"script", "integrity":"sha384-AAAA", "inFlight":true, "preload":"error", "consumer":"error", "requests":1}),
        serde_json::json!({"name":"script-pending-good", "destination":"script", "integrity":INTEGRITY, "inFlight":true, "preload":"load", "consumer":"load", "requests":1}),
        serde_json::json!({"name":"script-different-integrity", "destination":"script", "integrity":"sha384-AAAA", "consumerIntegrity":INTEGRITY, "preload":"error", "consumer":"load", "requests":2}),
        serde_json::json!({"name":"script-ignored-options", "destination":"script", "integrity":format!("{INTEGRITY}?ignored=yes"), "consumerIntegrity":INTEGRITY, "preload":"load", "consumer":"load", "requests":1}),
        serde_json::json!({"name":"script-different-mode", "destination":"script", "integrity":"sha384-AAAA", "consumerCrossOrigin":"anonymous", "preload":"error", "consumer":"load", "requests":2}),
        serde_json::json!({"name":"script-different-credentials", "destination":"script", "integrity":"sha384-AAAA", "crossOrigin":"anonymous", "consumerCrossOrigin":"use-credentials", "preload":"error", "consumer":"load", "requests":2}),
        serde_json::json!({"name":"fetch-bad", "destination":"fetch", "integrity":"sha384-AAAA", "preload":"error", "consumer":"error", "requests":1}),
        serde_json::json!({"name":"fetch-good", "destination":"fetch", "integrity":INTEGRITY, "preload":"load", "consumer":"load", "requests":1}),
    ];
    for (destination, integrity) in [
        (
            "image",
            "sha384-V/8sPn1el0M0lP2LSgMvCXV07vvO5PvRZnnI+1AQ8ZqCt3g97fwog7m7gXitggbU",
        ),
        (
            "track",
            "sha384-wC6e/GdYj1Y8agdNNjXalfDrHLdDzOJLInfVLaxYZlPVCLm6Nsu3tf89vs7ZwsgN",
        ),
    ] {
        for (suffix, integrity, event) in
            [("good", integrity, "load"), ("bad", "sha384-AAAA", "error")]
        {
            let mut case = serde_json::json!({"name":format!("{destination}-{suffix}"), "destination":destination, "integrity":integrity, "preload":event, "consumer":event, "requests":1});
            if destination == "track" {
                case["crossOrigin"] = "anonymous".into();
            }
            cases.push(case);
        }
    }
    for destination in ["script", "image", "fetch"] {
        for blob in [false, true] {
            cases.push(serde_json::json!({
                "name":format!("{destination}-local-{blob}"), "destination":destination,
                "integrity":"sha384-AAAA", "preload":"error", "consumer":"error", "requests":0,
                "blob":blob,
                "mime":if destination == "image" {"image/svg+xml"} else {"text/javascript"},
                "data":if destination == "image" {r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"></svg>"#} else {SCRIPT}
            }));
        }
    }
    for case in &mut cases {
        let path = match case["destination"].as_str().unwrap() {
            "image" => "/preload-image.svg",
            "track" => "/preload-track.vtt",
            _ => "/script.js",
        };
        case["src"] = format!(
            "{}{path}?consumer={}",
            servers.origin,
            case["name"].as_str().unwrap()
        )
        .into();
    }
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(
            &format!("{CONSUMER_PROBE}({})", serde_json::to_string(&cases)?),
            true,
        ),
    )
    .await??;
    let actual: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("consumer results")?)?;
    for (expected, actual) in cases
        .iter()
        .zip(actual.as_array().context("consumer observations")?)
    {
        assert_eq!(actual["name"], expected["name"]);
        assert_eq!(actual["preload"], expected["preload"], "{actual}");
        assert_eq!(actual["consumer"], expected["consumer"], "{actual}");
        let executions =
            usize::from(expected["destination"] == "script" && expected["consumer"] == "load");
        assert_eq!(actual["executions"], executions, "{actual}");
        let url = url::Url::parse(expected["src"].as_str().unwrap())?;
        let path = format!("{}?{}", url.path(), url.query().unwrap());
        let requests = servers
            .requests
            .lock()
            .iter()
            .filter(|request| request.path == path)
            .count();
        assert_eq!(
            serde_json::json!(requests),
            expected["requests"],
            "{actual}"
        );
        if expected.get("data").is_none()
            && matches!(expected["destination"].as_str(), Some("script" | "image"))
        {
            assert_eq!(actual["timings"], expected["requests"], "{actual}");
        }
    }
    assert_eq!(actual.as_array().unwrap().len(), cases.len());
    Ok(())
}
