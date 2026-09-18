use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn retained_child_windows_remain_readable_after_browsing_context_destruction() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&server.url("/compat/child-dynamic-markup-document?markup=entry"))
        .await?;
    let script = include_str!("../fixtures/retained-child-window.js");
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
            .expect("retained child Window observations"),
    )?;
    assert_eq!(observed["checks"], 111);
    assert_eq!(observed["failures"], serde_json::json!([]), "{observed}");
    server.shutdown().await;
    Ok(())
}
