use super::*;

use crate::page_task_queue::{
    PageMediaElementEventTargetEffect, RendererPageMediaElementEventTaskKind,
};

#[tokio::test(flavor = "current_thread")]
async fn media_canplaythrough_runs_in_a_separate_owned_task_after_canplay_microtasks() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("loader");
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://media-canplaythrough.test/")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page.vm_mut().eval(r#"
globalThis.mediaEvents = [];
globalThis.media = document.createElement('video');
for (const type of ['loadstart', 'loadedmetadata', 'loadeddata', 'canplay', 'canplaythrough']) {
  media.addEventListener(type, event => {
    mediaEvents.push(type + ':' + media.readyState);
    if (type === 'canplay' || type === 'canplaythrough') {
      Promise.resolve().then(() => mediaEvents.push(type + '-microtask'));
    }
  });
}
media.oncanplay = () => setTimeout(() => mediaEvents.push('timer'), 0);
document.body.append(media);
media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx';
"#)?;
        for _ in 0..4 {
            let turn = page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            ).expect("each load phase must have its own media-element task");
            page.run_claimed_selected_page_task_for_test(turn, &loader).await?;
        }
        assert_eq!(page.vm_mut().eval("mediaEvents.join('|')")?,
            "loadstart:0|loadedmetadata:1|loadeddata:2|canplay:4|canplay-microtask");
        assert!(page.vm().has_ready_timeout(), "the unrelated timer must remain queued");
        let through = page.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::MediaElementEvent,
        ).expect("HAVE_ENOUGH_DATA must queue canplaythrough after canplay");
        page.run_claimed_selected_page_task_for_test(through, &loader).await?;
        assert_eq!(page.vm_mut().eval("mediaEvents.join('|')")?,
            "loadstart:0|loadedmetadata:1|loadeddata:2|canplay:4|canplay-microtask|canplaythrough:4|canplaythrough-microtask");
        assert_eq!(page.vm_mut().eval("media.paused && media.networkState === 1")?, "true");
        assert!(page.vm().has_ready_timeout(), "canplaythrough must not consume timers");
        assert!(page.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::MediaElementEvent,
        ).is_none(), "the terminal phase must finish the load sequence exactly once");
        Ok::<_, anyhow::Error>(())
    }).await.expect("canplaythrough task ordering and timer isolation");
}

#[tokio::test(flavor = "current_thread")]
async fn media_canplaythrough_from_a_retired_document_does_not_checkpoint_the_replacement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://retired-media-canplaythrough.test/")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page.vm_mut().eval(
            r#"
globalThis.retiredMediaEvents = [];
const media = document.createElement('video');
media.oncanplaythrough = () => retiredMediaEvents.push('stale canplaythrough');
document.body.append(media);
media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx';
"#,
        )?;
        for _ in 0..4 {
            let turn = page
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .expect("load phase before Document retirement");
            page.run_claimed_selected_page_task_for_test(turn, &loader)
                .await?;
        }
        let retired = page
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .expect("capture the terminal task before its Document is retired");
        page.vm_mut().eval(
            "document.open();document.write('<!doctype html><body>replacement</body>');document.close();",
        )?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.replacementMediaEvents = [];
Promise.resolve().then(() => replacementMediaEvents.push('replacement microtask'));
"#,
        )?;
        page.vm_mut().enqueue_test_pending_runtime_source_load();
        page.run_claimed_selected_page_task_for_test(retired, &loader)
            .await?;
        assert_eq!(
            page.vm_mut().eval_without_microtask_checkpoint_for_test(
                "retiredMediaEvents.join('|') + ':' + replacementMediaEvents.join('|')",
            )?,
            ":"
        );
        assert_eq!(
            page.vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "retired terminal events must not advance replacement script work"
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .is_none()
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("retired canplaythrough owner and completion isolation");
}

#[tokio::test(flavor = "current_thread")]
async fn media_canplaythrough_is_cancelled_when_canplay_replaces_the_load_sequence() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://replaced-media-canplaythrough.test/")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page.vm_mut().eval(
            r#"
