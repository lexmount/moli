use super::*;

fn check_large_array_buffer_probe(reply: RendererPageReply) {
    let value = renderer_json_value(reply).expect("large ArrayBuffer probe result");
    let result: serde_json::Value =
        serde_json::from_str(value.as_str().expect("probe JSON")).expect("valid probe JSON");
    assert_eq!(
        result,
        serde_json::json!({
            "length": 2_u64.pow(31) + 17,
            "zeroed": true,
            "first": 63,
            "lastWord": 0x1234_5678,
            "sourceDetached": true,
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn large_array_buffer_window_allocation_preserves_bytes_and_transfer() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://large-array-buffer.test/").unwrap(),
        "<!doctype html><body></body>",
    )
    .await;
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "{}\nJSON.stringify(largeArrayBufferProbe())",
                include_str!("large_array_buffer.js")
            ),
            await_promise: false,
        })
        .await
        .expect("large Window ArrayBuffer should not terminate the process");
    page.close_async().await.expect("close probe page");
    check_large_array_buffer_probe(reply);
}

#[tokio::test(flavor = "multi_thread")]
async fn large_array_buffer_worker_allocation_preserves_bytes_and_transfer() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://large-array-buffer-worker.test/").unwrap(),
        "<!doctype html><body></body>",
    )
    .await;
    let worker = serde_json::to_string(&format!(
        "{}\npostMessage(largeArrayBufferProbe());",
        include_str!("large_array_buffer.js")
    ))
    .unwrap();
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"new Promise((resolve, reject) => {{
                    const url = URL.createObjectURL(new Blob([{worker}], {{type: 'text/javascript'}}));
                    const worker = new Worker(url);
                    worker.onmessage = event => {{
                        worker.terminate();
                        URL.revokeObjectURL(url);
                        resolve(JSON.stringify(event.data));
                    }};
                    worker.onerror = event => {{
                        worker.terminate();
                        URL.revokeObjectURL(url);
                        reject(new Error(event.message));
                    }};
                }})"#
            ),
            await_promise: true,
        })
        .await
        .expect("large Worker ArrayBuffer should not terminate the process");
    page.close_async().await.expect("close worker probe page");
    check_large_array_buffer_probe(reply);
}
