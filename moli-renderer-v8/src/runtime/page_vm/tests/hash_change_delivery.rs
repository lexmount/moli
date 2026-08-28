use super::*;

use crate::page_task_queue::{
    PageHashChangeDeliveryTargetEffect, RendererPageHashChangeDeliveryTask,
};

fn take_next_hash_change_task_for_authorization_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageHashChangeDeliveryTask> {
    let task = page_vm
        .take_dom_manipulation_body_task_for_test(PageDomManipulationTestFamily::HashChange)?;
    let crate::page_task_queue::RendererPageDomManipulationTask::HashChange(task) = task else {
        unreachable!("exact hashchange selection must preserve its task variant")
    };
    Some(task)
}

#[tokio::test(flavor = "current_thread")]
async fn hashchange_body_leaves_reactions_for_selected_callback_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/hashchange-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r##"
globalThis.__hashChangeBodyBoundary = [];
addEventListener("hashchange", event => {
  const fragment = event.newURL.split("#")[1];
  __hashChangeBodyBoundary.push("callback:" + fragment);
  Promise.resolve().then(() => {
    __hashChangeBodyBoundary.push("microtask:" + fragment);
  });
}, { once: true });
location.hash = "#body";
"queued"
"##,
        )?;

        let task = take_next_hash_change_task_for_authorization_test(&mut page_vm)
            .expect("one exact hashchange task should be ready");
        let body = page_vm.apply_selected_page_hash_change_delivery_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageHashChangeDeliveryTargetEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__hashChangeBodyBoundary.join('|')")?,
            "callback:body",
            "the body-only executor must leave Promise reactions pending"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__hashChangeBodyBoundary.join('|')")?,
            "callback:body|microtask:body",
            "the selected callback completion must own the single task checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("hashchange body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn hashchange_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/hashchange-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r##"
globalThis.__hashChangeChildOrder = [];
addEventListener("hashchange", () => {
  __hashChangeChildOrder.push("callback");
  Promise.resolve().then(() => {
    __hashChangeChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "hashchange-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
}, { once: true });
location.hash = "#child";
"queued"
"##,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::HashChange),
                    &loader,
                )
                .await?,
            "the exact hashchange task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__hashChangeChildOrder.join('|')")?,
            "callback|microtask",
            "the agent checkpoint must precede callback child-record synchronization"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during callback completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("hashchange post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn hashchange_completion_does_not_change_adjacent_dom_manipulation_variants() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/hashchange-shared-dom-source").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r##"
globalThis.__hashChangeSharedDomOrder = [];
globalThis.__hashChangeSharedBroadcastReceiver =
  new BroadcastChannel("hashchange-shared-dom-source");
__hashChangeSharedBroadcastReceiver.onmessage = () => {
  __hashChangeSharedDomOrder.push("broadcast");
  Promise.resolve().then(() => {
    __hashChangeSharedDomOrder.push("microtask:broadcast");
  });
};
globalThis.__hashChangeSharedBroadcastSender =
  new BroadcastChannel("hashchange-shared-dom-source");
__hashChangeSharedBroadcastSender.postMessage("first");

