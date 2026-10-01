use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoning_auxiliary_adoption_retires_only_the_reserved_page() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/creator").unwrap(),
        "<!doctype html><body>creator</body>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;
    rx.drain();
    let baseline = runtime.document_isolate_accounting_for_diagnostics();
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "window.p=open('about:blank','pending'); p.closed".to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!(false)));
    let publications = rx.drain();
    let activations = popup_activations_for_page(&publications, &page);
    assert_eq!(activations.len(), 1);
    let pending = activations[0].pending_auxiliary_page().unwrap();
    let retained = pending.clone();
    drop(activations);
    drop(publications);
    drop(pending);
    assert_eq!(
        runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved,
        baseline.reserved + 1
    );
    // A cloned protocol capability keeps the one initial Page alive. Only
    // the final drop cancels its owner-local reservation.
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "[p.closed,p.document.body.textContent,document.body.textContent].join('|')"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("false||creator"))
    );
    drop(retained);
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved
            != baseline.reserved
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("abandoned initial Page must release its isolate reservation");
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "p.closed && document.body.textContent === 'creator'".to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!(true)));
    assert_eq!(runtime.renderer_page_count_for_testing(), 1);
    page.close_async().await.unwrap();
}
