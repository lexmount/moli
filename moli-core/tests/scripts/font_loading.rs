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

#[tokio::test(flavor = "multi_thread")]
async fn font_face_sets_validate_load_and_check_queries_with_css_shorthand_syntax() -> Result<()> {
    run_font_probe(include_str!("../fixtures/font-query.js"), 24).await
}

async fn run_font_probe(probe: &str, count: usize) -> Result<()> {
    run_font_probe_with_argument(probe, count, None).await
}

#[tokio::test(flavor = "multi_thread")]
async fn font_faces_validate_binary_data_and_buffer_source_inputs() -> Result<()> {
    let fonts = serde_json::json!({
        "ttf": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.ttf").as_slice(),
        "woff": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff").as_slice(),
        "woff2": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff2").as_slice(),
    });
    run_font_probe_with_argument(include_str!("../fixtures/font-binary.js"), 58, Some(fonts)).await
}

async fn run_font_probe_with_argument(
    probe: &str,
    count: usize,
    argument: Option<serde_json::Value>,
) -> Result<()> {
    let server = PreloadServer::spawn().await?;
    let cross = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.origin).await?;
    let expression = format!(
        "({probe})({}).then(JSON.stringify, error => JSON.stringify({{error: String(error.stack || error)}}))",
        serde_json::to_string(&argument.unwrap_or_else(|| serde_json::json!(cross.origin)))?
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
