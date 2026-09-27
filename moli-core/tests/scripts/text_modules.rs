use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn text_modules_decode_and_cache_in_window_and_worker_realms() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let probe = include_str!("../fixtures/text-modules.js");
    let expression = format!(
        r#"(async () => {{
          const probe = {probe};
          const results = [await probe()];
          for (const type of ['classic', 'module']) {{
            const source = '(' + probe.toString() + ')().then(postMessage, e => postMessage({{error:e.name}}));';
            const url = URL.createObjectURL(new Blob([source], {{type:'text/javascript'}}));
            const worker = new Worker(url, {{type}});
            try {{
              results.push(await new Promise((resolve, reject) => {{
                worker.onmessage = event => resolve(event.data);
                worker.onerror = event => reject(new Error(event.message));
              }}));
            }} finally {{ worker.terminate(); URL.revokeObjectURL(url); }}
          }}
          return JSON.stringify(results);
        }})()"#,
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let observed: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("text module probe results"))?;
    for realm in observed.as_array().expect("three probe realms") {
        assert_eq!(realm["failures"], serde_json::json!([]), "{realm}");
        assert!(realm["checks"].as_u64().unwrap() >= 50, "{realm}");
    }
    assert_eq!(observed.as_array().unwrap().len(), 3);
    server.shutdown().await;
    Ok(())
}
