use super::*;

async fn run_main_document_runtime_action_after_wake_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    owner_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::page_task_queue::RendererOwnerWake,
    >,
    kind: crate::page_task_queue::PageMainDocumentRuntimeActionKind,
    label: &str,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(kind),
                    loader,
                )
                .await
                .unwrap_or_else(|error| panic!("{label} should execute: {error}"))
            {
                break;
            }
            owner_wake_rx
                .recv()
                .await
                .unwrap_or_else(|| panic!("{label} Page route must remain open"));
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} should become a concrete typed action"));
}

#[tokio::test(flavor = "current_thread")]
async fn selected_runtime_classic_document_script_owns_terminal_task_completion() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/runtime-classic-task.js",
            "HTTP/1.1 200 OK",
            concat!(
                "__runtimeClassicTaskOrder.push('script');",
                "queueMicrotask(() => __runtimeClassicTaskOrder.push('script-microtask'));",
            )
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse(&format!("{base_url}/runtime-classic.html")).expect("document URL");
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());
        let (script_node, script_handle) = {
            let runtime = &mut page_vm.vm_mut().document_runtime;
            let body = runtime
                .snapshot_document()
                .document_body_handle()
                .expect("test document body");
            let script_node = runtime.dom_host_mut().create_element("script");
            assert!(runtime.dom_host_mut().append_child(body, script_node));
            assert!(runtime.dom_host_mut().set_attribute(
                script_node,
                "id",
                "runtime-classic-task"
            ));
            assert!(runtime.dom_host_mut().set_attribute(
                script_node,
                "src",
                "/runtime-classic-task.js"
            ));
            let script_handle = runtime
                .bind_runtime_owned_script_handle_for_node(script_node, "runtime-classic-task");
            (script_node, script_handle)
        };
        page_vm
            .vm_mut()
            .eval(
                r#"
                globalThis.__runtimeClassicTaskOrder = [];
                document.getElementById("runtime-classic-task").onload = () => {
                  __runtimeClassicTaskOrder.push("load");
                  queueMicrotask(() => __runtimeClassicTaskOrder.push("load-microtask"));
                };
                "installed";
                "#,
            )
            .expect("load listener should install");

        let mut script = prepared_loaded_classic_for_page_vm_test(&page_vm, 9061, "");
        script.node_id = script_node;
        script.host_script_handle = Some(script_handle);
        script.mode = ScriptMode::Async;
        script.source_kind = ScriptSourceKind::External;
        script.source = ScriptSource::External;
        script.initiator_url = document_url.clone();
        script.base_url = document_url;
        script.url = Url::parse(&format!("{base_url}/runtime-classic-task.js"))
            .expect("runtime classic script URL");
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("runtime classic task requires a current Document");
        let lease = page_vm
            .vm_mut()
            .accept_main_document_script_load_delay_binding(
                owner,
                crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
            )
            .expect("runtime classic task should acquire an exact load-delay lease");
        page_vm
            .vm_mut()
            .document_runtime
            .note_dom_content_loaded_dispatched();
        let task_runner = page_vm.resource_task_runner();
        page_vm.vm_mut().admit_main_document_runtime_script_task(
            &loader,
            task_runner,
            crate::host::RuntimeScriptAdmission::new(
                crate::host::RuntimeScriptAdmissionPayload::Script(script),
                lease,
            ),
        );
        run_main_document_runtime_action_after_wake_for_test(
            &mut page_vm,
            &loader,
            &mut owner_wake_rx,
            crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
            "runtime classic completion",
        )
        .await;
        server
            .await
            .expect("runtime classic script server should finish");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork,
                    ),
                    &loader,
                )
                .await
                .expect("runtime classic DocumentScript should execute")
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__runtimeClassicTaskOrder.join('|')",
                )
                .expect("completed task order should be readable"),
            "script|script-microtask|load|load-microtask",
            "script evaluation keeps its algorithm checkpoint while the shared coordinator owns the terminal task-end"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false),
            "the task-owned lease must settle exactly once"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn selected_runtime_classic_source_failure_owns_error_task_completion() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/runtime-classic-error.js",
            "HTTP/1.1 404 Not Found",
            String::new(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse(&format!("{base_url}/runtime-classic-error.html")).expect("document URL");
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());
        let (script_node, script_handle) = {
            let runtime = &mut page_vm.vm_mut().document_runtime;
            let body = runtime
                .snapshot_document()
                .document_body_handle()
                .expect("test document body");
            let script_node = runtime.dom_host_mut().create_element("script");
            assert!(runtime.dom_host_mut().append_child(body, script_node));
            assert!(runtime.dom_host_mut().set_attribute(
                script_node,
                "id",
                "runtime-classic-error-task"
            ));
            assert!(runtime.dom_host_mut().set_attribute(
                script_node,
                "src",
                "/runtime-classic-error.js"
            ));
            let script_handle = runtime.bind_runtime_owned_script_handle_for_node(
                script_node,
                "runtime-classic-error-task",
            );
            (script_node, script_handle)
        };
        page_vm
            .vm_mut()
            .eval(
                r#"
                globalThis.__runtimeClassicErrorOrder = [];
                document.getElementById("runtime-classic-error-task").onerror = () => {
                  __runtimeClassicErrorOrder.push("error");
                  queueMicrotask(() => __runtimeClassicErrorOrder.push("error-microtask"));
                };
                "installed";
                "#,
            )
            .expect("error listener should install");

        let mut script = prepared_loaded_classic_for_page_vm_test(&page_vm, 9063, "");
        script.node_id = script_node;
        script.host_script_handle = Some(script_handle);
        script.mode = ScriptMode::Async;
        script.source_kind = ScriptSourceKind::External;
        script.source = ScriptSource::External;
        script.initiator_url = document_url.clone();
        script.base_url = document_url;
        script.url = Url::parse(&format!("{base_url}/runtime-classic-error.js"))
            .expect("runtime classic error script URL");
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("runtime classic error requires a current Document");
        let lease = page_vm
            .vm_mut()
            .accept_main_document_script_load_delay_binding(
                owner,
                crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
            )
            .expect("runtime classic error should acquire an exact load-delay lease");
        page_vm
            .vm_mut()
            .document_runtime
            .note_dom_content_loaded_dispatched();
        let task_runner = page_vm.resource_task_runner();
        page_vm.vm_mut().admit_main_document_runtime_script_task(
            &loader,
            task_runner,
            crate::host::RuntimeScriptAdmission::new(
                crate::host::RuntimeScriptAdmissionPayload::Script(script),
                lease,
            ),
        );

        run_main_document_runtime_action_after_wake_for_test(
            &mut page_vm,
            &loader,
            &mut owner_wake_rx,
            crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
            "runtime classic source failure",
        )
        .await;
        server
            .await
            .expect("runtime classic error server should finish");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork,
                    ),
                    &loader,
                )
                .await
                .expect("runtime classic source-failure task should execute")
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test("__runtimeClassicErrorOrder.join('|')",)
                .expect("completed error task order should be readable"),
            "error|error-microtask",
            "the source-failure body and its reaction must complete inside one selected task"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false),
            "the failed task must settle its task-owned lease exactly once"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn selected_document_script_replacement_preserves_entered_body_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/document-script-replacement.html")
                .expect("document URL"),
        );
        let old_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("old Document owner");
        let script = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9064,
            r#"
            globalThis.__documentScriptReplacementOrder = ["script"];
            document.open();
            document.write("<!doctype html><body>replacement</body>");
            document.close();
            queueMicrotask(() => __documentScriptReplacementOrder.push("microtask"));
            "#,
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                classic_defer_work(script),
            )
            .await
            .expect("replacement DocumentScript task should complete");

        assert_ne!(
            page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("replacement Document owner"),
            old_owner,
            "the body must replace the exact Document"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__documentScriptReplacementOrder.join('|')",
                )
                .expect("replacement task order should remain readable"),
            "script|microtask",
            "an entered body remains callback-capable even when it replaces its Document"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn selected_disabled_document_script_does_not_flush_unrelated_runtime_work() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/disabled-document-script.html")
                .expect("document URL"),
        );
        page_vm.set_script_execution_disabled(true);
        page_vm
            .vm_mut()
            .enqueue_test_pending_runtime_source_load();
        let script = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9062,
            "globalThis.__disabledDocumentScriptRan = true;",
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                classic_defer_work(script),
            )
            .await
            .expect("disabled DocumentScript task should still complete");

        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "a body that entered no script or callback owns only its checkpoint, not callback runtime reconciliation"
        );
        let run = page_vm
            .report
            .runs
            .last()
            .expect("disabled selected task should report its skipped script");
        assert!(matches!(run.outcome(), ScriptRunOutcome::Skipped(_)));
    })
    .await;
}
