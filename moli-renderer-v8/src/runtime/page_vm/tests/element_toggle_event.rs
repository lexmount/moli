use super::*;

use crate::page_task_queue::{
    PageElementToggleEventTargetEffect, RendererPageElementToggleEventKind,
    RendererPageElementToggleEventTask,
};

fn take_next_element_toggle_task_for_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageElementToggleEventTask> {
    let task = page_vm
        .take_dom_manipulation_body_task_for_test(PageDomManipulationTestFamily::ElementToggle)?;
    let crate::page_task_queue::RendererPageDomManipulationTask::ElementToggle(task) = task else {
        unreachable!("exact element-toggle selection must preserve its task variant")
    };
    Some(task)
}

#[tokio::test(flavor = "current_thread")]
async fn element_toggle_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/element-toggle-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__elementToggleBoundary = [];
const details = document.createElement("details");
details.addEventListener("toggle", () => {
  __elementToggleBoundary.push("callback");
  Promise.resolve().then(() => {
    __elementToggleBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__elementToggleBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
});
document.body.appendChild(details);
details.open = true;
"queued"
"#,
        )?;

        let task = take_next_element_toggle_task_for_test(&mut page_vm)
            .expect("one exact element-toggle task should be ready");
        let body = page_vm.apply_selected_page_element_toggle_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageElementToggleEventTargetEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval("__elementToggleBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "listener cleanup must drain reactions before the selected task completes"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__elementToggleBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("element-toggle body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn element_toggle_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/element-toggle-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__elementToggleChildOrder = [];
const details = document.createElement("details");
details.addEventListener("toggle", () => {
  __elementToggleChildOrder.push("callback");
  Promise.resolve().then(() => {
    __elementToggleChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "element-toggle-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
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
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::ElementToggle),
                    &loader,
                )
                .await?,
            "the exact element-toggle task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__elementToggleChildOrder.join('|')")?,
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
    .expect("element-toggle post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn element_toggle_coalescing_reposts_at_the_dom_source_tail_one_turn_at_a_time() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/element-toggle-one-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__typedElementToggleEvents = [];
for (const id of ["first", "second"]) {
  const details = document.createElement("details");
  details.id = id;
  details.addEventListener("toggle", event => {
    __typedElementToggleEvents.push(
      `${id}:${event.oldState}->${event.newState}`
    );
    Promise.resolve().then(() => {
      __typedElementToggleEvents.push(`microtask:${id}`);
    });
  });
  document.body.append(details);
}
const first = document.getElementById("first");
const second = document.getElementById("second");
first.open = true;
second.open = true;
first.open = false;
"queued"
"#,
        )?;

        assert!(
            !page_vm.vm().has_ready_timeout(),
            "element toggle events must not acquire PageTimer descriptors"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedElementToggleEvents.length")?,
            "0"
        );

        let first = take_next_element_toggle_task_for_test(&mut page_vm)
            .expect("the second element should remain at the live source head");
        assert_eq!(first.kind(), RendererPageElementToggleEventKind::Details);
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ElementToggle(first),
                &loader,
            )
            .await?;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedElementToggleEvents.join('|')")?,
            "second:closed->open|microtask:second",
            "cancelling and reposting the first element must move it behind the second task"
        );

        let second = take_next_element_toggle_task_for_test(&mut page_vm)
            .expect("the reposted first element should consume the next turn");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ElementToggle(second),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedElementToggleEvents.join('|')")?,
            "second:closed->open|microtask:second|first:closed->closed|microtask:first",
            "one selected DOM task must dispatch one event and checkpoint its microtasks"
        );
        assert!(
            take_next_element_toggle_task_for_test(&mut page_vm).is_none(),
            "cancelled closures must not become browser task turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("element toggle one-turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn element_toggle_is_document_exact_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/element-toggle-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
globalThis.__retiredElementToggleEvents = 0;
const details = document.createElement("details");
details.addEventListener("toggle", () => __retiredElementToggleEvents++);
document.body.append(details);
details.open = true;
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
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

        let stale_task = take_next_element_toggle_task_for_test(&mut page_vm)
            .expect("retired Document toggle work should settle explicitly");
        let stale = page_vm.apply_selected_page_element_toggle_event_turn(stale_task)?;
        let PageElementToggleEventTargetEffect::DiscardedStaleOwner {
            current_owner: Some(current_owner),
        } = stale.action.target_effect
        else {
            panic!("document.open toggle work should report the replacement Document owner")
        };
        assert_ne!(stale.action.owner, current_owner);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__retiredElementToggleEvents)")?,
            "0"
        );
        assert!(
            take_next_element_toggle_task_for_test(&mut page_vm).is_none(),
            "stale settlement must retire the Host-local coalescing slot"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact element toggle test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn element_toggle_uses_the_elements_child_document_owner() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/element-toggle-child-owner").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "element-toggle-child";
document.body.append(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "element-toggle-child")?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__childElementToggleEvents = [];
const childFrame = document.getElementById("element-toggle-child");
const childDetails = childFrame.contentDocument.createElement("details");
childDetails.addEventListener("toggle", event => {
  parent.__childElementToggleEvents.push(
    `${event.oldState}->${event.newState}`
  );
});
childFrame.contentDocument.body.append(childDetails);
globalThis.__childDetails = childDetails;
childDetails.open = true;
"queued-current"
"#,
        )?;

        let current = take_next_element_toggle_task_for_test(&mut page_vm)
            .expect("parent script should queue against the element's child Document");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ElementToggle(current),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__childElementToggleEvents.join('|')")?,
            "closed->open"
        );

        page_vm.vm_mut().eval(
            r#"
__childDetails.open = false;
document.getElementById("element-toggle-child").remove();
"retired"
"#,
        )?;
        let stale_task = take_next_element_toggle_task_for_test(&mut page_vm)
            .expect("retired child toggle task should settle explicitly");
        let stale = page_vm.apply_selected_page_element_toggle_event_turn(stale_task)?;
        assert_eq!(
            stale.action.target_effect,
            PageElementToggleEventTargetEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__childElementToggleEvents.join('|')")?,
            "closed->open",
            "a retired child task must not dispatch through the parent realm"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child-Document element toggle test should run");
}

