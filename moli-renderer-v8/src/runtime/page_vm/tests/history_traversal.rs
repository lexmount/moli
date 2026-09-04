use super::*;

use crate::{
    page_task_queue::PageHistoryTraversalTargetEffect,
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
};

#[tokio::test(flavor = "current_thread")]
async fn user_agent_popstate_is_trusted_but_author_constructed_event_is_not() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/popstate-event-trust").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r##"
history.pushState(null, "", "#one");
globalThis.__popStateTrust = null;
addEventListener("popstate", event => {
  __popStateTrust = JSON.stringify([
    event.isTrusted,
    event instanceof PopStateEvent,
    new PopStateEvent("popstate").isTrusted
  ]);
}, { once: true });
history.back();
"queued"
"##,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::HistoryTraversal,
                    &loader
                )
                .await?,
            "one exact history traversal should dispatch popstate"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__popStateTrust")?,
            "[true,true,false]"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("popstate trust test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn history_traversal_body_leaves_reaction_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/history-body-completion-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r##"
history.pushState(null, "", "#one");
globalThis.__historyBodyBoundary = [];
navigation.addEventListener("navigate", event => {
  if (event.navigationType !== "traverse") {
    return;
  }
  __historyBodyBoundary.push("navigate");
  Promise.resolve().then(() => {
    __historyBodyBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__historyBodyBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
  event.preventDefault();
});
history.back();
"queued"
"##,
        )?;
        let task = page_vm
            .take_history_traversal_body_task_for_test()
            .expect("one exact history traversal should be ready");
        let outcome = page_vm.apply_selected_page_history_traversal_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageHistoryTraversalTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__historyBodyBoundary.join('|')",
                )?,
            "navigate",
            "the traversal body must leave its Promise reaction for selected-task completion"
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CallbackCompletion));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__historyBodyBoundary.join('|')")?,
            "navigate|microtask|runtime-script",
            "central callback completion must own the reaction and its runtime-script follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("history traversal body/completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn history_back_coalesces_into_one_typed_turn_and_never_enters_page_timer() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/history-typed-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r##"
history.pushState(null, "", "#one");
history.pushState(null, "", "#two");
location.hash
"##,
        )?;
        page_vm
            .vm_mut()
            .advance_timers_until_deadline_for_test(&loader)
            .await?;
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "entry-creation fixture timers should be drained before traversal admission"
        );

        let queued = page_vm.vm_mut().eval(
            r##"
globalThis.__historyTurnLog = [];
addEventListener("popstate", () => {
  __historyTurnLog.push("popstate:" + location.hash);
  Promise.resolve().then(() => __historyTurnLog.push("microtask:" + location.hash));
});
history.back();
history.back();
location.hash
"##,
        )?;
        assert_eq!(queued, "#two");
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "history traversal admission must not manufacture a PageTimer descriptor"
        );
        assert_eq!(
            page_vm.vm().ms_to_next_timeout(),
            None
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::HistoryTraversal, &loader)
                .await?,
            "the coalesced traversal should consume one production selected task"
        );
        assert_eq!(page_vm.vm_mut().eval("location.hash")?, "");
        assert_eq!(
            page_vm.vm_mut().eval("__historyTurnLog.join('|')")?,
            "popstate:|microtask:",
            "the selected traversal must checkpoint its event microtasks before the next turn"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::HistoryTraversal, &loader)
                .await?,
            "two pending history.back() calls for one LocalWindow must coalesce into one source position"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed history traversal test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn history_traversal_retains_its_local_window_and_realm_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/history-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r##"
