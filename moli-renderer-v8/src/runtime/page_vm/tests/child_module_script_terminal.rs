use super::*;

use anyhow::Context;

use crate::page_task_queue::{PageChildModuleScriptTerminalTargetEffect, RendererOwnerWakeSource};

pub(super) async fn queue_real_child_module_terminal(
    page_vm: &mut PageVm,
    resource_source: &mut crate::page_task_queue::RendererPageResourceCompletionTestSource,
    owner_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::page_task_queue::RendererOwnerWake,
    >,
    frame_id: &str,
    module_url: &str,
) -> anyhow::Result<ChildDocumentModuleFetchTarget> {
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.id = {frame_id:?};
  frame.srcdoc = `<script type="module" src="{module_url}"><\/script>`;
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(frame_id)
        .expect("module-terminal fixture should retain its child handle");
    let startup_sources =
        drive_child_frame_task_sources_until_resource_completion_ready(page_vm, 8).await;
    anyhow::ensure!(
        startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
        "child module root fetch should start from its existing source: {startup_sources:?}"
    );

    super::child_document_completion::wait_for_page_resource_completion(
        resource_source,
        owner_wake_rx,
        "child module root completion",
    )
    .await;
    let completion = page_vm
        .apply_one_page_resource_terminal_owner_admission_for_test(resource_source)?
        .expect("registered child module root should consume one stable resource turn");
    let _ = completion;
    page_vm
        .vm()
        .current_child_document_module_fetch_target(child_handle)
        .context("real terminal producer should retain an exact child realm")
}