globalThis.replacedMediaEvents = [];
globalThis.media = document.createElement('video');
let first = true;
media.oncanplay = () => {
  replacedMediaEvents.push('canplay');
  if (first) {
    first = false;
    Promise.resolve().then(() => {
      media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQxAA==';
    });
  }
};
media.oncanplaythrough = () => replacedMediaEvents.push('canplaythrough');
document.body.append(media);
media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx';
"#,
        )?;
        for _ in 0..4 {
            let turn = page
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .expect("first load phase");
            page.run_claimed_selected_page_task_for_test(turn, &loader)
                .await?;
        }
        assert_eq!(
            page.vm_mut().eval("replacedMediaEvents.join('|')")?,
            "canplay"
        );
        let restarted = page
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .expect("the next task must belong to the replacement loadstart");
        page.run_claimed_selected_page_task_for_test(restarted, &loader)
            .await?;
        assert_eq!(page.vm_mut().eval("media.readyState")?, "0");
        assert_eq!(
            page.vm_mut().eval("replacedMediaEvents.join('|')")?,
            "canplay"
        );
        for _ in 0..4 {
            let turn = page
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .expect("replacement load phase");
            page.run_claimed_selected_page_task_for_test(turn, &loader)
                .await?;
        }
        assert_eq!(
            page.vm_mut().eval("replacedMediaEvents.join('|')")?,
            "canplay|canplay|canplaythrough"
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .is_none()
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("load sequence replacement cancels only the old terminal event");
}

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

