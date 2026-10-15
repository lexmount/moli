use super::*;

#[tokio::test(flavor = "current_thread")]
async fn canvas_svg_image_sources_render_at_the_consumer_viewport_without_layout() {
    run_page_vm_async_test(async move {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="lime"/></svg>"#;
        let (base_url, server) = spawn_path_response_http_server(vec![
            ("/cross-main.svg", "HTTP/1.1 200 OK", svg.to_owned(), Duration::ZERO),
            ("/cross-iframe.svg", "HTTP/1.1 200 OK", svg.to_owned(), Duration::ZERO),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("loader");
        loader.set_image_fetch_enabled(true);
        let (mut page, _resource_source, _owner_wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://svg-image-source.test/")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page.vm_mut().eval(&format!(
            "document.body.innerHTML = '<iframe id=child></iframe>'; globalThis.__svgCrossOriginURL = {base_url:?};"
        ))?;
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/canvas-svg-images.js"
        ))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page
                .run_next_child_frame_task_source_for_semantic_test()
                .await;
            for selector in [
                PageSelectedTaskTestSelector::ResourceCompletion,
                PageSelectedTaskTestSelector::WorkerHostBridge,
                PageSelectedTaskTestSelector::BitmapTask,
                PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                PageSelectedTaskTestSelector::DomManipulation(
                    PageDomManipulationTestFamily::ImageLoadEvent,
                ),
            ] {
                let _ = page
                    .run_exact_selected_page_task_for_test(selector, &loader)
                    .await?;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "SVG image fixture did not finish: {}",
                page.vm_mut().eval("JSON.stringify(__svgImageProgress)")?
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        server.abort();
        let result: serde_json::Value =
            serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let failures: Vec<_> = result["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["passed"] != true)
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(
            result["total"], 186,
            "all source/callee realms and both Canvas contexts must run"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG image sources, bitmap snapshots, transfer and origin cleanliness");
}

#[tokio::test(flavor = "current_thread")]
async fn canvas_draw_image_reads_accepted_images_without_layout() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut()
            .eval("document.body.innerHTML = '<iframe id=child></iframe>';")?;
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/canvas-loaded-images.js"
        ))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            for selector in [
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
                PageSelectedTaskTestSelector::BitmapTask,
                PageSelectedTaskTestSelector::DomManipulation(
                    PageDomManipulationTestFamily::ImageLoadEvent,
                ),
            ] {
                let _ = page
                    .run_exact_selected_page_task_for_test(selector, &loader)
                    .await?;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "loaded image fixture did not finish"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let result: serde_json::Value =
            serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let failures: Vec<_> = result["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["passed"] != true)
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(
            result["total"], 301,
            "both source and callee realms must run"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("loaded raster images and drawImage conversion/brand semantics");
}

#[tokio::test(flavor = "current_thread")]
async fn canvas_draw_image_accepts_registered_native_proxies() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<iframe id=child></iframe>';
            globalThis.nativeCanvasImage = new Image();
            globalThis.nativeCanvasContext = new OffscreenCanvas(1,1).getContext('2d');
            globalThis.__nativeImageLoaded = false;
            nativeCanvasImage.onload = () => { __nativeImageLoaded = true; };
            const sourceCanvas=document.createElement('canvas');sourceCanvas.width=sourceCanvas.height=1;
            const sourceContext=sourceCanvas.getContext('2d');sourceContext.fillStyle='red';sourceContext.fillRect(0,0,1,1);
            nativeCanvasImage.src = sourceCanvas.toDataURL();
        "#)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while page.vm_mut().eval("__nativeImageLoaded")? != "true" {
            let _ = page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::DomManipulation(
                    PageDomManipulationTestFamily::ImageLoadEvent,
                ), &loader,
            ).await?;
            anyhow::ensure!(Instant::now() < deadline, "native proxy image did not load");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        page.vm_mut().register_canvas_image_proxies_for_test()?;
        assert_eq!(page.vm_mut().eval(r#"
            (() => {
                const other = document.getElementById('child').contentWindow;
                const draw = other.OffscreenCanvasRenderingContext2D.prototype.drawImage;
                draw.call(canvasContextProxy, canvasImageProxy, 0, 0);
                if (JSON.stringify(Array.from(nativeCanvasContext.getImageData(0,0,1,1).data)) !== '[255,0,0,255]') return 'native pixels';
                let conversions=0, traps=0;
                const value={valueOf(){conversions++;return 0;}};
                for (const [receiver,source] of [
                    [new Proxy(canvasContextProxy,{}),canvasImageProxy],
                    [canvasContextProxy,new Proxy(canvasImageProxy,{get(){traps++;throw Error('trap');}})],
                ]) {
                    let error;
                    try {draw.call(receiver,source,value,0);} catch(caught) {error=caught;}
                    if (!(error instanceof other.TypeError) || conversions!==0 || traps!==0) return 'author proxy';
                }
                return 'ok';
            })()
        "#)?, "ok");
        Ok::<_, anyhow::Error>(())
    }).await.expect("native drawImage proxies preserve source and receiver brands");
}
