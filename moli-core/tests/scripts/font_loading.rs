use super::preload_consumption::PreloadServer;
use super::*;
use anyhow::Context;

#[tokio::test(flavor = "multi_thread")]
async fn font_faces_fetch_validate_fallback_and_wait_for_font_sets() -> Result<()> {
    run_font_probe(include_str!("../fixtures/font-loading.js"), 14).await
}

#[tokio::test(flavor = "multi_thread")]
async fn font_faces_consume_matching_pending_and_completed_preloads() -> Result<()> {
    run_font_probe(include_str!("../fixtures/font-preload.js"), 12).await
}

#[tokio::test(flavor = "multi_thread")]
async fn font_faces_enforce_document_csp_before_initial_and_redirect_requests() -> Result<()> {
    run_font_probe(include_str!("../fixtures/font-csp.js"), 4).await
}

async fn run_font_probe(probe: &str, count: usize) -> Result<()> {
    let server = PreloadServer::spawn().await?;
    let cross = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.origin).await?;
    let expression = format!(
        "({probe})({}).then(JSON.stringify, error => JSON.stringify({{error: String(error.stack || error)}}))",
        serde_json::to_string(&cross.origin)?
    );
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("FontFace loading results")?,
    )?;
    assert_eq!(value["failures"], serde_json::json!([]), "{value}");
    assert_eq!(value["rows"].as_array().unwrap().len(), count, "{value}");
    Ok(())
}
