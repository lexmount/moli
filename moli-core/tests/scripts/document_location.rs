use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn document_location_requires_a_fully_active_receiver_before_forwarding() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&server.url("/compat/child-dynamic-markup-document?markup=entry"))
        .await?;
    let script = include_str!("../fixtures/document-location.js");
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
            .expect("Document.location observations"),
    )?;
    assert_eq!(observed["failures"], serde_json::json!([]), "{observed}");
    assert_eq!(observed["scenarios"].as_array().unwrap().len(), 12);
    server.shutdown().await;
    Ok(())
}