#[tokio::test(flavor = "current_thread")]
async fn child_parser_modules_check_preparation_document_before_execution() {
    run_page_vm_async_test(async {
        for fetch_fails in [false, true] {
            for movement in ["before", "return", "during"] {
                let source = if movement == "during" {
                    "parent.__adoptionEvents.push('execute'); parent.document.body.append(parent.__movedModule);"
                } else {
                    "parent.__adoptionEvents.push('execute');"
                };
                let (base_url, server) = spawn_path_response_http_server(vec![(
                    "/adopted.mjs",
                    if fetch_fails { "HTTP/1.1 404 Not Found" } else { "HTTP/1.1 200 OK" },
                    source.to_owned(), Duration::ZERO,
                )]).await;
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page_vm, mut resource_source, mut owner_wake_rx) =
                    page_vm_with_bound_task_sources_and_owner_wake(
                        &loader, Url::parse(&format!("{base_url}/page"))?,
                    );
                queue_real_child_module_terminal(
                    &mut page_vm, &mut resource_source, &mut owner_wake_rx,
                    "adopted-child", &format!("{base_url}/adopted.mjs"),
                ).await?;
                page_vm.vm_mut().eval(r#"
                    globalThis.__adoptionEvents = [];
                    globalThis.__childDocument = document.getElementById('adopted-child').contentDocument;
                    globalThis.__movedModule = __childDocument.querySelector('script');
                    __movedModule.onload = () => __adoptionEvents.push('load');
                    __movedModule.onerror = () => __adoptionEvents.push('error');
                "#)?;
                if movement != "during" {
                    page_vm.vm_mut().eval("document.body.append(__movedModule)")?;
                    if movement == "return" {
                        page_vm.vm_mut().eval("__childDocument.body.append(__movedModule)")?;
                    }
                }
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm, "adopted child module source terminal",
                ).await;
                assert!(page_vm.run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentScriptReady, &loader,
                ).await?);
                let expected = if movement == "before" { "" } else if fetch_fails { "error" } else { "execute|load" };
                assert_eq!(page_vm.vm_mut().eval("__adoptionEvents.join('|')")?, expected,
                    "fetch_fails={fetch_fails}, movement={movement}");
                for source in [
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    ChildFrameSemanticTurnKind::HostLoad,
                ] {
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm, source, "lifecycle after adopted module",
                    ).await;
                }
                assert_eq!(page_vm.vm_mut().eval("__childDocument.readyState")?, "complete");
                server.await?;
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn child_parser_module_roots_compile_binary_wasm_responses() {
    run_page_vm_async_test(async {
        for (path, body, expected) in [
            // A custom section contains arbitrary bytes, including invalid UTF-8.
            ("/valid.WASM?v=1", vec![0, 97, 115, 109, 1, 0, 0, 0, 0, 2, 0, 255], "load"),
            ("/invalid.wasm", vec![0], "CompileError:true|load"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let base_url = format!("http://{}", listener.local_addr()?);
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request_head(&mut stream).await.unwrap();
                assert_eq!(request.lines().next().unwrap().split_whitespace().nth(1), Some(path));
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/wasm\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                );
                stream.write_all(headers.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            });
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
            let (mut page_vm, mut resource_source, mut owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(
                    &loader, Url::parse(&format!("{base_url}/page"))?,
                );
            queue_real_child_module_terminal(
                &mut page_vm, &mut resource_source, &mut owner_wake_rx,
                "wasm-child", &format!("{base_url}{path}"),
            ).await?;
            page_vm.vm_mut().eval(r#"
                globalThis.__wasmRootEvents = [];
                const child = document.getElementById('wasm-child').contentWindow;
                child.addEventListener('error', event => {
                    __wasmRootEvents.push(event.error.name + ':' +
                        (event.error.constructor === child.WebAssembly.CompileError));
                    event.preventDefault();
                });
                const script = child.document.querySelector('script');
                script.addEventListener('error', () => __wasmRootEvents.push('element-error'));
                script.addEventListener('load', () => __wasmRootEvents.push('load'));
            "#)?;
            run_expected_child_module_script_terminal_turn(
                &mut page_vm, "binary Wasm parser root",
            ).await;
            assert!(page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildDocumentScriptReady, &loader,
            ).await?);
            assert_eq!(page_vm.vm_mut().eval("__wasmRootEvents.join('|')")?, expected, "{path}");
            server.await?;
        }
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn real_terminal_captures_realm_and_is_discarded_after_realm_replacement() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/realm-terminal.js",
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

        let retired_target = queue_real_child_module_terminal(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            "terminal-realm",
            &format!("{base_url}/realm-terminal.js"),
        )
        .await?;
        server
            .await
            .expect("module-terminal realm response server should finish");
        let terminal_wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .filter(|wake| {
                wake.source_for_test() == RendererOwnerWakeSource::ChildModuleScriptTerminal
            })
            .count();
        assert_eq!(
            terminal_wakes, 1,
            "the real producer must publish one empty-to-nonempty terminal-source wake"
        );

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(retired_target.child_handle());
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "terminal-realm")?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(retired_target.child_handle())
            .expect("replacement realm should expose its exact target");
        assert_eq!(retired_target.task_owner(), current_target.task_owner());
        assert_ne!(retired_target.realm_id(), current_target.realm_id());

        let outcome = page_vm
            .run_child_module_script_terminal_body_for_test()
            .expect("stale module terminal should consume one discard turn");
        assert_eq!(
            outcome.action.target_effect,
            PageChildModuleScriptTerminalTargetEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child module terminal realm must be isolated");
}

#[test]
fn page_vm_replacement_rejects_naturally_colliding_terminal_owner() {
    run_page_vm_large_stack_async_test(
        "child-module-terminal-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![
                (
                    "/initial-terminal.js",
                    "HTTP/1.1 200 OK",
                    "export const initial = 1;".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement.html",
                    "HTTP/1.1 200 OK",
                    "<!doctype html><html><body></body></html>".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement-terminal.js",
                    "HTTP/1.1 200 OK",
                    "export const replacement = 2;".to_owned(),
                    Duration::ZERO,
                ),
            ])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (mut page_vm, mut resource_source, mut owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let retired_target = queue_real_child_module_terminal(
                        &mut page_vm,
                        &mut resource_source,
                        &mut owner_wake_rx,
                        "terminal-collision",
                        &format!("{base_url}/initial-terminal.js"),
                    )
                    .await?;
                    let retired_root = page_vm.document_lifecycle.identity().document;

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
                    assert_ne!(retired_root, current_root);

                    let current_target = queue_real_child_module_terminal(
                        &mut page_vm,
                        &mut resource_source,
                        &mut owner_wake_rx,
                        "terminal-collision",
                        &format!("{base_url}/replacement-terminal.js"),
                    )
                    .await?;
                    assert_eq!(
                        retired_target, current_target,
                        "fresh PageVm local owner and realm counters should naturally collide"
                    );

                    let stale = page_vm
                        .run_child_module_script_terminal_body_for_test()
                        .expect("retired PageVm terminal should consume one stale turn");
                    assert_eq!(stale.action.owner.root_document(), retired_root);
                    assert_eq!(
                        stale.action.target_effect,
                        PageChildModuleScriptTerminalTargetEffect::DiscardedStaleOwner {
                            current_owner: None,
                        }
                    );
                    let current = page_vm
                        .run_child_module_script_terminal_body_for_test()
                        .expect("replacement PageVm terminal must survive stale-head discard");
                    assert_eq!(current.action.owner.root_document(), current_root);
                    assert!(matches!(
                        current.action.target_effect,
                        PageChildModuleScriptTerminalTargetEffect::AppliedToCurrentOwner { .. }
                    ));
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("PageVm replacement terminals should use the exact owner arbiter");
            server
                .await
                .expect("PageVm replacement terminal server should finish");
        },
    );
}
