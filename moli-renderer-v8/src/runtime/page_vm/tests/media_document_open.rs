use super::*;
use base64::Engine;

#[tokio::test(flavor = "current_thread")]
async fn media_document_open_retains_resource_events_history_and_clock() {
    run_page_vm_async_test(async move {
        let video: &[u8] = include_bytes!("../../../../../moli-media/tests/fixtures/counting.webm");
        let audio: &[u8] = include_bytes!("../../../../tests/fixtures/media/sound_5.oga");
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.expect("document.open fixture request");
                let request = read_http_request_head(&mut stream).await.expect("request head");
                let (body, mime) = if request.starts_with("GET /audio ") {
                    (audio, "audio/ogg")
                } else {
                    assert!(request.starts_with("GET /video "), "{request}");
                    (video, "video/webm")
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(header.as_bytes()).await.is_ok() {
                    let _ = stream.write_all(body).await;
                }
            }
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for kind in [crate::types::SubresourceResourceType::Audio, crate::types::SubresourceResourceType::Video] {
            loader.set_optional_resource_fetch_enabled(kind, true);
        }
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse(&base)?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        let sources = serde_json::json!({
            "data": {
                "video": format!("data:video/webm;base64,{}", base64::engine::general_purpose::STANDARD.encode(video)),
                "audio": format!("data:audio/ogg;base64,{}", base64::engine::general_purpose::STANDARD.encode(audio)),
            },
            "http": {"video": format!("{base}/video"), "audio": format!("{base}/audio")},
        });
        page.vm_mut().eval(&format!(
            "document.body.innerHTML='<iframe></iframe>';globalThis.__mediaTimelineSources={sources};"
        ))?;
        page.vm_mut().eval(include_str!("../../../../tests/fixtures/media-document-open.js"))?;
        let deadline = Instant::now() + Duration::from_secs(180);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page.run_next_child_frame_task_source_for_semantic_test().await;
            for selector in [PageSelectedTaskTestSelector::ResourceCompletion, PageSelectedTaskTestSelector::MediaElementEvent] {
                let _ = page.run_exact_selected_page_task_for_test(selector, &loader).await?;
            }
            let _ = page.run_one_due_timer_selected_task_for_test(&loader).await?;
            anyhow::ensure!(Instant::now() < deadline, "document.open fixture did not finish: {}",
                page.vm_mut().eval("__mediaDefaultStartProgress.stage")?);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        server.abort();
        let result: serde_json::Value = serde_json::from_str(
            &page.vm_mut().eval("JSON.stringify(__uiEventResults)")?,
        )?;
        let failures: Vec<_> = result["checks"].as_array().unwrap().iter()
            .filter(|check| check["passed"] != true).collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(result["total"], 144);
        Ok::<_, anyhow::Error>(())
    }).await.expect("document.open preserves media resources and Window task ownership");
}
