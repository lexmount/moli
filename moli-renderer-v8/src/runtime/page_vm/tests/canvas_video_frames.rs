use super::*;
use base64::Engine;

#[tokio::test(flavor = "current_thread")]
async fn canvas_video_sources_decode_real_frames_and_follow_media_ownership() {
    run_page_vm_async_test(async move {
        let pattern: &[u8] = include_bytes!("../../../../../moli-media/tests/fixtures/pattern.webm");
        let counting: &[u8] = include_bytes!("../../../../../moli-media/tests/fixtures/counting.webm");
        let vp9: &[u8] = include_bytes!("../../../../../moli-media/tests/fixtures/three-frames-vp9.webm");
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.expect("media request");
                let request = read_http_request_head(&mut stream).await.expect("request head");
                let bytes = if request.starts_with("GET /pattern ") { pattern }
                    else if request.starts_with("GET /counting ") { counting }
                    else { assert!(request.starts_with("GET /vp9 "), "{request}"); vp9 };
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: video/webm\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                stream.write_all(header.as_bytes()).await.expect("media headers");
                stream.write_all(bytes).await.expect("media body");
            }
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Video, true);
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse(&base)?);
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        let sources = serde_json::json!({
            "data": {
                "pattern": format!("data:video/webm;base64,{}", base64::engine::general_purpose::STANDARD.encode(pattern)),
                "counting": format!("data:video/webm;base64,{}", base64::engine::general_purpose::STANDARD.encode(counting)),
                "vp9": format!("data:video/webm;base64,{}", base64::engine::general_purpose::STANDARD.encode(vp9)),
            },
            "http": { "pattern": format!("{base}/pattern"), "counting": format!("{base}/counting"), "vp9": format!("{base}/vp9") },
        });
        page.vm_mut().eval(&format!("document.body.innerHTML='<iframe></iframe>';globalThis.__videoFrameSources={sources};"))?;
        page.vm_mut().eval(include_str!("../../../../tests/fixtures/canvas-video-frames.js"))?;
        let deadline = Instant::now() + Duration::from_secs(120);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page.run_next_child_frame_task_source_for_semantic_test().await;
            for selector in [PageSelectedTaskTestSelector::ResourceCompletion, PageSelectedTaskTestSelector::MediaElementEvent, PageSelectedTaskTestSelector::BitmapTask] {
                let _ = page.run_exact_selected_page_task_for_test(selector, &loader).await?;
            }
            let _ = page.run_one_due_timer_selected_task_for_test(&loader).await?;
            anyhow::ensure!(Instant::now() < deadline, "video fixture did not finish: {}", page.vm_mut().eval("__videoFrameProgress.stage")?);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        server.abort();
        let result: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let failures: Vec<_> = result["checks"].as_array().unwrap().iter().filter(|check| check["passed"] != true).collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(result["total"], 192);
        Ok::<_, anyhow::Error>(())
    }).await.expect("decoded video sources across realms and transport paths");
}

