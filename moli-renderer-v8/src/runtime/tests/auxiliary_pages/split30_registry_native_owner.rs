use super::*;
#[tokio::test]
async fn borrowed_registry_methods_use_the_receivers_native_owner() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        url::Url::parse("https://registry-owner.test/").unwrap(),
        "<!doctype html><body></body>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: include_str!("../custom_element_registry_owner.js").to_owned(),
            await_promise: true,
        })
        .await
        .unwrap();
    let Some(serde_json::Value::String(result)) = renderer_json_value(reply) else {
        panic!("registry owner probe must return its observations");
    };
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    let rows = result["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 8, "all owner and receiver cases must execute");
    for row in rows {
        for (name, value) in row.as_object().unwrap() {
            if name != "case" {
                assert_eq!(value, true, "{}: {name}", row["case"]);
            }
        }
    }
    page.close_async().await.unwrap();
}
