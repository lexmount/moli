use super::*;

#[tokio::test(flavor = "current_thread")]
async fn document_write_async_classic_waits_in_its_exact_main_runtime_source() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/written-async.js",
            "HTTP/1.1 200 OK",
            "globalThis.__writtenAsyncClassicExecuted = 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse(&format!("{base_url}/page.html")).expect("Document URL"),
            );
        while owner_wake_rx.try_recv().is_ok() {}

        page_vm.vm_mut().eval(&format!(
            r#"document.write("<script async src='{base_url}/written-async.js'><\/script>"); "written""#
        ))?;
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.write async classic requires a current owner");
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "parser discovery must acquire the exact load-delay lease before source readiness"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::PostParseWork,
                    ),
                )
                .is_none(),
            "an open source load must remain in its completion-backed residence"
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                owner_wake_rx
                    .recv()
                    .await
                    .expect("owner wake route should remain open");
                if page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                            PageMainDocumentRuntimeActionKind::PostParseWork,
                        ))
                    .await?
                {
                    return Ok::<(), anyhow::Error>(());
                }
            }
        })
        .await
        .expect("source completion should publish a concrete script task")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__writtenAsyncClassicExecuted)")?,
            "1"
        );

        run_main_async_script_load_delay_settlement_for_test(&mut page_vm,
owner,
"document.write async classic")
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false),
            "the exact lease must settle after the selected script and event work"
        );
        server.await.expect("script server should finish");
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("document.write async classic residence test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn document_write_async_module_uses_one_shot_pending_script_admission() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/written-async-module.html").expect("Document URL"),
        );

        page_vm.vm_mut().eval(
            r#"
document.write("<script type='module' async>globalThis.__writtenAsyncModuleExecuted = 1; export const value = 1;<\/script>");
"written"
"#,
        )?;
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.write async module requires a current owner");
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "parser discovery must transfer its exact lease with the admission"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "the parser module must not enter PendingScript before its selected admission task"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::ParserAsyncModuleAdmission,
                    ))
                .await?,
            "the exact admission must be consumed by the production selected dispatcher"
        );
        assert!(
            page_vm.has_ready_parser_owned_document_script_action(),
            "an immediately-ready inline graph must notify the installed PendingScript watch"
        );
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm)
                .await?,
            "the ready parser-owned module must receive its own selected continuation"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__writtenAsyncModuleExecuted)")?,
            "1"
        );

        run_main_async_script_load_delay_settlement_for_test(&mut page_vm,
owner,
"document.write async module")
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false),
            "module evaluation start and its lifecycle successor must settle the exact lease"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("document.write async module admission test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn retired_parser_async_module_admission_cannot_enter_the_replacement_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/stale-written-module.html").expect("Document URL"),
        );
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("retired async-module admission requires a current owner");
        let mut script = prepared_inline_module_for_page_vm_test(
            &page_vm,
            9711,
            "globalThis.__retiredWrittenModuleExecuted = 1;",
        );
        script.mode = ScriptMode::Async;
        let lease = page_vm
            .vm_mut()
            .accept_main_document_script_load_delay_binding(
                retired_owner,
                crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Module,
            )
            .expect("retired admission should acquire its exact lease");
        page_vm
            .vm()
            .document_runtime
            .enqueue_main_parser_async_module_admission(
                crate::document_script_scheduler::MainParserAsyncModuleAdmission::new(
                    script, lease,
                ),
            )
            .expect("retired admission should enter the stable source");
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::ParserAsyncModuleAdmission,
                ),
            )
            .expect("the exact admission should be claimable before replacement");

        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        let replacement_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement Document owner");
        assert_ne!(replacement_owner, retired_owner);
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "globalThis.__staleAdmissionCheckpoint = 0; Promise.resolve().then(() => __staleAdmissionCheckpoint = 1); 'queued'",
        )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(globalThis.__staleAdmissionCheckpoint)"
            )?,
            "0",
            "discarding an old exact-Document admission must not checkpoint its replacement"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "the retired module must not install a PendingScript in the replacement Document"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(replacement_owner),
            Some(false),
            "dropping the retired exact lease must not block the replacement Document"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("stale parser async-module admission test should run");
}
