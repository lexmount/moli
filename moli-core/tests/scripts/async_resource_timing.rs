use super::*;

async fn check_parser_async_resource_timing(path: &str, script_count: usize) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let document_url = server.url(path);
    let mut page = browser.fetch(&document_url).await?;
    let result = page
        .evaluate_runtime_expression_by_value_async(
            r#"Array.from(document.scripts).filter(script => script.async && script.src).map(script => {
                const entries = performance.getEntriesByName(script.src, 'resource');
                return {name: script.src, entries: entries.map(entry => ({
                    initiatorType: entry.initiatorType, initiatorUrl: entry.initiatorUrl,
                    encodedBodySize: entry.encodedBodySize, responseEnd: entry.responseEnd
                }))};
            })"#,
        )
        .await?;
    let scripts = result["value"]
        .as_array()
        .expect("async script resource timings must serialize by value");
    assert_eq!(scripts.len(), script_count, "fixture={path}: {result}");
    for script in scripts {
        let entries = script["entries"].as_array().expect("resource entries");
        assert_eq!(
            entries.len(),
            1,
            "each fetched async script must publish exactly one timing: fixture={path}: {script}"
        );
        let entry = &entries[0];
        assert_eq!(entry["initiatorType"], "script", "{path}: {script}");
        assert_eq!(entry["initiatorUrl"], document_url, "{path}: {script}");
        assert!(
            entry["encodedBodySize"]
                .as_u64()
                .is_some_and(|size| size > 0),
            "the timing must retain the fetched response body size: {path}: {script}"
        );
        assert!(
            entry["responseEnd"].as_f64().is_some_and(|end| end > 0.0),
            "the timing must describe a completed response: {path}: {script}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_async_before_eof_keeps_the_network_timing() -> Result<()> {
    check_parser_async_resource_timing("/compat/parse-time-async-classic-chunked", 1).await
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_async_after_eof_keeps_the_network_timing() -> Result<()> {
    check_parser_async_resource_timing("/compat/parse-time-async-classic-slow", 1).await
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_async_from_one_chunk_has_one_network_timing() -> Result<()> {
    check_parser_async_resource_timing("/compat/parse-time-async-classic-pumped", 1).await
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_async_task_turns_keep_each_network_timing() -> Result<()> {
    check_parser_async_resource_timing("/compat/parse-time-async-classic-task-turns", 2).await
}
