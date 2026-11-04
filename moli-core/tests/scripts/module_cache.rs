use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn same_url_script_and_text_modules_keep_destination_specific_cached_responses() -> Result<()>
{
    let servers = IntegrityServers::spawn().await?;
    let mut config = AppConfig::default();
    // Otherwise different default Accept headers could hide a missing
    // destination in the shared source-cache key.
    config
        .fetch_mut()
        .set_default_request_headers(vec![("Accept".to_owned(), "*/*".to_owned())]);
    let browser = Browser::new(config)?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let observed = tokio::time::timeout(Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(r#"(async () => {
          const results = [];
          for (const order of [['script', 'text'], ['text', 'script']]) {
            const url = '/module-destination?' + order.join('-');
            const load = type => type === 'script' ? import(url) : import(url, {with: {type: 'text'}});
            const first = await load(order[0]);
            const second = await load(order[1]);
            results.push([first.default, second.default, first === await load(order[0]),
              second === await load(order[1]), first !== second]);
          }
          return JSON.stringify([results, globalThis.destinationExecutions]);
        })()"#, true)).await??;
    let observed: serde_json::Value = serde_json::from_str(
        observed["value"]
            .as_str()
            .unwrap_or_else(|| panic!("module destination results: {observed}")),
    )?;
    assert_eq!(
        observed,
        serde_json::json!([
            [
                ["script value", "text module body", true, true, true],
                ["text module body", "script value", true, true, true]
            ],
            2,
        ])
    );
    let requests = servers.requests.lock();
    for query in ["script-text", "text-script"] {
        let path = format!("/module-destination?{query}");
        let matching: Vec<_> = requests
            .iter()
            .filter(|request| request.path == path)
            .collect();
        assert_eq!(matching.len(), 2, "{requests:?}");
        for destination in ["script", "text"] {
            assert_eq!(
                matching
                    .iter()
                    .filter(|request| request.sec_fetch_dest.as_deref() == Some(destination))
                    .count(),
                1,
                "{matching:?}"
            );
        }
    }
    Ok(())
}
