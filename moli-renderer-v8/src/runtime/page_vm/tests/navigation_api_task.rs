use super::*;

use crate::{
    page_task_queue::PageNavigationApiTaskTargetEffect,
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
};

#[tokio::test(flavor = "current_thread")]
async fn navigation_api_task_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/navigation-api-body-completion-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r##"
globalThis.__navigationApiBodyBoundary = [];
navigation.onnavigatesuccess = () => {
  __navigationApiBodyBoundary.push("success");
  Promise.resolve().then(() => {
    __navigationApiBodyBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__navigationApiBodyBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
};
navigation.navigate("/next-document");
navigation.navigate("#replacement");
"queued"
"##,
        )?;
        let task = page_vm
            .take_navigation_api_body_task_for_test()
            .expect("one exact Navigation API FinishResult task should be ready");
        let outcome = page_vm.apply_selected_page_navigation_api_task_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageNavigationApiTaskTargetEffect::FinishResultAppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__navigationApiBodyBoundary.join('|')",
                )?,
            "success|microtask|runtime-script",
            "listener cleanup must drain reactions before the selected task completes"
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CallbackCompletion));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__navigationApiBodyBoundary.join('|')")?,
            "success|microtask|runtime-script",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Navigation API task body/completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_cross_document_followup_uses_selected_navigation_task_not_timer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/navigation-api-task-not-timer").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        let setup = page_vm.vm_mut().eval(
            r##"
(() => {
  globalThis.__lmNavigationTaskFinishedOrder = [];
  navigation.onnavigatesuccess = () => {
    __lmNavigationTaskFinishedOrder.push("success");
  };
  navigation.navigate("/next-document");
  const result = navigation.navigate("#replacement");
  result.committed.then(() => __lmNavigationTaskFinishedOrder.push("committed"));
  result.finished.then(() => __lmNavigationTaskFinishedOrder.push("finished"));
  Promise.resolve().then(() => __lmNavigationTaskFinishedOrder.push("microtask"));
  return `${location.hash}:${__lmNavigationTaskFinishedOrder.join("|")}`;
})()
"##,
        )?;

        assert_eq!(setup, "#replacement:");
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "cross-document unload lifecycle must not be admitted through PageTimer"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__lmNavigationTaskFinishedOrder.join('|')")?,
            "committed|microtask"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::NavigationApi,
                    &loader,
                )
                .await?,
            "Navigation API task-finished turn must use the production selected dispatcher"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::NavigationApi,
                    &loader,
                )
                .await?,
            "one local payload must produce exactly one scheduler task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__lmNavigationTaskFinishedOrder.join('|')")?,
            "committed|microtask|success|finished"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Navigation API selected-task witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn navigation_api_completion_reconciles_success_listener_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/navigation-api-listener-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let root_document = page_vm.document_lifecycle.identity().document;
        let retired_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial exact Document owner");

        page_vm.vm_mut().eval(
            r##"
globalThis.__navigationApiDocumentOpenBoundary = [];
navigation.onnavigatesuccess = () => {
  __navigationApiDocumentOpenBoundary.push("success");
  document.open();
  document.write("<!doctype html><body>replacement</body>");
  document.close();
  Promise.resolve().then(() => {
    __navigationApiDocumentOpenBoundary.push("microtask");
  });
};
navigation.navigate("/next-document");
navigation.navigate("#replacement");
"queued"
"##,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::NavigationApi,
                    &loader
                )
                .await?,
            "FinishResult must return through the production selected dispatcher"
        );

        let current_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.open replacement owner");
        assert_ne!(current_document_owner, retired_document_owner);
        assert_eq!(
            page_vm.document_lifecycle.identity().document,
            root_document,
            "document.open must rotate the ScriptVm Document without replacing the Page root"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__navigationApiDocumentOpenBoundary.join('|')")?,
            "success|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Navigation API listener document.open completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_navigation_api_task_discards_its_stale_ticket_without_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/navigation-api-inactive-attempt").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r##"
