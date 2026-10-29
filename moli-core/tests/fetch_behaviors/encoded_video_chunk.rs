use super::*;

#[tokio::test]
async fn encoded_video_chunks_roundtrip_message_ports_and_reject_indexed_db_storage() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let expression = format!(
        "document.body.innerHTML='<iframe id=child></iframe>'; void ({});",
        include_str!("../fixtures/encoded-video-chunk-storage.js")
    );
    page.evaluate_runtime_expression_with_await_async(&expression, false)
        .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let checks = loop {
        let result=page.evaluate_runtime_expression_with_await_async("JSON.stringify(globalThis.__uiEventResults || {stage: globalThis.__encodedVideoChunkStage})",false).await?;
        let checks: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .expect("chunk storage observations"),
        )?;
        if checks["complete"] == true {
            break checks;
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "chunk storage tasks did not finish: {checks}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(checks["total"], 10, "{checks}");
    assert_eq!(checks["passed"], checks["total"], "{checks}");
    server.shutdown().await;
    Ok(())
}
