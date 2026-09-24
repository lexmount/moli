use super::*;

fn install_child_dynamic_import_target(
    page_vm: &mut PageVm,
    element_id: &str,
) -> anyhow::Result<ChildDocumentModuleFetchTarget> {
    page_vm.vm_mut().eval(&format!(
        "const frame = document.createElement('iframe'); \
         frame.id = {element_id:?}; \
         document.body.appendChild(frame);"
    ))?;
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(element_id)
        .expect("dynamic-import fixture should install a child handle");
    materialize_child_realm_through_page_turn_for_test(page_vm, element_id)?;
    Ok(page_vm
        .vm()
        .current_child_document_module_fetch_target(child_handle)
        .expect("materialized child realm should expose an exact module target"))
}

#[tokio::test(flavor = "current_thread")]
async fn dynamic_import_terminal_rejects_replaced_realm_without_network_effect() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let retired_target =
            install_child_dynamic_import_target(&mut page_vm, "dynamic-import-retired-realm")?;
        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(retired_target.child_handle());
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "dynamic-import-retired-realm",
        )?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(retired_target.child_handle())
            .expect("replacement realm should expose its exact target");
        assert_eq!(retired_target.task_owner(), current_target.task_owner());
        assert_ne!(retired_target.realm_id(), current_target.realm_id());

        let root_document = page_vm.document_lifecycle.identity().document;
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_dynamic_import_fetch(
            root_document,
            test_child_dynamic_import_completion_for_target(
                retired_target,
                211,
                "retired-dynamic-import-realm",
            ),
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired-realm terminal should consume one bounded Page turn");
        assert_eq!(
            outcome.action.owner,
            RendererPageResourceCompletionOwner::child_module_fetch(root_document, retired_target,)
        );
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageResourceCompletionOwner::child_module_fetch(
                    root_document,
                    current_target,
                )),
            }
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "historical dynamic-import Network output must not become replacement-realm activity"
        );
        assert!(
            !page_vm
                .page_task_executor_sources_for_test()
                .dynamic_import_owner_action()
                .has_ready_task(),
            "a stale terminal must not enqueue an owner action on the stable typed source"
        );

        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "business completion must not manufacture Network output"
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Document stale dynamic-import realm test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn dynamic_import_source_consumes_one_terminal_per_turn_in_fifo_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let current_target =
            install_child_dynamic_import_target(&mut page_vm, "dynamic-import-fifo")?;
        let stale_target = ChildDocumentModuleFetchTarget::new(
            current_target.child_handle(),
            current_target.task_owner(),
            FrameRealmId(current_target.realm_id().0 + 1),
        );
        let root_document = page_vm.document_lifecycle.identity().document;
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_dynamic_import_fetch(
            root_document,
            test_child_dynamic_import_completion_for_target(
                stale_target,
                223,
                "dynamic-import-fifo-first",
            ),
        ));
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_dynamic_import_fetch(
            root_document,
            test_child_dynamic_import_completion_for_target(
                stale_target,
                227,
                "dynamic-import-fifo-second",
            ),
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("first dynamic-import terminal should consume one turn");
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );

        assert!(queue.has_ready_completion());

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("second dynamic-import terminal should consume its own turn");
        assert_eq!(
            second.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );

        assert!(!queue.has_ready_completion());
        assert!(page_vm.vm_mut().take_network_output().is_empty());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "two stale terminals must not advance current Document activity"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dynamic-import one-terminal-per-turn test should run");
}
