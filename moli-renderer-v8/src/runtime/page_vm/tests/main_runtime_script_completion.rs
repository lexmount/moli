use super::*;

fn runtime_script_test_page_vm(
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) -> PageVm {
    let mut page_vm = test_page_vm_with_loader_and_document_url(
        loader,
        Vec::new(),
        Url::parse(&format!("https://{label}.test/page.html")).expect("Document URL"),
    );
    page_vm
        .vm_mut()
        .document_runtime
        .note_dom_content_loaded_dispatched();
    page_vm
}

fn publish_runtime_classic_admission(
    page_vm: &mut PageVm,
    node: u32,
    script_label: &str,
    script_body: &str,
) -> anyhow::Result<()> {
    let mut script = prepared_loaded_classic_for_page_vm_test(page_vm, node, script_body);
    script.mode = ScriptMode::Async;
    publish_runtime_script_admission(
        page_vm,
        script,
        script_label,
        crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
    )
}

fn publish_runtime_script_admission(
    page_vm: &mut PageVm,
    mut script: PreparedScript,
    script_label: &str,
    load_delay_kind: crate::frame_owner_model::MainDocumentScriptLoadDelayKind,
) -> anyhow::Result<()> {
    let (script_node, script_handle) = {
        let runtime = &mut page_vm.vm_mut().document_runtime;
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("test Document body");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(runtime.dom_host_mut().append_child(body, script_node));
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(script_node, "id", script_label)
        );
        let script_handle =
            runtime.bind_runtime_owned_script_handle_for_node(script_node, script_label);
        (script_node, script_handle)
    };
    script.node_id = script_node;
    script.host_script_handle = Some(script_handle);
    let owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("runtime admission requires a current main Document");
    let lease = page_vm
        .vm_mut()
        .accept_main_document_script_load_delay_binding(owner, load_delay_kind)
        .expect("runtime admission must acquire an exact load-delay lease");
    page_vm
        .vm()
        .document_runtime
        .publish_runtime_script_admission(crate::host::RuntimeScriptAdmission::new(
            crate::host::RuntimeScriptAdmissionPayload::Script(script),
            lease,
        ))
        .map_err(|_| anyhow::anyhow!("runtime-script producer route rejected its admission"))
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_script_admission_body_is_checkpoint_free() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-admission-body");
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__runtimeAdmissionBodyOrder = [];
queueMicrotask(() => __runtimeAdmissionBodyOrder.push("microtask"));
"installed"
"#,
            )?;
        publish_runtime_classic_admission(
            &mut page_vm,
            9501,
            "runtime-admission-body-script",
            "__runtimeAdmissionBodyOrder.push('script')",
        )?;

        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test()
            .await?
            .expect("runtime script insertion must publish one exact admission");
        assert_eq!(
            outcome.action.kind(),
            PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission
        );
        assert_eq!(
            outcome.action.target_effect(),
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__runtimeAdmissionBodyOrder.join('|')",
                )?,
            "",
            "the admission body must leave the selected task checkpoint to the central dispatcher"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime script admission body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_runtime_script_admission_owns_checkpoint_without_running_its_continuation() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-admission-selected");
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__runtimeAdmissionSelectedOrder = [];
queueMicrotask(() => __runtimeAdmissionSelectedOrder.push("microtask"));
"installed"
"#,
        )?;
        publish_runtime_classic_admission(
            &mut page_vm,
            9502,
            "runtime-admission-selected-script",
            "__runtimeAdmissionSelectedOrder.push('script')",
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission,
                    ))
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__runtimeAdmissionSelectedOrder.join('|')",
            )?,
            "microtask",
            "the selected admission must checkpoint its existing reaction without synchronously running the published continuation"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
                    ),
                )
                .is_some(),
            "admission may publish one concrete continuation but must leave it for a later selected turn"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected runtime script admission witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_script_continuation_body_precedes_its_task_end_reaction() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-continuation-order");
        page_vm
            .vm_mut()
            .eval("globalThis.__runtimeContinuationOrder = []; 'installed'")?;
        publish_runtime_classic_admission(
            &mut page_vm,
            9503,
            "runtime-continuation-first",
            "__runtimeContinuationOrder.push('first-script')",
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission,
                    ))
                .await?
        );

        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
