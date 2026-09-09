use super::*;

use crate::page_task_queue::{
    PageStorageEventDeliveryTargetEffect, RendererPageStorageEventDeliveryTask,
};

fn take_next_storage_event_task_for_authorization_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageStorageEventDeliveryTask> {
    let task = page_vm
        .take_dom_manipulation_body_task_for_test(PageDomManipulationTestFamily::StorageEvent)?;
    let crate::page_task_queue::RendererPageDomManipulationTask::StorageEvent(task) = task else {
        unreachable!("exact StorageEvent selection must preserve its task variant")
    };
    Some(task)
}

#[tokio::test(flavor = "current_thread")]
async fn storage_event_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/storage-event-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const source = document.createElement("iframe");
source.id = "storage-event-body-source";
document.body.appendChild(source);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "storage-event-body-source",
        )?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__storageEventBodyBoundary = [];
addEventListener("storage", event => {
  __storageEventBodyBoundary.push("callback:" + event.newValue);
  Promise.resolve().then(() => {
    __storageEventBodyBoundary.push("microtask:" + event.newValue);
  });
}, { once: true });
document.getElementById("storage-event-body-source").contentWindow.localStorage
  .setItem("storage-event-body-key", "one");
"queued"
"#,
        )?;

        let task = take_next_storage_event_task_for_authorization_test(&mut page_vm)
            .expect("one exact StorageEvent task should be ready");
        let body = page_vm.apply_selected_page_storage_event_delivery_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageStorageEventDeliveryTargetEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__storageEventBodyBoundary.join('|')")?,
            "callback:one|microtask:one",
            "listener cleanup must drain reactions before the selected task completes"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__storageEventBodyBoundary.join('|')")?,
            "callback:one|microtask:one",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("StorageEvent body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn storage_event_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/storage-event-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const source = document.createElement("iframe");
source.id = "storage-event-child-source";
document.body.appendChild(source);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "storage-event-child-source",
        )?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__storageEventChildOrder = [];
addEventListener("storage", () => {
  __storageEventChildOrder.push("callback");
  Promise.resolve().then(() => {
    __storageEventChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "storage-event-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
}, { once: true });
document.getElementById("storage-event-child-source").contentWindow.localStorage
  .setItem("storage-event-child-key", "one");
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::StorageEvent),
                    &loader,
                )
                .await?,
            "the exact StorageEvent task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__storageEventChildOrder.join('|')")?,
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
    .expect("StorageEvent post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn storage_event_completion_does_not_change_adjacent_dom_manipulation_variants() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/storage-event-shared-dom-source").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const source = document.createElement("iframe");
source.id = "shared-dom-storage-source";
document.body.appendChild(source);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "shared-dom-storage-source",
        )?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__sharedDomCallbackOrder = [];
globalThis.__sharedDomBroadcastReceiver = new BroadcastChannel("shared-dom-source");
__sharedDomBroadcastReceiver.onmessage = () => {
  __sharedDomCallbackOrder.push("broadcast");
  Promise.resolve().then(() => __sharedDomCallbackOrder.push("microtask:broadcast"));
};
globalThis.__sharedDomBroadcastSender = new BroadcastChannel("shared-dom-source");
__sharedDomBroadcastSender.postMessage("first");

addEventListener("storage", () => {
  __sharedDomCallbackOrder.push("storage");
  Promise.resolve().then(() => __sharedDomCallbackOrder.push("microtask:storage"));
}, { once: true });
document.getElementById("shared-dom-storage-source").contentWindow.localStorage
  .setItem("shared-dom-storage-key", "second");
"queued"
"#,
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
                .eval("__sharedDomCallbackOrder.join('|')")?,
            "broadcast|microtask:broadcast",
            "the pre-existing BroadcastChannel completion must remain unchanged"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::StorageEvent
                    ),
                    &loader,
                )
                .await?,
            "StorageEvent should retain the second shared DOM turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__sharedDomCallbackOrder.join('|')")?,
            "broadcast|microtask:broadcast|storage|microtask:storage",
            "StorageEvent must use its callback completion without draining another DOM task"
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "the two shared source tasks must consume exactly two selected turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("StorageEvent shared DOM source isolation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn storage_events_use_typed_one_recipient_turns_and_never_enter_page_timer() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/storage-event-owner-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
for (const id of ["storage-recipient-a", "storage-recipient-b"]) {
  const frame = document.createElement("iframe");
  frame.id = id;
  document.body.appendChild(frame);
}
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "storage-recipient-a",
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "storage-recipient-b",
        )?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__typedStorageEvents = [];
