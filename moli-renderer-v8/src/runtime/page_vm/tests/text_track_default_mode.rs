use super::*;

#[tokio::test(flavor = "current_thread")]
async fn text_track_default_mode_completion_does_not_flush_unrelated_runtime_work() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-default-completion").unwrap();
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
globalThis.__defaultCompletionTrack = track;
"queued"
"#,
        )?;
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
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "the automatic default-mode task should be selected"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__defaultCompletionTrack.track.mode")?,
            "showing"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "automatic selection dispatched no callback and must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("text-track default-mode completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn text_track_default_mode_shares_dom_fifo_without_a_timer_turn() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-dom-fifo").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
const details = document.createElement("details");
document.body.append(details);
details.open = true;

const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
track.src = "data:text/vtt,WEBVTT";
video.append(track);
document.body.append(video);
globalThis.__typedDefaultTrack = track;
"queued"
"#,
        )?;
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "default-mode admission must not acquire a PageTimer descriptor"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedDefaultTrack.track.mode")?,
            "disabled"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::ElementToggle
                    )
                )
                .await?,
            "element toggle should be the shared source head"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedDefaultTrack.track.mode")?,
            "disabled",
            "one family turn may not consume the following text-track task"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "text-track default-mode task should remain at the FIFO tail"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedDefaultTrack.track.mode")?,
            "showing"
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "text-track load-start must not acquire a PageTimer descriptor"
        );
        assert!(
            page_vm.has_ready_page_networking_task(),
            "default-mode application should naturally admit one Networking task"
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "two family tasks must consume exactly two turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("text-track DOM FIFO test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn text_track_default_mode_discards_document_open_work_before_the_current_tail() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/text-track-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let retired_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial Document owner");

        page_vm.vm_mut().eval(
            r#"
const retiredVideo = document.createElement("video");
const retiredTrack = document.createElement("track");
retiredTrack.default = true;
retiredTrack.src = "data:text/vtt,WEBVTT";
retiredVideo.append(retiredTrack);
document.body.append(retiredVideo);

document.open();
document.write("<!doctype html><body></body>");
document.close();

const currentVideo = document.createElement("video");
const currentTrack = document.createElement("track");
currentTrack.default = true;
currentTrack.src = "data:text/vtt,WEBVTT";
currentVideo.append(currentTrack);
document.body.append(currentVideo);
globalThis.__currentTypedDefaultTrack = currentTrack;
"queued"
"#,
        )?;
        let current_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement Document owner");
        assert_ne!(retired_document, current_document);
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "neither old nor current default-mode work may fall back to PageTimer"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "retired task should settle explicitly"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__currentTypedDefaultTrack.track.mode")?,
            "disabled",
            "discarding the old exact owner may not consume the replacement payload"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::TextTrackDefaultMode
                    )
                )
                .await?,
            "replacement task should survive stale-head settlement"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__currentTypedDefaultTrack.track.mode")?,
            "showing"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("text-track exact-Document test should run");
}
