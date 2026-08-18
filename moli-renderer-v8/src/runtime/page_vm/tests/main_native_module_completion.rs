//! Shared P5 completion contracts for main native-module owner tasks.
//!
//! Dynamic-module jobs and native module-owner events share terminal
//! machinery, but retain distinct selected-task variants. These tests cover
//! the shared handoff into later typed runtime-module continuations without
//! rebuilding either selected executor in test code.

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn state_only_dynamic_module_job_checkpoints_without_draining_runtime_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/state-only-dynamic.mjs",
            "HTTP/1.1 200 OK",
            "export const value = 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse(&format!("{base_url}/page.html")).expect("Document URL"),
        );
        let request_url = format!("{base_url}/state-only-dynamic.mjs");

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(&format!(
                r#"
globalThis.__dynamicJobCheckpoint = 0;
Promise.resolve().then(() => __dynamicJobCheckpoint = 1);
import({request_url:?}).catch(() => {{}});
"queued"
"#,
            ))?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::DynamicModuleJob,
                    ),
                    &loader,
                )
                .await?,
            "the concrete dynamic-module graph job must enter its selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__dynamicJobCheckpoint)",
                )?,
            "1",
            "a state-only current graph task still owns one ordinary task-end checkpoint"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "checkpoint-only completion must not synchronously drain unrelated runtime work"
        );

        server
            .await
            .expect("state-only dynamic-module server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("state-only dynamic-module completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn spent_dynamic_module_job_reservation_does_not_manufacture_a_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/spent-dynamic-job.html").expect("Document URL"),
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__spentDynamicJobCheckpoint = 0;
import("./spent.mjs").catch(() => {});
"queued"
"#,
            )?;
        drop(
            page_vm
                .vm_mut()
                .document_runtime
                .take_next_native_dynamic_module_import()
                .expect("test must spend the concrete job behind the stable reservation"),
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                "Promise.resolve().then(() => __spentDynamicJobCheckpoint = 1); 'reaction queued'",
            )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::DynamicModuleJob,
                    ),
                    &loader,
                )
                .await?,
            "the spent stable reservation must still be consumed exactly once"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__spentDynamicJobCheckpoint)",
                )?,
            "0",
            "a reservation without concrete owner work is not a task-end authority"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::DynamicModuleJob,
                    ),
                    &loader,
                )
                .await?,
            "a spent reservation must not create a phantom successor"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("spent dynamic-module reservation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn modulepreload_owner_event_checkpoints_listener_reaction_without_runtime_drain() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/modulepreload-event.html").expect("Document URL"),
        );
        page_vm.vm_mut().eval(
            r#"
globalThis.__modulepreloadOwnerEvents = [];
const link = document.createElement("link");
link.id = "selected-modulepreload";
link.addEventListener("error", () => {
  __modulepreloadOwnerEvents.push("listener");
  Promise.resolve().then(() => __modulepreloadOwnerEvents.push("reaction"));
});
document.head.appendChild(link);
"installed"
"#,
        )?;
        let link = page_vm
            .vm()
            .document_runtime
            .get_element_by_id("selected-modulepreload")
            .expect("modulepreload test link");
        page_vm
            .vm_mut()
            .document_runtime
            .post_modulepreload_link_error_event(link);
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent,
                    ),
                    &loader,
                )
                .await?,
            "the modulepreload owner event must run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__modulepreloadOwnerEvents.join('|')",
                )?,
            "listener|reaction",
            "the event body must leave its Promise reaction to the selected task-end checkpoint"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "native owner-event completion must not execute unrelated runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("modulepreload owner-event completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn native_module_failure_fanout_requeues_every_terminal_for_later_selected_turns() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .document_runtime
            .note_dom_content_loaded_dispatched();
        page_vm.vm_mut().eval(
            r#"
globalThis.__runtimeModuleFailureFanout = [];
for (let index = 0; index < 2; index++) {
  const script = document.createElement("script");
  script.type = "module";
  script.id = `runtime-module-failure-${index}`;
  script.textContent = "import './shared.mjs';";
  script.onerror = () => __runtimeModuleFailureFanout.push(script.id);
  document.body.appendChild(script);
}
"installed"
"#,
        )?;

        for index in 0..2 {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::MainDocumentRuntime(
                            PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission,
                        ),
                        &loader,
                    )
                    .await?,
                "runtime module {index} must enter DynamicScriptOwner through its selected admission"
            );
        }

        let pending_runtime_continuation = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
                ),
            )
            .expect("admitting ready runtime modules must publish one stable continuation");

        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("runtime module Document owner");
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "the admitted runtime modules must retain exact Window-load leases"
        );
        let mut actions = NativeModuleOwnerActions::empty();

        for _ in 0..2 {
            let DynamicScriptRunnable::Execute { id, script, .. } = page_vm
                .vm_mut()
                .document_runtime
                .runtime_script_work_mut()
                .dynamic_scripts
                .next_runnable_script()
                .expect("each concrete admission must retain one exact runnable script")
            else {
                panic!("runtime module admission must become concrete execution work");
            };
            let continuation = ModuleScriptContinuation::new_runtime(script, id, owner);
            actions.merge(NativeModuleOwnerActions::from_runtime_module_failure(
                continuation,
                ModuleLoadError::new(ModuleLoadStage::Fetch, "shared dependency failed"),
            ));
        }

        page_vm
            .vm_mut()
            .commit_runtime_module_graph_start_actions_for_selected_task_test(actions);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__runtimeModuleFailureFanout.join('|')")?,
            "",
            "committing owner actions must not synchronously execute their terminal callbacks"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "the later selected terminals must retain both exact load leases"
        );

        page_vm
            .run_claimed_selected_page_task_for_test(pending_runtime_continuation, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__runtimeModuleFailureFanout.length")?,
            "0",
            "runtime-owner continuations only publish concrete DocumentScript terminals"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "publishing terminal tasks must not consume their exact leases"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::PostParseWork,
                    ),
                    &loader,
                )
                .await?,
            "the first published DocumentScript failure must consume one later selected task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__runtimeModuleFailureFanout.length")?,
            "1",
            "the first selected DocumentScript must dispatch exactly one terminal"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
                    ),
                    &loader,
                )
                .await?,
            "the runtime owner must publish one continuation for the remaining failure"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__runtimeModuleFailureFanout.length")?,
            "1",
            "the continuation may publish but must not execute the second terminal"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::PostParseWork,
                    ),
                    &loader,
                )
                .await?,
            "the second published DocumentScript failure must consume its own selected task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__runtimeModuleFailureFanout.length")?,
            "2",
            "the second selected DocumentScript must dispatch exactly one terminal"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__runtimeModuleFailureFanout.sort().join('|')")?,
            "runtime-module-failure-0|runtime-module-failure-1"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false),
            "the two later terminal turns must consume their own exact load leases"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("native module terminal handoff test should run");
}
