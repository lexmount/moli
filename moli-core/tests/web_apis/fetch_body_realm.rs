use super::*;

async fn assert_binary_body_realms(scenario: &str, expected_cases: usize) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let expected = b"<!doctype html><html><body><main id=\"stream\">naive-\xe4\xbd\xa0\xe5\xa5\xbd</main><script>document.body.setAttribute('data-stream-script','seen');</script></body></html>";
    let source = format!(
        "{}\nrunFetchBodyRealmProbe({}, {}, {}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/fetch_body_realm.js"),
        serde_json::to_string(scenario)?,
        serde_json::to_string(&server.url("/streaming/chunked-html"))?,
        serde_json::to_string(expected.as_slice())?,
    );
    for target in ["window", "child"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            super::event_dispatch::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(
            observed,
            serde_json::json!({"errors": [], "checked": expected_cases}),
            "{scenario}/{target}",
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_binary_body_results_use_receiver_realm() -> Result<()> {
    assert_binary_body_realms("memory", 64).await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_binary_network_body_results_use_receiver_realm() -> Result<()> {
    assert_binary_body_realms("network", 24).await
}
