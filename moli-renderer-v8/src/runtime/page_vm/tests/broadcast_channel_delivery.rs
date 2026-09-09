use super::*;

use crate::page_task_queue::{
    PageBroadcastChannelDeliveryDocumentEffect, RendererPageBroadcastChannelDeliveryTask,
    RendererPageSchedulerTask,
};

fn take_next_broadcast_channel_task_for_authorization_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageBroadcastChannelDeliveryTask> {
    let task = page_vm.take_dom_manipulation_body_task_for_test(
        PageDomManipulationTestFamily::BroadcastChannel,
    )?;
    let crate::page_task_queue::RendererPageDomManipulationTask::BroadcastChannel(task) = task
    else {
        unreachable!("exact BroadcastChannel selection must preserve its task variant")
    };
    Some(task)
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/broadcast-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__broadcastBodyBoundary = [];
globalThis.__broadcastBodyReceiver = new BroadcastChannel("broadcast-body-boundary");
__broadcastBodyReceiver.onmessage = () => {
  __broadcastBodyBoundary.push("callback");
  Promise.resolve().then(() => {
    __broadcastBodyBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__broadcastBodyBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
};
globalThis.__broadcastBodySender = new BroadcastChannel("broadcast-body-boundary");
__broadcastBodySender.postMessage("go");
"queued"
"#,
        )?;

        let task = take_next_broadcast_channel_task_for_authorization_test(&mut page_vm)
            .expect("one exact BroadcastChannel task should be ready");
        let body = page_vm.apply_selected_page_broadcast_channel_delivery_turn(task)?;
        assert_eq!(
            body.action.document_effect,
            PageBroadcastChannelDeliveryDocumentEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval("__broadcastBodyBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "listener cleanup must drain reactions before the selected task completes"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__broadcastBodyBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("BroadcastChannel body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/broadcast-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__broadcastChildOrder = [];
globalThis.__broadcastChildReceiver = new BroadcastChannel("broadcast-microtask-child");
__broadcastChildReceiver.onmessage = () => {
  __broadcastChildOrder.push("callback");
  Promise.resolve().then(() => {
    __broadcastChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "broadcast-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
};
globalThis.__broadcastChildSender = new BroadcastChannel("broadcast-microtask-child");
__broadcastChildSender.postMessage("go");
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::BroadcastChannel),
                    &loader,
                )
                .await?,
            "the exact BroadcastChannel task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__broadcastChildOrder.join('|')")?,
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
    .expect("BroadcastChannel post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_and_element_toggle_each_complete_one_shared_dom_turn() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/broadcast-shared-dom-source").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__broadcastSharedDomOrder = [];
globalThis.__broadcastSharedReceiver =
  new BroadcastChannel("broadcast-shared-dom-source");
__broadcastSharedReceiver.onmessage = () => {
  __broadcastSharedDomOrder.push("broadcast");
  Promise.resolve().then(() => {
    __broadcastSharedDomOrder.push("microtask:broadcast");
  });
};
globalThis.__broadcastSharedSender =
  new BroadcastChannel("broadcast-shared-dom-source");
__broadcastSharedSender.postMessage("first");

const details = document.createElement("details");
details.addEventListener("toggle", () => {
  __broadcastSharedDomOrder.push("toggle");
  Promise.resolve().then(() => {
    __broadcastSharedDomOrder.push("microtask:toggle");
  });
});
document.body.appendChild(details);
details.open = true;
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
            "BroadcastChannel should consume the first shared DOM turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__broadcastSharedDomOrder.join('|')")?,
            "broadcast|microtask:broadcast",
            "BroadcastChannel must complete only its own callback task"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::ElementToggle
                    ),
                    &loader,
                )
                .await?,
            "the existing element-toggle task should retain the second shared DOM turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__broadcastSharedDomOrder.join('|')")?,
            "broadcast|microtask:broadcast|toggle|microtask:toggle",
            "the selected element-toggle task must complete only its own callback"
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "the two shared source tasks must consume exactly two selected turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("BroadcastChannel shared DOM source isolation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_delivery_applies_a_real_producer_task_and_microtask_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/broadcast-owner-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__typedBroadcastEvents = [];
  globalThis.__typedBroadcastReceiver = new BroadcastChannel("typed-current-owner");
  globalThis.__typedBroadcastReceiver.onmessage = event => {
    __typedBroadcastEvents.push("message:" + event.data);
    Promise.resolve().then(() => __typedBroadcastEvents.push("microtask"));
  };
  globalThis.__typedBroadcastSender = new BroadcastChannel("typed-current-owner");
  __typedBroadcastSender.postMessage("go");
})()
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
            "real producer task should consume one selected typed turn"
        );

        assert_eq!(
            page_vm.vm_mut().eval("__typedBroadcastEvents.join('|')")?,
            "message:go|microtask",
            "one authorized delivery turn should include its required microtask checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("BroadcastChannel current-owner task should run through the PageVm executor");
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_closed_after_post_consumes_one_no_event_turn() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/broadcast-close-after-post").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__closedTypedBroadcastEvents = [];
  const receiver = new BroadcastChannel("typed-close-after-post");
  receiver.onmessage = event => __closedTypedBroadcastEvents.push(event.data);
  const sender = new BroadcastChannel("typed-close-after-post");
  sender.postMessage("must-not-dispatch");
  receiver.close();
})()
"#,
        )?;

        let task = take_next_broadcast_channel_task_for_authorization_test(&mut page_vm)
            .expect("closed channel task should consume one bounded no-op turn");
        let outcome = page_vm.apply_selected_page_broadcast_channel_delivery_turn(task)?;
        assert_eq!(
            outcome.action.document_effect,
            PageBroadcastChannelDeliveryDocumentEffect::CurrentOwnerHadNoPendingEvent
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__closedTypedBroadcastEvents.join('|')")?,
            ""
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("BroadcastChannel close-after-post task should run through the PageVm executor");
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_delivery_consumes_one_exact_owner_task_per_turn() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/broadcast-one-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__oneTurnBroadcastEvents = [];
  globalThis.__oneTurnBroadcastReceivers = [
    new BroadcastChannel("one-task-per-turn"),
    new BroadcastChannel("one-task-per-turn")
  ];
  for (const receiver of __oneTurnBroadcastReceivers) {
    receiver.onmessage = event => __oneTurnBroadcastEvents.push(event.data);
  }
  globalThis.__oneTurnBroadcastSender = new BroadcastChannel("one-task-per-turn");
  __oneTurnBroadcastSender.postMessage("go");
})()
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
            "first delivery should consume one selected turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__oneTurnBroadcastEvents.join('|')")?,
            "go",
            "one selected turn must dispatch exactly one channel event"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::BroadcastChannel
                    ),
                    &loader,
                )
                .await?,
            "second delivery should consume the following selected turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__oneTurnBroadcastEvents.join('|')")?,
            "go|go"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::BroadcastChannel
                    ),
                    &loader,
                )
                .await?,
            "two recipient deliveries must consume exactly two selected turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("BroadcastChannel one-turn contract should run through the PageVm executor");
}

