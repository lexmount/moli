use super::*;

async fn check_fetch_client_referrer(opaque: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!(
            "{}/document-referrer.html?response=unsafe-url",
            servers.origin,
        ))
        .await?;
    let script = format!(
        "globalThis.__clientReferrerOrigins = {}; globalThis.__opaqueReferrerProbe = {opaque}; {}",
        serde_json::to_string(&[&servers.origin, &servers.cross_origin])?,
        include_str!("fetch_client_referrer.js"),
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&script, true),
    )
    .await??;
    assert_eq!(result["value"], true, "opaque={opaque}: {result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_client_referrer_uses_live_document_urls_and_srcdoc_ancestors() -> Result<()> {
    check_fetch_client_referrer(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_client_referrer_does_not_reveal_opaque_srcdoc_ancestors() -> Result<()> {
    check_fetch_client_referrer(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_memory_cache_separates_resolved_referrer_values() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"(async () => {
                const selected = location.origin + '/selected';
                for (const [init, expected] of [
                    [{}, location.href],
                    [{referrerPolicy:'origin'}, location.origin + '/'],
                    [{referrerPolicy:'no-referrer'}, null],
                    [{referrer:selected,referrerPolicy:'unsafe-url'}, selected],
                ]) {
                    for (let repeat = 0; repeat < 2; repeat++) {
                        const actual = await (await fetch('/cached-client-referrer', init)).json();
                        if (actual !== expected)
                            throw new Error(JSON.stringify(init) + ': ' + actual + ' != ' + expected);
                    }
                }
                return true;
            })()"#,
            true,
        )
        .await?;
    assert_eq!(result["value"], true, "{result}");
    assert_eq!(
        servers
            .requests
            .lock()
            .iter()
            .filter(|request| request.path == "/cached-client-referrer")
            .count(),
        4,
        "identical resolved referrers should reuse their cached response"
    );
    Ok(())
}
