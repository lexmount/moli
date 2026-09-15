use super::*;

use crate::page_task_queue::RendererOwnerWakeSource;

fn take_window_message_wake(
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) -> bool {
    while let Ok(wake) = wake_rx.try_recv() {
        if wake.source_for_test() == RendererOwnerWakeSource::WindowMessageTask {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_uses_typed_one_turn_execution_and_not_the_legacy_wait_driver() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/window-message-owner-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__typedWindowMessageEvents = [];
  addEventListener("message", event => {
    __typedWindowMessageEvents.push("message:" + event.data);
    Promise.resolve().then(() => {
      __typedWindowMessageEvents.push("microtask:" + event.data);
    });
  });
  postMessage("first", "*");
  postMessage("second", "*");
})()
"#,
        )?;

        assert_eq!(page_vm.vm().ms_to_next_timeout(), None);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedWindowMessageEvents.join('|')")?,
            "",
            "a timer-deadline observation must not consume a migrated Window.postMessage task"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                .await?,
            "the first real producer task should consume one selected typed turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedWindowMessageEvents.join('|')")?,
            "message:first|microtask:first",
            "one selected turn must checkpoint its microtasks without draining the next message"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                .await?,
            "the second producer task should remain for the next selected turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedWindowMessageEvents.join('|')")?,
            "message:first|microtask:first|message:second|microtask:second"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage typed one-turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/window-message-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__windowMessageChildOrder = [];
  onmessage = () => {
    __windowMessageChildOrder.push("callback");
    Promise.resolve().then(() => {
      __windowMessageChildOrder.push("microtask");
      const frame = document.createElement("iframe");
      frame.id = "window-message-microtask-child";
      frame.srcdoc = "<!doctype html><body>child</body>";
      document.body.appendChild(frame);
    });
  };
  postMessage("create-child", "*");
})()
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                .await?,
            "the exact Window.postMessage task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__windowMessageChildOrder.join('|')")?,
            "callback|microtask",
            "the agent checkpoint must run before callback completion synchronizes child records"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a srcdoc frame created by the reaction must publish its typed navigation commit during post-checkpoint child synchronization"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created srcdoc frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_retains_its_local_window_target_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/window-message-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");
        page_vm.vm_mut().eval(
            r#"
postMessage("queued-before-document-open", "*");
document.open();
document.close();
globalThis.__documentOpenTypedMessages = [];
onmessage = event => __documentOpenTypedMessages.push(event.data);
"queued"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.open replacement owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id, after_document.local_window_id,
            "document.open replaces the Document but retains its LocalWindow task target"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                .await?,
            "the pre-document.open task should remain scheduler-visible"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentOpenTypedMessages.join('|')")?,
            "queued-before-document-open"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage document.open ownership test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn retiring_a_ready_local_window_readmits_its_stale_stable_task() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/window-message-retirement").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "retired-window-message-target";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "retired-window-message-target",
        )?;
        while owner_wake_rx.try_recv().is_ok() {}

        page_vm.vm_mut().eval(
            r#"
document.getElementById("retired-window-message-target")
  .contentWindow.postMessage("must-not-run", "*");
"queued"
"#,
        )?;
        assert!(
            take_window_message_wake(&mut owner_wake_rx),
            "the original empty-to-nonempty transition should publish one typed wake"
        );
        assert!(!take_window_message_wake(&mut owner_wake_rx));

        page_vm.vm_mut().eval(
            r#"
document.getElementById("retired-window-message-target").remove();
"retired"
"#,
        )?;
        assert!(
            take_window_message_wake(&mut owner_wake_rx),
            "retiring the local payload must readmit the already-ready stable source"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                .await?,
            "the retired target's stable task should remain drainable"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                .await?,
            "retirement reconsideration must not create a duplicate task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage retirement readmission test should run");
}

#[test]
fn window_message_rejects_a_real_page_vm_replacement_task_id_collision() {
    run_page_vm_large_stack_async_test("window-message-page-vm-replacement-collision", || async {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/replacement.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><body>replacement</body>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
globalThis.__retiredWindowMessageEvents = [];
onmessage = event => __retiredWindowMessageEvents.push(event.data);
postMessage("retired", "*");
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
globalThis.__replacementWindowMessageEvents = [];
onmessage = event => __replacementWindowMessageEvents.push(event.data);
postMessage("current", "*");
"queued"
"#,
                )?;

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                        .await?,
                    "the retired PageVm task should consume one stale selected turn"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__replacementWindowMessageEvents.join('|')")?,
                    "",
                    "the stale root task must not dispatch or remove the replacement Host payload"
                );

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage)
                        .await?,
                    "the replacement task must survive the stale local-id collision"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__replacementWindowMessageEvents.join('|')")?,
                    "current",
                    "discarding the old root task must not remove the replacement Host payload"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("Window.postMessage PageVm replacement should run through the typed executor");
        server
            .await
            .expect("Window.postMessage replacement server should finish");
    });
}
