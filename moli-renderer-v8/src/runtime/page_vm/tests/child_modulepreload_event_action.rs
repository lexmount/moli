use super::*;

use crate::page_task_queue::{
    PageChildModulepreloadEventActionTargetEffect, RendererOwnerWakeSource,
};
use crate::runtime::PageTaskCompletion;

async fn install_child_modulepreload_event_fixture(
    page_vm: &mut PageVm,
    frame_id: &str,
) -> anyhow::Result<crate::document_runtime::DomHandle> {
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  globalThis.__lmChildModulepreloadEventActions = [];
  const frame = document.createElement("iframe");
  frame.id = {frame_id:?};
  frame.srcdoc = `
    <script>parent.__lmChildModulepreloadEventActions.push("before");<\/script>
    <link rel="modulepreload"
          href="/invalid-modulepreload.bin"
          as="image"
          onerror="parent.__lmChildModulepreloadEventActions.push('preload-error')">
    <script>parent.__lmChildModulepreloadEventActions.push("after");<\/script>
  `;
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(frame_id)
        .expect("modulepreload event fixture should retain its iframe handle");

    let mut observed_sources = Vec::new();
    for _ in 0..12 {
        let Some(source) = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
        else {
            break;
        };
        observed_sources.push(source);
    }
    anyhow::ensure!(
        page_vm
            .vm_mut()
            .eval("__lmChildModulepreloadEventActions.join('|')")?
            == "before|after",
        "fixture setup must not dispatch the typed modulepreload event inline; child sources: {observed_sources:?}"
    );
    Ok(child_handle)
}

async fn install_child_modulepreload_callback_completion_fixture(
    page_vm: &mut PageVm,
    frame_id: &str,
) -> anyhow::Result<crate::document_runtime::DomHandle> {
    let child_handle = install_child_modulepreload_event_fixture(page_vm, frame_id).await?;
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  globalThis.__lmChildModulepreloadTaskBoundary = [];
  const frame = document.getElementById({frame_id:?});
  const link = frame.contentDocument.querySelector('link[rel="modulepreload"]');
  link.onerror = () => {{
    __lmChildModulepreloadTaskBoundary.push("callback");
    Promise.resolve().then(() => {{
      __lmChildModulepreloadTaskBoundary.push("microtask");
    }});
  }};
  return "installed";
}})()
"#,
    ))?;
    Ok(child_handle)
}

fn take_child_modulepreload_event_action_body_task(
    page_vm: &mut PageVm,
) -> crate::page_task_queue::RendererPageChildModulepreloadEventActionTask {
    let task_sources = page_vm.page_task_executor_sources_for_test();
    task_sources
        .take_child_modulepreload_event_action_for_executor_test(|owner| {
            page_vm.page_child_modulepreload_event_action_is_eligible_for_owner_turn(owner)
        })
        .expect("real child modulepreload producer should leave one typed event action")
}

#[tokio::test(flavor = "current_thread")]
async fn child_modulepreload_event_body_cleans_up_listener_reactions() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-modulepreload-task-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_modulepreload_callback_completion_fixture(
            &mut page_vm,
            "modulepreload-task-boundary",
        )
        .await?;

        let task = take_child_modulepreload_event_action_body_task(&mut page_vm);
        let body = page_vm.apply_selected_page_child_modulepreload_event_action_turn(task);
        assert!(matches!(
            body.action.target_effect,
            PageChildModulepreloadEventActionTargetEffect::AppliedToCurrentOwner {
                outcome
            } if outcome.event_was_dispatched()
        ));
        assert!(matches!(
            body.action.into_page_task_completion(),
            PageTaskCompletion::CallbackCompletion
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildModulepreloadTaskBoundary.join('|')"
                )?,
            "callback|microtask",
            "listener cleanup must drain reactions before the selected task completes"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child modulepreload body boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_modulepreload_event_completes_reactions_and_runtime_followup() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-modulepreload-selected-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_modulepreload_callback_completion_fixture(
            &mut page_vm,
            "modulepreload-selected-completion",
        )
        .await?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildModulepreloadEventAction,
                    &loader,
                )
                .await?,
            "the exact modulepreload event action should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildModulepreloadTaskBoundary.join('|')"
                )?,
            "callback|microtask",
            "the selected modulepreload task must include listener cleanup"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "selected callback completion must publish its typed runtime-script follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child modulepreload selected completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_modulepreload_event_syncs_a_microtask_created_child() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-modulepreload-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_child_modulepreload_event_fixture(
            &mut page_vm,
            "modulepreload-microtask-child-source",
        )
        .await?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__lmChildModulepreloadChildOrder = [];
document
  .getElementById("modulepreload-microtask-child-source")
  .contentDocument
  .querySelector('link[rel="modulepreload"]')
  .onerror = () => {
    __lmChildModulepreloadChildOrder.push("callback");
    Promise.resolve().then(() => {
      __lmChildModulepreloadChildOrder.push("microtask");
      const frame = document.createElement("iframe");
      frame.id = "modulepreload-microtask-child-result";
      frame.srcdoc = "<!doctype html><body>child</body>";
      document.body.appendChild(frame);
    });
  };
