use super::*;

#[test]
fn iframe_javascript_attribute_navigation_completes_the_parent_load_once() {
    run_page_vm_large_stack_async_test("iframe-javascript-attribute-load", || async move {
        for (before_insertion, javascript_url, expected) in [
            (
                false,
                "javascript:parent.__javascriptAttributeLoadOrder.push('javascript'); void 0",
                "frame-load|javascript|parent-load",
            ),
            (
                true,
                "javascript:parent.__javascriptAttributeLoadOrder.push('javascript'); void 0",
                "javascript|frame-load|parent-load",
            ),
            (
                false,
                "javascript:throw new Error('boom')",
                "frame-load|parent-load",
            ),
            (
                true,
                "javascript:throw new Error('boom')",
                "frame-load|parent-load",
            ),
            (
                false,
                r#"javascript:{"foo": "bar"}"#,
                "frame-load|parent-load",
            ),
            (
                true,
                r#"javascript:{"foo": "bar"}"#,
                "frame-load|parent-load",
            ),
        ] {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();
            local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
globalThis.__javascriptAttributeLoadOrder = [];
addEventListener('load', () => __javascriptAttributeLoadOrder.push('parent-load'));
const frame = document.createElement('iframe');
frame.onload = () => __javascriptAttributeLoadOrder.push('frame-load');
const source = {javascript_url:?};
if ({before_insertion}) frame.src = source;
document.body.appendChild(frame);
if (!{before_insertion}) frame.src = source;
"#,
                    ))?;
                    // Drive the real lifecycle owner: initial about:blank and
                    // javascript navigation can have separate commit tasks.
                    // The assertions below retain the browser-visible load order.
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while page_vm
                            .run_next_child_frame_task_source_for_semantic_test()
                            .await
                            .is_some()
                        {}
                    })
                    .await
                    .expect("initial iframe realm and script tasks should finish");
                    let outcome = tokio::time::timeout(
                        Duration::from_secs(2),
                        page_vm.finish_post_parse_execution_on_named_owner_lane(
                            Vec::new(),
                            PageVmInitStage::Load,
                            Instant::now(),
                        ),
                    )
                    .await
                    .expect("javascript URL without a replacement must release the parent load")?;
                    let PageVmNavigationTurnOutcome::Completed(mut page_vm) = outcome else {
                        panic!("javascript URL must retain the current parent document");
                    };
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__javascriptAttributeLoadOrder.join('|')")?,
                        expected,
                        "the initial iframe load must finish once, before the parent load: {javascript_url:?}, before_insertion={before_insertion}"
                    );
                    assert_eq!(page_vm.vm_mut().eval("document.readyState")?, "complete");
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("javascript attribute navigation should complete its parent load");
        }
    });
}

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
  frame.srcdoc = `<script>
    parent.__lmChildClassicScriptTaskBoundary.push("script");
    Promise.resolve().then(() => {{
      parent.__lmChildClassicScriptTaskBoundary.push("microtask");
      const sibling = parent.document.createElement("iframe");
      sibling.id = "classic-script-reaction-child";
      sibling.srcdoc = "<!doctype html><body>reaction child</body>";
      parent.document.body.appendChild(sibling);
    }});
  <\/script>`;
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child parser classic script fixture navigation",
    )
    .await;
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
            "selected classic-script completion must own the Promise-reaction checkpoint"
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
async fn child_classic_script_ready_body_leaves_reactions_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-classic-script-body").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_classic_script_ready_fixture(&mut page_vm, "classic-script-body").await?;

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
            "script",
            "the classic-script body must leave Promise reactions pending for selected completion"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child classic script body boundary witness should run");
}
