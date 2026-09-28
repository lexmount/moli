use super::*;

pub(super) fn fixture_response(
    path: &str,
    query: &str,
    referer: Option<&str>,
) -> (&'static str, String, String) {
    let policy = url::form_urlencoded::parse(query.as_bytes())
        .find(|(name, _)| name == "policy")
        .map(|(_, value)| value.into_owned());
    let content_type = if path == "/local-referrer-page.html" {
        "text/html"
    } else {
        "text/javascript"
    };
    let mut headers = format!("Content-Type: {content_type}\r\nCache-Control: no-store\r\n");
    if let Some(policy) = policy {
        headers.push_str(&format!("Referrer-Policy: {policy}\r\n"));
    }
    if path == "/local-referrer-page.html" {
        return (
            "200 OK",
            headers,
            "<!doctype html><body>Worker creator".to_owned(),
        );
    }
    (
        "200 OK",
        headers,
        format!(
            "globalThis.workerEntryReferrer = {};\n{}",
            serde_json::to_string(&referer).unwrap(),
            include_str!("local_worker_referrer_worker.js"),
        ),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn local_workers_inherit_the_correct_environment_referrer_policy() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let expression = format!(
        "globalThis.localWorkerCrossOrigin = {}; {}",
        serde_json::to_string(&servers.cross_origin)?,
        include_str!("local_worker_referrer.js"),
    );
    let result = tokio::time::timeout(
        Duration::from_secs(40),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    assert_eq!(result["value"], true, "{result}");
    Ok(())
}
