use super::*;

use crate::page_task_queue::PageChildHostLoadTargetEffect;

async fn install_child_host_load_completion_fixture(
    page_vm: &mut PageVm,
    frame_id: &str,
) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  globalThis.__lmChildHostLoadTaskBoundary = [];
  const frame = document.createElement("iframe");
  frame.id = {frame_id:?};
  frame.onload = () => {{
    __lmChildHostLoadTaskBoundary.push("callback");
    Promise.resolve().then(() => {{
      __lmChildHostLoadTaskBoundary.push("microtask");
    }});
  }};
  frame.srcdoc = "<!doctype html><body>child</body>";
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;

    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child HostLoad fixture navigation commit",
    )
    .await;
    for label in [
        "interactive transition",
        "DOMContentLoaded transition",
        "complete transition",
    ] {
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            page_vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child HostLoad fixture {label}"),
        )
        .await;
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn child_host_load_body_cleans_up_listener_reactions() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-host-load-body").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_host_load_completion_fixture(&mut page_vm, "host-load-body").await?;

        let body = page_vm
            .run_page_child_host_load_body_for_test()
            .expect("completed child Document should leave one exact HostLoad body");
        assert!(matches!(
            body.action.target_effect,
            PageChildHostLoadTargetEffect::CallbackDispatchedToCurrentOwner
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildHostLoadTaskBoundary.join('|')"
                )?,
            "callback|microtask",
            "listener cleanup must drain reactions before the selected task completes"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child HostLoad body boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_host_load_body_reports_callback_before_child_replacement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-host-load-body-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_host_load_completion_fixture(&mut page_vm, "host-load-body-replacement")
            .await?;
        page_vm.vm_mut().eval(
            r#"
document.getElementById("host-load-body-replacement").onload = function () {
  __lmChildHostLoadTaskBoundary.push("callback");
  this.srcdoc = "<!doctype html><body>replacement from callback</body>";
  Promise.resolve().then(() => {
    __lmChildHostLoadTaskBoundary.push("microtask");
  });
};
"installed"
"#,
        )?;

        let body = page_vm
            .run_page_child_host_load_body_for_test()
            .expect("completed child Document should leave one exact HostLoad body");
        assert!(matches!(
            body.action.target_effect,
            PageChildHostLoadTargetEffect::CallbackDispatchedToCurrentOwner
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildHostLoadTaskBoundary.join('|')"
                )?,
            "callback|microtask",
            "replacement must not erase the fact that the callback body already ran"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child HostLoad replacement body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_host_load_completes_reactions_and_runtime_followup() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-host-load-selected").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_host_load_completion_fixture(&mut page_vm, "host-load-selected").await?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildHostLoad,
                    &loader,
                )
                .await?,
            "the exact HostLoad task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildHostLoadTaskBoundary.join('|')"
                )?,
            "callback|microtask",
            "the selected HostLoad task must include listener cleanup"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "selected HostLoad callback completion must publish its typed runtime-script follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child HostLoad selected completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_host_load_completes_callback_that_replaces_the_child() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-host-load-selected-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_host_load_completion_fixture(&mut page_vm, "host-load-selected-replacement")
            .await?;
        page_vm.vm_mut().eval(
            r#"
document.getElementById("host-load-selected-replacement").onload = function () {
  __lmChildHostLoadTaskBoundary.push("callback");
  this.srcdoc = "<!doctype html><body>replacement from selected callback</body>";
  Promise.resolve().then(() => {
    __lmChildHostLoadTaskBoundary.push("microtask");
    const sibling = document.createElement("iframe");
    sibling.id = "host-load-selected-reaction-child";
    sibling.srcdoc = "<!doctype html><body>reaction child</body>";
    document.body.appendChild(sibling);
  });
};
"installed"
"#,
        )?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildHostLoad,
                    &loader,
                )
                .await?,
            "the replacing HostLoad callback should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildHostLoadTaskBoundary.join('|')"
                )?,
            "callback|microtask"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "callback completion must reconcile replacement and reaction-created child records"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "a callback that replaces its child still publishes selected-task runtime follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child HostLoad replacement callback should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_host_load_syncs_a_microtask_created_child() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-host-load-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_host_load_completion_fixture(&mut page_vm, "host-load-child-source").await?;
        page_vm.vm_mut().eval(
            r#"
document.getElementById("host-load-child-source").onload = () => {
  __lmChildHostLoadTaskBoundary.push("callback");
  Promise.resolve().then(() => {
    __lmChildHostLoadTaskBoundary.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "host-load-microtask-child-result";
    frame.srcdoc = "<!doctype html><body>nested child</body>";
    document.body.appendChild(frame);
  });
};
"installed"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildHostLoad,
                    &loader,
                )
                .await?,
            "the exact HostLoad task should run through the selected dispatcher"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "HostLoad callback completion must synchronize the child created by its reaction"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildHostLoadTaskBoundary.join('|')"
                )?,
            "callback|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child HostLoad reaction-created child witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn claimed_old_child_host_load_does_not_complete_in_the_replacement_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-host-load-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_host_load_completion_fixture(&mut page_vm, "host-load-replacement").await?;

        let retired = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ChildHostLoad)
            .expect("the first child Document should publish one exact HostLoad");
        page_vm.vm_mut().eval(
            r#"
document.getElementById("host-load-replacement").srcdoc =
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
globalThis.__lmStaleChildHostLoadCheckpoint = [];
Promise.resolve().then(() => {
  __lmStaleChildHostLoadCheckpoint.push("microtask");
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
                    "__lmStaleChildHostLoadCheckpoint.join('|')"
                )?,
            "",
            "the retired HostLoad must not checkpoint the replacement root Window"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the retired HostLoad must not advance replacement runtime residence"
        );

        for label in [
            "replacement interactive transition",
            "replacement DOMContentLoaded transition",
            "replacement complete transition",
        ] {
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                label,
            )
            .await;
        }
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildHostLoad,
                    &loader,
                )
                .await?,
            "discarding the retired claim must not consume the replacement HostLoad"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildHostLoadTaskBoundary.join('|')"
                )?,
            "callback|microtask",
            "only the replacement HostLoad should dispatch the element callback"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child HostLoad replacement boundary should run");
}
