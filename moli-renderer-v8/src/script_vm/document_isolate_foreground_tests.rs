use std::{pin::pin, time::Duration};

use crate::{
    page_task_queue::{RendererOwnerWakeSender, RendererPageV8ForegroundTaskSource},
    runtime::{PageId, RendererPageToken},
    script_vm::RendererDocumentIsolateHandle,
};

fn page_source(id: u64) -> RendererPageV8ForegroundTaskSource {
    let (wake, _receiver) = tokio::sync::mpsc::unbounded_channel();
    RendererPageV8ForegroundTaskSource::new(RendererOwnerWakeSender::new(
        wake,
        RendererPageToken::new_for_testing(PageId::new_for_testing(id)),
    ))
}

fn evaluate(
    isolate: &RendererDocumentIsolateHandle,
    context: &v8::Global<v8::Context>,
    source: &str,
) -> String {
    isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let source = v8::String::new(scope, source).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            Ok(
                crate::script_execution::execute_compiled_script(scope, script)
                    .unwrap()
                    .to_rust_string_lossy(scope),
            )
        })
        .unwrap()
}

enum PageSourceRetirement {
    Clear,
    Drop,
    CancelSelectedTask,
}

async fn run_wasm_across_page_retirement(retirement: PageSourceRetirement) {
    crate::ensure_v8_for_test();
    let mut first = page_source(1);
    let mut second = page_source(2);
    let bootstrap =
        RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
            first.sender(),
        )
        .unwrap();
    let isolate = bootstrap.clone_renderer_document_isolate_handle_for_owner_retention();
    drop(bootstrap);
    let membership = first.sender().isolate_membership().unwrap();
    membership.admit_related_page(second.sender()).unwrap();
    assert!(membership.admit_related_page(second.sender()).is_err());
    let context = isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            Ok(v8::Global::new(scope, context))
        })
        .unwrap();
    evaluate(
        &isolate,
        &context,
        "globalThis.completed = []; WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])).then(() => completed.push('before')); 'started'",
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !first.has_ready_task() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("V8 must post a concrete foreground task before retiring its destination");
    let accepted_order = first.next_ready_metadata().unwrap();
    // The scheduler can select a task before closing its Page, then cancel
    // the owner-local future before the selected task is executed.
    let cancel_selected = matches!(retirement, PageSourceRetirement::CancelSelectedTask);
    let selected_task = cancel_selected.then(|| first.pop_front().unwrap().1);
    if matches!(retirement, PageSourceRetirement::Clear) {
        first.clear();
        assert!(!first.has_ready_task());
        assert!(first.sender().isolate_membership().is_err());
    }
    drop(first);
    drop(selected_task);
    assert!(
        second.has_ready_task(),
        "accepted work must reach the surviving Page"
    );
    if !cancel_selected {
        assert_eq!(
            second.next_ready_metadata(),
            Some(accepted_order),
            "transferring a queued task must preserve its original arbitration order"
        );
    }
    let third = page_source(3);
    assert!(
        membership.admit_related_page(third.sender()).is_err(),
        "a retired source cannot admit another Page"
    );
    complete_wasm(&isolate, &context, &mut second, "1").await;
    evaluate(
        &isolate,
        &context,
        "WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0,0,2,1,120])).then(() => completed.push('after')); 'started'",
    );
    complete_wasm(&isolate, &context, &mut second, "2").await;
    assert_eq!(
        evaluate(&isolate, &context, "JSON.stringify(completed)"),
        r#"["before","after"]"#
    );
    second.clear();
    assert!(!second.has_ready_task());
}

async fn complete_wasm(
    isolate: &RendererDocumentIsolateHandle,
    context: &v8::Global<v8::Context>,
    source: &mut RendererPageV8ForegroundTaskSource,
    expected_count: &str,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some((_, task)) = source.pop_front() {
                isolate.with_entered_renderer_document_isolate(|_| {
                    assert!(task.into_task().run());
                    Ok(())
                }).unwrap();
                isolate.with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = v8::Local::new(scope, context);
                    let scope = &mut v8::ContextScope::new(scope, context);
                    crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                    Ok(())
                }).unwrap();
            }
            if evaluate(isolate, context, "completed.length") == expected_count {
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.expect("both old and newly posted V8 work must complete through the surviving Page");
}

#[tokio::test]
async fn clearing_page_source_transfers_accepted_and_future_wasm_work() {
    run_wasm_across_page_retirement(PageSourceRetirement::Clear).await;
}

