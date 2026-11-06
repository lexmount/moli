use super::*;
use crate::script_vm::main_world::MainWorldBootstrapFailureForTest;
use crate::{RendererPageCommand, RendererPageReply};

async fn assert_failed_main_world_recovers(failure: MainWorldBootstrapFailureForTest) {
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

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.title+':'+document.readyState+':'+document.body.textContent"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("retry must activate the same live Document");
    let RendererPageReply::RuntimeEvaluationResult(result) = reply else {
        panic!("evaluation result")
    };
    assert_eq!(
        result.into_protocol_payload()["value"],
        "Live Document:complete:Native content"
    );
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
}

#[tokio::test(flavor = "multi_thread")]
async fn lazy_main_world_bootstrap_failure_keeps_proxy_unpublished() {
    assert_failed_main_world_recovers(MainWorldBootstrapFailureForTest::Bindings).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lazy_main_world_baseline_failure_keeps_inspector_unregistered() {
    assert_failed_main_world_recovers(MainWorldBootstrapFailureForTest::Baseline).await;
}
