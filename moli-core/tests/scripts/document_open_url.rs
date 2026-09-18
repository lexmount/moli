use super::*;

async fn check_document_open_urls(script: &str, checks: u64) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    url.query_pairs_mut()
        .append_pair("markup", "<!doctype html><body>entry");
    url.set_fragment(Some("entry-fragment"));
    let mut page = browser.fetch(url.as_str()).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        page.evaluate_runtime_expression_with_await_async(
            &format!("({script}).then(JSON.stringify)"),
            true,
        ),
    )
    .await??;
    let observed: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .expect("document.open URL observations"),
    )?;
    assert_eq!(observed["checks"], checks);
    assert_eq!(observed["failures"], serde_json::json!([]));
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_updates_active_document_urls_from_entry_document() -> Result<()> {
    check_document_open_urls(include_str!("../fixtures/document-open-url.js"), 73).await
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_updates_root_url_from_child_entry_document() -> Result<()> {
    check_document_open_urls(include_str!("../fixtures/document-open-root-url.js"), 6).await
}
