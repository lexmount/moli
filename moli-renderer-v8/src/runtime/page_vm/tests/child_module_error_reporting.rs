use super::child_module_document_script_ready::queue_child_module_document_script_ready;
use super::child_parser_module_root_start_completion::advance_to_child_parser_module_root;
use super::*;

async fn queue_inline_child_error(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    source: &str,
    value: &str,
) -> anyhow::Result<()> {
    let html = format!("<script id='broken' type='module'>{source}</script>");
    page_vm.vm_mut().eval(&format!(
        r#"
        const frame = document.createElement('iframe');
        frame.id = 'module-error-child';
        frame.srcdoc = {html};
        document.body.appendChild(frame);
    "#,
        html = serde_json::to_string(&html)?
    ))?;
    advance_to_child_parser_module_root(page_vm, loader).await?;
    install_child_error_observers(page_vm, "module-error-child", value)?;
    assert!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildParserModuleRootStart,
                loader,
            )
            .await?
    );
    // The inline root completes its graph in the root-start body. Parser
    // completion still has to release the deferred-script ordering barrier.
    for _ in 0..4 {
        if page_vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady,
        ) {
            return Ok(());
        }
        anyhow::ensure!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentLifecycle,
                    loader,
                )
                .await?,
            "inline module must finish its parser before becoming runnable"
        );
    }
    anyhow::bail!("inline child module did not become runnable")
}

