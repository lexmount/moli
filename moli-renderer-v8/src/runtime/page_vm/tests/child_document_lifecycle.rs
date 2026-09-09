use super::*;

async fn install_child_document_lifecycle_fixture(
    page_vm: &mut PageVm,
    frame_id: &str,
) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  globalThis.__lmChildDocumentLifecycleBoundary = [];
  const frame = document.createElement("iframe");
  frame.id = {frame_id:?};
  frame.srcdoc = "<!doctype html><body>child</body>";
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child lifecycle fixture navigation commit",
    )
    .await;
    run_expected_child_realm_materialization_for_wait(page_vm, "child lifecycle fixture realm")
        .await;
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  const frame = document.getElementById({frame_id:?});
  frame.contentDocument.addEventListener("readystatechange", () => {{
    const state = frame.contentDocument.readyState;
    __lmChildDocumentLifecycleBoundary.push("callback:" + state);
    Promise.resolve().then(() => {{
      __lmChildDocumentLifecycleBoundary.push("microtask:" + state);
    }});
  }});
  frame.contentDocument.addEventListener("DOMContentLoaded", () => {{
    __lmChildDocumentLifecycleBoundary.push("callback:dcl");
    Promise.resolve().then(() => {{
      __lmChildDocumentLifecycleBoundary.push("microtask:dcl");
    }});
  }});
  return "installed";
}})()
"#,
    ))?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn child_document_lifecycle_body_cleans_up_listener_reactions() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-document-lifecycle-body").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_document_lifecycle_fixture(&mut page_vm, "lifecycle-body").await?;

        let body = page_vm
            .run_page_child_document_lifecycle_body_for_test()
            .expect("the child Document should leave one interactive lifecycle body");
        assert!(matches!(
            body.action.target_effect,
            crate::page_task_queue::PageChildDocumentLifecycleTargetEffect::EventDispatchedToCurrentOwner
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildDocumentLifecycleBoundary.join('|')"
                )?,
            "callback:interactive|microtask:interactive",
            "listener cleanup must drain reactions before the selected task completes"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child Document lifecycle body boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_document_lifecycle_completes_each_event_reaction_and_runtime_followup() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-document-lifecycle-selected").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_document_lifecycle_fixture(&mut page_vm, "lifecycle-selected").await?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        for (index, expected) in [
            "callback:interactive|microtask:interactive",
            "callback:interactive|microtask:interactive|callback:dcl|microtask:dcl",
            "callback:interactive|microtask:interactive|callback:dcl|microtask:dcl|callback:complete|microtask:complete",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildDocumentLifecycle,
                        &loader,
                    )
                    .await?,
                "the next exact child lifecycle task should run through the selected dispatcher"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        "__lmChildDocumentLifecycleBoundary.join('|')"
                    )?,
                expected,
                "each lifecycle callback must finish its reactions before the next lifecycle task"
            );
            if index == 0 {
                assert!(
                    has_ready_runtime_script_continuation_for_test(&page_vm),
                    "the first selected lifecycle callback must publish its typed runtime-script follow-up"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child Document lifecycle completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_document_lifecycle_completes_callback_that_replaces_the_child() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-document-lifecycle-replacing-callback").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_document_lifecycle_fixture(&mut page_vm, "lifecycle-replacing-callback")
            .await?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.getElementById("lifecycle-replacing-callback");
  frame.contentDocument.addEventListener("readystatechange", () => {
    if (frame.contentDocument.readyState !== "interactive") return;
    frame.srcdoc = "<!doctype html><body>replacement from lifecycle callback</body>";
    Promise.resolve().then(() => {
      const sibling = document.createElement("iframe");
      sibling.id = "lifecycle-reaction-child";
      sibling.srcdoc = "<!doctype html><body>reaction child</body>";
      document.body.appendChild(sibling);
    });
  });
  return "installed";
})()
"#,
        )?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentLifecycle,
                    &loader,
                )
                .await?,
            "the replacing lifecycle callback should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildDocumentLifecycleBoundary.join('|')"
                )?,
            "callback:interactive|microtask:interactive",
            "replacement must not erase the fact that the lifecycle callback already ran"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "callback completion must reconcile replacement and reaction-created child records"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "a lifecycle callback that replaces its child still publishes its typed runtime follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected replacing child lifecycle callback should run");
}

#[tokio::test(flavor = "current_thread")]
async fn claimed_old_child_document_lifecycle_does_not_complete_in_replacement_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-document-lifecycle-stale").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_document_lifecycle_fixture(&mut page_vm, "lifecycle-stale").await?;

        let retired = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildDocumentLifecycle,
            )
            .expect("the first child Document should publish one exact lifecycle action");
        page_vm.vm_mut().eval(
            r#"
document.getElementById("lifecycle-stale").srcdoc =
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
globalThis.__lmStaleChildLifecycleCheckpoint = [];
Promise.resolve().then(() => {
  __lmStaleChildLifecycleCheckpoint.push("microtask");
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
                    "__lmStaleChildLifecycleCheckpoint.join('|')"
                )?,
            "",
            "the retired lifecycle action must not checkpoint the replacement root Window"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the retired lifecycle action must not advance replacement runtime residence"
        );

        run_expected_child_realm_materialization_for_wait(
            &mut page_vm,
            "replacement child lifecycle realm",
        )
        .await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentLifecycle,
                    &loader,
                )
                .await?,
            "discarding the retired claim must preserve the replacement lifecycle action"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmStaleChildLifecycleCheckpoint.join('|')"
                )?,
            "microtask",
            "only the replacement lifecycle action may complete replacement microtasks"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child Document lifecycle boundary should run");
}
