use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn font_face_binary_sources_validate_payloads_and_buffer_conversion_order() -> Result<()> {
    let fonts = serde_json::json!({
        "ttf": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.ttf").as_slice(),
        "woff": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff").as_slice(),
        "woff2": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff2").as_slice(),
    });
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let probe = include_str!("../fixtures/font-binary.js");
    let expression = format!("({probe})({fonts}).then(JSON.stringify)");
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("binary FontFace results")?,
    )?;
    assert_eq!(value["failures"], serde_json::json!([]), "{value}");
    assert_eq!(value["rows"].as_array().unwrap().len(), 52, "{value}");
    server.shutdown().await;
    Ok(())
}
