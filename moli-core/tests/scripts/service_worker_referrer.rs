use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_native_requests_preserve_resolved_referrer_and_policy() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            include_str!("service_worker_referrer.js"),
            true,
        ),
    )
    .await??;
    assert_eq!(
        result["value"],
        true,
        "{result}; network requests: {:?}",
        servers
            .requests
            .lock()
            .iter()
            .map(|request| &request.path)
            .collect::<Vec<_>>()
    );
    assert!(
        !servers
            .requests
            .lock()
            .iter()
            .any(|request| request.path.starts_with("/native-referrer/")),
        "all probes must be handled by the controlling service worker"
    );
    Ok(())
}