#[test]
fn broadcast_channel_delivery_rejects_a_real_page_vm_replacement_identity_collision() {
    run_page_vm_large_stack_async_test(
        "broadcast-channel-real-page-vm-replacement-collision",
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
(() => {
  globalThis.__retiredBroadcastReceiver = new BroadcastChannel("retired-page-vm");
  globalThis.__retiredBroadcastReceiver.onmessage = () => {
    throw new Error("retired PageVm handler must not run");
  };
  globalThis.__retiredBroadcastSender = new BroadcastChannel("retired-page-vm");
  __retiredBroadcastSender.postMessage("late");
})()
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
                | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. }
        ));

                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(retired_root, current_root);
                    page_vm.vm_mut().eval(
                        r#"
(() => {
  globalThis.__replacementBroadcastEvents = [];
  globalThis.__replacementBroadcastReceiver =
    new BroadcastChannel("replacement-page-vm");
  __replacementBroadcastReceiver.onmessage = event => {
    __replacementBroadcastEvents.push(event.data);
  };
  globalThis.__replacementBroadcastSender =
    new BroadcastChannel("replacement-page-vm");
  __replacementBroadcastSender.postMessage("current");
})()
"#,
                    )?;
                    let stale_task =
                        take_next_broadcast_channel_task_for_authorization_test(&mut page_vm)
                            .expect("old PageVm task should consume one stale discard turn");
                    let outcome =
                        page_vm.apply_selected_page_broadcast_channel_delivery_turn(stale_task)?;
                    let retired_owner = outcome.action.owner;
                    let PageBroadcastChannelDeliveryDocumentEffect::DiscardedStaleOwner {
                        current_owner: Some(current_owner),
                    } = outcome.action.document_effect
                    else {
                        panic!("retired task should report the replacement exact owner");
                    };
                    assert_ne!(retired_owner, current_owner);
                    assert_eq!(
                        retired_owner.execution_context(),
                        current_owner.execution_context(),
                        "fresh PageVm counters should naturally reuse the top Window/realm identity"
                    );

                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(
                                PageSelectedTaskTestSelector::DomManipulation(
                                    PageDomManipulationTestFamily::BroadcastChannel
                                ),
                                &loader,
                            )
                            .await?,
                        "replacement producer task should consume the next selected turn"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementBroadcastEvents.join('|')")?,
                        "current"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("BroadcastChannel PageVm replacement should run through the task executor");
            server
                .await
                .expect("BroadcastChannel PageVm replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_delivery_rejects_a_replaced_child_realm_without_rebinding() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/broadcast-realm-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            "const frame = document.createElement('iframe'); \
             frame.id = 'broadcast-realm-replacement'; \
             document.body.appendChild(frame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("broadcast-realm-replacement")
            .expect("realm replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "broadcast-realm-replacement",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.getElementById("broadcast-realm-replacement");
  const ChildBroadcastChannel = frame.contentWindow.BroadcastChannel;
  globalThis.__retiredRealmBroadcastReceiver =
    new ChildBroadcastChannel("retired-child-realm");
  __retiredRealmBroadcastReceiver.onmessage = () => {
    throw new Error("retired child realm handler must not run");
  };
  globalThis.__retiredRealmBroadcastSender =
    new ChildBroadcastChannel("retired-child-realm");
  __retiredRealmBroadcastSender.postMessage("retired");
})()
"#,
        )?;
        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "broadcast-realm-replacement",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.getElementById("broadcast-realm-replacement");
  const ChildBroadcastChannel = frame.contentWindow.BroadcastChannel;
  globalThis.__currentRealmBroadcastEvents = [];
  globalThis.__currentRealmBroadcastReceiver =
    new ChildBroadcastChannel("current-child-realm");
  __currentRealmBroadcastReceiver.onmessage = event => {
    parent.__currentRealmBroadcastEvents.push(event.data);
  };
  globalThis.__currentRealmBroadcastSender =
    new ChildBroadcastChannel("current-child-realm");
  __currentRealmBroadcastSender.postMessage("current");
})()
"#,
        )?;
        let stale_task = take_next_broadcast_channel_task_for_authorization_test(&mut page_vm)
            .expect("retired-realm delivery should consume one discard turn");
        let outcome = page_vm.apply_selected_page_broadcast_channel_delivery_turn(stale_task)?;
        let retired_owner = outcome.action.owner;
        let retired_context = retired_owner.execution_context();
        assert_eq!(
            outcome.action.document_effect,
            PageBroadcastChannelDeliveryDocumentEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(outcome.action.owner, retired_owner);

        let current_task = take_next_broadcast_channel_task_for_authorization_test(&mut page_vm)
            .expect("replacement child realm task should remain queued after stale discard");
        let current_owner = current_task.owner();
        let current_context = current_owner.execution_context();
        assert_eq!(retired_context.owner(), current_context.owner());
        assert_eq!(
            retired_context.dispatch_scope(),
            current_context.dispatch_scope()
        );
        assert_ne!(retired_context.realm_token(), current_context.realm_token());
        page_vm
            .apply_selected_page_scheduler_task_on_owner_lane_for_test(
                RendererPageSchedulerTask::DomManipulation(
                    crate::page_task_queue::RendererPageDomManipulationTask::BroadcastChannel(
                        current_task,
                    ),
                ),
                loader.clone(),
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__currentRealmBroadcastEvents.join('|')")?,
            "current"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("BroadcastChannel realm replacement should run through the task executor");
}
