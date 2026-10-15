use super::*;
use base64::Engine;

fn media_sources() -> serde_json::Value {
    serde_json::json!({
        "video": format!("data:video/webm;base64,{}", base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../../../tests/fixtures/media/movie_5.webm"))),
        "audio": format!("data:audio/ogg;base64,{}", base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../../../tests/fixtures/media/sound_5.oga"))),
    })
}

#[tokio::test(flavor = "current_thread")]
async fn media_clock_and_windowless_loads_share_owned_tasks_and_realms() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.expect("media request");
                let request = read_http_request_head(&mut stream).await.expect("request head");
                let (mime, bytes): (&str, &[u8]) = if request.starts_with("GET /audio ") {
                    ("audio/ogg", include_bytes!("../../../../tests/fixtures/media/sound_5.oga"))
                } else {
                    assert!(request.starts_with("GET /video "), "{request}");
                    ("video/webm", include_bytes!("../../../../tests/fixtures/media/movie_5.webm"))
                };
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                stream.write_all(head.as_bytes()).await.expect("media headers");
                stream.write_all(bytes).await.expect("media body");
            }
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for kind in [crate::types::SubresourceResourceType::Audio, crate::types::SubresourceResourceType::Video] {
            loader.set_optional_resource_fetch_enabled(kind, true);
        }
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse(&base)?);
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        let sources = media_sources();
        let http = serde_json::json!({"audio":format!("{base}/audio"),"video":format!("{base}/video")});
        page.vm_mut().eval(&format!("document.body.innerHTML='<iframe></iframe>';globalThis.__mediaPlaybackSources={sources};globalThis.__mediaClockHttpSources={http};"))?;
        page.vm_mut().eval(include_str!("../../../../tests/fixtures/media-playback-clock.js"))?;
        let deadline = Instant::now() + Duration::from_secs(120);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page.run_next_child_frame_task_source_for_semantic_test().await;
            for selector in [PageSelectedTaskTestSelector::ResourceCompletion, PageSelectedTaskTestSelector::MediaElementEvent] {
                let _ = page.run_exact_selected_page_task_for_test(selector, &loader).await?;
            }
            // These timers belong to the fixture's quiet periods/watchdogs.
            // The clock itself publishes only native media-source envelopes.
            let _ = page.run_one_due_timer_selected_task_for_test(&loader).await?;
            anyhow::ensure!(Instant::now() < deadline, "clock fixture did not finish: {}", page.vm_mut().eval("__mediaClockProgress.stage")?);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        server.abort();
        let result: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let failures: Vec<_> = result["checks"].as_array().unwrap().iter().filter(|check| check["passed"] != true).collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(result["total"], 80);
        Ok::<_, anyhow::Error>(())
    }).await.expect("native clock and load-neutral relevant-global routing");
}

#[tokio::test(flavor = "current_thread")]
async fn captured_clock_ticks_retire_without_checkpointing_other_work() {
    run_page_vm_async_test(async move {
        for windowless in [false, true] {
            for retire in ["pause", "load", "document"] {
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://media-clock-retire.test/")?);
                let sources = media_sources();
                page.vm_mut().eval(&format!(r#"
const clockDocument = {windowless} ? document.implementation.createHTMLDocument('') : document;
globalThis.clockMedia = clockDocument.createElement('video');
clockDocument.body.appendChild(clockMedia);
clockMedia.src = ({sources}).video;
globalThis.clockStarted = false;
clockMedia.play().then(() => {{ clockStarted = true; }}, () => {{}});
"#))?;
                let deadline = Instant::now() + Duration::from_secs(15);
                while page.vm_mut().eval("clockStarted")? != "true" {
                    let _ = page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent, &loader).await?;
                    anyhow::ensure!(Instant::now() < deadline, "video must finish decoding before capturing its playback clock");
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                while page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent, &loader).await? {}
                tokio::time::sleep(Duration::from_millis(300)).await;
                let captured = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent).expect("capture the delayed clock tick");
                page.vm_mut().eval(match retire {
                    "pause" => "clockMedia.pause()",
                    "load" => "clockMedia.removeAttribute('src'); clockMedia.load()",
                    _ => "document.open(); document.write('<!doctype html><body>replacement'); document.close();",
                })?;
                page.vm_mut().eval_without_microtask_checkpoint_for_test(r#"
globalThis.unrelatedClockWork = [];
Promise.resolve().then(() => unrelatedClockWork.push('microtask'));
setTimeout(() => unrelatedClockWork.push('timer'), 0);
"#)?;
                page.vm_mut().enqueue_test_pending_runtime_source_load();
                page.run_claimed_selected_page_task_for_test(captured, &loader).await?;
                assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("unrelatedClockWork.join('|')")?, "", "{windowless}/{retire}");
                assert!(page.vm().has_ready_timeout());
                assert_eq!(page.vm().document_runtime.runtime_script_work().dynamic_scripts.pending_source_load_count_for_test(), 1);
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("stale clock envelopes do not receive a completion checkpoint");
}

#[tokio::test(flavor = "current_thread")]
async fn windowless_network_load_is_cancelled_when_its_relevant_document_retires() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/pending.ogg", listener.local_addr()?);
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("windowless request");
            read_http_request_head(&mut stream).await.expect("request head");
            let _ = accepted_tx.send(());
            let _ = release_rx.await;
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Audio, true);
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://windowless-load-retire.test/")?);
        page.vm_mut().eval(&format!(r#"
globalThis.retiredMediaEvents = [];
const inert = document.implementation.createHTMLDocument('');
globalThis.pendingMedia = inert.createElement('audio');
inert.body.appendChild(pendingMedia);
pendingMedia.onerror = () => retiredMediaEvents.push('error');
pendingMedia.src = {url:?};
pendingMedia.play().then(() => retiredMediaEvents.push('fulfilled'), () => retiredMediaEvents.push('rejected'));
"#))?;
        tokio::time::timeout(Duration::from_secs(5), accepted_rx).await??;
        let pending_before = page.vm_mut().with_default_context_scope_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8str(scope, "pendingMedia");
            let value = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let (host, media) = crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)?;
            Ok(unsafe { &*host }.pending_media_load_sequence(media).is_some())
        })?;
        assert!(pending_before);
        page.vm_mut().eval("document.open(); document.write('<!doctype html><body>replacement'); document.close();")?;
        let pending_after = page.vm_mut().with_default_context_scope_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8str(scope, "pendingMedia");
            let value = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let (host, media) = crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)?;
            Ok(unsafe { &*host }.pending_media_load_sequence(media).is_some())
        })?;
        assert!(!pending_after, "retiring a relevant global cancels its load-neutral request");
        let _ = release_tx.send(());
        server.await?;
        assert_eq!(page.vm_mut().eval("retiredMediaEvents.join('|')")?, "");
        Ok::<_, anyhow::Error>(())
    }).await.expect("windowless resources share the relevant global lifetime");
}