#[tokio::test]
async fn dropping_page_source_transfers_accepted_and_future_wasm_work() {
    run_wasm_across_page_retirement(PageSourceRetirement::Drop).await;
}

#[tokio::test]
async fn cancelling_selected_task_after_page_close_preserves_wasm_completion() {
    run_wasm_across_page_retirement(PageSourceRetirement::CancelSelectedTask).await;
}

#[tokio::test]
async fn retired_isolate_tasks_cannot_run_in_a_rebound_page_source() {
    crate::ensure_v8_for_test();
    let mut source = page_source(1);
    let bootstrap =
        RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
            source.sender(),
        )
        .unwrap();
    let old_isolate = bootstrap.clone_renderer_document_isolate_handle_for_owner_retention();
    drop(bootstrap);
    let old_context = old_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            Ok(v8::Global::new(scope, context))
        })
        .unwrap();
    evaluate(
        &old_isolate,
        &old_context,
        "WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])); 'started'",
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !source.has_ready_task() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
            source.sender(),
        )
        .is_err(),
        "a source must not enter two live isolates"
    );
    old_isolate.unregister_renderer_document_isolate_platform();
    let retired_tasks = std::iter::from_fn(|| source.pop_front()).collect::<Vec<_>>();
    let bootstrap =
        RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
            source.sender(),
        )
        .unwrap();
    let current = bootstrap.clone_renderer_document_isolate_handle_for_owner_retention();
    drop(bootstrap);
    let mut discarded = 0;
    for (_, task) in retired_tasks {
        current
            .with_entered_renderer_document_isolate(|_| {
                assert!(!task.into_task().run());
                discarded += 1;
                Ok(())
            })
            .unwrap();
    }
    assert!(discarded > 0, "the test must exercise an accepted old task");
    let context = current
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            Ok(v8::Global::new(scope, context))
        })
        .unwrap();
    evaluate(
        &current,
        &context,
        "globalThis.completed = []; WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])).then(() => completed.push('current')); 'started'",
    );
    complete_wasm(&current, &context, &mut source, "1").await;
    source.clear();
}

async fn assert_cpu_profile_survives_isolate_retirement(
    retire: impl FnOnce(RendererDocumentIsolateHandle),
) {
    crate::ensure_v8_for_test();
    let trace = moli_v8_platform::start_v8_cpu_trace(
        moli_v8_platform::V8CpuTraceConfiguration::bounded_for_trace_buffer(4 * 1024 * 1024),
    )
    .expect("CPU tracing should start before the page is created");
    let source = page_source(1);
    let bootstrap =
        RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
            source.sender(),
        )
        .unwrap();
    let isolate = bootstrap.clone_renderer_document_isolate_handle_for_owner_retention();
    drop(bootstrap);
    let context = isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            Ok(v8::Global::new(scope, context))
        })
        .unwrap();
    evaluate(
        &isolate,
        &context,
        "function closedPageTraceHotFunction() { let value = 1; for (let i = 0; i < 20000000; i++) value = Math.imul(value + 3, 1103515245) | 0; return value; } closedPageTraceHotFunction();",
    );
    drop(context);
    // Page operations leave the isolate exited. Retirement must enter it to
    // flush the profiler even when Tracing.end has not been requested yet.
    retire(isolate);
    let result = tokio::time::timeout(Duration::from_secs(5), trace.stop().wait())
        .await
        .expect("CPU tracing should finish after the page retires");
    assert!(
        !result.data_loss_occurred(),
        "page retirement lost its profile"
    );
    assert_eq!(result.profiles().len(), 1);
    let profile = &result.profiles()[0];
    assert!(profile.sample_count() > 0);
    let profile: serde_json::Value = serde_json::from_slice(profile.profile_json()).unwrap();
    assert!(
        profile["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| { node["callFrame"]["functionName"] == "closedPageTraceHotFunction" })
    );
}

#[tokio::test]
async fn unregistering_document_isolate_preserves_cpu_trace_samples() {
    assert_cpu_profile_survives_isolate_retirement(|isolate| {
        isolate.unregister_renderer_document_isolate_platform();
    })
    .await;
}

#[tokio::test]
async fn dropping_document_isolate_preserves_cpu_trace_samples() {
    assert_cpu_profile_survives_isolate_retirement(drop).await;
}
