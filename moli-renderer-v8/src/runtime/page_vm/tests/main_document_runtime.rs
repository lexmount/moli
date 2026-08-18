use super::*;

#[tokio::test(flavor = "current_thread")]
async fn document_open_discards_an_already_queued_main_runtime_task() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/main-runtime-stale.html")
                .expect("Document URL"),
        );
        page_vm
            .vm_mut()
            .document_runtime
            .note_dom_content_loaded_dispatched();
        let old_root = page_vm.document_lifecycle.identity().document;
        let old_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("old main Document owner");

        page_vm.vm_mut().eval(
            r#"
globalThis.__staleMainRuntimeTaskRan = 0;
const staleScript = document.createElement("script");
staleScript.type = "module";
staleScript.textContent = "globalThis.__staleMainRuntimeTaskRan = 1";
document.body.appendChild(staleScript);
"queued"
"#,
        )?;
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;

        let new_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner");
        assert_ne!(new_document_owner, old_document_owner);

        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test(&loader)
            .await?
            .expect("the old runtime task must remain durable until its stale-discard turn");
        assert_eq!(outcome.action.owner().root_document(), old_root);
        assert_eq!(outcome.action.owner().document_owner(), old_document_owner);
        assert_eq!(
            outcome.action.kind(),
            crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission
        );
        assert_eq!(
            outcome.action.target_effect(),
            crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__staleMainRuntimeTaskRan)")?,
            "0",
            "an old exact-Document runtime task must not execute in the document.open replacement"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("main runtime document.open stale-task test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_discards_an_already_ready_parser_module_action() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/main-parser-runtime-stale.html")
                .expect("Document URL"),
        );
        let old_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("old main Document owner");
        let mut script = prepared_inline_module_for_page_vm_test(
            &page_vm,
            9091,
            "globalThis.__staleParserModuleRan = true;",
        );
        script.mode = ScriptMode::Async;

        assert!(
            page_vm
                .vm_mut()
                .accept_main_parser_async_module_script(old_owner, &script)?
        );
        assert!(page_vm.has_ready_parser_owned_document_script_action());

        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        assert_ne!(
            page_vm.vm().current_main_document_task_owner(),
            Some(old_owner)
        );

        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test(&loader)
            .await?
            .expect("the retired parser action must consume a stale-discard turn");
        assert_eq!(
            outcome.action.kind(),
            crate::page_task_queue::PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation
        );
        assert_eq!(outcome.action.owner().document_owner(), old_owner);
        assert_eq!(
            outcome.action.target_effect(),
            crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__staleParserModuleRan)")?,
            "undefined"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("main parser runtime document.open stale-task test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_discards_an_already_posted_native_module_owner_event() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/native-owner-event-stale.html")
                .expect("Document URL"),
        );
        page_vm
            .vm_mut()
            .document_runtime
            .post_modulepreload_link_error_event(crate::dom::native::NativeNodeId::new(7301));
        assert!(page_vm.vm_mut().has_ready_native_module_owner_actions());

        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        assert!(
            !page_vm.vm_mut().has_ready_native_module_owner_actions(),
            "document.open must retire the old Document-local owner-event payload"
        );

        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "globalThis.__staleNativeOwnerCheckpoint = 0; Promise.resolve().then(() => __staleNativeOwnerCheckpoint = 1); 'queued'",
        )?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__staleNativeOwnerCheckpoint)",
                )?,
            "0"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent,
                    ),
                    &loader,
                )
                .await?,
            "the retired owner-event token must consume a stale-discard turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__staleNativeOwnerCheckpoint)",
                )?,
            "0",
            "a stale exact owner event must not checkpoint the replacement Document"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("native owner-event document.open stale-task test should run");
}

#[test]
fn page_vm_replacement_discards_the_old_root_before_running_the_colliding_new_task() {
    run_page_vm_large_stack_async_test(
        "main-runtime-real-page-vm-replacement-collision",
        || async move {
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let document_url =
                Url::parse(&format!("{base_url}/initial.html")).expect("initial Document URL");
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();
            local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .document_runtime
                    .note_dom_content_loaded_dispatched();

                let retired_root = page_vm.document_lifecycle.identity().document;
                let retired_owner = page_vm
                    .vm()
                    .current_main_document_task_owner()
                    .expect("retired PageVm main Document owner");
                page_vm.vm_mut().eval(
                    r#"
globalThis.__retiredMainRuntimeTaskRan = 0;
const retiredScript = document.createElement("script");
retiredScript.type = "module";
retiredScript.textContent = "globalThis.__retiredMainRuntimeTaskRan = 1";
document.body.appendChild(retiredScript);
"queued"
"#,
                )?;

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

                let current_root = page_vm.document_lifecycle.identity().document;
                let current_owner = page_vm
                    .vm()
                    .current_main_document_task_owner()
                    .expect("replacement PageVm main Document owner");
                assert_ne!(retired_root, current_root);
                assert_eq!(
                    retired_owner, current_owner,
                    "fresh PageVm owner stores should naturally reuse the same local main-Document ids"
                );
                // `follow_pending_location_navigation_one_turn_async()` may
                // return at the replacement's post-parse continuation rather
                // than after DCL. Establish the post-DCL producer boundary
                // explicitly before creating the replacement runtime script;
                // otherwise that script correctly belongs to the pre-DCL
                // lifecycle queue and does not exercise this source collision.
                page_vm
                    .vm_mut()
                    .document_runtime
                    .note_dom_content_loaded_dispatched();
                page_vm.vm_mut().eval(
                    r#"
globalThis.__replacementMainRuntimeTaskRan = 0;
const replacementScript = document.createElement("script");
replacementScript.type = "module";
replacementScript.textContent = "globalThis.__replacementMainRuntimeTaskRan = 1";
document.body.appendChild(replacementScript);
"queued"
"#,
                )?;

                let stale = page_vm
                    .run_page_main_document_runtime_body_for_test(&loader)
                    .await?
                    .expect("retired PageVm task should consume one stale-discard turn");
                assert_eq!(stale.action.owner().root_document(), retired_root);
                assert_eq!(stale.action.owner().document_owner(), retired_owner);
                assert_eq!(
                    stale.action.target_effect(),
                    crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
                );


                let current = page_vm
                    .run_page_main_document_runtime_body_for_test(&loader)
                    .await?
                    .expect("replacement PageVm task must remain behind the retired head");
                assert_eq!(current.action.owner().root_document(), current_root);
                assert_eq!(current.action.owner().document_owner(), current_owner);
                assert_eq!(
                    current.action.kind(),
                    crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission
                );
                assert_eq!(
                    current.action.target_effect(),
                    crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("String(globalThis.__retiredMainRuntimeTaskRan)")?,
                    "undefined",
                    "the retired task must not execute through the replacement PageVm"
                );

                server
                    .await
                    .expect("main runtime replacement server should finish");
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("main runtime PageVm replacement test should run");
        },
    );
}