globalThis.__navigationApiStaleTicketLog = [];
navigation.onnavigatesuccess = () => {
  __navigationApiStaleTicketLog.push("success:" + location.hash);
};
navigation.onnavigateerror = () => {
  __navigationApiStaleTicketLog.push("error:" + location.hash);
};
navigation.navigate("/retired-cross-document");
const retiredResult = navigation.navigate("#retired");
retiredResult.finished.catch(() => {});
"queued"
"##,
        )?;
        page_vm.vm_mut().eval(
            r##"
navigation.navigate("/current-cross-document");
navigation.navigate("#current");
"queued"
"##,
        )?;
        let before_stale_turn = page_vm
            .vm_mut()
            .eval("globalThis.__navigationApiStaleTicketLog.join('|')")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let retired = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::NavigationApi)
            .expect("the source must retain the task for the retired attempt");
        page_vm
            .run_claimed_selected_page_task_for_test(retired, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__navigationApiStaleTicketLog.join('|')")?,
            before_stale_turn,
            "the stale selected turn must not publish another Navigation event"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "a canceled task's stale source ticket must not enter V8 or borrow runtime follow-up authority"
        );

        let current = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::NavigationApi)
            .expect("the current attempt must remain behind the retired task in source FIFO order");
        page_vm
            .run_claimed_selected_page_task_for_test(current, &loader)
            .await?;
        let expected_after_current = if before_stale_turn.is_empty() {
            "success:#current".to_owned()
        } else {
            format!("{before_stale_turn}|success:#current")
        };
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__navigationApiStaleTicketLog.join('|')")?,
            expected_after_current,
            "only the current attempt may publish a Navigation success callback"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::NavigationApi,
                )
                .is_none(),
            "both exact Navigation API tasks must settle once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("canceled Navigation API stale-ticket test should run");
}

#[test]
fn navigation_api_task_rejects_a_real_page_vm_replacement_id_collision() {
    run_page_vm_large_stack_async_test(
        "navigation-api-task-page-vm-replacement-id-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    page_vm.vm_mut().eval(
                        r##"
navigation.navigate("/retired-cross-document");
const retiredResult = navigation.navigate("#retired");
retiredResult.finished.catch(() => {});
"queued"
"##,
                    )?;
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
                    page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
                    page_vm.vm_mut().eval(
                        r##"
globalThis.__replacementNavigationApiTaskLog = [];
navigation.onnavigatesuccess = () => {
  __replacementNavigationApiTaskLog.push("success:" + location.hash);
};
navigation.navigate("/current-cross-document");
const currentResult = navigation.navigate("#current");
currentResult.finished.then(
  () => __replacementNavigationApiTaskLog.push("finished"),
  error => __replacementNavigationApiTaskLog.push("error:" + error.name),
);
"queued"
"##,
                    )?;

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::NavigationApi,
                        )
                        .expect("the retired PageVm Navigation API task should settle as stale");
                    let (stale_owner, stale_task_id) = stale
                        .navigation_api_owner_and_task_id()
                        .expect("exact Navigation API claim must retain its identity");
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .runtime_script_work()
                            .dynamic_scripts.pending_source_load_count_for_test(),
                        1,
                        "retired Navigation API task must not checkpoint or advance replacement runtime work"
                    );

                    let current = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::NavigationApi,
                        )
                        .expect("replacement task must survive the stale local-id collision");
                    let (current_owner, current_task_id) = current
                        .navigation_api_owner_and_task_id()
                        .expect("replacement Navigation API claim must retain its identity");
                    assert_ne!(stale_owner, current_owner);
                    assert_eq!(
                        stale_owner.target(),
                        current_owner.target(),
                        "fresh PageVm counters should naturally reuse the top Window target"
                    );
                    assert_eq!(
                        current_task_id, stale_task_id,
                        "fresh JsContextHost counters should naturally reuse the local task id"
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(current, &loader)
                        .await?;
                    assert!(
                        page_vm
                            .claim_exact_selected_page_task_for_test(
                                PageSelectedTaskTestSelector::NavigationApi,
                            )
                            .is_none(),
                        "the two colliding tasks must each consume exactly one shared-source position"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementNavigationApiTaskLog.join('|')")?,
                        "success:#current|finished",
                        "discarding the old root task must not remove or execute the replacement Host payload"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("Navigation API PageVm replacement should use exact root arbitration");
            server
                .await
                .expect("Navigation API replacement server should finish");
        },
    );
}