for (const id of ["storage-recipient-a", "storage-recipient-b"]) {
  document.getElementById(id).contentWindow.addEventListener("storage", event => {
    parent.__typedStorageEvents.push(id + ":" + event.key + ":" + event.newValue);
    Promise.resolve().then(() => parent.__typedStorageEvents.push("microtask:" + id));
  });
}
localStorage.setItem("typed-storage-key", "value");
"queued"
"#,
        )?;

        assert!(
            !page_vm.vm().has_ready_timeout(),
            "a queued StorageEvent must not create a PageTimer descriptor"
        );
        assert_eq!(
            page_vm.vm().ms_to_next_timeout(),
            None
        );
        assert_eq!(page_vm.vm_mut().eval("__typedStorageEvents.length")?, "0");

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::StorageEvent),
                    &loader,
                )
                .await?,
            "first exact recipient should consume one selected typed turn"
        );

        assert_eq!(
            page_vm.vm_mut().eval("__typedStorageEvents.join('|')")?,
            "storage-recipient-a:typed-storage-key:value|microtask:storage-recipient-a",
            "one DOM-manipulation turn must dispatch one recipient and checkpoint its microtasks"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::StorageEvent),
                    &loader,
                )
                .await?,
            "second exact recipient should remain queued"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedStorageEvents.join('|')")?,
            "storage-recipient-a:typed-storage-key:value|microtask:storage-recipient-a|storage-recipient-b:typed-storage-key:value|microtask:storage-recipient-b"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::StorageEvent),
                    &loader,
                )
                .await?,
            "global turn readiness may include realm/lifecycle follow-up, but the DOM source must contain exactly two StorageEvent recipients"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("StorageEvent typed one-recipient test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn storage_event_retains_top_local_window_target_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/storage-event-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "storage-source-child";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "storage-source-child")?;
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");
        page_vm.vm_mut().eval(
            r#"
document.getElementById("storage-source-child").contentWindow.localStorage
  .setItem("document-open-storage-key", "queued");
document.open();
document.close();
globalThis.__documentOpenStorageEvents = [];
addEventListener("storage", event => __documentOpenStorageEvents.push(event.newValue));
"replaced"
"#,
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
                        PageDomManipulationTestFamily::StorageEvent
                    ),
                    &loader,
                )
                .await?,
            "the old-Document task should retain its LocalWindow target"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentOpenStorageEvents.join('|')")?,
            "queued"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("StorageEvent document.open ownership test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn storage_event_discards_a_retired_child_local_window() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/storage-event-stale-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "stale-storage-recipient";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "stale-storage-recipient",
        )?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__staleStorageEvents = 0;
document.getElementById("stale-storage-recipient").contentWindow
  .addEventListener("storage", () => parent.__staleStorageEvents++);
localStorage.setItem("stale-storage-key", "queued");
document.getElementById("stale-storage-recipient").remove();
"retired"
"#,
        )?;

        let task = take_next_storage_event_task_for_authorization_test(&mut page_vm)
            .expect("the stable task should settle through an explicit stale turn");
        let outcome = page_vm.apply_selected_page_storage_event_delivery_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageStorageEventDeliveryTargetEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(page_vm.vm_mut().eval("String(__staleStorageEvents)")?, "0");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("StorageEvent stale child ownership test should run");
}

#[test]
fn storage_event_rejects_a_real_page_vm_replacement_local_window_collision() {
    run_page_vm_large_stack_async_test(
        "storage-event-page-vm-replacement-local-window-collision",
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
                        r#"
const frame = document.createElement("iframe");
frame.id = "retired-storage-source";
document.body.appendChild(frame);
"created"
"#,
                    )?;
                    materialize_child_realm_through_page_turn_for_test(
                        &mut page_vm,
                        "retired-storage-source",
                    )?;
                    page_vm.vm_mut().eval(
                        r#"
document.getElementById("retired-storage-source").contentWindow.localStorage
  .setItem("replacement-storage-key", "retired");
"queued"
"#,
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
                        r#"
globalThis.__replacementStorageEvents = [];
addEventListener("storage", event => __replacementStorageEvents.push(event.newValue));
const frame = document.createElement("iframe");
frame.id = "current-storage-source";
document.body.appendChild(frame);
"created"
"#,
                    )?;
                    materialize_child_realm_through_page_turn_for_test(
                        &mut page_vm,
                        "current-storage-source",
                    )?;
                    page_vm.vm_mut().eval(
                        r#"
document.getElementById("current-storage-source").contentWindow.localStorage
  .setItem("replacement-storage-key", "current");
"queued"
"#,
                    )?;

                    let task = take_next_storage_event_task_for_authorization_test(&mut page_vm)
                        .expect("the retired PageVm delivery should consume one stale turn");
                    let stale = page_vm.apply_selected_page_storage_event_delivery_turn(task)?;
                    let PageStorageEventDeliveryTargetEffect::DiscardedStaleOwner {
                        current_owner: Some(current_owner),
                    } = stale.action.target_effect
                    else {
                        panic!("retired delivery should report the replacement LocalWindow owner");
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
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::StorageEvent),
                                &loader,
                            )
                            .await?,
                        "the replacement delivery must survive the stale-head discard"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementStorageEvents.join('|')")?,
                        "current",
                        "the retired root task must not dispatch into the replacement PageVm"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("StorageEvent PageVm replacement should use the exact owner arbiter");
            server
                .await
                .expect("StorageEvent PageVm replacement server should finish");
        },
    );
}
