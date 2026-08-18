use super::*;

use crate::{
    page_task_queue::PageModuleReactionTargetEffect,
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
};

fn enqueue_missing_document_module_reaction(page_vm: &mut PageVm, reaction_id: u64) {
    page_vm
        .vm_mut()
        .queue_missing_document_module_reaction_for_test(reaction_id);
}

#[tokio::test(flavor = "current_thread")]
async fn missing_current_module_reaction_body_maps_to_no_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://module-reaction.test/missing-current").expect("document URL");
        let (mut page_vm, _resource_source, _wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        enqueue_missing_document_module_reaction(&mut page_vm, 101);
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__missingModuleReactionBoundary = [];
Promise.resolve().then(() => {
  __missingModuleReactionBoundary.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let task = page_vm
            .take_module_reaction_body_task_for_test()
            .expect("one exact ModuleReaction ticket should be ready");
        let outcome = page_vm.apply_selected_page_module_reaction_turn(task)?;
        let action = outcome.action;
        assert_eq!(
            action.target_effect(),
            PageModuleReactionTargetEffect::DiscardedMissingReaction
        );
        let completion = action.into_page_task_completion();
        assert!(matches!(&completion, PageTaskCompletion::NoCompletion));
        // This test intentionally stops at the body/result boundary. A
        // NoCompletion result has no selected-task work to execute.
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__missingModuleReactionBoundary.join('|')",
                )?,
            "",
            "a spent one-shot ticket must not manufacture a V8 checkpoint"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "discarding a missing reaction must not advance unrelated runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("missing ModuleReaction completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_rejects_a_module_reaction_from_the_previous_exact_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://module-reaction.test/document-open").expect("document URL");
        let (mut page_vm, _resource_source, _wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let root_document = page_vm.document_lifecycle.identity().document;
        let old_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("old main Document owner");

        enqueue_missing_document_module_reaction(&mut page_vm, 151);
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;

        assert_eq!(page_vm.document_lifecycle.identity().document, root_document);
        assert_ne!(
            page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("replacement main Document owner"),
            old_document_owner
        );
        let task = page_vm
            .take_module_reaction_body_task_for_test()
            .expect("the previous Document reaction must remain queued for stale retirement");
        let action = page_vm.apply_selected_page_module_reaction_turn(task)?.action;
        assert_eq!(
            action.target_effect(),
            PageModuleReactionTargetEffect::IgnoredStaleOwner
        );
        assert!(matches!(
            action.into_page_task_completion(),
            PageTaskCompletion::NoCompletion
        ));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("document.open module-reaction identity test should run");
}

#[test]
fn old_root_module_reaction_is_discarded_by_the_selected_dispatcher() {
    run_page_vm_large_stack_async_test(
        "module-reaction-old-root-no-replacement-checkpoint",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url =
                Url::parse(&format!("{base_url}/initial.html")).expect("document URL");
            let (page_vm, _resource_source, _wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let retired_root = page_vm.document_lifecycle.identity().document;
                    enqueue_missing_document_module_reaction(&mut page_vm, 202);

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
                    let mut pending_document_lifecycle_turn = None;
                    let navigation = page_vm
                        .follow_pending_location_navigation_one_turn_async(
                            &mut pending_document_lifecycle_turn,
                            PageVmInitStage::Load,
                        )
                        .await?;
                    assert!(matches!(
                        navigation,
                        crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                            | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                                ..
                            }
                    ));
                    assert_ne!(
                        page_vm.document_lifecycle.identity().document,
                        retired_root,
                        "test setup must install a replacement root Document"
                    );

                    page_vm
                        .vm_mut()
                        .eval_without_microtask_checkpoint_for_test(
                            r#"
globalThis.__staleModuleReactionBoundary = [];
Promise.resolve().then(() => {
  __staleModuleReactionBoundary.push("microtask");
});
"queued"
"#,
                    )?;
                    page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ModuleReaction,
                        )
                        .expect("the old-root task must remain in the stable source");
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "__staleModuleReactionBoundary.join('|')",
                            )?,
                        "",
                        "an old-root reaction must not checkpoint replacement V8"
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .runtime_script_work()
                            .dynamic_scripts.pending_source_load_count_for_test(),
                        1,
                        "an old-root discard must not advance replacement runtime work"
                    );
                    assert!(
                        page_vm
                            .claim_exact_selected_page_task_for_test(
                                PageSelectedTaskTestSelector::ModuleReaction,
                            )
                            .is_none(),
                        "the stale exact task must retire once"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("old-root ModuleReaction should be discarded exactly");
            server.await.expect("replacement server should finish");
        },
    );
}