#[tokio::test(flavor = "current_thread")]
async fn empty_local_media_fails_before_readiness_without_resetting_controls() {
    run_page_vm_async_test(async move {
        for kind in ["audio", "video"] {
            for source in ["data:,", "data:audio/ogg,", "data:video/webm;base64,"] {
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                    .expect("loader");
                let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse("https://empty-local-media.test/")?,
                );
                page.vm_mut().eval(&format!(
                    r#"
globalThis.emptyMediaTrace = [];
globalThis.emptyMedia = document.createElement({kind:?});
emptyMedia.defaultMuted = true;
emptyMedia.muted = false;
emptyMedia.volume = 0.25;
"#,
                ))?;
                // Finish setup changes before observing loading notifications.
                while page
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::MediaElementEvent,
                        &loader,
                    )
                    .await?
                {}
                page.vm_mut().eval(&format!(
                    r#"
for (const type of ['loadstart', 'loadedmetadata', 'loadeddata', 'canplay', 'volumechange']) {{
  emptyMedia.addEventListener(type, () => emptyMediaTrace.push(type));
}}
emptyMedia.onerror = () => {{
  emptyMediaTrace.push('error');
  emptyMedia.pause();
}};
setTimeout(() => emptyMediaTrace.push('timer'), 0);
emptyMedia.src = {source:?};
"#,
                ))?;
                assert_eq!(page.vm_mut().eval("emptyMediaTrace.join('|')")?, "");
                let mut tasks = 0;
                while page
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::MediaElementEvent,
                        &loader,
                    )
                    .await?
                {
                    tasks += 1;
                    anyhow::ensure!(tasks <= 12, "empty media kept scheduling tasks");
                }
                assert_eq!(
                    page.vm_mut().eval(
                        r#"emptyMediaTrace.filter(type => type === 'error').length === 1 &&
emptyMediaTrace.filter(type => type === 'loadstart').length === 1 &&
emptyMediaTrace.indexOf('loadstart') < emptyMediaTrace.indexOf('error') &&
!emptyMediaTrace.some(type => ['loadedmetadata', 'loadeddata', 'canplay', 'volumechange', 'fulfilled', 'timer'].includes(type)) &&
emptyMedia.error.code === 4 && emptyMedia.networkState === 3 && emptyMedia.readyState === 0 &&
emptyMedia.volume === 0.25 && !emptyMedia.muted && emptyMedia.defaultMuted"#,
                    )?,
                    "true",
                    "{kind}/{source}: {}",
                    page.vm_mut().eval("emptyMediaTrace.join('|')")?
                );
                assert!(page.vm().has_ready_timeout());
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("empty local media uses its owned error task without readiness events");
}

#[tokio::test(flavor = "current_thread")]
async fn media_canplaythrough_runs_in_a_separate_owned_task_after_canplay_microtasks() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("loader");
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://media-canplaythrough.test/")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page.vm_mut().eval(r#"
globalThis.mediaEvents = [];
globalThis.media = document.createElement('video');
for (const type of ['loadstart', 'loadedmetadata', 'loadeddata', 'canplay', 'canplaythrough']) {
  media.addEventListener(type, event => {
    mediaEvents.push(type + ':' + media.readyState);
    if (type === 'canplay' || type === 'canplaythrough') {
      Promise.resolve().then(() => mediaEvents.push(type + '-microtask'));
    }
  });
}
media.oncanplay = () => setTimeout(() => mediaEvents.push('timer'), 0);
document.body.append(media);
media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx';
"#)?;
        for _ in 0..4 {
            let turn = page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            ).expect("each load phase must have its own media-element task");
            page.run_claimed_selected_page_task_for_test(turn, &loader).await?;
        }
        assert_eq!(page.vm_mut().eval("mediaEvents.join('|')")?,
            "loadstart:0|loadedmetadata:1|loadeddata:2|canplay:4|canplay-microtask");
        assert!(page.vm().has_ready_timeout(), "the unrelated timer must remain queued");
        let through = page.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::MediaElementEvent,
        ).expect("HAVE_ENOUGH_DATA must queue canplaythrough after canplay");
        page.run_claimed_selected_page_task_for_test(through, &loader).await?;
        assert_eq!(page.vm_mut().eval("mediaEvents.join('|')")?,
            "loadstart:0|loadedmetadata:1|loadeddata:2|canplay:4|canplay-microtask|canplaythrough:4|canplaythrough-microtask");
        assert_eq!(page.vm_mut().eval("media.paused && media.networkState === 1")?, "true");
        assert!(page.vm().has_ready_timeout(), "canplaythrough must not consume timers");
        assert!(page.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::MediaElementEvent,
        ).is_none(), "the terminal phase must finish the load sequence exactly once");
        Ok::<_, anyhow::Error>(())
    }).await.expect("canplaythrough task ordering and timer isolation");
}

#[tokio::test(flavor = "current_thread")]
async fn media_canplaythrough_from_a_retired_document_does_not_checkpoint_the_replacement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://retired-media-canplaythrough.test/")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page.vm_mut().eval(
            r#"
globalThis.retiredMediaEvents = [];
const media = document.createElement('video');
media.oncanplaythrough = () => retiredMediaEvents.push('stale canplaythrough');
document.body.append(media);
media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx';
"#,
        )?;
        for _ in 0..4 {
            let turn = page
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .expect("load phase before Document retirement");
            page.run_claimed_selected_page_task_for_test(turn, &loader)
                .await?;
        }
        let retired = page
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .expect("capture the terminal task before its Document is retired");
        page.vm_mut().eval(
            "document.open();document.write('<!doctype html><body>replacement</body>');document.close();",
        )?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.replacementMediaEvents = [];
Promise.resolve().then(() => replacementMediaEvents.push('replacement microtask'));
"#,
        )?;
        page.vm_mut().enqueue_test_pending_runtime_source_load();
        page.run_claimed_selected_page_task_for_test(retired, &loader)
            .await?;
        assert_eq!(
            page.vm_mut().eval_without_microtask_checkpoint_for_test(
                "retiredMediaEvents.join('|') + ':' + replacementMediaEvents.join('|')",
            )?,
            ":"
        );
        assert_eq!(
            page.vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "retired terminal events must not advance replacement script work"
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .is_none()
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("retired canplaythrough owner and completion isolation");
}

