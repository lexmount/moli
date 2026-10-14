use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

#[tokio::test(flavor = "multi_thread")]
async fn main_document_close_can_resolve_an_awaited_lifecycle_promise() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&markup_url(&server, "<!doctype html><body>initial"))
        .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        page.evaluate_runtime_expression_with_await_async(
            r#"new Promise(resolve => {
              document.open();
              document.addEventListener('DOMContentLoaded', () => resolve(JSON.stringify({
                readyState: document.readyState,
                text: document.getElementById('replacement').textContent
              })), {once: true});
              document.write('<!doctype html><body><p id=replacement>replacement</p>');
              document.close();
            })"#,
            true,
        ),
    )
    .await??;
    let observed: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .expect("replacement lifecycle observation"),
    )?;
    assert_eq!(
        observed,
        serde_json::json!({"readyState": "interactive", "text": "replacement"})
    );
    server.shutdown().await;
    Ok(())
}
