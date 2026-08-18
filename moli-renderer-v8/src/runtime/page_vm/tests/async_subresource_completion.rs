use super::*;

use crate::{
    page_resource_completion::{
        PageResourceCompletionDocumentEffect, PageResourceCompletionOutputEffect,
        RendererPageResourceCompletion, RendererPageResourceCompletionOwner,
    },
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceFetchEvent,
        AsyncSubresourceFetchEventTarget, AsyncSubresourceStreamingChunk,
        AsyncSubresourceStreamingFinished, AsyncSubresourceStreamingStarted,
    },
};

fn register_intercepted_fetch(page_vm: &mut PageVm, request_url: &Url) -> u64 {
    page_vm
        .vm_mut()
        .set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    page_vm
        .vm_mut()
        .eval(&format!(
            r#"
globalThis.__typedSubresourceResult = "pending";
fetch({:?}).then(
  () => {{ globalThis.__typedSubresourceResult = "fulfilled"; }},
  () => {{ globalThis.__typedSubresourceResult = "rejected"; }}
);
"queued"
"#,
            request_url.as_str()
        ))
        .expect("intercepted Fetch should register");

    let pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1, "test should create one intercepted Fetch");
    pending[0].internal_id
}

fn failed_fetch_completion(internal_id: u64, request_url: &Url) -> AsyncSubresourceFetchEvent {
    AsyncSubresourceFetchEvent::Completion(Box::new(AsyncSubresourceFetchCompletion {
        internal_id,
        request_url: request_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: Some("typed test failure".to_owned()),
        result: Err("typed test failure".to_owned()),
    }))
}

fn queue_checkpoint_marker(page_vm: &mut PageVm, marker: &str) -> anyhow::Result<()> {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!(
            r#"
globalThis.{marker} = 0;
Promise.resolve().then(() => globalThis.{marker} += 1);
"reaction queued"
"#,
        ))?;
    Ok(())
}

fn checkpoint_marker(page_vm: &mut PageVm, marker: &str) -> anyhow::Result<String> {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!("String(globalThis.{marker})"))
}

fn run_completion(
    page_vm: &mut PageVm,
    root_document: crate::runtime::RendererDocumentToken,
    event: AsyncSubresourceFetchEvent,
) -> PageResourceCompletionTurnOutcome {
    let mut source = RendererPageNetworkingSource::new_for_test();
    source.enqueue_local_for_test(RendererPageResourceCompletion::async_subresource(
        root_document,
        event,
    ));
    page_vm
        .apply_one_page_resource_terminal_owner_admission_for_test(&mut source)
        .expect("typed async-subresource turn should apply")
        .expect("typed async-subresource terminal should consume one bounded turn")
}