#[tokio::test(flavor = "current_thread")]
async fn media_canplaythrough_is_cancelled_when_canplay_replaces_the_load_sequence() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://replaced-media-canplaythrough.test/")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page.vm_mut().eval(
            r#"
globalThis.replacedMediaEvents = [];
globalThis.media = document.createElement('video');
let first = true;
media.oncanplay = () => {
  replacedMediaEvents.push('canplay');
  if (first) {
    first = false;
    Promise.resolve().then(() => {
      media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQxAA==';
    });
  }
};
media.oncanplaythrough = () => replacedMediaEvents.push('canplaythrough');
document.body.append(media);
media.src = 'data:video/mp4;base64,AAAAGGZ0eXBtcDQyAAAAAG1wNDJtcDQx';
"#,
        )?;
        for _ in 0..4 {
            let turn = page
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .expect("first load phase");
            page.run_claimed_selected_page_task_for_test(turn, &loader)
                .await?;
        }
        assert_eq!(
            page.vm_mut().eval("replacedMediaEvents.join('|')")?,
            "canplay"
        );
        let restarted = page
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .expect("consume the old terminal retained ahead of the replacement");
        page.run_claimed_selected_page_task_for_test(restarted, &loader)
            .await?;
        assert_eq!(page.vm_mut().eval("media.readyState")?, "0");
        assert_eq!(
            page.vm_mut().eval("replacedMediaEvents.join('|')")?,
            "canplay"
        );
        let loadstart = page
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .expect("replacement loadstart follows the discarded old terminal");
        page.run_claimed_selected_page_task_for_test(loadstart, &loader)
            .await?;
        assert_eq!(page.vm_mut().eval("media.readyState")?, "0");
        for _ in 0..4 {
            let turn = page
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                )
                .expect("replacement load phase");
            page.run_claimed_selected_page_task_for_test(turn, &loader)
                .await?;
        }
        assert_eq!(
            page.vm_mut().eval("replacedMediaEvents.join('|')")?,
            "canplay|canplay|canplaythrough"
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
            )
            .is_none()
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("load sequence replacement cancels only the old terminal event");
}

#[tokio::test(flavor = "current_thread")]
async fn volume_changes_use_ordered_media_tasks_only_when_state_changes() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _resource_source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://example.test/volume-events")?,
        );
        page_vm.vm_mut().eval(r#"
            globalThis.__volumeTrace = [];
            const media = document.createElement('video');
            document.body.append(media);
            media.addEventListener('volumechange', event => {
                if (!event.isTrusted || event.target !== media || event.bubbles || event.cancelable || event.composed)
                    throw Error('volumechange native event flags');
                __volumeTrace.push(`event:${media.volume}:${media.muted}`);
                Promise.resolve().then(() => __volumeTrace.push('microtask'));
            });
            // Volume already equals its initial value. Explicitly setting muted
            // leaves its distinct initial default state and queues one event.
            media.volume = 1;
            media.muted = false;
            media.volume = 0.5;
            media.volume = 0.5;
            media.muted = true;
            media.muted = true;
            media.volume = 1;
            __volumeTrace.push('synchronous');
        "#)?;
        assert_eq!(page_vm.vm_mut().eval("__volumeTrace.join('|')")?, "synchronous");
        for _ in 0..4 {
            let task = page_vm.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent)
                .expect("each changed state queues one media event");
            assert_eq!(task.media_element_event_kind(), Some(RendererPageMediaElementEventTaskKind::VolumeChange));
            page_vm.run_claimed_selected_page_task_for_test(task, &loader).await?;
        }
        assert!(page_vm.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent).is_none());
        assert_eq!(page_vm.vm_mut().eval("__volumeTrace.join('|')")?,
            "synchronous|event:1:true|microtask|event:1:true|microtask|event:1:true|microtask|event:1:true|microtask");
        Ok::<_, anyhow::Error>(())
    }).await.expect("volumechange uses the media task source");
}