addEventListener("hashchange", () => {
  __hashChangeSharedDomOrder.push("hashchange");
  Promise.resolve().then(() => {
    __hashChangeSharedDomOrder.push("microtask:hashchange");
  });
}, { once: true });
location.hash = "#second";
"queued"
"##,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::BroadcastChannel
                    ),
                    &loader,
                )
                .await?,
            "the BroadcastChannel head should consume the first shared DOM turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__hashChangeSharedDomOrder.join('|')")?,
            "broadcast|microtask:broadcast",
            "the pre-existing BroadcastChannel completion must remain unchanged"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::HashChange
                    ),
                    &loader,
                )
                .await?,
            "hashchange should retain the second shared DOM turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__hashChangeSharedDomOrder.join('|')")?,
            "broadcast|microtask:broadcast|hashchange|microtask:hashchange",
            "hashchange must use its callback completion without draining another DOM task"
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "the two shared source tasks must consume exactly two selected turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("hashchange shared DOM source isolation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn hashchange_uses_one_dom_manipulation_turn_per_event_without_timer_driving() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/hashchange-one-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r##"
globalThis.__typedHashChanges = [];
addEventListener("hashchange", event => {
  const fragment = event.newURL.split("#")[1];
  __typedHashChanges.push("event:" + fragment);
  Promise.resolve().then(() => __typedHashChanges.push("microtask:" + fragment));
});
location.hash = "#one";
location.hash = "#two";
"queued"
"##,
        )?;

        // Fragment navigation may independently queue scroll/lifecycle work.
        // Consume the hashchange through its production typed source without
        // advancing any PageTimer, proving that timer readiness is not its
        // delivery mechanism.
        assert_eq!(page_vm.vm_mut().eval("__typedHashChanges.length")?, "0");

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::HashChange
                    ),
                    &loader,
                )
                .await?,
            "first hashchange should consume one selected typed turn"
        );

        assert_eq!(
            page_vm.vm_mut().eval("__typedHashChanges.join('|')")?,
            "event:one|microtask:one",
            "one selected task must dispatch one event and checkpoint its microtasks"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::HashChange
                    ),
                    &loader,
                )
                .await?,
            "second hashchange should remain queued"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedHashChanges.join('|')")?,
            "event:one|microtask:one|event:two|microtask:two"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::HashChange
                    ),
                    &loader,
                )
                .await?
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("hashchange one-turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn hashchange_retains_the_top_local_window_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/hashchange-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r##"
location.hash = "#queued";
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
globalThis.__documentOpenHashChanges = [];
addEventListener("hashchange", event => {
  __documentOpenHashChanges.push(event.oldURL + "->" + event.newURL);
});
"replaced"
"##,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id,
            after_document.local_window_id
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::HashChange
                    ),
                    &loader,
                )
                .await?,
            "old-Document hashchange should retain its LocalWindow target"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentOpenHashChanges.length.toString()")?,
            "1"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("hashchange document.open ownership test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn hashchange_discards_a_retired_child_local_window() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/hashchange-stale-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "stale-hashchange-recipient";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "stale-hashchange-recipient",
        )?;
        page_vm.vm_mut().eval(
            r##"
globalThis.__staleHashChanges = 0;
const staleFrame = document.getElementById("stale-hashchange-recipient");
staleFrame.contentWindow.addEventListener("hashchange", () => parent.__staleHashChanges++);
staleFrame.contentWindow.history.replaceState(null, "", "/child-hashchange");
staleFrame.contentWindow.location.hash = "#queued";
staleFrame.remove();
"retired"
"##,
        )?;

        let task = take_next_hash_change_task_for_authorization_test(&mut page_vm)
            .expect("stable hashchange task should settle through a stale turn");
        let outcome = page_vm.apply_selected_page_hash_change_delivery_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageHashChangeDeliveryTargetEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(page_vm.vm_mut().eval("String(__staleHashChanges)")?, "0");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("hashchange stale child test should run");
}

#[test]
fn hashchange_rejects_a_real_page_vm_replacement_local_window_collision() {
    run_page_vm_large_stack_async_test(
        "hashchange-page-vm-replacement-local-window-collision",
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
location.hash = "#retired";
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
                    page_vm.vm_mut().eval(
                        r##"
globalThis.__replacementHashChanges = [];
addEventListener("hashchange", event => {
  __replacementHashChanges.push(event.newURL.split("#")[1]);
});
location.hash = "#current";
"queued"
"##,
                    )?;

                    let stale_task =
                        take_next_hash_change_task_for_authorization_test(&mut page_vm)
                            .expect("retired PageVm hashchange should consume one stale turn");
                    let stale =
                        page_vm.apply_selected_page_hash_change_delivery_turn(stale_task)?;
                    let PageHashChangeDeliveryTargetEffect::DiscardedStaleOwner {
                        current_owner: Some(current_owner),
                    } = stale.action.target_effect
                    else {
                        panic!("retired hashchange should report replacement LocalWindow owner");
                    };
                    assert_ne!(stale.action.owner, current_owner);
                    assert_eq!(
                        stale.action.owner.target(),
                        current_owner.target(),
                        "fresh PageVm counters should naturally reuse the top LocalWindow identity"
                    );
                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::HashChange),
                                &loader,
                            )
                            .await?,
                        "replacement hashchange must survive stale-head discard"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementHashChanges.join('|')")?,
                        "current",
                        "retired root task must not dispatch into replacement PageVm"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("hashchange PageVm replacement should use exact owner arbiter");
            server
                .await
                .expect("hashchange PageVm replacement server should finish");
        },
    );
}
