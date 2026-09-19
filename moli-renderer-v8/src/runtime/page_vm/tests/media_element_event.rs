use super::*;

use crate::page_task_queue::{
    PageMediaElementEventTargetEffect, RendererPageMediaElementEventTaskKind,
};

#[tokio::test(flavor = "current_thread")]
async fn media_element_event_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/media-event-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__mediaEventTaskBoundary = [];
const media = document.createElement("video");
media.addEventListener("seeking", () => {
  __mediaEventTaskBoundary.push("callback");
  Promise.resolve().then(() => {
    __mediaEventTaskBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__mediaEventTaskBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
});
document.body.appendChild(media);
media.currentTime = 2;
"queued"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let task = page_vm
            .take_media_element_event_body_task_for_test()
            .expect("one exact media-element event should be ready");
        let body = page_vm.apply_selected_page_media_element_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageMediaElementEventTargetEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mediaEventTaskBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "listener cleanup must drain reactions before the selected task completes"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the media-event body must not consume unrelated runtime residence"
        );

        page_vm
            .finish_selected_page_task_completion(body.action.into_page_task_completion(), &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mediaEventTaskBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("media-element body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn media_event_is_document_exact_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/media-event-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let retired_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
globalThis.__documentExactMediaEvents = [];
const retiredMedia = document.createElement("video");
for (const type of ["seeking", "seeked"]) {
  retiredMedia.addEventListener(type, () => __documentExactMediaEvents.push(`retired:${type}`));
}
document.body.append(retiredMedia);
retiredMedia.currentTime = 1;

document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
"replaced"
"#,
        )?;
        let current_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(retired_document, current_document);
        assert_eq!(
            retired_document.local_window_id,
            current_document.local_window_id
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "media events must not acquire PageTimer descriptors"
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleMediaCompletionBoundary = [];
Promise.resolve().then(() => {
  __staleMediaCompletionBoundary.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        for expected_kind in [
            RendererPageMediaElementEventTaskKind::Seeking,
            RendererPageMediaElementEventTaskKind::SeekCompletion,
        ] {
            let stale = page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .expect("each retired media event should remain an exact source entry");
            assert_eq!(
                stale
                    .media_element_event_kind()
                    .expect("media selector must retain the exact event kind"),
                expected_kind
            );
            page_vm
                .run_claimed_selected_page_task_for_test(stale, &loader)
                .await?;
        }
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__documentExactMediaEvents.join('|')",
                )?,
            ""
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__staleMediaCompletionBoundary.join('|')",
                )?,
            "",
            "retired media work must not checkpoint the replacement realm"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "retired media work must not advance replacement runtime residence"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .is_none(),
            "the two retired events must consume exactly two source turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact media event test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn media_seek_events_preserve_fifo_and_exact_seek_token() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/media-seek-events").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__documentExactMediaEvents = [];
const currentMedia = document.createElement("video");
for (const type of ["seeking", "seeked"]) {
  currentMedia.addEventListener(type, () => __documentExactMediaEvents.push(`current:${type}`));
}
document.body.append(currentMedia);
currentMedia.currentTime = 2;
"queued-current"
"#,
        )?;
        let seeking = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .expect("replacement media seeking event should remain runnable");
        assert_eq!(
            seeking
                .media_element_event_kind()
                .expect("media selector must retain the exact event kind"),
            RendererPageMediaElementEventTaskKind::Seeking
        );
        page_vm
            .run_claimed_selected_page_task_for_test(seeking, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(currentMedia.seeking)")?,
            "true",
            "dispatching seeking must not settle the pending exact seek"
        );
        let seeked = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .expect("replacement media seek completion should remain runnable");
        assert_eq!(
            seeked
                .media_element_event_kind()
                .expect("media selector must retain the exact event kind"),
            RendererPageMediaElementEventTaskKind::SeekCompletion
        );
        page_vm
            .run_claimed_selected_page_task_for_test(seeked, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentExactMediaEvents.join('|')")?,
            "current:seeking|current:seeked"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .is_none(),
            "seeking and seeked must consume exactly two source turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("media seek event FIFO test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn media_event_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/media-event-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__mediaEventChildOrder = [];
const media = document.createElement("video");
media.addEventListener("seeking", () => {
  __mediaEventChildOrder.push("callback");
  Promise.resolve().then(() => {
    __mediaEventChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "media-event-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
});
document.body.appendChild(media);
media.currentTime = 3;
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent, &loader)
                .await?,
            "the exact media-element task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__mediaEventChildOrder.join('|')")?,
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
    .expect("media-element post-checkpoint child synchronization test should run");
}