queueMicrotask(() => {
  __runtimeContinuationOrder.push("continuation-microtask");
  const script = document.createElement("script");
  script.type = "module";
  script.id = "runtime-continuation-second";
  script.textContent = "__runtimeContinuationOrder.push('second-script')";
  document.body.appendChild(script);
});
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
                    ))
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__runtimeContinuationOrder.join('|')",
            )?,
            "continuation-microtask",
            "the continuation body must finish before its task-end checkpoint runs the queued reaction"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::PostParseWork,
                    ))
                .await?,
            "the first script body published by the continuation must remain ahead of the admission created by its task-end reaction"
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__runtimeContinuationOrder.join('|')",
            )?,
            "continuation-microtask|first-script",
            "the task-end reaction may publish later work but must not overtake the continuation body result"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission,
                    ),
                )
                .is_some(),
            "the reaction-created second script admission must remain behind the first script task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime continuation ordering witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_script_continuation_body_leaves_checkpoint_to_selected_dispatcher() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-continuation-body");
        page_vm
            .vm_mut()
            .eval("globalThis.__runtimeContinuationBody = 'installed'; 'installed'")?;
        publish_runtime_classic_admission(
            &mut page_vm,
            9505,
            "runtime-continuation-body-script",
            "globalThis.__runtimeContinuationBody = 'script'",
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission,
                    )
                )
                .await?
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__runtimeContinuationBodyCheckpoint = "pending";
queueMicrotask(() => { __runtimeContinuationBodyCheckpoint = "wrong"; });
"queued"
"#,
            )?;

        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test()
            .await?
            .expect("admitted runtime script must publish one continuation body");
        assert_eq!(
            outcome.action.kind(),
            PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation
        );
        assert_eq!(
            outcome.action.target_effect(),
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.runtime_script_continuation_body_effect(),
            Some(
                crate::script_vm::RuntimeScriptContinuationBodyEffect::AdvancedRuntimeOwner(
                    crate::script_vm::RuntimeScriptOwnerAdvance::PublishedDocumentScript,
                ),
            ),
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__runtimeContinuationBodyCheckpoint",
                )?,
            "pending",
            "the continuation body must not retain the removed legacy pre-task checkpoint"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::PostParseWork,
                    ),
                )
                .is_some(),
            "body-only execution must still publish the concrete script task without executing it"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime continuation body-only witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn spent_runtime_script_continuation_body_is_current_but_checkpoint_free() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-continuation-spent-body");
        page_vm.vm_mut().enqueue_runtime_script_work_continuation();
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__spentRuntimeContinuationBody = "pending";
queueMicrotask(() => { __spentRuntimeContinuationBody = "wrong"; });
"queued"
"#,
            )?;

        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test()
            .await?
            .expect("spent continuation reservation must retain one selected body turn");
        assert_eq!(
            outcome.action.kind(),
            PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation
        );
        assert_eq!(
            outcome.action.target_effect(),
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.runtime_script_continuation_body_effect(),
            Some(crate::script_vm::RuntimeScriptContinuationBodyEffect::ReservationSpent),
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test("__spentRuntimeContinuationBody",)?,
            "pending",
            "body-only execution must leave the spent turn's checkpoint to the selected dispatcher"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("spent runtime continuation body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn spent_current_runtime_script_continuation_still_owns_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-continuation-spent");
        page_vm.vm_mut().enqueue_runtime_script_work_continuation();
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__spentRuntimeContinuation = "pending";
queueMicrotask(() => { __spentRuntimeContinuation = "checkpointed"; });
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
                    ))
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__spentRuntimeContinuation",
            )?,
            "checkpointed",
            "a spent reservation is still an exact current selected task and must not lose its task-end checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("spent runtime continuation checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_data_module_becomes_document_script_work_without_continuation_loop() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-data-module");
        let mut script = prepared_inline_module_for_page_vm_test(
            &page_vm,
            9506,
            "globalThis.__runtimeDataModuleRan = true;",
        );
        script.mode = ScriptMode::Async;
        script.url =
            Url::parse("data:text/javascript,globalThis.__runtimeDataModuleRan%20%3D%20true%3B")?;
        script.base_url = script.url.clone();
        publish_runtime_script_admission(
            &mut page_vm,
            script,
            "runtime-data-module-script",
            crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Module,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission,
                    )
                )
                .await?
        );
        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test()
            .await?
            .expect("the admitted data module must publish one continuation");
        assert_eq!(
            outcome.action.runtime_script_continuation_body_effect(),
            Some(
                crate::script_vm::RuntimeScriptContinuationBodyEffect::AdvancedRuntimeOwner(
                    crate::script_vm::RuntimeScriptOwnerAdvance::PublishedDocumentScript,
                ),
            ),
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::PostParseWork,
                    ),
                )
                .is_some(),
            "a data module must move into a concrete DocumentScript task"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
                    ),
                )
                .is_none(),
            "materializing the data module must not requeue the same generic continuation"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime data-module successor witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_runtime_script_admission_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-admission-stale");
        publish_runtime_classic_admission(
            &mut page_vm,
            9504,
            "runtime-admission-stale-script",
            "globalThis.__retiredRuntimeAdmissionRan = true",
        )?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission,
                ),
            )
            .expect("old Document admission must remain claimable");
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__replacementAdmissionCheckpoint = "pending";
queueMicrotask(() => { __replacementAdmissionCheckpoint = "wrong"; });
"queued"
"#,
        )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__replacementAdmissionCheckpoint",
            )?,
            "pending",
            "a stale admission must not enter replacement V8 to manufacture a checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale runtime admission checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_runtime_script_continuation_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = runtime_script_test_page_vm(&loader, "runtime-continuation-stale");
        page_vm.vm_mut().enqueue_runtime_script_work_continuation();
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
                ),
            )
            .expect("old Document continuation must remain claimable");
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__replacementContinuationCheckpoint = "pending";
queueMicrotask(() => { __replacementContinuationCheckpoint = "wrong"; });
"queued"
"#,
        )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__replacementContinuationCheckpoint",
            )?,
            "pending",
            "a stale continuation must not checkpoint the replacement Document"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale runtime continuation checkpoint witness should run");
}
