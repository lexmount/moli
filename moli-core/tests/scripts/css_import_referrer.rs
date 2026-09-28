use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn css_import_requests_use_the_parent_stylesheet_referrer() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        page.evaluate_runtime_expression_with_await_async(
            include_str!("css_import_referrer.js"),
            true,
        ),
    )
    .await??;
    assert_eq!(result["value"], true, "{result}");
    assert!(
        !servers
            .requests
            .lock()
            .iter()
            .any(|request| request.path.starts_with("/css-referrer/")),
        "import requests must pass through the controlling service worker"
    );
    Ok(())
}