#[tokio::test(flavor = "current_thread")]
async fn failed_video_decoders_release_admission_for_later_valid_resources() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_enabled(
            crate::types::SubresourceResourceType::Video,
            true,
        );
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://failed-video-decoder-release.test/")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        let source = format!(
            "data:video/webm;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(include_bytes!(
                "../../../../../moli-media/tests/fixtures/pattern.webm"
            ),)
        );
        page.vm_mut().eval(&format!(
            "globalThis.__videoFrameSources={{data:{{pattern:{source:?}}}}};"
        ))?;
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/video-decode-failures.js"
        ))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MediaElementEvent,
                    &loader,
                )
                .await?;
            let _ = page
                .run_one_due_timer_selected_task_for_test(&loader)
                .await?;
            anyhow::ensure!(
                Instant::now() < deadline,
                "decoder failure recovery fixture did not finish"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let result: serde_json::Value =
            serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let failures: Vec<_> = result["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|check| check["passed"] != true)
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(result["total"], 2);
        let probe = page
            .vm_mut()
            .with_default_context_scope_for_test(|scope, _| {
                let global = scope.get_current_context().global(scope);
                let key = crate::util::v8str(scope, "failureProbeMedia");
                let value = global.get(scope, key.into()).unwrap();
                let object = v8::Local::<v8::Object>::try_from(value).unwrap();
                let (host, _) =
                    crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                        scope, object,
                    )?;
                Ok(unsafe { &*host }.video_decode_budget_probe_for_test())
            })?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while !probe.is_released() {
            anyhow::ensure!(
                Instant::now() < deadline,
                "completed failure fixture must release all decoder charges"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("decoder failures release admission without retiring the Document");
}

#[tokio::test(flavor = "current_thread")]
async fn captured_video_decode_tasks_retire_without_checkpointing_other_work() {
    run_page_vm_async_test(async move {
        for windowless in [false, true] {
            for retire in ["load", "document"] {
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader, Url::parse("https://video-decode-retire.test/")?,
                );
                let source = format!("data:video/webm;base64,{}", base64::engine::general_purpose::STANDARD.encode(
                    include_bytes!("../../../../../moli-media/tests/fixtures/counting.webm"),
                ));
                page.vm_mut().eval(&format!(r#"
globalThis.videoDecodeEvents = [];
globalThis.videoDecodeLoadStarted = false;
const videoDocument = {windowless} ? document.implementation.createHTMLDocument('') : document;
globalThis.decodeMedia = videoDocument.createElement('video');
videoDocument.body.appendChild(decodeMedia);
decodeMedia.onloadstart = () => {{ videoDecodeLoadStarted = true; }};
decodeMedia.onloadeddata = () => videoDecodeEvents.push('loaded');
decodeMedia.onerror = () => videoDecodeEvents.push('error');
decodeMedia.src = {source:?};
decodeMedia.load();
"#))?;
                let deadline = Instant::now() + Duration::from_secs(15);
                while page.vm_mut().eval("videoDecodeLoadStarted")? != "true" {
                    let _ = page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent, &loader).await?;
                    anyhow::ensure!(Instant::now() < deadline, "loadstart must precede decoder publication");
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                assert_eq!(page.vm_mut().eval("decodeMedia.readyState + ':' + decodeMedia.videoWidth")?, "0:0");
                let probe = page.vm_mut().with_default_context_scope_for_test(|scope, _| {
                    let global = scope.get_current_context().global(scope);
                    let key = crate::util::v8str(scope, "decodeMedia");
                    let object = v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
                    let (host, _) = crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)?;
                    Ok(unsafe { &*host }.video_decode_budget_probe_for_test())
                })?;
                let captured = loop {
                    if let Some(task) = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MediaElementEvent) {
                        break task;
                    }
                    anyhow::ensure!(Instant::now() < deadline, "background decode must publish its owned task");
                    tokio::time::sleep(Duration::from_millis(1)).await;
                };
                page.vm_mut().eval(match retire {
                    "load" => "decodeMedia.removeAttribute('src'); decodeMedia.load();",
                    _ => "document.open(); document.write('<!doctype html><body>replacement'); document.close();",
                })?;
                // Rebind the same element before consuming the captured decoder task.
                // Its retired resource epoch must never become current again.
                page.vm_mut().eval("decodeMedia.volume = 0.25")?;
                page.vm_mut().eval_without_microtask_checkpoint_for_test(r#"
globalThis.unrelatedVideoWork = [];
Promise.resolve().then(() => unrelatedVideoWork.push('microtask'));
setTimeout(() => unrelatedVideoWork.push('timer'), 0);
"#)?;
                page.vm_mut().enqueue_test_pending_runtime_source_load();
                page.run_claimed_selected_page_task_for_test(captured, &loader).await?;
                assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("unrelatedVideoWork.join('|')")?, "", "{windowless}/{retire}");
                assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("videoDecodeEvents.join('|')")?, "", "{windowless}/{retire}");
                assert!(page.vm().has_ready_timeout());
                assert_eq!(page.vm().document_runtime.runtime_script_work().dynamic_scripts.pending_source_load_count_for_test(), 1);
                let release_deadline = Instant::now() + Duration::from_secs(15);
                while !probe.is_released() {
                    anyhow::ensure!(Instant::now() < release_deadline, "retired video must release native allocation and job charges");
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("retired decode tasks do not run checkpoints or retain codec storage");
}
