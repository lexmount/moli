use super::*;
use crate::script_vm::main_world::MainWorldBootstrapFailureForTest;
use crate::{RendererPageCommand, RendererPageReply};

async fn assert_failed_main_world_recovers(
    failure: MainWorldBootstrapFailureForTest,
    inspect_before_retry: bool,
) {
    let runtime = crate::JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = Url::parse("https://lazy-main-world-recovery.test/").unwrap();
    let (mut page, _, _, _, _) = runtime
        .create_html_page_from_response(
            url.clone(),
            url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><title>Live Document</title><main>Native content</main>".to_owned(),
            crate::RendererDocumentOptions::default(),
        )
        .await
        .unwrap();
    failure.arm(page.page_id());
    let result = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.title".to_owned(),
            await_promise: false,
        })
        .await;
    let error = result.err().expect("one bootstrap phase must fail");
    assert!(
        error
            .to_string()
            .contains("forced main-world bootstrap failure"),
        "{error:#}"
    );
    if inspect_before_retry {
        let (usage, _) = page
            .run_async_command(RendererPageCommand::RuntimeHeapUsage)
            .await
            .unwrap();
        let RendererPageReply::RuntimeHeapUsage(usage) = usage else {
            panic!("heap usage")
        };
        let usage = usage.to_diagnostics_json();
        assert_eq!(
            usage["moli"]["runtime"]["inspectorContextRegistrationCount"], 0,
            "failed initialization must not publish an inspector Context: {failure:?}: {usage}"
        );
    }

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
                contents: document.title + ':' + document.readyState + ':' + document.body.textContent,
                currentDefaultView: document.defaultView === window,
                currentDocumentPrototype: document instanceof Document,
                currentHtmlDocumentPrototype: document instanceof HTMLDocument,
                currentBodyPrototype: document.body instanceof HTMLElement,
                sameOwnerDocument: document.body.ownerDocument === document
            })"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("retry must activate the same live Document");
    let RendererPageReply::RuntimeEvaluationResult(result) = reply else {
        panic!("evaluation result")
    };
    let payload = result.into_protocol_payload();
    let actual: serde_json::Value = serde_json::from_str(
        payload["value"]
            .as_str()
            .expect("retry returned a JSON object with Document identity checks"),
    )
    .unwrap();

    let (usage, _) = page
        .run_async_command(RendererPageCommand::RuntimeHeapUsage)
        .await
        .unwrap();
    let RendererPageReply::RuntimeHeapUsage(usage) = usage else {
        panic!("heap usage")
    };
    assert_eq!(
        usage.to_diagnostics_json()["moli"]["runtime"]["inspectorContextRegistrationCount"],
        1
    );
    page.close_async().await.unwrap();
    assert_eq!(
        actual,
        serde_json::json!({
            "contents": "Live Document:complete:Native content",
            "currentDefaultView": true,
            "currentDocumentPrototype": true,
            "currentHtmlDocumentPrototype": true,
            "currentBodyPrototype": true,
            "sameOwnerDocument": true
        }),
        "{failure:?}, inspected before retry: {inspect_before_retry}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lazy_main_world_bootstrap_failure_keeps_proxy_unpublished() {
    assert_failed_main_world_recovers(MainWorldBootstrapFailureForTest::Bindings, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bindings_failure_direct_retry() {
    assert_failed_main_world_recovers(MainWorldBootstrapFailureForTest::Bindings, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lazy_main_world_baseline_failure_keeps_inspector_unregistered() {
    assert_failed_main_world_recovers(MainWorldBootstrapFailureForTest::Baseline, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn baseline_failure_direct_retry() {
    assert_failed_main_world_recovers(MainWorldBootstrapFailureForTest::Baseline, false).await;
}
