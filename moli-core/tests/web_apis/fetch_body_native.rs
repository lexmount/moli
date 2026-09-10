use super::*;

async fn assert_native_body_consumption(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunFetchBodyNativeProbe({}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/fetch_body_native.js"),
        serde_json::to_string(scenario)?,
    );
    for target in ["window", "child", "worker"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            super::event_dispatch::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(
            observed,
            serde_json::json!({"errors": []}),
            "{scenario}/{target}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_body_reads_hide_internal_results_from_thenable_assimilation() -> Result<()> {
    assert_native_body_consumption("then").await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_body_reads_validate_and_copy_chunks_without_author_properties() -> Result<()> {
    assert_native_body_consumption("chunks").await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_body_reads_preserve_errors_and_consumption_access_state() -> Result<()> {
    assert_native_body_consumption("errors").await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_body_materialization_uses_native_intrinsics() -> Result<()> {
    assert_native_body_consumption("intrinsics").await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_network_body_rewrapping_avoids_reentrant_source_borrows() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let expected = "<!doctype html><html><body><main id=\"stream\">naive-你好</main><script>document.body.setAttribute('data-stream-script','seen');</script></body></html>";
    for scenario in ["response", "request"] {
        let source = format!(
            "{}\nrunNetworkBodyProbe({}, {}).then(finish, error => finish({{error: String(error)}}));",
            include_str!("../fixtures/runtime/fetch_body_network.js"),
            serde_json::to_string(scenario)?,
            serde_json::to_string(&server.url("/streaming/chunked-html"))?,
        );
        for target in ["window", "child", "worker"] {
            let observed = tokio::time::timeout(
                Duration::from_secs(20),
                super::event_dispatch::run_probe(&browser, &server, target, &source),
            )
            .await??;
            assert_eq!(
                observed,
                serde_json::json!({"text": expected}),
                "{scenario}/{target}"
            );
        }
    }
    server.shutdown().await;
    Ok(())
}