"installed"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildModulepreloadEventAction,
                    &loader,
                )
                .await?,
            "the exact modulepreload action should run through the selected dispatcher"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "callback completion must synchronize the child created by its Promise reaction"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__lmChildModulepreloadChildOrder.join('|')"
                )?,
            "callback|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child modulepreload callback child-sync witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn replaced_realm_discards_real_modulepreload_event_without_lifecycle_side_effects() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-modulepreload-stale-realm").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        let child_handle =
            install_child_modulepreload_event_fixture(&mut page_vm, "modulepreload-realm").await?;
        let event_wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .filter(|wake| {
                wake.source_for_test() == RendererOwnerWakeSource::ChildModulepreloadEventAction
            })
            .count();
        assert_eq!(
            event_wakes, 1,
            "the real producer should publish one empty-to-nonempty event-source wake"
        );

        let retired_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("modulepreload producer should capture the current exact realm");
        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "modulepreload-realm")?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("replacement realm should expose its exact target");
        assert_eq!(
            retired_target.task_owner(),
            current_target.task_owner(),
            "realm replacement must retain the child Document owner"
        );
        assert_ne!(
            retired_target.realm_id(),
            current_target.realm_id(),
            "realm replacement must invalidate the producer-captured realm"
        );

        let task = take_child_modulepreload_event_action_body_task(&mut page_vm);

        let outcome = page_vm.apply_selected_page_child_modulepreload_event_action_turn(task);
        assert_eq!(
            outcome.action.target_effect,
            PageChildModulepreloadEventActionTargetEffect::DiscardedStaleOwner {
                current_owner: None,
            },
            "a stale realm must discard its identity-only modulepreload event"
        );
        assert!(matches!(
            outcome.action.into_page_task_completion(),
            PageTaskCompletion::NoCompletion
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__lmChildModulepreloadEventActions.join('|')")?,
            "before|after",
            "the old realm event must not enter the replacement realm"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child modulepreload realm must be isolated");
}

#[test]
fn page_vm_replacement_rejects_naturally_colliding_modulepreload_event_owner() {
    run_page_vm_large_stack_async_test(
        "child-modulepreload-event-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><html><head></head><body></body></html>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let retired_child = install_child_modulepreload_event_fixture(
                        &mut page_vm,
                        "modulepreload-collision",
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

                    let current_child = install_child_modulepreload_event_fixture(
                        &mut page_vm,
                        "modulepreload-collision",
                    )
                    .await?;
                    assert_eq!(
                        retired_child, current_child,
                        "fresh PageVm DOM allocation should naturally reproduce the child handle"
                    );

                    let retired_task =
                        take_child_modulepreload_event_action_body_task(&mut page_vm);
                    let retired_owner = retired_task.owner();
                    assert_eq!(retired_owner.root_document(), retired_root);
                    assert!(
                        page_vm
                            .page_task_executor_sources_for_test()
                            .has_resident_task(),
                        "the replacement action must remain behind the stale source head"
                    );
                    let stale = page_vm
                        .apply_selected_page_child_modulepreload_event_action_turn(retired_task);
                    assert_eq!(
                        stale.action.target_effect,
                        PageChildModulepreloadEventActionTargetEffect::DiscardedStaleOwner {
                            current_owner: None,
                        },
                        "an old root token must prevent all replacement-Document application"
                    );
                    assert!(matches!(
                        stale.action.into_page_task_completion(),
                        PageTaskCompletion::NoCompletion
                    ));

                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__lmChildModulepreloadEventActions.join('|')")?,
                        "before|after",
                        "the retired event must not dispatch through colliding local IDs"
                    );

                    let current_task =
                        take_child_modulepreload_event_action_body_task(&mut page_vm);
                    let current_owner = current_task.owner();
                    assert_eq!(current_owner.root_document(), current_root);
                    assert_eq!(
                        retired_owner.document_owner(),
                        current_owner.document_owner(),
                        "fresh PageVm local owner counters should naturally collide"
                    );
                    assert_eq!(
                        retired_owner.realm_id(),
                        current_owner.realm_id(),
                        "fresh PageVm realm counters should naturally collide"
                    );
                    let current = page_vm
                        .apply_selected_page_child_modulepreload_event_action_turn(current_task);
                    assert!(matches!(
                        current.action.target_effect,
                        PageChildModulepreloadEventActionTargetEffect::AppliedToCurrentOwner {
                            outcome
                        } if outcome.event_was_dispatched()
                    ));
                    assert!(matches!(
                        current.action.into_page_task_completion(),
                        PageTaskCompletion::CallbackCompletion
                    ));
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__lmChildModulepreloadEventActions.join('|')")?,
                        "before|after|preload-error",
                        "the replacement action must remain intact after stale-head discard"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("PageVm replacement event actions should use the exact owner arbiter");
            server
                .await
                .expect("PageVm replacement response server should finish");
        },
    );
}
