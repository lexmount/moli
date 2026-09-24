//! P5 task-end contracts for child module-map terminal batches.
//!
//! The body consumes one exact child module-map terminal and may compile a
//! module or publish typed dependency, graph-ready, or failure successors. It
//! never evaluates the module, dispatches a callback, or executes those
//! successors. Only the production selected-task dispatcher may submit the
//! ordinary task-end checkpoint for a current terminal batch.

use super::*;

async fn queue_simple_child_module_terminal(
    page_vm: &mut PageVm,
    resource_source: &mut crate::page_task_queue::RendererPageResourceCompletionTestSource,
    owner_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::page_task_queue::RendererOwnerWake,
    >,
    frame_id: &str,
    module_url: &str,
) -> anyhow::Result<ChildDocumentModuleFetchTarget> {
    super::child_module_script_terminal::queue_real_child_module_terminal(
        page_vm,
        resource_source,
        owner_wake_rx,
        frame_id,
        module_url,
    )
    .await
}

async fn await_module_response_server(server: JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("child module request should reach the test server")
        .expect("child module response server should finish");
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_script_terminal_body_does_not_checkpoint_or_execute_successor() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/terminal-body.js",
            "HTTP/1.1 200 OK",
            "parent.__childTerminalBodyRuns += 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            "globalThis.__childTerminalBodyRuns = 0; 'body marker installed'",
        )?;

        queue_simple_child_module_terminal(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            "terminal-body-child",
            &format!("{base_url}/terminal-body.js"),
        )
        .await?;
        await_module_response_server(server).await;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__childTerminalBodyCheckpoint = 0;
Promise.resolve().then(() => __childTerminalBodyCheckpoint += 1);
"reaction queued"
"#,
            )?;

        let outcome = page_vm
            .run_child_module_script_terminal_body_for_test()
            .expect("one exact child module-terminal body should be ready");
        assert!(matches!(
            outcome.action.target_effect,
            crate::page_task_queue::PageChildModuleScriptTerminalTargetEffect::AppliedToCurrentOwner {
                ..
            }
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__childTerminalBodyCheckpoint)",
                )?,
            "0",
            "module-terminal body must leave the ordinary checkpoint to the selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__childTerminalBodyRuns)",
                )?,
            "0",
            "terminal processing must publish rather than execute the module successor",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child module-terminal body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_module_script_terminal_checkpoints_without_draining_successors() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/selected-terminal.js",
            "HTTP/1.1 200 OK",
            "parent.__selectedChildTerminalBodyRuns += 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm
            .vm_mut()
            .eval("globalThis.__selectedChildTerminalBodyRuns = 0; 'body marker installed'")?;

        queue_simple_child_module_terminal(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            "selected-terminal-child",
            &format!("{base_url}/selected-terminal.js"),
        )
        .await?;
        await_module_response_server(server).await;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__selectedChildTerminalCheckpoint = 0;
Promise.resolve().then(() => __selectedChildTerminalCheckpoint += 1);
"reaction queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildModuleScriptTerminal
                )
                .await?,
            "one exact module-terminal task must enter the production selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedChildTerminalCheckpoint)",
                )?,
            "1",
            "the production dispatcher must submit the state-only terminal checkpoint",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedChildTerminalBodyRuns)",
                )?,
            "0",
            "task completion must not synchronously execute the module successor",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "checkpoint-only terminal completion must not drain unrelated runtime work",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child module-terminal completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_child_module_script_terminal_does_not_checkpoint_current_document() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/stale-terminal.js",
            "HTTP/1.1 200 OK",
            "export const value = 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        let retired_target = queue_simple_child_module_terminal(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            "stale-terminal-child",
            &format!("{base_url}/stale-terminal.js"),
        )
        .await?;
        await_module_response_server(server).await;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildModuleScriptTerminal,
            )
            .expect("the old child realm must retain one opaque module-terminal claim");

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(retired_target.child_handle());
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "stale-terminal-child")?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleChildTerminalCheckpoint = 0;
Promise.resolve().then(() => __staleChildTerminalCheckpoint += 1);
"replacement reaction queued"
"#,
            )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__staleChildTerminalCheckpoint)",
                )?,
            "0",
            "a stale terminal claim must not checkpoint the current Document",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child module-terminal completion witness should run");
}
