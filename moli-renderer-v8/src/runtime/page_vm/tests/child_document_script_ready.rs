use super::*;

async fn install_child_classic_script_ready_fixture(
    page_vm: &mut PageVm,
    frame_id: &str,
) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  globalThis.__lmChildClassicScriptTaskBoundary = [];
  const frame = document.createElement("iframe");
  frame.id = {frame_id:?};
  document.body.appendChild(frame);
  void frame.contentWindow.Function;
  const script = frame.contentDocument.createElement("script");
  script.textContent = `
    parent.__lmChildClassicScriptTaskBoundary.push("script");
    Promise.resolve().then(() => {{
      parent.__lmChildClassicScriptTaskBoundary.push("microtask");
      const sibling = parent.document.createElement("iframe");
      sibling.id = "classic-script-reaction-child";
      sibling.srcdoc = "<!doctype html><body>reaction child</body>";
      parent.document.body.appendChild(sibling);
    }});
  `;
  frame.contentDocument.body.appendChild(script);
  return "queued";
}})()
"#,
    ))?;
    run_expected_child_realm_materialization_for_wait(
        page_vm,
        "child classic script task fixture realm",
    )
    .await;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_classic_script_ready_completes_reaction_and_runtime_followup() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-classic-script-selected").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_classic_script_ready_fixture(&mut page_vm, "classic-script-selected").await?;
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
            "the exact classic-script task should run through the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildClassicScriptTaskBoundary.join('|')"
                )?,
            "script|microtask",
            "selected classic-script completion must observe reactions completed by script cleanup"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "selected callback completion must synchronize the child created by the reaction"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "selected classic-script callback completion must publish its typed runtime-script follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child classic-script completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn claimed_old_child_classic_script_does_not_complete_in_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-classic-script-stale").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_classic_script_ready_fixture(&mut page_vm, "classic-script-stale").await?;

        let retired = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildDocumentScriptReady,
            )
            .expect("the first child Document should publish one exact classic-script task");
        page_vm.vm_mut().eval(
            r#"
document.getElementById("classic-script-stale").srcdoc =
  "<!doctype html><body>replacement child</body>";
"replacement queued"
"#,
        )?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "replacement child navigation commit",
        )
        .await;

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__lmStaleChildClassicCheckpoint = [];
Promise.resolve().then(() => {
  __lmStaleChildClassicCheckpoint.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        page_vm
            .run_claimed_selected_page_task_for_test(retired, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmStaleChildClassicCheckpoint.join('|')"
                )?,
            "",
            "the retired classic-script task must not checkpoint the replacement root Window"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "the retired classic-script task must not advance replacement runtime residence"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "document.getElementById('classic-script-stale').contentDocument.body.textContent"
                )?,
            "replacement child",
            "the retired claim must not alter the replacement child Document"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildClassicScriptTaskBoundary.join('|')"
                )?,
            "",
            "the retired classic script body must never execute"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child classic-script boundary should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_classic_script_cleanup_leaves_runtime_followup_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-classic-script-body").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_classic_script_ready_fixture(&mut page_vm, "classic-script-body").await?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        let body = page_vm
            .run_page_child_document_script_ready_body_for_test()
            .await?
            .expect("the realm-bound classic script must retain one exact task body");
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
                    "__lmChildClassicScriptTaskBoundary.join('|')"
                )?,
            "script|microtask",
            "classic-script cleanup must run Promise reactions before returning the task body"
        );
        assert!(
            !has_ready_runtime_script_continuation_for_test(&page_vm),
            "script cleanup must leave runtime follow-up publication to selected task completion"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child classic script body boundary witness should run");
}