history.pushState(null, "", "#queued");
history.back();
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"replaced"
"##,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.open replacement owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id,
            after_document.local_window_id
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::HistoryTraversal,
                    &loader
                )
                .await?,
            "the retained LocalWindow traversal should remain schedulable"
        );
        assert_eq!(page_vm.vm_mut().eval("location.hash")?, "");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("history traversal document.open ownership test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn history_traversal_completion_reconciles_listener_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/history-listener-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let root_document = page_vm.document_lifecycle.identity().document;
        let retired_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial exact Document owner");

        page_vm.vm_mut().eval(
            r##"
history.pushState(null, "", "#one");
globalThis.__historyDocumentOpenBoundary = [];
navigation.addEventListener("navigate", event => {
  if (event.navigationType !== "traverse") {
    return;
  }
  event.preventDefault();
  __historyDocumentOpenBoundary.push("callback");
  document.open();
  document.write("<!doctype html><body>replacement</body>");
  document.close();
  Promise.resolve().then(() => {
    __historyDocumentOpenBoundary.push("microtask");
  });
});
history.back();
"queued"
"##,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::HistoryTraversal,
                    &loader
                )
                .await?,
            "listener replacement must return through the production selected dispatcher"
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
                .eval("globalThis.__historyDocumentOpenBoundary.join('|')")?,
            "callback|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("history traversal listener document.open completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn history_traversal_discards_a_retired_child_local_window() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/history-stale-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r##"
const frame = document.createElement("iframe");
frame.id = "history-stale-child";
frame.srcdoc = "<!doctype html><p>child</p>";
document.body.appendChild(frame);
"created"
"##,
        )?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "history stale-child srcdoc commit",
        )
        .await;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "history-stale-child")?;
        page_vm.vm_mut().eval(
            r##"
globalThis.__retiredChildHistoryEvents = 0;
const child = document.getElementById("history-stale-child").contentWindow;
child.addEventListener("popstate", () => {
  parent.__retiredChildHistoryEvents += 1;
});
child.history.pushState(null, "", "about:srcdoc#queued");
child.history.back();
document.getElementById("history-stale-child").remove();
"retired"
"##,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let stale = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::HistoryTraversal)
            .expect("the stale child traversal must settle through one explicit turn");
        let (stale_owner, _) = stale
            .history_traversal_owner_and_task_id()
            .expect("the exact selector must retain the child traversal identity");
        assert_eq!(
            stale_owner.root_document(),
            page_vm.document_lifecycle.identity().document
        );
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__retiredChildHistoryEvents)")?,
            "0",
            "the retired LocalWindow must not receive popstate"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a stale LocalWindow traversal must not checkpoint or consume unrelated runtime work"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::HistoryTraversal,
                )
                .is_none(),
            "stale settlement must retire the local pending payload"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("history traversal stale child test should run");
}

#[test]
fn history_traversal_rejects_a_real_page_vm_replacement_id_collision() {
    run_page_vm_large_stack_async_test(
        "history-traversal-page-vm-replacement-id-collision",
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
                        r##"history.pushState(null, "", "#retired"); "created""##,
                    )?;
                    page_vm
                        .vm_mut()
                        .advance_timers_until_deadline_for_test(&loader)
                        .await?;
                    page_vm.vm_mut().eval("history.back(); 'queued'")?;
                    let retired_root = page_vm.document_lifecycle.identity().document;

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'navigating'"))?;
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
                        r##"history.pushState(null, "", "#current"); "created""##,
                    )?;
                    page_vm
                        .vm_mut()
                        .advance_timers_until_deadline_for_test(&loader)
                        .await?;
                    page_vm.vm_mut().eval("history.back(); 'queued'")?;

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::HistoryTraversal,
                        )
                        .expect("the retired PageVm traversal should consume one stale turn");
                    let (stale_owner, stale_task_id) = stale
                        .history_traversal_owner_and_task_id()
                        .expect("the exact selector must retain the retired traversal identity");
                    assert_eq!(
                        stale_owner.root_document(),
                        retired_root,
                        "the first selected traversal must remain bound to the retired PageVm"
                    );
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
                        "retired traversal must not checkpoint or advance replacement runtime work"
                    );

                    let current = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::HistoryTraversal,
                        )
                        .expect("the replacement traversal must survive stale-head settlement");
                    let (current_owner, current_task_id) = current
                        .history_traversal_owner_and_task_id()
                        .expect("the exact selector must retain the replacement traversal identity");
                    assert_eq!(
                        current_owner.root_document(),
                        current_root,
                        "the second selected traversal must belong to the replacement PageVm"
                    );
                    assert_ne!(stale_owner, current_owner);
                    assert_eq!(
                        stale_owner.target(),
                        current_owner.target(),
                        "fresh PageVm counters should naturally reuse the top LocalWindow identity"
                    );
                    assert_eq!(
                        stale_task_id, current_task_id,
                        "fresh PageVm-local ledgers should reuse the traversal task id"
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(current, &loader)
                        .await?;
                    assert_eq!(page_vm.vm_mut().eval("location.hash")?, "");
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("history traversal replacement should use exact root arbitration");
            server
                .await
                .expect("history traversal replacement server should finish");
        },
    );
}