fn install_child_error_observers(
    page_vm: &mut PageVm,
    frame_id: &str,
    value: &str,
) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
        globalThis.__childErrors = [];
        globalThis.__childErrorLocations = [];
        globalThis.__childOnerrors = [];
        globalThis.__childErrorOrder = [];
        globalThis.__parentErrors = 0;
        globalThis.__scriptErrors = 0;
        globalThis.__scriptLoads = 0;
        globalThis.__coercions = 0;
        globalThis.__unhandled = 0;
        addEventListener('error', event => {{ ++__parentErrors; event.preventDefault(); }});
        const child = document.getElementById({frame_id:?}).contentWindow;
        child.__reason = {value};
        const expectedReason = child.__reason;
        child.addEventListener('unhandledrejection', event => {{ ++__unhandled; event.preventDefault(); }});
        child.addEventListener('error', event => {{
            __childErrorLocations.push([event.filename, event.lineno, event.colno]);
            __childErrors.push([
                event.error, event instanceof child.ErrorEvent,
                event.target === child, event.isTrusted,
                event.error instanceof child.SyntaxError
            ]);
            __childErrorOrder.push('window-error');
            Promise.resolve().then(() => __childErrorOrder.push('error-microtask'));
            event.preventDefault();
        }});
        child.onerror = (message, filename, line, column, error) => {{
            __childOnerrors.push(error);
            return true;
        }};
        const script = child.document.querySelector('script[type=module]');
        script.addEventListener('error', () => ++__scriptErrors);
        script.addEventListener('load', () => {{ ++__scriptLoads; __childErrorOrder.push('script-load'); }});
    "#
    ))?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_error_reporting_preserves_inline_source_location() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page_vm, _resource, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/child-module-location.html").unwrap(),
        );
        queue_inline_child_error(&mut page_vm, &loader, "\n\nmissingModuleReference", "null")
            .await?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    r#"
            JSON.stringify([__childErrorLocations.length,
                __childErrorLocations[0]?.[0] === child.document.URL,
                __childErrorLocations[0]?.slice(1), __parentErrors, __scriptErrors, __scriptLoads])
        "#
                )?,
            "[1,true,[3,1],0,0,0]"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_error_reporting_reports_root_parse_error_to_its_window() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page_vm, _resource, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/child-module-errors.html").unwrap(),
        );
        queue_inline_child_error(&mut page_vm, &loader, "export const broken = ;", "null").await?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    r#"
            JSON.stringify([
                __childErrors.length, __childOnerrors.length,
                __childErrors[0]?.slice(1).every(Boolean),
                __childErrors[0]?.[0] === __childOnerrors[0],
                __parentErrors, __scriptErrors, __scriptLoads,
                __childErrorOrder.join('|')
            ])
        "#
                )?,
            r#"[1,1,true,true,0,0,0,"window-error|error-microtask"]"#
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_error_reporting_preserves_synchronous_thrown_values() {
    run_page_vm_async_test(async {
        for value in [
            "Object.freeze(new child.TypeError('original'))",
            "undefined",
            "null",
            "false",
            "-0",
            "NaN",
            "17n",
            "Symbol('original')",
            "Object.freeze({marker: 42})",
            "({toString() { ++__coercions; throw new Error('must not coerce'); }})",
        ] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page_vm, _resource, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/child-module-errors.html").unwrap(),
            );
            queue_inline_child_error(&mut page_vm, &loader, "throw globalThis.__reason;", value)
                .await?;
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                        &loader,
                    )
                    .await?
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        r#"
                JSON.stringify([
                    __childErrors.length, __childOnerrors.length,
                    Object.is(__childErrors[0]?.[0], child.__reason),
                    Object.is(__childOnerrors[0], child.__reason),
                    __childErrors[0]?.slice(1, 4).every(Boolean),
                    __parentErrors, __scriptErrors, __scriptLoads, __coercions
                ])
            "#
                    )?,
                "[1,1,true,true,true,0,0,0,0]",
                "{value}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_error_reporting_distinguishes_fetch_parse_and_evaluation_failures() {
    run_page_vm_async_test(async {
        for kind in ["fetch", "parse", "evaluate", "retire"] {
            let status = if kind == "fetch" { "HTTP/1.1 404 Not Found" } else { "HTTP/1.1 200 OK" };
            let source = if kind == "parse" { "export const broken = ;" } else { "throw globalThis.__reason;" };
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/child-module-task-boundary.js", status, source.to_owned(), Duration::from_millis(50),
            )]).await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page_vm, _resource, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader, Url::parse(&format!("{base_url}/page")).unwrap(),
            );
            queue_child_module_document_script_ready(&mut page_vm, &base_url, false).await?;
            install_child_error_observers(&mut page_vm, "child-module-task-boundary", "Object.freeze(new child.TypeError('original'))")?;
            if kind == "retire" {
                page_vm.vm_mut().eval("child.addEventListener('error', () => document.getElementById('child-module-task-boundary').remove())")?;
            }
            assert!(page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildDocumentScriptReady, &loader,
            ).await?);
            let expected = match kind {
                "fetch" => "0|0|0|1|0|0",
                "retire" => "1|1|0|0|0|0",
                _ => "1|1|0|0|1|0",
            };
            assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "[__childErrors.length, __childOnerrors.length, __parentErrors, __scriptErrors, __scriptLoads, __unhandled].join('|')",
            )?, expected, "{kind}");
            assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(&format!(r#"
                __childErrors.every((entry, i) => entry[1] && entry[2] && entry[3] &&
                    entry[0] === __childOnerrors[i] &&
                    ({parse} ? entry[4] : Object.is(entry[0], expectedReason)))
            "#, parse = kind == "parse"))?, "true", "{kind}");
            if matches!(kind, "parse" | "evaluate") {
                assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                    "__childErrorOrder.indexOf('window-error') < __childErrorOrder.indexOf('script-load') && __childErrorOrder.includes('error-microtask')",
                )?, "true", "{kind}");
            }
            server.await.unwrap();
        }
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_error_reporting_preserves_deferred_rejections_without_repeating_load() {
    run_page_vm_async_test(async {
        for value in [
            "Object.freeze(new child.TypeError('original'))",
            "undefined",
            "({toString() { ++__coercions; throw new Error('must not coerce'); }})",
        ] {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/child-module-task-boundary.js",
                "HTTP/1.1 200 OK",
                "await new Promise((_, reject) => { globalThis.__reject = reject; });".to_owned(),
                Duration::from_millis(50),
            )])
            .await;
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page_vm, _resource, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse(&format!("{base_url}/page")).unwrap(),
            );
            queue_child_module_document_script_ready(&mut page_vm, &base_url, false).await?;
            install_child_error_observers(&mut page_vm, "child-module-task-boundary", value)?;
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                        &loader,
                    )
                    .await?
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        "[__childErrors.length, __scriptLoads].join('|')",
                    )?,
                "0|1"
            );
            run_child_domcontentloaded_then_host_load_for_wait(&mut page_vm, "pending child TLA")
                .await;
            page_vm.vm_mut().eval("child.__reject(expectedReason)")?;
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ModuleReaction,
                        &loader,
                    )
                    .await?
            );
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                        &loader,
                    )
                    .await?
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        r#"
                JSON.stringify([
                    __childErrors.length, __childOnerrors.length,
                    Object.is(__childErrors[0]?.[0], expectedReason),
                    Object.is(__childOnerrors[0], expectedReason),
                    __childErrors[0]?.slice(1, 4).every(Boolean),
                    __scriptErrors, __scriptLoads, __parentErrors, __unhandled, __coercions,
                    __childErrorOrder.includes('error-microtask')
                ])
            "#
                    )?,
                "[1,1,true,true,true,0,1,0,0,0,true]",
                "{value}"
            );
            server.await.unwrap();
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_error_reporting_body_leaves_the_checkpoint_to_the_selected_task() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page_vm, _resource, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://example.com/child-module-errors.html").unwrap(),
        );
        queue_inline_child_error(&mut page_vm, &loader, "export const broken = ;", "null").await?;
        let outcome = page_vm.run_page_child_document_script_ready_body_for_test().await?.unwrap();
        assert!(matches!(outcome.action.target_effect,
            crate::page_task_queue::PageChildDocumentScriptReadyTargetEffect::AppliedScriptOrEventToCurrentOwner { made_progress: true }
        ));
        assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "__childErrorOrder.join('|')",
        )?, "window-error");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_error_reporting_discards_a_claim_from_a_replaced_document() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page_vm, _resource, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://example.com/child-module-errors.html").unwrap(),
        );
        queue_inline_child_error(&mut page_vm, &loader, "export const broken = ;", "null").await?;
        let claim = page_vm.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::ChildDocumentScriptReady,
        ).unwrap();
        page_vm.vm_mut().eval("document.getElementById('module-error-child').srcdoc = '<body>replacement</body>'")?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::ChildNavigationCommit, &loader,
        ).await?);
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "globalThis.__retiredCheckpoint = 0; Promise.resolve().then(() => ++__retiredCheckpoint)",
        )?;
        page_vm.run_claimed_selected_page_task_for_test(claim, &loader).await?;
        assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "[__childErrors.length, __parentErrors, __scriptErrors, __scriptLoads, __retiredCheckpoint].join('|')",
        )?, "0|0|0|0|0");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}
