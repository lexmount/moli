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
                    ),
                    &loader,
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
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
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
                    ),
                    &loader,
                )
                .await?,
            "automatic selection should run first"
        );
        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
                .await?,
            "load start should run second"
        );
        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
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
                    ),
                    &loader,
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
            .run_claimed_selected_page_task_for_test(stale, &loader)
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
                    ),
                    &loader,
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
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
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
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
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
                    ),
                    &loader,
                )
                .await?,
            "default-mode DOM turn"
        );
        take_wake_sources(&mut wake_rx);

        assert!(
            page_vm
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
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
                    ),
                    &loader,
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

#[tokio::test(flavor = "current_thread")]
async fn text_track_http_files_decode_utf8_and_report_signature_failures() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let mut server = tokio::task::JoinSet::new();
        server.spawn(async move {
            for _ in 0..12 {
                let (mut stream, _) = listener.accept().await?;
                let request = read_http_request_head(&mut stream).await?;
                let path = request.split_whitespace().nth(1).expect("HTTP path");
                let path = path.split('?').next().expect("resource path");
                let body: &[u8] = match path {
                    "/plain" => "WEBVTT\n\nlabel\n00:00.000 --> 00:01.000\ncaf\u{e9}\n".as_bytes(),
                    "/bom" => "\u{feff}WEBVTT\n\nlabel\n00:00.000 --> 00:01.000\ncaf\u{e9}\n".as_bytes(),
                    "/two-boms" => "\u{feff}\u{feff}WEBVTT\n".as_bytes(),
                    "/invalid" => b"webvtt\n\n00:00.000 --> 00:01.000\ninvalid\n",
                    "/empty-valid" => b"WEBVTT",
                    "/invalid-utf8" => b"WEBVTT\n\nlabel\n00:00.000 --> 00:01.000\ninvalid\xff\n",
                    _ => anyhow::bail!("unexpected text-track path: {path}"),
                };
                // WebVTT is UTF-8 even when HTTP claims a different charset.
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/vtt; charset=UTF-16LE\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                stream.write_all(head.as_bytes()).await?;
                stream.write_all(body).await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::TextTrack, true);
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse(&base)?);
        page.vm_mut().eval(r#"
globalThis.trackFileResults = [];
(async () => {
  const cases = [
    ['plain', 'load', 'café'], ['bom', 'load', 'café'],
    ['two-boms', 'error', null], ['invalid', 'error', null],
    ['empty-valid', 'load', null], ['invalid-utf8', 'load', 'invalid\uFFFD'],
  ];
  for (const kind of ['audio', 'video']) for (const [path, event, text] of cases) {
    const media = document.createElement(kind), track = document.createElement('track');
    track.default = true; track.kind = 'subtitles'; track.src = '/' + path + '?kind=' + kind;
    const result = await new Promise(resolve => {
      for (const type of ['load', 'error']) track.addEventListener(type, () => resolve(type), {once: true});
      media.append(track); document.body.append(media);
    });
    const cues = track.track.cues;
    if (result !== event || track.readyState !== (event === 'load' ? 2 : 3)) throw Error(kind + '/' + path + '/terminal');
    if (cues.length !== (text === null ? 0 : 1)) throw Error(kind + '/' + path + '/cue count');
    if (text !== null && (cues[0].id !== 'label' || cues[0].text !== text)) throw Error(kind + '/' + path + '/UTF-8 payload');
    trackFileResults.push(kind + '/' + path); media.remove();
  }
  globalThis.trackFilesDone = true;
})().catch(error => {globalThis.trackFileError = String(error);});
"#)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof trackFilesDone")? == "undefined" {
            anyhow::ensure!(page.vm_mut().eval("typeof trackFileError")? == "undefined", "{}", page.vm_mut().eval("trackFileError")?);
            let _ = page.run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader).await?;
            anyhow::ensure!(Instant::now() < deadline, "text-track HTTP fixture timed out: {}", page.vm_mut().eval("trackFileResults.join('|')")?);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(page.vm_mut().eval("trackFileResults.length")?, "12");
        server.join_next().await.expect("text-track server task")??;
        Ok::<_, anyhow::Error>(())
    }).await.expect("text-track files use UTF-8 and distinguish invalid signatures from empty files");
}
