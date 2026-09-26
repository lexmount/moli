use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn preload_as_admission_matches_parser_and_dynamic_links() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    for context in ["top-dynamic", "top-parser"] {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(
                &format!(
                    "({}).then(JSON.stringify)",
                    include_str!("../fixtures/preload-as.js")
                ),
                true,
            ),
        )
        .await??;
        let observed: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .expect("preload destination observations"),
        )?;
        assert_eq!(observed["context"], context, "{observed}");
        assert_eq!(observed["observations"].as_array().unwrap().len(), 36);
        assert_eq!(observed["failures"], serde_json::json!([]), "{observed}");
        assert_eq!(observed["recovered"], true, "{observed}");
        if context == "top-dynamic" {
            let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
            url.query_pairs_mut()
                .append_pair("markup", observed["parserMarkup"].as_str().unwrap());
            page = browser.fetch(url.as_str()).await?;
        }
    }
    server.shutdown().await;
    Ok(())
}
