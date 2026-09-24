use super::*;

use crate::page_task_queue::RendererOwnerWakeSource;

fn take_wake_sources(
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) -> Vec<RendererOwnerWakeSource> {
    std::iter::from_fn(|| wake_rx.try_recv().ok())
        .map(|wake| wake.source_for_test())
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn text_track_load_start_completion_does_not_flush_unrelated_runtime_work() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-start-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
track.src = "data:text/vtt,WEBVTT";
video.append(track);
document.body.append(video);
globalThis.__startCompletionTrack = track;
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "automatic selection should publish the load-start task"
        );
        assert!(
            page_vm.has_ready_page_networking_task(),
            "load start should be the next stable Page task"
        );

        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the setup must leave unrelated runtime work pending"
        );

        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test()
                .await?,
            "the load-start Networking task should be selected"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__startCompletionTrack.readyState)")?,
            "1",
            "the selected task must apply only the load-start synchronous section"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "fetch start dispatched no callback and must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("text-track load-start completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn text_track_network_terminal_completion_syncs_a_microtask_created_child() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-terminal-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__textTrackTerminalOrder = [];
const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
track.src = "data:text/vtt,WEBVTT";
track.addEventListener("load", () => {
  __textTrackTerminalOrder.push("callback");
  Promise.resolve().then(() => {
    __textTrackTerminalOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "text-track-terminal-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
});
video.append(track);
document.body.append(video);
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "automatic selection should run first"
        );
        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test()
                .await?,
            "load start should run second"
        );
        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test()
                .await?,
            "successful terminal should run third"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__textTrackTerminalOrder.join('|')")?,
            "callback|microtask"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a load-listener reaction-created child must be synchronized during terminal completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit)
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("text-track terminal callback completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_text_track_start_with_no_payload_does_not_manufacture_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-stale-start").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__staleTextTrackEvents = [];
const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
track.src = "data:text/vtt,WEBVTT";
track.addEventListener("load", () => __staleTextTrackEvents.push("load"));
track.addEventListener("error", () => __staleTextTrackEvents.push("error"));
video.append(track);
document.body.append(video);
globalThis.__staleTextTrack = track;
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "automatic selection should publish one exact start task"
        );
        let stale = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::TextTrackNetworking,
            )
            .expect("the exact load-start task should be ready");

        page_vm
            .vm_mut()
            .eval("__staleTextTrack.remove(); 'removed'")?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleTextTrackCompletionBoundary = [];
Promise.resolve().then(() => {
  __staleTextTrackCompletionBoundary.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        page_vm
            .run_claimed_selected_page_task_for_test(stale)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a payload already retired by DOM mutation must not manufacture runtime completion"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test("__staleTextTrackEvents.join('|')",)?,
            "",
            "retiring a stale start must not manufacture a load/error callback"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__staleTextTrackCompletionBoundary.join('|')",
                )?,
            "",
            "a stale load start must not checkpoint the current realm"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::TextTrackNetworking,
                )
                .is_none(),
            "the stale exact load-start task must retire once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale text-track load settlement test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn text_track_data_load_uses_two_natural_networking_turns_without_a_timer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-networking").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
track.src = "data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Ahello";
globalThis.__typedTrackEvents = [];
track.addEventListener("load", () => __typedTrackEvents.push(`load:${track.readyState}`));
video.append(track);
document.body.append(video);
globalThis.__typedTrack = track;
"queued"
"#,
        )?;
        assert!(!page_vm.vm().has_ready_timeout());
        assert_eq!(
            take_wake_sources(&mut wake_rx),
            vec![RendererOwnerWakeSource::DomManipulationTask]
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "default mode should run through DOM manipulation"
        );
        let wakes = take_wake_sources(&mut wake_rx);
        assert_eq!(
            wakes
                .iter()
                .filter(|source| **source == RendererOwnerWakeSource::NetworkingTask)
                .count(),
            1,
            "default-mode application must naturally publish one Networking admission"
        );
        assert!(!page_vm.vm().has_ready_timeout());

        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test()
                .await?,
            "text-track start should be the Networking head"
        );
        assert_eq!(
            page_vm.vm_mut().eval("String(__typedTrack.readyState)")?,
            "1"
        );
        assert_eq!(page_vm.vm_mut().eval("__typedTrackEvents.join('|')")?, "");
        let wakes = take_wake_sources(&mut wake_rx);
        assert_eq!(
            wakes
                .iter()
                .filter(|source| **source == RendererOwnerWakeSource::NetworkingTask)
                .count(),
            1,
            "local fetch completion must naturally publish exactly one terminal admission"
        );

        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test()
                .await?,
            "successful text-track terminal should remain in Networking FIFO"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedTrackEvents.join('|')")?,
            "load:2"
        );
        assert!(!page_vm.vm().has_ready_timeout());
        assert!(
            !page_vm.has_ready_page_networking_task(),
            "two accepted networking tasks must consume exactly two turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("text-track Networking liveness test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn text_track_fetch_failure_terminal_uses_dom_manipulation_not_networking() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-fetch-failure").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
globalThis.__typedTrackErrors = [];
track.addEventListener("error", () => {
  __typedTrackErrors.push("callback");
  Promise.resolve().then(() => __typedTrackErrors.push("microtask"));
});
video.append(track);
document.body.append(video);
globalThis.__failedTypedTrack = track;
"queued"
"#,
        )?;
        take_wake_sources(&mut wake_rx);
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "default-mode DOM turn"
        );
        take_wake_sources(&mut wake_rx);

        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test()
                .await?,
            "empty-source load start should still be a Networking task"
        );
        let wakes = take_wake_sources(&mut wake_rx);
        assert!(wakes.contains(&RendererOwnerWakeSource::DomManipulationTask));
        assert!(!wakes.contains(&RendererOwnerWakeSource::NetworkingTask));
        assert!(
            !page_vm.has_ready_page_networking_task(),
            "fetch failure may not remain in the Networking FIFO"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackLoad
                    )
                )
                .await?,
            "fetch failure should be an element task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("`${__failedTypedTrack.readyState}:${__typedTrackErrors.join('|')}`")?,
            "3:callback|microtask"
        );
        assert!(!page_vm.vm().has_ready_timeout());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("text-track fetch-failure source test should run");
}
