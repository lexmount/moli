use super::*;

pub(super) async fn queue_child_module_document_script_ready(
    page_vm: &mut PageVm,
    base_url: &str,
    create_reaction_child: bool,
) -> anyhow::Result<()> {
    let module_url = format!("{base_url}/child-module-task-boundary.js");
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  globalThis.__lmChildModuleTaskBoundary = [];
  const frame = document.createElement("iframe");
  frame.id = "child-module-task-boundary";
  frame.onload = () => parent.__lmChildModuleTaskBoundary.push("frame-load");
  frame.srcdoc = `
    <!doctype html>
    <script id="module" type="module" src="{module_url}"><\/script>
    <script>
      document.getElementById("module").addEventListener("load", () => {{
        parent.__lmChildModuleTaskBoundary.push("script-load");
        Promise.resolve().then(() => {{
          parent.__lmChildModuleTaskBoundary.push("load-microtask");
          if ({create_reaction_child}) {{
            const sibling = parent.document.createElement("iframe");
            sibling.id = "module-load-reaction-child";
            sibling.srcdoc = "<!doctype html><body>module reaction child</body>";
            parent.document.body.appendChild(sibling);
          }}
        }});
      }});
    <\/script>
  `;
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;

    let startup_sources =
        drive_child_frame_task_sources_until_resource_completion_ready(page_vm, 8).await;
    assert!(
        startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
        "the real child parser-module producer must start its root fetch: {startup_sources:?}"
    );
    if !page_vm
        .page_resource_completion_queue()
        .has_ready_completion()
    {
        let arrived = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_typed_page_resource_completion(page_vm),
        )
        .await
        .expect("child module task-boundary source should complete before timeout");
        assert!(arrived, "child module completion source must remain open");
    }
    let terminal = run_next_resource_completion_as_typed_page_turn(page_vm)?;
    assert_eq!(
        terminal.action.source(),
        RendererOwnerResourceActivitySource::ModuleGraphFetch
    );
    run_expected_child_module_script_terminal_turn(
        page_vm,
        "child module task-boundary terminal fanout",
    )
    .await;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_module_ready_completes_load_reaction_and_runtime_followup() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-module-task-boundary.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__lmChildModuleTaskBoundary.push("module-body");
Promise.resolve().then(() => {
  parent.__lmChildModuleTaskBoundary.push("module-microtask");
});
"#
            .to_owned(),
            Duration::from_millis(50),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        queue_child_module_document_script_ready(&mut page_vm, &base_url, true).await?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                    &loader,
                )
                .await?,
            "the exact child module task must run through the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildModuleTaskBoundary.join('|')"
                )?,
            "module-body|module-microtask|script-load|load-microtask",
            "module error handling must checkpoint before load, while selected task completion owns the load-listener reaction"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "module callback completion must synchronize the child created by the load reaction"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "module callback completion must publish its typed runtime-script follow-up"
        );

        server
            .await
            .expect("child module selected-task server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child module completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_task_completion_precedes_host_load_without_host_load_checkpoint_repair() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-module-task-boundary.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__lmChildModuleTaskBoundary.push("module-body");
Promise.resolve().then(() => {
  parent.__lmChildModuleTaskBoundary.push("module-microtask");
});
"#
            .to_owned(),
            Duration::from_millis(50),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        queue_child_module_document_script_ready(&mut page_vm, &base_url, false).await?;

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
                    "__lmChildModuleTaskBoundary.join('|')"
                )?,
            "module-body|module-microtask|script-load|load-microtask",
            "all module-task reactions must settle before child lifecycle can reach HostLoad"
        );

        assert_eq!(
            run_child_domcontentloaded_then_host_load_for_wait(
                &mut page_vm,
                "child module task-end HostLoad",
            )
            .await,
            ChildFrameSemanticTurnKind::HostLoad
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildModuleTaskBoundary.join('|')"
                )?,
            "module-body|module-microtask|script-load|load-microtask|frame-load",
            "HostLoad must only dispatch load; it must not be required to repair predecessor module reactions"
        );

        server
            .await
            .expect("child module HostLoad boundary server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child module/HostLoad boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_ready_body_cleans_up_script_before_load_callback() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-module-task-boundary.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__lmChildModuleTaskBoundary.push("module-body");
Promise.resolve().then(() => {
  parent.__lmChildModuleTaskBoundary.push("module-microtask");
});
"#
            .to_owned(),
            Duration::from_millis(50),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        queue_child_module_document_script_ready(&mut page_vm, &base_url, false).await?;

        let body = page_vm
            .run_page_child_document_script_ready_body_for_test()
            .await?
            .expect("module graph-ready work must retain one exact DocumentScriptReady body");
        assert!(matches!(
            body.action.target_effect,
            crate::page_task_queue::PageChildDocumentScriptReadyTargetEffect::AppliedScriptOrEventToCurrentOwner {
                made_progress: true
            }
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildModuleTaskBoundary.join('|')"
                )?,
            "module-body|module-microtask|script-load|load-microtask",
            "module cleanup precedes script load, whose callback has its own cleanup checkpoint"
        );

        server
            .await
            .expect("child module task-boundary server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child module body/task-end witness should run");
}
