use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_uses_current_root_computed_background_and_viewport() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/root-screenshot").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><style>html { background-color: rgb(255, 0, 0); scrollbar-width: none }</style>",
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 4,
        inner_height: 3,
        outer_width: 4,
        outer_height: 3,
        device_pixel_ratio: 1.0,
        screen_width: 4,
        screen_height: 3,
        screen_avail_width: 4,
        screen_avail_height: 3,

        ..Default::default()
    };
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");
    assert!(matches!(reply, RendererPageReply::Unit));

    let red = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!((red.width, red.height), (4, 3));
    assert_eq!(decoded_png_pixel(&red.bytes, 2, 1), [255, 0, 0, 255]);
    let red_warm = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        red.bytes, red_warm.bytes,
        "cold and warm Stylo caches must produce identical snapshots"
    );

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "document.documentElement.style.backgroundColor = 'rgb(0, 255, 0)'".to_owned(),
        await_promise: false,
    })
    .await
    .expect("root background mutation should complete");
    let green = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&green.bytes, 2, 1), [0, 255, 0, 255]);
    assert_ne!(red.bytes, green.bytes);
    let green_warm = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        green.bytes, green_warm.bytes,
        "post-mutation cold and warm snapshots must agree"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_paints_downloaded_raster_image_pixels() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    loader.set_image_fetch_enabled(true);
    let fixture = moli_image::RgbaImage::try_new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255])
        .expect("valid two-pixel image");
    let encoded = moli_image::encode_png(&fixture).expect("fixture PNG should encode");
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_binary_server_with_content_type(
            "/fixture.png",
            encoded.bytes,
            "image/png",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page.html")).expect("valid fixture page URL");
    let page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0;background:white}",
            "img{display:block;width:20px;height:10px;image-rendering:pixelated}",
            "</style><img src='/fixture.png'>"
        ),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), request_seen)
        .await
        .expect("image request should start before the test deadline")
        .expect("image request signal should remain open");
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 10,
        outer_width: 20,
        outer_height: 10,
        device_pixel_ratio: 1.0,
        screen_width: 20,
        screen_height: 10,
        screen_avail_width: 20,
        screen_avail_height: 10,

        ..Default::default()
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let pending = tokio::time::timeout(
        Duration::from_secs(1),
        capture_screenshot_for_renderer_page(&page),
    )
    .await
    .expect("pending image decode must not block a fresh screenshot");
    let pending_pixel = decoded_png_pixel(&pending.bytes, 2, 5);
    assert_ne!(pending_pixel, [255, 0, 0, 255]);
    assert_ne!(pending_pixel, [0, 255, 0, 255]);

    release_response
        .send(())
        .expect("image response should release once");
    server.await.expect("image fixture server should finish");
    page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
        expression: "document.querySelector('img')?.complete && document.querySelector('img')?.naturalWidth === 2 && document.querySelector('img')?.naturalHeight === 1".to_owned(),
        timeout_ms: 2_000,
        loader: loader.clone(),
    })
    .await
    .expect("downloaded image should finish decode and dispatch load");

    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!((screenshot.width, screenshot.height), (20, 10));
    assert_ne!(screenshot.bytes, pending.bytes);
    assert_eq!(decoded_png_pixel(&screenshot.bytes, 2, 5), [255, 0, 0, 255]);
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 17, 5),
        [0, 255, 0, 255]
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn screenshot_and_screencast_paint_downloaded_svg_vectors() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    loader.set_image_fetch_enabled(true);
    let encoded = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#00ff00"/></svg>"##;
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_binary_server_with_content_type(
            "/fixture.svg",
            encoded.to_vec(),
            "image/svg+xml",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page.html")).expect("valid fixture page URL");
    let page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0;background:white}",
            "img{display:block;width:20px;height:10px;object-fit:fill}",
            "</style><img src='/fixture.svg'>"
        ),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), request_seen)
        .await
        .expect("SVG request should start before the test deadline")
        .expect("SVG request signal should remain open");
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 10,
        outer_width: 20,
        outer_height: 10,
        device_pixel_ratio: 1.0,
        screen_width: 20,
        screen_height: 10,
        screen_avail_width: 20,
        screen_avail_height: 10,

        ..Default::default()
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let pending = tokio::time::timeout(
        Duration::from_secs(1),
        capture_screenshot_for_renderer_page(&page),
    )
    .await
    .expect("pending SVG parse must not block a fresh screenshot");
    assert_ne!(decoded_png_pixel(&pending.bytes, 2, 5), [255, 0, 0, 255]);

    release_response
        .send(())
        .expect("SVG response should release once");
    server.await.expect("SVG fixture server should finish");
    page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
        expression: "document.querySelector('img')?.complete && document.querySelector('img')?.naturalWidth === 2 && document.querySelector('img')?.naturalHeight === 1".to_owned(),
        timeout_ms: 2_000,
        loader: loader.clone(),
    })
    .await
    .expect("downloaded SVG should finish parsing and dispatch load");

    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&screenshot.bytes, 2, 5), [255, 0, 0, 255]);
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 17, 5),
        [0, 255, 0, 255]
    );

    let screencast = capture_screencast_frame_with_request(
        &page,
        super::RendererCaptureScreencastFrameRequest {
            base_background_color: [255; 4],
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
            known_visual_state: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&screencast.image.bytes, 2, 5),
        [255, 0, 0, 255]
    );
    assert_eq!(
        decoded_png_pixel(&screencast.image.bytes, 17, 5),
        [0, 255, 0, 255]
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_consumes_the_shadow_flat_tree_without_light_dom_leaks() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/layout-shadow-flat-tree").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>html,body{margin:0;background:white}",
            "x-layout{display:block;width:40px}</style>",
            "<x-layout id='host'>",
            "<span slot='selected' style='display:block;width:20px;height:20px;background:red'></span>",
            "<span slot='missing' style='display:block;width:20px;height:20px;background:blue'></span>",
            "</x-layout>",
            "<div hidden style='display:block;width:20px;height:20px;background:fuchsia'></div>",
            "<script>",
            "const shadow=host.attachShadow({mode:'open'});",
            "const selected=document.createElement('slot');selected.name='selected';",
            "const suppressed=document.createElement('span');",
            "suppressed.style='display:block;width:20px;height:20px;background:yellow';",
            "selected.append(suppressed);",
            "const fallbackSlot=document.createElement('slot');fallbackSlot.name='fallback';",
            "const fallback=document.createElement('span');",
            "fallback.style='display:block;width:20px;height:20px;background:lime';",
            "fallbackSlot.append(fallback);shadow.append(selected,fallbackSlot);",
            "</script>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 40,
        inner_height: 60,
        outer_width: 40,
        outer_height: 60,
        device_pixel_ratio: 1.0,
        screen_width: 40,
        screen_height: 60,
        screen_avail_width: 40,
        screen_avail_height: 60,

        ..Default::default()
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&screenshot.bytes, 5, 5), [255, 0, 0, 255]);
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 5, 25),
        [0, 255, 0, 255]
    );
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 5, 45),
        [255, 0, 255, 255],
        "CSS must override hidden without exposing unassigned nodes or replaced slot fallback"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_encodes_jpeg_and_limits_device_dimensions() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/jpeg-screenshot").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><style>html { background: rgb(20, 80, 160) }</style>",
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 4,
        inner_height: 3,
        outer_width: 4,
        outer_height: 3,
        device_pixel_ratio: 2.0,
        screen_width: 4,
        screen_height: 3,
        screen_avail_width: 4,
        screen_avail_height: 3,

        ..Default::default()
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let request = super::RendererCaptureScreenshotRequest {
        base_background_color: [255; 4],
        format: super::RendererScreenshotFormat::Jpeg,
        quality: 80,
        region: super::RendererScreenshotRegion::Viewport,
        optimize_for_speed: false,
        max_width: Some(5),
        max_height: Some(4),
    };
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreenshot(request))
        .await
        .expect("renderer page should capture JPEG");
    let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) =
        reply
    else {
        panic!("expected captured JPEG reply");
    };
    assert_eq!(image.mime_type, "image/jpeg");
    assert_eq!((image.width, image.height), (5, 4));
    assert_eq!(&image.bytes[..2], &[0xff, 0xd8]);
    assert_eq!(&image.bytes[image.bytes.len() - 2..], &[0xff, 0xd9]);
}
#[tokio::test(flavor = "multi_thread")]
async fn print_to_pdf_uses_print_media_controls_backgrounds_and_restores_screen_media() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/print-capture").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0;background:white}",
            "#probe{width:10px;height:10px;background:red}",
            "iframe{position:absolute;left:10px;top:0;width:10px;height:10px;border:0}",
            "@media print{#probe{background:rgb(0,255,0)}}",
            "</style><div id='probe'></div>",
            "<iframe srcdoc='<style>html,body{margin:0;background:blue}</style>'></iframe>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 20,
        outer_width: 20,
        outer_height: 20,
        device_pixel_ratio: 1.0,
        screen_width: 20,
        screen_height: 20,
        screen_avail_width: 20,
        screen_avail_height: 20,

        ..Default::default()
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let screen = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&screen.bytes, 5, 5), [255, 0, 0, 255]);
    assert_eq!(decoded_png_pixel(&screen.bytes, 15, 5), [0, 0, 255, 255]);

    for print_background in [true, false] {
        let (reply, _) = page
            .run_async_command(RendererPageCommand::PrintToPdf(
                super::RendererPrintToPdfRequest {
                    print_background,
                    ..Default::default()
                },
            ))
            .await
            .expect("renderer page should print to PDF");
        let RendererPageReply::PrintToPdf(super::RendererPrintToPdfReply::Printed(bytes)) = reply
        else {
            panic!("expected printed PDF reply");
        };
        let document = lopdf::Document::load_mem(&bytes).expect("printed PDF should parse");
        let colors: Vec<[f32; 3]> = document
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .map(|stream| {
                stream
                    .decompressed_content()
                    .unwrap_or_else(|_| stream.content.clone())
            })
            .filter_map(|bytes| lopdf::content::Content::decode(&bytes).ok())
            .flat_map(|content| content.operations)
            .filter(|operation| {
                matches!(operation.operator.as_str(), "rg" | "sc" | "scn")
                    && operation.operands.len() == 3
            })
            .map(|operation| {
                std::array::from_fn(|index| {
                    operation.operands[index]
                        .as_float()
                        .expect("PDF color should be numeric")
                })
            })
            .collect();
        assert_eq!(
            colors.contains(&[0.0, 1.0, 0.0]),
            print_background,
            "print-only parent background"
        );
        assert_eq!(
            colors.contains(&[0.0, 0.0, 1.0]),
            print_background,
            "iframe background"
        );
        assert!(
            !colors.contains(&[1.0, 0.0, 0.0]),
            "PDF must not use the screen background"
        );

        let restored_screen = capture_screenshot_for_renderer_page(&page).await;
        assert_eq!(
            decoded_png_pixel(&restored_screen.bytes, 5, 5),
            [255, 0, 0, 255]
        );
        assert_eq!(
            decoded_png_pixel(&restored_screen.bytes, 15, 5),
            [0, 0, 255, 255]
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_clip_and_full_document_keep_the_live_layout_viewport() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/capture-surfaces").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>html,body{margin:0}",
            ".band{width:20px;height:20px}</style>",
            "<div class='band' style='background:red'></div>",
            "<div class='band' style='background:rgb(0,255,0)'></div>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 20,
        outer_width: 20,
        outer_height: 20,
        device_pixel_ratio: 2.0,
        screen_width: 20,
        screen_height: 20,
        screen_avail_width: 20,
        screen_avail_height: 20,

        ..Default::default()
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let full = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            base_background_color: [255; 4],
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::FullDocument,
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!((full.width, full.height), (40, 80));
    assert_eq!(decoded_png_pixel(&full.bytes, 10, 10), [255, 0, 0, 255]);
    assert_eq!(decoded_png_pixel(&full.bytes, 10, 60), [0, 255, 0, 255]);

    let clip = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            base_background_color: [255; 4],
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::PageClip(super::RendererScreenshotClip {
                x: 0.0,
                y: 20.0,
                width: 20.0,
                height: 20.0,
                scale: 0.5,
            }),
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!((clip.width, clip.height), (20, 20));
    assert_eq!(decoded_png_pixel(&clip.bytes, 10, 10), [0, 255, 0, 255]);
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_rejects_full_document_at_the_128k_css_boundary() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/capture-budget").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><style>html,body{margin:0}.page{height:131072px}</style><div class=page></div>",
    )
    .await;
    let request = super::RendererCaptureScreenshotRequest {
        base_background_color: [255; 4],
        format: super::RendererScreenshotFormat::Png,
        quality: 100,
        region: super::RendererScreenshotRegion::FullDocument,
        optimize_for_speed: false,
        max_width: None,
        max_height: None,
    };

    let error = match page
        .run_async_command(RendererPageCommand::CaptureScreenshot(request))
        .await
    {
        Ok(_) => panic!("full-document screenshot at 128K CSS pixels must be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("must each be less than 131072 CSS pixels"),
        "{error:#}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_lays_out_real_flex_mixed_flow_and_pseudo() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/layout-screenshot-poc").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        include_str!("../../../../tests/fixtures/layout-screenshot-poc.html"),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 800,
        screen_height: 600,
        screen_avail_width: 800,
        screen_avail_height: 600,

        ..Default::default()
    };
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");
    assert!(matches!(reply, RendererPageReply::Unit));

    let row = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!((row.width, row.height), (800, 600));
    assert_eq!(decoded_png_pixel(&row.bytes, 50, 20), [240, 40, 40, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 150, 20), [40, 200, 80, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 250, 20), [40, 100, 240, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 55, 130), [250, 200, 30, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 10, 150), [30, 190, 210, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 55, 170), [210, 40, 180, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 10, 190), [240, 130, 20, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 35, 210), [100, 50, 180, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 3, 250), [15, 25, 35, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 60, 265), [30, 170, 90, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 225, 295), [245, 140, 25, 255]);
    assert_eq!(
        decoded_png_pixel(&row.bytes, 250, 295),
        [255, 255, 255, 255]
    );
    assert!(
        decoded_png_dark_pixel_count(&row.bytes, 125, 205, 235, 265) > 8,
        "Parley glyphs should produce dark pixels inside the label region"
    );

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "document.querySelector('#cards').style.flexDirection = 'column'".to_owned(),
        await_promise: false,
    })
    .await
    .expect("layout mutation should complete");
    let column = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        decoded_png_pixel(&column.bytes, 150, 20),
        [255, 255, 255, 255]
    );
    assert_eq!(decoded_png_pixel(&column.bytes, 50, 60), [40, 200, 80, 255]);
    assert_eq!(
        decoded_png_pixel(&column.bytes, 50, 100),
        [40, 100, 240, 255]
    );
    assert_ne!(row.bytes, column.bytes);
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_respects_mock_layout_policy() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Mock)
        .expect("layout policy should configure before page creation");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/no-layout-screenshot").unwrap();
    let page = create_test_html_page(&runtime, &loader, url, "<!doctype html>").await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreenshot(
            super::RendererCaptureScreenshotRequest::viewport_png(),
        ))
        .await
        .expect("layout-disabled screenshot should return a typed reply");
    assert!(matches!(
        reply,
        RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::LayoutDisabled)
    ));
}
#[tokio::test(flavor = "multi_thread")]
async fn screenshot_and_screencast_flush_pending_wheel_actions_before_paint() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/action-window-capture-barriers").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        r#"<!doctype html>
<style>
html, body { margin: 0; background: white; }
html { scrollbar-width: none; }
body { height: 1200px; }
#witness { position: fixed; inset: 0; background: white; }
</style>
<div id="witness"></div>
<script>
globalThis.__lmActionWindowCaptureDeltas = [];
addEventListener("wheel", event => {
  __lmActionWindowCaptureDeltas.push(event.deltaY);
  document.getElementById("witness").style.background =
    __lmActionWindowCaptureDeltas.length === 1 ? "rgb(255, 0, 0)" : "rgb(0, 255, 0)";
}, { capture: true });
</script>"#,
    )
    .await;
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
        crate::protocol_types::ViewportSurface {
            inner_width: 20,
            inner_height: 20,
            outer_width: 20,
            outer_height: 20,
            device_pixel_ratio: 1.0,
            screen_width: 20,
            screen_height: 20,
            screen_avail_width: 20,
            screen_avail_height: 20,

            ..Default::default()
        },
    )))
    .await
    .expect("capture barrier viewport should update");

    capture_screenshot_for_renderer_page(&page).await;

    assert!(
        dispatch_wheel_for_action_window_test(&page, 10.0)
            .await
            .handled
    );
    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 10, 10),
        [255, 0, 0, 255]
    );

    assert!(
        dispatch_wheel_for_action_window_test(&page, 20.0)
            .await
            .handled
    );
    let screencast = capture_screencast_frame_with_request(
        &page,
        super::RendererCaptureScreencastFrameRequest {
            base_background_color: [255; 4],
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
            known_visual_state: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&screencast.image.bytes, 10, 10),
        [0, 255, 0, 255]
    );

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify({ scrollY, deltas: __lmActionWindowCaptureDeltas })"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("capture barrier state should remain observable");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(r#"{"scrollY":30,"deltas":[10,20]}"#))
    );

    page.close_async()
        .await
        .expect("capture barrier page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn awaited_protocol_turn_preserves_thin_capture_policy_across_wakes() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/globals-snapshot/await").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>await globals snapshot</body>",
    )
    .await;

    let output = page
        .enqueue_protocol_command(RendererPageCommand::EvaluateExpression {
            expression: r#"new Promise(resolve => {
  setTimeout(() => {
    globalThis.__lm_snapshot_after_await = "settled";
    resolve("done");
  }, 10);
})"#
            .to_owned(),
            await_promise: true,
        })
        .expect("awaited protocol command should enqueue")
        .wait()
        .await
        .expect("awaited protocol command should finish");

    assert_eq!(
        output
            .completion()
            .page_state()
            .script_execution
            .globals_snapshot_state(),
        crate::types::ScriptGlobalsSnapshotState::Dirty
    );
    let (_, refreshed) = page
        .run_async_command(RendererPageCommand::RefreshFullPageState)
        .await
        .expect("full report refresh should finish");
    assert_eq!(
        refreshed
            .script_execution
            .global("__lm_snapshot_after_await"),
        Some(&crate::types::JsValueSnapshot::String("settled".to_owned()))
    );

    page.close_async()
        .await
        .expect("awaited globals freshness test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn realtime_child_resize_updates_parent_io_and_screen_without_capture_driving_callbacks() {
    async fn command(
        page: &RendererPageHandle,
        command: RendererPageCommand,
    ) -> anyhow::Result<RendererPageReply> {
        let (reply, _) =
            tokio::time::timeout(Duration::from_secs(5), page.run_async_command(command)).await??;
        Ok(reply)
    }

    fn json_witness(reply: RendererPageReply) -> anyhow::Result<serde_json::Value> {
        let value = renderer_json_value(reply)
            .ok_or_else(|| anyhow::anyhow!("expected a stored JSON-string witness"))?;
        let serialized = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("expected a JSON string, got {value}"))?;
        Ok(serde_json::from_str(serialized)?)
    }

    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (base_url, ready, release_ready, mut ready_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/child-ro-io-ready",
            "ready",
            "text/plain; charset=utf-8",
        )
        .await;
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse(&format!("{base_url}/page")).unwrap(),
        "<!doctype html><style>html,body{margin:0;padding:0;background:white;overflow:hidden}</style><body></body>",
    )
    .await;
    let expected = serde_json::json!({
        "phases": [
            "raf:first", "root-ro:100", "child-ro:80", "child-write:320",
            "raf:next", "root-ro:320"
        ],
        "rootWidths": [100, 320],
        "rootTargets": [true, true],
        "rootRafRuns": [[1, 0], [1, 1]],
        "childWidths": [80],
        "childTargets": [true],
        "parentWidthsBeforeChild": [100],
        "parentWidthsAtNextRaf": [100],
        "intersections": [[320, 320, 20, true, 1, true]],
        "firstRafRuns": 1,
        "nextRafRuns": 1,
        "completed": true,
        "errors": ["ResizeObserver loop completed with undelivered notifications."]
    });
    let mut ready_seen = false;

    // Keep cleanup outside the fallible case, including setup and business
    // assertions. A failed business must never be repaired by a screenshot.
    let result = async {
        command(
            &page,
            RendererPageCommand::SetViewportSurface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 400,
                inner_height: 160,
                outer_width: 400,
                outer_height: 160,
                screen_width: 400,
                screen_height: 160,
                screen_avail_width: 400,
                screen_avail_height: 160,
                device_pixel_ratio: 1.0,
                ..Default::default()
            })),
        )
        .await?;
        let installed = command(
            &page,
            RendererPageCommand::EvaluateExpression {
                expression: r#"
(() => {
  const state = window.n24 = {
    phases: [], rootWidths: [], rootTargets: [], rootRafRuns: [],
    childWidths: [], childTargets: [], intersections: [], errors: [],
    parentWidthsBeforeChild: null, parentWidthsAtNextRaf: null,
    firstRafRuns: 0, nextRafRuns: 0, completed: false
  };
  const recordError = event => {
    state.errors.push(String(event.message || event.reason || 'unknown error'));
  };
  window.addEventListener('error', recordError);
  window.addEventListener('unhandledrejection', recordError);
  window.snapshotN24 = () => JSON.stringify(state);
  window.maybeCompleteN24 = () => {
    if (state.completed || state.rootWidths.length < 2 ||
        state.childWidths.length < 1 || state.intersections.length < 1 ||
        state.nextRafRuns < 1) return;
    state.completed = true;
    window.n24Result = snapshotN24();
    fetch(__READY_URL__).catch(error => state.errors.push(String(error)));
  };
  window.scheduleNextN24Frame = () => requestAnimationFrame(() => {
    state.parentWidthsAtNextRaf = state.rootWidths.slice();
    state.nextRafRuns++;
    state.phases.push('raf:next');
    maybeCompleteN24();
  });
  const parentBox = document.createElement('div');
  parentBox.id = 'parentBox';
  parentBox.style.cssText =
    'position:absolute;left:0;top:0;width:100px;height:20px;padding:0;border:0;background:red';
  document.body.appendChild(parentBox);
  const frame = document.createElement('iframe');
  frame.style.cssText =
    'position:absolute;left:0;top:60px;width:200px;height:40px;border:0;padding:0;margin:0';
  const installed = new Promise((resolve, reject) => {
    frame.addEventListener('load', () => {
      try {
        const child = frame.contentWindow;
        const childBox = child.document.getElementById('childBox');
        if (!childBox) throw new Error('loaded child is missing its RO target');
        child.addEventListener('error', recordError);
        child.addEventListener('unhandledrejection', recordError);
        window.n24RootRO = new ResizeObserver(entries => {
          for (const entry of entries) {
            state.rootWidths.push(entry.contentRect.width);
            state.rootTargets.push(entry.target === parentBox);
            state.rootRafRuns.push([state.firstRafRuns, state.nextRafRuns]);
            state.phases.push('root-ro:' + entry.contentRect.width);
          }
          maybeCompleteN24();
        });
        child.n24ChildRO = new child.ResizeObserver(child.Function('entries', `
          const state = parent.n24;
          for (const entry of entries) {
            state.childWidths.push(entry.contentRect.width);
            state.childTargets.push(entry.target === document.getElementById('childBox'));
            state.phases.push('child-ro:' + entry.contentRect.width);
          }
          if (state.childWidths.length === 1) {
            state.parentWidthsBeforeChild = state.rootWidths.slice();
            const target = parent.document.getElementById('parentBox');
            target.style.width = '320px';
            target.style.backgroundColor = 'blue';
            state.phases.push('child-write:320');
            parent.scheduleNextN24Frame();
          }
          parent.maybeCompleteN24();
        `));
        window.n24IO = new IntersectionObserver(entries => {
          for (const entry of entries) {
            state.intersections.push([
              entry.boundingClientRect.width, entry.intersectionRect.width,
              entry.boundingClientRect.height, entry.isIntersecting,
              entry.intersectionRatio, entry.target === parentBox
            ]);
          }
          maybeCompleteN24();
        }, {threshold: [0, 1]});
        n24RootRO.observe(parentBox);
        child.n24ChildRO.observe(childBox);
        n24IO.observe(parentBox);
        requestAnimationFrame(() => {
          state.firstRafRuns++;
          state.phases.push('raf:first');
        });
        resolve('installed');
      } catch (error) {
        reject(error);
      }
    }, {once: true});
  });
  frame.srcdoc = `<!doctype html>
    <style>html,body{margin:0;padding:0;background:white;overflow:hidden}</style>
    <div id="childBox" style="width:80px;height:10px;padding:0;border:0"></div>`;
  document.body.appendChild(frame);
  return installed;
})()
"#
                .replace(
                    "__READY_URL__",
                    &serde_json::to_string(&format!("{base_url}/child-ro-io-ready"))?,
                ),
                await_promise: true,
            },
        )
        .await?;
        anyhow::ensure!(
            renderer_json_value(installed) == Some(serde_json::json!("installed")),
            "the ordinary child load must install both Documents' observers"
        );

        // After installation only the owner scheduler and page callbacks run.
        // No getter, follow-up evaluate, screenshot or manual frame/task pump
        // can cause the RO/IO business to send this passive HTTP signal.
        tokio::time::timeout(Duration::from_secs(5), ready).await??;
        ready_seen = true;
        let before = json_witness(
            command(
                &page,
                RendererPageCommand::EvaluateExpression {
                    expression: "window.n24Result".to_owned(),
                    await_promise: false,
                },
            )
            .await?,
        )?;
        anyhow::ensure!(before == expected, "business before capture: {before}");

        // IO owns its sampled geometry, but its notification task need not run
        // before the next rAF/RO opportunity. Only now request the first image.
        let capture = command(
            &page,
            RendererPageCommand::CaptureScreenshot(
                super::RendererCaptureScreenshotRequest::viewport_png(),
            ),
        )
        .await?;
        let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) =
            capture
        else {
            anyhow::bail!("expected the first normal screen capture to succeed");
        };
        let after = json_witness(
            command(
                &page,
                RendererPageCommand::EvaluateExpression {
                    expression: "window.snapshotN24()".to_owned(),
                    await_promise: false,
                },
            )
            .await?,
        )?;
        Ok::<_, anyhow::Error>((after, image))
    }
    .await;

    let released = release_ready.send(());
    let server_result = if ready_seen {
        match tokio::time::timeout(Duration::from_secs(5), &mut ready_server).await {
            Ok(joined) => joined.map_err(anyhow::Error::from),
            Err(error) => {
                ready_server.abort();
                let _ = ready_server.await;
                Err(anyhow::Error::from(error))
            }
        }
    } else {
        ready_server.abort();
        match ready_server.await {
            Err(error) if error.is_cancelled() => Ok(()),
            joined => joined.map_err(anyhow::Error::from),
        }
    };
    let closed = tokio::time::timeout(Duration::from_secs(5), page.close_async()).await;
    assert!(
        result.is_ok(),
        "RO/IO/capture case failed: {:?}; release={released:?}; server={server_result:?}; close={closed:?}",
        result.as_ref().err()
    );
    assert!(
        released.is_ok(),
        "completion response release: {released:?}"
    );
    server_result.expect("completion server must finish or be cancelled during cleanup");
    closed
        .expect("Page close must reach its deadline")
        .expect("Page must close through its owner");
    let (after, image) = result.unwrap();
    assert_eq!(after, expected, "screen capture must not replay callbacks");
    assert_eq!((image.width, image.height), (400, 160));
    assert_eq!(decoded_png_pixel(&image.bytes, 310, 10), [0, 0, 255, 255]);
    assert_eq!(decoded_png_pixel(&image.bytes, 330, 10), [255; 4]);
}