#[test]
fn element_toggle_rejects_a_real_page_vm_replacement_task_id_collision() {
    run_page_vm_large_stack_async_test(
        "element-toggle-page-vm-replacement-task-id-collision",
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
const retiredDetails = document.createElement("details");
document.body.append(retiredDetails);
retiredDetails.open = true;
"queued-retired"
"#,
                    )?;
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
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__replacementElementToggleEvents = [];
const currentDetails = document.createElement("details");
currentDetails.addEventListener("toggle", event => {
  __replacementElementToggleEvents.push(
    `${event.oldState}->${event.newState}`
  );
});
document.body.append(currentDetails);
currentDetails.open = true;
"queued-current"
"#,
                    )?;

                    let stale_task = take_next_element_toggle_task_for_test(&mut page_vm)
                        .expect("retired PageVm toggle task should consume one stale turn");
                    let stale =
                        page_vm.apply_selected_page_element_toggle_event_turn(stale_task)?;
                    let PageElementToggleEventTargetEffect::DiscardedStaleOwner {
                        current_owner: Some(current_owner),
                    } = stale.action.target_effect
                    else {
                        panic!("retired toggle task should report the replacement owner");
                    };
                    assert_ne!(stale.action.owner, current_owner);
                    assert_eq!(
                        stale.action.owner.target(),
                        current_owner.target(),
                        "fresh PageVm counters should naturally reuse the local Document target"
                    );

                    let current = take_next_element_toggle_task_for_test(&mut page_vm)
                        .expect("replacement toggle task must survive stale-head settlement");
                    assert_eq!(current.owner(), current_owner);
                    assert_eq!(
                        stale.action.task_id,
                        current.task_id(),
                        "fresh PageVm Host counters should naturally reuse the local task id"
                    );
                    page_vm
                        .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                            crate::page_task_queue::RendererPageDomManipulationTask::ElementToggle(
                                current,
                            ),
                            &loader,
                        )
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementElementToggleEvents.join('|')")?,
                        "closed->open",
                        "retired root task must not consume the replacement Host payload"
                    );
                    assert!(!page_vm.vm().has_ready_timeout());
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("element toggle replacement should use exact root arbitration");
            server
                .await
                .expect("element toggle replacement server should finish");
        },
    );
}
