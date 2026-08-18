use moli_module_script_tree as module_tree;

use super::*;

fn install_dynamic_import_action_target(
    page_vm: &mut PageVm,
    element_id: &str,
) -> anyhow::Result<ChildDocumentModuleFetchTarget> {
    page_vm.vm_mut().eval(&format!(
        "(() => {{ \
             const frame = document.createElement('iframe'); \
             frame.id = {element_id:?}; \
             document.body.appendChild(frame); \
         }})()"
    ))?;
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(element_id)
        .expect("dynamic-import owner-action fixture should install a child handle");
    materialize_child_realm_through_page_turn_for_test(page_vm, element_id)?;
    Ok(page_vm
        .vm()
        .current_child_document_module_fetch_target(child_handle)
        .expect("materialized child realm should expose an exact target"))
}

fn prepared_terminal_action(
    target: ChildDocumentModuleFetchTarget,
    sequence: u64,
) -> crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction {
    let key = ModuleMapKey::java_script(
        Url::parse(&format!("https://dynamic-owner-turn.test/{sequence}.mjs"))
            .expect("dynamic-import owner-action test URL"),
    );
    let client = crate::module_runtime::NativeDynamicImportSingleModuleClient::new(
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(sequence),
            sequence,
        },
        module_tree::ModuleImportPhase::Evaluation,
    );
    crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction::from_terminal_work(
        crate::frame_owner_model::FrameDocumentDynamicImportTerminalWork::from_terminal_parts(
            target.task_owner(),
            target.realm_id(),
            key,
            client,
        ),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn current_dynamic_import_owner_action_is_applied_by_one_typed_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let target = install_dynamic_import_action_target(&mut page_vm, "dynamic-action-current")?;
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
            .enqueue_local_for_test(root_document, prepared_terminal_action(target, 101));

        let outcome = page_vm
            .run_page_dynamic_import_owner_action_body_for_test()
            .expect("current owner action should consume one typed Page turn");
        assert_eq!(
            outcome.action.owner,
            crate::page_task_queue::RendererPageDynamicImportOwnerActionOwner::new(
                root_document,
                target.task_owner(),
                target.realm_id(),
            )
        );
        let crate::page_task_queue::PageDynamicImportOwnerActionDocumentEffect::AppliedToCurrentOwner {
            outcome: action_outcome,
        } = outcome.action.document_effect
        else {
            panic!("current exact owner action must not be classified stale");
        };
        assert!(action_outcome.terminal_work_was_consumed());
        assert!(action_outcome.missing_joined_client_was_recorded());

        assert!(
            !page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
                .has_ready_task(),
            "the typed owner-action source should be empty after its one turn"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("current dynamic-import owner-action turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn current_owner_action_checkpoint_is_owned_by_selected_dispatcher() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        let target =
            install_dynamic_import_action_target(&mut page_vm, "dynamic-action-checkpoint")?;
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm
            .page_task_executor_sources_for_test()
            .dynamic_import_owner_action()
            .enqueue_local_for_test(root_document, prepared_terminal_action(target, 102));
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__dynamicOwnerActionCheckpoint = [];
Promise.resolve().then(() => {
  __dynamicOwnerActionCheckpoint.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DynamicImportOwnerAction,
                    &loader
                )
                .await?,
            "one exact DynamicImportOwnerAction task should run"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__dynamicOwnerActionCheckpoint.join('|')",
                )?,
            "microtask",
            "the selected dispatcher must submit the current task's one checkpoint"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a module owner action owns only checkpoint completion, not generic runtime follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected DynamicImportOwnerAction checkpoint test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn owner_action_source_consumes_exactly_one_action_per_turn_in_fifo_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let first_target =
            install_dynamic_import_action_target(&mut page_vm, "dynamic-action-fifo-first")?;
        let second_target =
            install_dynamic_import_action_target(&mut page_vm, "dynamic-action-fifo-second")?;
        let root_document = page_vm.document_lifecycle.identity().document;
        let source = page_vm
            .page_task_executor_sources_for_test()
            .dynamic_import_owner_action();
        source.enqueue_local_for_test(root_document, prepared_terminal_action(first_target, 103));
        source.enqueue_local_for_test(root_document, prepared_terminal_action(second_target, 107));

        let first = page_vm
            .run_page_dynamic_import_owner_action_body_for_test()
            .expect("first owner action should consume one turn");
        assert_eq!(
            first.action.owner,
            crate::page_task_queue::RendererPageDynamicImportOwnerActionOwner::new(
                root_document,
                first_target.task_owner(),
                first_target.realm_id(),
            )
        );

        assert!(source.has_ready_task());

        let second = page_vm
            .run_page_dynamic_import_owner_action_body_for_test()
            .expect("second owner action should consume a separate turn");
        assert_eq!(
            second.action.owner,
            crate::page_task_queue::RendererPageDynamicImportOwnerActionOwner::new(
                root_document,
                second_target.task_owner(),
                second_target.realm_id(),
            )
        );

        assert!(!source.has_ready_task());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dynamic-import owner-action FIFO test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn owner_action_rejects_replaced_realm_without_legacy_fallback() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let retired_target =
            install_dynamic_import_action_target(&mut page_vm, "dynamic-action-retired-realm")?;
        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(retired_target.child_handle());
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "dynamic-action-retired-realm",
        )?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(retired_target.child_handle())
            .expect("replacement realm should expose its exact target");
        assert_eq!(retired_target.task_owner(), current_target.task_owner());
        assert_ne!(retired_target.realm_id(), current_target.realm_id());
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
            .enqueue_local_for_test(
                root_document,
                prepared_terminal_action(retired_target, 109),
            );

        let outcome = page_vm
            .run_page_dynamic_import_owner_action_body_for_test()
            .expect("stale-realm action should consume one discard turn");
        assert_eq!(
            outcome.action.document_effect,
            crate::page_task_queue::PageDynamicImportOwnerActionDocumentEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert!(
            !page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
                .has_ready_task(),
            "discarding a stale action should consume it from the typed source"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale dynamic-import realm test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn root_document_namespace_rejects_reused_local_owner_identity() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        let target =
            install_dynamic_import_action_target(&mut page_vm, "dynamic-action-root-namespace")?;
        let current_root = page_vm.document_lifecycle.identity().document;
        let non_current_root = current_root.successor_for_testing();
        page_vm
            .page_task_executor_sources_for_test()
            .dynamic_import_owner_action()
            .enqueue_local_for_test(non_current_root, prepared_terminal_action(target, 113));
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleDynamicOwnerActionCheckpoint = [];
Promise.resolve().then(() => {
  __staleDynamicOwnerActionCheckpoint.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DynamicImportOwnerAction,
                    &loader,
                )
                .await?,
            "the production dispatcher should consume the old-root owner action"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__staleDynamicOwnerActionCheckpoint.join('|')",
                )?,
            "",
            "an old-root action must not manufacture a checkpoint in the current V8 owner"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "discarding an old-root action must not advance current runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dynamic-import root namespace collision test should run");
}
