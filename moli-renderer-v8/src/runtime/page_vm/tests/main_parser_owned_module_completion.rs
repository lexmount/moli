use super::*;

fn parser_owned_module_test_page_vm(
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) -> PageVm {
    let document_url =
        Url::parse(&format!("https://{label}.test/page.html")).expect("Document URL");
    let (page_vm, _resource_source, _wake_rx) =
        page_vm_with_bound_task_sources_and_owner_wake(loader, document_url);
    page_vm
}

fn install_ready_parser_owned_module(
    page_vm: &mut PageVm,
    node: u32,
    source: &str,
) -> anyhow::Result<()> {
    let owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("parser-owned module test requires a current Document owner");
    let mut script = prepared_inline_module_for_page_vm_test(page_vm, node, source);
    script.mode = ScriptMode::Async;
    assert!(
        page_vm
            .vm_mut()
            .accept_main_parser_async_module_script(owner, &script)?,
        "an inline parser-owned module must install one ready action"
    );
    assert!(page_vm.has_ready_parser_owned_document_script_action());
    Ok(())
}

fn install_style_turn_exit_witness(page_vm: &mut PageVm, color: &str) -> anyhow::Result<()> {
    let selector = format!(".parser-module-active {{ color: {color}; }}");
    assert_eq!(
        page_vm.vm_mut().eval(&format!(
            r#"
(() => {{
  const style = document.createElement("style");
  style.textContent = {selector:?};
  document.head.appendChild(style);
  globalThis.__parserOwnedModuleStyleTarget = document.createElement("div");
  document.body.appendChild(globalThis.__parserOwnedModuleStyleTarget);
  return getComputedStyle(globalThis.__parserOwnedModuleStyleTarget).color;
}})()
"#
        ))?,
        "rgb(0, 0, 0)",
        "the witness must cache selector-dependent style before module execution"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn parser_owned_module_body_leaves_turn_exit_to_selected_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = parser_owned_module_test_page_vm(&loader, "parser-module-body");
        install_style_turn_exit_witness(&mut page_vm, "rgb(1, 2, 3)")?;
        install_ready_parser_owned_module(
            &mut page_vm,
            9601,
            "globalThis.__parserOwnedModuleStyleTarget.className = 'parser-module-active'; globalThis.__parserOwnedModuleBodyRan = true;",
        )?;

        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test()
            .await?
            .expect("parser-owned module must retain one exact body turn");
        assert_eq!(
            outcome.action.kind(),
            PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation
        );
        assert_eq!(
            outcome.action.target_effect(),
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__parserOwnedModuleBodyRan)",
                )?,
            "true"
        );
        assert!(
            page_vm
                .vm()
                .pending_style_invalidation_work_item_count_for_current_document_for_test()
                > 0,
            "the module body must not submit the selected task's turn-exit drain"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("parser-owned module body boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_parser_owned_module_submits_its_turn_exit() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = parser_owned_module_test_page_vm(&loader, "parser-module-selected");
        install_style_turn_exit_witness(&mut page_vm, "rgb(4, 5, 6)")?;
        install_ready_parser_owned_module(
            &mut page_vm,
            9602,
            "globalThis.__parserOwnedModuleStyleTarget.className = 'parser-module-active'; globalThis.__selectedParserOwnedModuleRan = true;",
        )?;

        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm)
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedParserOwnedModuleRan)",
                )?,
            "true"
        );
        assert_eq!(
            page_vm
                .vm()
                .pending_style_invalidation_work_item_count_for_current_document_for_test(),
            0,
            "the production selected dispatcher must submit the module task's turn-exit drain"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected parser-owned module boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn parser_owned_module_ticket_consumes_only_one_ready_action() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = parser_owned_module_test_page_vm(&loader, "parser-module-one-action");
        install_ready_parser_owned_module(
            &mut page_vm,
            9603,
            "globalThis.__firstParserOwnedModuleRan = true;",
        )?;
        install_ready_parser_owned_module(
            &mut page_vm,
            9604,
            "globalThis.__secondParserOwnedModuleRan = true;",
        )?;

        assert!(run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm).await?);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__firstParserOwnedModuleRan)",
                )?,
            "true"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__secondParserOwnedModuleRan)",
                )?,
            "undefined",
            "one selected continuation must not scan forward into a second ready module"
        );
        assert!(
            page_vm.has_ready_parser_owned_document_script_action(),
            "the second concrete action must remain in its owner queue for a later ticket"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("parser-owned module one-action test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn spent_parser_owned_module_ticket_does_not_manufacture_a_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = parser_owned_module_test_page_vm(&loader, "parser-module-spent");
        assert!(page_vm.vm_mut().enqueue_parser_owned_module_continuation());
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__spentParserOwnedModuleBoundary = [];
Promise.resolve().then(() => __spentParserOwnedModuleBoundary.push("microtask"));
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm).await?);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__spentParserOwnedModuleBoundary.join('|')",
                )?,
            "",
            "a current ticket with no concrete action must not manufacture a checkpoint"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a spent parser ticket must not advance unrelated runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("spent parser-owned module ticket test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_parser_owned_module_ticket_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = parser_owned_module_test_page_vm(&loader, "parser-module-stale");
        install_ready_parser_owned_module(
            &mut page_vm,
            9605,
            "globalThis.__staleParserOwnedModuleMustNotRun = true;",
        )?;
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleParserOwnedModuleBoundary = [];
Promise.resolve().then(() => __staleParserOwnedModuleBoundary.push("microtask"));
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation,
                    ))
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__staleParserOwnedModuleBoundary.join('|')",
            )?,
            "",
            "an old-Document ticket must not checkpoint the replacement realm"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__staleParserOwnedModuleMustNotRun)",
                )?,
            "undefined"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "stale retirement must not advance replacement runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale parser-owned module ticket test should run");
}