#[tokio::test(flavor = "current_thread")]
async fn async_subresource_completion_uses_exact_typed_networking_owner() {
    run_page_vm_async_test(async move {
        let request_url = Url::parse("https://typed-subresource.test/current").unwrap();
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let internal_id = register_intercepted_fetch(&mut page_vm, &request_url);
        let target = AsyncSubresourceFetchEventTarget::Completion { internal_id };
        let owner = RendererPageResourceCompletionOwner::async_subresource(root_document, target);

        assert!(
            page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(target),
            "the intercepted request must be resident before its terminal"
        );
        let outcome = run_completion(
            &mut page_vm,
            root_document,
            failed_fetch_completion(internal_id, &request_url),
        );

        assert_eq!(outcome.action.owner, owner);
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert_eq!(
            outcome.action.source,
            RendererOwnerResourceActivitySource::AsyncSubresource
        );
        assert!(
            !page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(target),
            "one terminal must retire the exact request phase"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn selected_current_async_subresource_terminal_submits_checkpoint_without_runtime_drain() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let request_url = Url::parse("https://typed-subresource.test/selected").unwrap();
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let internal_id = register_intercepted_fetch(&mut page_vm, &request_url);
        queue_checkpoint_marker(&mut page_vm, "__resourceCompletionCheckpoint")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        page_vm
            .page_resource_completion_queue()
            .enqueue_local_for_test(RendererPageResourceCompletion::async_subresource(
                root_document,
                failed_fetch_completion(internal_id, &request_url),
            ));
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?,
            "one exact resource terminal must enter the production selected dispatcher",
        );

        assert_eq!(
            checkpoint_marker(&mut page_vm, "__resourceCompletionCheckpoint")?,
            "1",
            "a current Window-realm resource terminal owns one ordinary task-end checkpoint",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__typedSubresourceResult)",
                )?,
            "rejected",
            "the terminal's Promise reaction must settle at its own task boundary",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "resource completion must not execute unrelated runtime residence",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected async-subresource terminal witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_streaming_chunk_settles_reader_reaction_at_its_own_task_end() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let request_url = Url::parse("https://example.com/typed-stream-checkpoint").unwrap();
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm
            .vm_mut()
            .set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
        page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__typedStreamState = "pending";
fetch({:?})
  .then(response => {{
    globalThis.__typedStreamState = "reading";
    return response.body.getReader().read();
  }})
  .then(result => {{
    globalThis.__typedStreamState = new TextDecoder().decode(result.value);
  }});
"queued"
"#,
            request_url.as_str()
        ))?;
        let pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
        assert_eq!(pending.len(), 1);
        let internal_id = pending[0].internal_id;
        let body_source_id = 91_101;

        page_vm
            .page_resource_completion_queue()
            .enqueue_local_for_test(RendererPageResourceCompletion::async_subresource(
                root_document,
                AsyncSubresourceFetchEvent::StreamingStarted(Box::new(
                    AsyncSubresourceStreamingStarted {
                        internal_id,
                        request_url: request_url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        body_source_id,
                        network_request_headers: None,
                        head: moli_fetch::ResponseHead {
                            final_url: request_url,
                            status: 200,
                            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                            request_cookie_report: None,
                            cookie_set_reports: Vec::new(),
                            redirected: false,
                            redirect_chain: Vec::new(),
                            from_cache: false,
                            negotiated_http_version: None,
                        },
                    },
                )),
            ));
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?,
            "stream start must enter the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__typedStreamState)",
                )?,
            "reading",
            "the stream-start task must resolve Fetch and install its reader at task end"
        );

        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        page_vm
            .page_resource_completion_queue()
            .enqueue_local_for_test(RendererPageResourceCompletion::async_subresource(
                root_document,
                AsyncSubresourceFetchEvent::StreamingChunk(AsyncSubresourceStreamingChunk {
                    body_source_id,
                    bytes: b"chunk-arrived".to_vec(),
                }),
            ));
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?,
            "stream chunk must enter the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__typedStreamState)",
                )?,
            "chunk-arrived",
            "the read Promise reaction must run at the selected chunk task boundary"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "stream delivery must not drain unrelated runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected streaming-chunk checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_root_with_reused_async_subresource_id_cannot_consume_current_request() {
    run_page_vm_async_test(async move {
        let request_url = Url::parse("https://typed-subresource.test/reused-id").unwrap();
        let mut page_vm = test_page_vm();
        let current_root = page_vm.document_lifecycle.identity().document;
        let stale_root = current_root.successor_for_testing();
        let internal_id = register_intercepted_fetch(&mut page_vm, &request_url);
        let target = AsyncSubresourceFetchEventTarget::Completion { internal_id };
        let stale_owner =
            RendererPageResourceCompletionOwner::async_subresource(stale_root, target);
        let current_owner =
            RendererPageResourceCompletionOwner::async_subresource(current_root, target);

        let stale = run_completion(
            &mut page_vm,
            stale_root,
            failed_fetch_completion(internal_id, &request_url),
        );
        assert_eq!(stale.action.owner, stale_owner);
        assert_eq!(
            stale.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(current_owner),
            }
        );
        assert_eq!(
            stale.action.output_effect,
            PageResourceCompletionOutputEffect::None,
            "an unconsumed stale request terminal has no historical output fact"
        );
        assert!(
            page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(target),
            "a stale root must not consume the colliding current local request id"
        );

        let current = run_completion(
            &mut page_vm,
            current_root,
            failed_fetch_completion(internal_id, &request_url),
        );
        assert_eq!(
            current.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert!(
            !page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(target)
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_preserves_a_window_fetch_and_its_typed_terminal_owner() {
    run_page_vm_async_test(async move {
        let request_url = Url::parse("https://typed-subresource.test/document-open").unwrap();
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let document_owner_before = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner");
        let internal_id = register_intercepted_fetch(&mut page_vm, &request_url);
        let target = AsyncSubresourceFetchEventTarget::Completion { internal_id };

        page_vm
            .vm_mut()
            .eval(
                r#"
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
"replaced"
"#,
            )
            .expect("document.open should replace the Document");
        let document_owner_after = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner");
        assert_ne!(document_owner_before, document_owner_after);
        assert_eq!(
            page_vm.document_lifecycle.identity().document,
            root_document,
            "document.open rotates the local Document owner without replacing the PageVm root"
        );
        assert!(
            page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(target),
            "a Window-owned Fetch must survive same-Window document.open"
        );

        let outcome = run_completion(
            &mut page_vm,
            root_document,
            failed_fetch_completion(internal_id, &request_url),
        );
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert!(
            !page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(target)
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_finish_requires_matching_request_and_body_source_identity() {
    run_page_vm_async_test(async move {
        // Keep the response same-origin so this test reaches the streaming
        // residence instead of correctly terminating at the CORS gate.
        let request_url = Url::parse("https://example.com/typed-subresource-stream").unwrap();
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let internal_id = register_intercepted_fetch(&mut page_vm, &request_url);
        let body_source_id = 91_001;

        let started = AsyncSubresourceFetchEvent::StreamingStarted(Box::new(
            AsyncSubresourceStreamingStarted {
                internal_id,
                request_url: request_url.clone(),
                request_method: "GET".to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                body_source_id,
                network_request_headers: None,
                head: moli_fetch::ResponseHead {
                    final_url: request_url,
                    status: 200,
                    headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
            },
        ));
        assert_eq!(
            run_completion(&mut page_vm, root_document, started)
                .action
                .document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );

        let correct_target = AsyncSubresourceFetchEventTarget::StreamingFinish {
            internal_id,
            body_source_id,
        };
        assert!(
            page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(correct_target)
        );

        let wrong_body_source_id = body_source_id + 1;
        let wrong_target = AsyncSubresourceFetchEventTarget::StreamingFinish {
            internal_id,
            body_source_id: wrong_body_source_id,
        };
        let stale = run_completion(
            &mut page_vm,
            root_document,
            AsyncSubresourceFetchEvent::StreamingFinished(AsyncSubresourceStreamingFinished {
                internal_id,
                body_source_id: wrong_body_source_id,
                result: Ok(()),
            }),
        );
        assert_eq!(
            stale.action.owner,
            RendererPageResourceCompletionOwner::async_subresource(root_document, wrong_target)
        );
        assert_eq!(
            stale.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: None
            }
        );
        assert_eq!(
            stale.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );
        assert!(
            page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(correct_target),
            "a mismatched body id must leave the real stream resident"
        );

        let current = run_completion(
            &mut page_vm,
            root_document,
            AsyncSubresourceFetchEvent::StreamingFinished(AsyncSubresourceStreamingFinished {
                internal_id,
                body_source_id,
                result: Ok(()),
            }),
        );
        assert_eq!(
            current.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert!(
            !page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(correct_target)
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn stale_observed_network_record_is_captured_without_consuming_current_request() {
    run_page_vm_async_test(async move {
        let request_url = Url::parse("https://typed-subresource.test/historical").unwrap();
        let document_url = Url::parse("https://typed-subresource.test/retired-document").unwrap();
        let mut page_vm = test_page_vm();
        let current_root = page_vm.document_lifecycle.identity().document;
        let stale_root = current_root.successor_for_testing();
        let internal_id = register_intercepted_fetch(&mut page_vm, &request_url);
        let request_target = AsyncSubresourceFetchEventTarget::Completion { internal_id };

        let record = SubresourceNetworkRecord::failure(
            Some("retired-frame".to_owned()),
            document_url,
            request_url.clone(),
            "OPTIONS".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            "retired preflight failed".to_owned(),
        );
        let outcome = run_completion(
            &mut page_vm,
            stale_root,
            AsyncSubresourceFetchEvent::ObservedNetworkRecord(Box::new(record)),
        );
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageResourceCompletionOwner::async_subresource(
                    current_root,
                    AsyncSubresourceFetchEventTarget::ObservedNetworkRecord,
                )),
            }
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert!(
            page_vm
                .vm()
                .async_subresource_fetch_event_target_is_current(request_target),
            "a historical Network fact must not consume unrelated live request state"
        );

        let (records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].frame_id(), Some("retired-frame"));
        assert_eq!(records[0].url(), &request_url);
        assert_eq!(
            records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "retired preflight failed".to_owned(),
            }
        );
    })
    .await;
}
