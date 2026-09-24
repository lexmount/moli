use super::*;

use crate::page_task_queue::RendererOwnerWakeSource;

async fn wait_for_networking_wake(
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let wake = wake_rx
                .recv()
                .await
                .expect("main parser continuation route must remain attached");
            if wake.source_for_test() == RendererOwnerWakeSource::NetworkingTask {
                return;
            }
        }
    })
    .await
    .expect("main parser continuation should wake the Networking source");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_continuation_admits_only_the_current_active_parser() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/main-parser-continuation-current").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main Document owner");
        page_vm
            .vm_mut()
            .document_runtime
            .activate_main_parser_continuation(owner);
        let producer = page_vm
            .vm()
            .document_runtime
            .main_parser_continuation_producer()
            .expect("active continuation producer");

        assert_eq!(
            producer.request().expect("continuation request"),
            crate::page_task_queue::MainParserContinuationRequest::Enqueued
        );
        wait_for_networking_wake(&mut wake_rx).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainParserContinuation
                )
                .await?,
            "one exact parser-continuation task must enter the production selected dispatcher"
        );
        assert!(
            page_vm
                .vm_mut()
                .document_runtime
                .take_main_parser_continuation_admission(),
            "selected current continuation should grant one phase-one admission"
        );
        assert!(
            !page_vm
                .vm_mut()
                .document_runtime
                .take_main_parser_continuation_admission(),
            "the admission must be consumed exactly once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("current main parser continuation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn continuation_captured_before_document_open_cannot_admit_replacement_parser() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/main-parser-continuation-stale").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner");
        page_vm
            .vm_mut()
            .document_runtime
            .activate_main_parser_continuation(retired_owner);
        let retired_producer = page_vm
            .vm()
            .document_runtime
            .main_parser_continuation_producer()
            .expect("retired continuation producer");
        retired_producer
            .request()
            .expect("retired continuation request");
        wait_for_networking_wake(&mut wake_rx).await;

        page_vm.vm_mut().eval(
            r#"
document.open();
document.write("<!doctype html><main>replacement</main>");
document.close();
"replaced"
"#,
        )?;
        let replacement_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner");
        assert_ne!(retired_owner, replacement_owner);

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleParserContinuationBoundary = [];
Promise.resolve().then(() => {
  __staleParserContinuationBoundary.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainParserContinuation
                )
                .await?,
            "the retired parser continuation must remain a concrete selected task"
        );
        assert!(
            !page_vm
                .vm_mut()
                .document_runtime
                .take_main_parser_continuation_admission(),
            "stale continuation must not grant access to the replacement parser"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__staleParserContinuationBoundary.join('|')",
                )?,
            "",
            "discarding a stale parser continuation must not checkpoint the replacement realm"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "discarding a stale parser continuation must not advance replacement runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale main parser continuation test should run");
}
