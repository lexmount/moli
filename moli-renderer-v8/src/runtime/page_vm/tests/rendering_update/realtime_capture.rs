#![cfg(feature = "devtools")]

use super::realtime_opportunity::run_opportunity;
use super::*;
use crate::runtime::{
    RendererCaptureScreencastFrameReply as Reply, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererOutputStreamIdentity, RendererOwnerLocalHostId,
    RendererScreencastPaintSession, RendererTurnOutputJournal,
};
use moli_page_types::{DevToolsSessionKey, LayoutPolicy};

#[test]
fn paused_screencast_repaints_after_a_scanned_preload_decodes() {
    use crate::{
        native_bridge::ScannedImagePreloadAdmission,
        network::{RendererResourceTaskRunner, ResourceRequestClient},
        protocol_types::{NavigationResponse, ViewportSurface},
        types::{ImageRequestCorsMode, ImageRequestKey},
    };
    use std::{sync::Arc, time::Duration};

    run_page_vm_large_stack_async_test("paused-preload-screencast", || async move {
        let loader = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let url = Url::parse("https://example.test/").unwrap();
        let image_url = url.join("held.png").unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), url);
        page.layout_configuration.policy = LayoutPolicy::Lazy;
        page.vm_mut().set_layout_policy(LayoutPolicy::Lazy);
        page.set_viewport_surface(Some(ViewportSurface {
            inner_width: 32,
            inner_height: 32,
            outer_width: 32,
            outer_height: 32,
            screen_width: 32,
            screen_height: 32,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        page.vm_mut()
            .eval(
                r#"document.body.style.margin = '0';
                document.body.innerHTML =
                    '<div style="width:20px;height:20px;background:lime url(/held.png)"></div>';"#,
            )
            .unwrap();
        page.vm_mut().publish_layout().unwrap();

        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let key = ImageRequestKey::with_density(
            image_url.as_str().to_owned(),
            ImageRequestCorsMode::NoCors,
            1.0,
        );
        let ScannedImagePreloadAdmission::Fetch(preload) =
            host.borrow_mut().admit_scanned_image_preload(key)
        else {
            panic!("the current document should admit the scanned image");
        };

        let target = page.vm().devtools_target();
        let pause = target.pause_ref();
        let agent = page.devtools_agent_token();
        pause.configure_page_route(RendererTurnOutputJournal::new(
            RendererOutputStreamIdentity::new_page(
                RendererOwnerLocalHostId::new_for_testing(1),
                PageId::new_for_testing(1),
                agent,
            ),
        ));
        // Keep the renderer in the pause-loop state while the worker decodes.
        target
            .outbound_route(agent, DevToolsSessionKey::Primary)
            .route_notification(
                &serde_json::json!({"method":"Debugger.paused","params":{"callFrames":[]}}).into(),
            );
        assert!(pause.enter_pause().is_some());
        let Reply::Captured(pending) = page
            .capture_screencast_frame(viewport_screencast_request(None))
            .unwrap()
        else {
            panic!("expected a paused frame with the pending preload");
        };
        let pixels = moli_image::decode_png(&pending.image.bytes).unwrap();
        assert_eq!(&pixels.rgba[..4], &[0, 255, 0, 255]);
        assert!(
            page.vm()
                .css_image_resource_observability_for_test()
                .4
                .is_empty()
        );
        let next = viewport_screencast_request(Some(pending.visual_state));
        assert!(matches!(
            page.capture_screencast_frame(next.clone()).unwrap(),
            Reply::Unchanged
        ));

        let pixels = moli_image::RgbaImage::try_new(1, 1, vec![255, 0, 0, 255]).unwrap();
        let encoded = moli_parkable_image::ParkableImageManager::default()
            .from_frozen_bytes(moli_image::encode_png(&pixels).unwrap().bytes);
        let response = NavigationResponse::from_text_body(
            image_url.clone(),
            200,
            vec![("Content-Type".into(), b"image/png".to_vec())],
            String::new(),
        );
        preload.finish_network_result(
            RendererResourceTaskRunner::for_test(),
            Arc::new(Ok(response)),
            Some(encoded),
            true,
        );
        tokio::time::timeout(Duration::from_secs(3), preload.wait_outcome())
            .await
            .expect("the preload worker should finish decoding");
        assert!(
            page.vm()
                .css_image_resource_is_ready_for_test(image_url.as_str())
        );
        assert!(page.vm().is_debugger_paused());
        let next = page.capture_screencast_frame(next);
        pause.leave_pause();
        let Reply::Captured(ready) = next.unwrap() else {
            panic!("a paint-ready scanned preload should invalidate the previous frame");
        };
        let pixels = moli_image::decode_png(&ready.image.bytes).unwrap();
        assert_eq!(&pixels.rgba[..4], &[255, 0, 0, 255]);
    });
}

#[test]
fn realtime_css_images_load_during_rendering_and_repaint_with_retained_layout() {
    use crate::runtime::tests::spawn_owner_wake_gated_binary_server_with_content_type;
    use std::time::Duration;

    run_page_vm_large_stack_async_test("realtime-css-images", || async move {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="3"><rect width="3" height="3" fill="red"/></svg>"#;
        let (base, request_seen, release_image, server) =
            spawn_owner_wake_gated_binary_server_with_content_type(
                "/css.svg",
                svg.to_vec(),
                "image/svg+xml",
            )
            .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::IMAGE,
        );
        let (mut page, _, mut wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse(&format!("{base}/document")).unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut().eval(r#"
            document.head.innerHTML = `<style>
                html,body { margin: 0; background: white }
                div { position: absolute; top: 0; width: 12px; height: 12px }
                #border { left: 0; border: 4px solid lime; border-image: url(/css.svg) 1 / 4px }
                #background { left: 24px; background-image: url(/unused.svg) }
                #background { background: url(/css.svg) center / cover }
                #mask { left: 48px; background: blue; mask: url(/css.svg) center / cover }
            </style>`;
            document.body.innerHTML = '<div id=border></div><div id=background></div><div id=mask></div>';
            requestAnimationFrame(() => {});
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), request_seen)
            .await
            .unwrap()
            .unwrap();
        let image_url = format!("{base}/css.svg");
        assert_eq!(
            page.vm()
                .css_image_resource_observability_for_test()
                .4
                .as_slice(),
            std::slice::from_ref(&image_url)
        );

        let session = RendererScreencastPaintSession::default();
        let request = |state| crate::runtime::RendererCaptureScreencastFrameRequest {
            paint_session: Some(session.clone()),
            ..viewport_screencast_request(state)
        };
        let Reply::Captured(pending) = page.capture_screencast_frame(request(None)).unwrap() else {
            panic!("expected a frame with a pending CSS image");
        };
        let pixels = moli_image::decode_png(&pending.image.bytes).unwrap();
        assert_eq!(&pixels.rgba[4..8], &[255; 4]);
        assert_eq!(&pixels.rgba[100..104], &[255; 4]);
        let passes = page.vm().layout_pass_observability_for_test().1;

        release_image.send(()).unwrap();
        server.await.unwrap();
        let completion = page.vm().css_image_completion_notify_for_test();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notified = completion.notified();
                if page.vm().css_image_resource_is_ready_for_test(&image_url) {
                    break;
                }
                if !page
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ResourceCompletion,
                        &loader,
                    )
                    .await
                    .unwrap()
                {
                    tokio::select! {
                        _ = notified => {},
                        message = wake.recv() => assert!(message.is_some()),
                    }
                }
            }
        })
        .await
        .expect("CSS image must decode");

        let Reply::Captured(ready) = page
            .capture_screencast_frame(request(Some(pending.visual_state)))
            .unwrap()
        else {
            panic!("decoding the CSS image must produce a new frame");
        };
        let pixels = moli_image::decode_png(&ready.image.bytes).unwrap();
        assert_eq!(&pixels.rgba[4..8], &[255, 0, 0, 255]);
        assert_eq!(&pixels.rgba[100..104], &[255, 0, 0, 255]);
        assert_eq!(&pixels.rgba[196..200], &[0, 0, 255, 255]);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
    });
}

#[test]
fn realtime_capture_keeps_clean_tokens_across_frames_and_paints_current_style() {
    run_page_vm_large_stack_async_test("realtime-capture", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-capture").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 40,
            inner_height: 30,
            outer_width: 40,
            outer_height: 30,
            screen_width: 40,
            screen_height: 30,
            screen_avail_width: 40,
            screen_avail_height: 30,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        // The live viewport setter also queues its existing environment event.
        // Complete that initialization task before driving explicit frames.
        page.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate,
            &loader,
        )
        .await
        .unwrap();
        page.vm_mut().eval(r#"
            document.documentElement.style.margin='0';document.body.style.margin='0';
            document.body.innerHTML='<div id=target style="width:10px;height:10px;background:red"></div>';
            const canvas = document.createElement('canvas');
            canvas.width = 8;canvas.height = 8;
            canvas.style.cssText = 'position:absolute;top:0;left:32px';
            document.body.append(canvas);
            window.paintCanvas = color => {
                const context = canvas.getContext('2d');
                context.fillStyle = color;
                context.fillRect(0, 0, 8, 8);
            };
            paintCanvas('green');
            window.ticks=0;window.report=0;
            window.ro=new ResizeObserver(entries=>report=entries[0].contentRect.width);ro.observe(target);
            requestAnimationFrame(()=>target.style.width='20px');
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(report)").unwrap(), "20");
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
        let session = RendererScreencastPaintSession::default();
        let request = |known_visual_state| crate::runtime::RendererCaptureScreencastFrameRequest {
            paint_session: Some(session.clone()),
            ..viewport_screencast_request(known_visual_state)
        };
        let Reply::Captured(first) = page.capture_screencast_frame(request(None)).unwrap() else {
            panic!("initial capture");
        };
        let after_capture = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page.vm_mut()
                .eval("getComputedStyle(target).width")
                .unwrap(),
            "20px"
        );
        page.vm_mut()
            .eval("requestAnimationFrame(()=>ticks++)")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(ticks)").unwrap(), "1");
        assert_eq!(
            page.capture_screencast_frame(request(Some(first.visual_state.clone())))
                .unwrap(),
            Reply::Unchanged
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            after_capture,
            "clean CSSOM, rAF, RO and Unchanged must reuse the canonical tree"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );

        page.vm_mut()
            .eval("target.style.width='30px';target.style.background='blue'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(report)").unwrap(), "30");
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
        let after_rendering = page.vm().layout_pass_observability_for_test().1;
        let Reply::Captured(second) = page
            .capture_screencast_frame(request(Some(first.visual_state)))
            .unwrap()
        else {
            panic!("source change needs fresh pixels");
        };
        let image = moli_image::decode_png(&second.image.bytes).unwrap();
        assert_eq!(&image.rgba[..4], &[0, 0, 255, 255]);
        assert_eq!(&image.rgba[128..132], &[0, 128, 0, 255]);
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            after_rendering
        );
        assert_eq!(
            page.capture_screencast_frame(request(Some(second.visual_state.clone())))
                .unwrap(),
            Reply::Unchanged
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            after_rendering
        );
        page.vm_mut().eval("paintCanvas('red')").unwrap();
        let Reply::Captured(canvas_frame) = page
            .capture_screencast_frame(request(Some(second.visual_state)))
            .unwrap()
        else {
            panic!("canvas resource change needs fresh pixels");
        };
        let image = moli_image::decode_png(&canvas_frame.image.bytes).unwrap();
        assert_eq!(&image.rgba[128..132], &[255, 0, 0, 255]);
    });
}

#[test]
fn realtime_screencast_refreshes_document_selection_and_live_range_paint() {
    run_page_vm_large_stack_async_test("realtime-screencast-selection", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-screencast-selection").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 240,
            inner_height: 120,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        page.vm_mut()
            .eval(
                r#"
            document.body.style.cssText = 'margin:0;background:white';
            document.body.innerHTML = '<div id="target">Select this text</div><iframe id="child" style="width:190px;height:48px;border:0"></iframe>';
            child.contentDocument.body.style.cssText = 'margin:0;background:white';
            child.contentDocument.body.innerHTML = '<div id="target">Child text selection</div>';
            window.selectText = doc => {
                const text = doc.getElementById('target').firstChild;
                window.liveRange = doc.createRange();
                liveRange.selectNodeContents(text);
                doc.getSelection().addRange(liveRange);
            };
            "#,
            )
            .unwrap();

        let session = RendererScreencastPaintSession::default();
        let request = |known| crate::runtime::RendererCaptureScreencastFrameRequest {
            paint_session: Some(session.clone()),
            ..viewport_screencast_request(known)
        };
        let capture = |page: &mut PageVm, known| {
            let Reply::Captured(frame) = page.capture_screencast_frame(request(known)).unwrap()
            else {
                panic!("selection-only changes must produce a frame");
            };
            (
                moli_image::decode_png(&frame.image.bytes).unwrap().rgba,
                frame.visual_state,
            )
        };
        let fresh_pixels = |page: &mut PageVm| {
            let viewport = moli_layout::LayoutViewport::new(240, 120, 1.0);
            let snapshot = page
                .vm_mut()
                .screenshot_layout_snapshot(viewport)
                .unwrap()
                .expect("fresh screenshot");
            moli_paint::raster_snapshot(&snapshot).unwrap().rgba
        };
        let (unselected, mut known) = capture(&mut page, None);
        for document in ["document", "child.contentDocument"] {
            page.vm_mut()
                .eval(&format!("selectText({document})"))
                .unwrap();
            let (selected, state) = capture(&mut page, Some(known));
            assert_ne!(selected, unselected, "{document}");
            assert_eq!(selected, fresh_pixels(&mut page), "{document}");

            let shorten = "liveRange.setEnd(liveRange.endContainer, 4)";
            page.vm_mut().eval(shorten).unwrap();
            let (shortened, state) = capture(&mut page, Some(state));
            assert_ne!(shortened, selected, "{document}");
            assert_ne!(shortened, unselected, "{document}");
            page.vm_mut().eval(shorten).unwrap();
            assert_eq!(
                page.capture_screencast_frame(request(Some(state.clone())))
                    .unwrap(),
                Reply::Unchanged,
                "{document}"
            );
            assert_eq!(shortened, fresh_pixels(&mut page), "{document}");

            page.vm_mut()
                .eval(&format!("{document}.getSelection().removeAllRanges()"))
                .unwrap();
            let (cleared, state) = capture(&mut page, Some(state));
            assert_eq!(cleared, unselected, "{document}");
            known = state;
        }
    });
}

#[test]
fn realtime_screencast_reuses_numeric_layout_with_an_active_session() {
    run_page_vm_large_stack_async_test("realtime-screencast-layout-cache", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-screencast-layout-cache").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval(
                r#"
            document.head.innerHTML = `<style>
                body{margin:0;display:flex;height:200px;align-items:flex-start}
                section{width:140px;flex:none}
                section div{height:12px;background:blue}
                #stable div{background:red}
            </style>`;
            document.body.innerHTML = '<section id=edited><div></div><div></div></section>' +
                '<section id=stable><div></div></section>';
        "#,
            )
            .unwrap();
        let session = RendererScreencastPaintSession::default();
        let request = |known| crate::runtime::RendererCaptureScreencastFrameRequest {
            paint_session: Some(session.clone()),
            ..viewport_screencast_request(known)
        };
        let Reply::Captured(first) = page.capture_screencast_frame(request(None)).unwrap() else {
            panic!("initial screencast frame");
        };
        let before = moli_image::decode_png(&first.image.bytes).unwrap();
        assert_eq!(&before.rgba[500..504], &[0, 0, 255, 255]);

        page.vm_mut().eval("edited.style.width='120px'").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let (_, builds, _, metrics) = page.vm().layout_pass_observability_for_test();
        let metrics = metrics.unwrap();
        assert!(metrics.retained_box_count > 0);
        assert!(metrics.retained_numeric_layout_hits > 0);
        assert!(page.vm().has_screencast_paint_for_test());

        let Reply::Captured(second) = page
            .capture_screencast_frame(request(Some(first.visual_state)))
            .unwrap()
        else {
            panic!("edited geometry must produce a new frame");
        };
        let after = moli_image::decode_png(&second.image.bytes).unwrap();
        assert_eq!(&after.rgba[500..504], &[255, 0, 0, 255]);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
    });
}

#[test]
fn realtime_screencast_releases_paint_when_the_document_becomes_empty() {
    run_page_vm_large_stack_async_test("realtime-screencast-empty", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-screencast-empty").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        // Vertical writing mode keeps screen paint without a reusable layout.
        page.vm_mut()
            .eval("document.documentElement.style.writingMode = 'vertical-rl'; document.body.textContent = 'Visible text'")
            .unwrap();

        let session = RendererScreencastPaintSession::default();
        let _ = page
            .capture_screencast_frame(crate::runtime::RendererCaptureScreencastFrameRequest {
                paint_session: Some(session.clone()),
                ..viewport_screencast_request(None)
            })
            .unwrap();
        assert!(page.vm().has_screencast_paint_for_test());
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        assert!(
            !host.borrow().has_retained_layout_for_test(),
            "screen paint can outlive a pass that cannot retain layout",
        );

        page.vm_mut()
            .eval("window.savedRoot = document.documentElement; savedRoot.remove()")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert!(!page.vm().has_screencast_paint_for_test());

        page.vm_mut().eval("document.append(savedRoot)").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert!(page.vm().has_screencast_paint_for_test());
    });
}

#[test]
fn realtime_screenshot_reuses_active_screen_paint_and_svg_scenes() {
    use crate::runtime::{RendererScreenshotClip, RendererScreenshotRegion};

    run_page_vm_large_stack_async_test("realtime-screenshot-screen-paint", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-screenshot-screen-paint").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.set_scrollbars_hidden(true).unwrap();
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 96,
            inner_height: 64,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        page.vm_mut().eval(r#"
            document.body.style.cssText = 'margin:0;height:260px;background:white';
            document.body.innerHTML = `
                <svg style="display:block" width="20" height="20">
                    <rect id="shape" width="20" height="20" fill="red"/>
                </svg>
                <iframe id="distant" style="position:absolute;left:0;top:120px;width:32px;height:32px;border:0"></iframe>
                <div style="position:fixed;top:5px;left:60px;width:10px;height:10px;background:yellow"></div>
            `;
            const child = document.getElementById('distant').contentDocument;
            child.body.style.margin = '0';
            child.body.innerHTML =
                '<svg style="display:block" width="20" height="20"><rect width="20" height="20" fill="blue"/></svg>';
        "#).unwrap();

        let session = RendererScreencastPaintSession::default();
        let screencast = |page: &mut PageVm| {
            let Reply::Captured(frame) = page
                .capture_screencast_frame(crate::runtime::RendererCaptureScreencastFrameRequest {
                    paint_session: Some(session.clone()),
                    ..viewport_screencast_request(None)
                })
                .unwrap()
            else {
                panic!("expected a screencast frame");
            };
            moli_image::decode_png(&frame.image.bytes).unwrap()
        };
        let screenshot = |page: &mut PageVm, region| {
            let RendererCaptureScreenshotReply::Captured(image) = page
                .capture_screenshot(RendererCaptureScreenshotRequest {
                    region,
                    ..RendererCaptureScreenshotRequest::viewport_png()
                })
                .unwrap()
            else {
                panic!("expected an explicit screenshot");
            };
            moli_image::decode_png(&image.bytes).unwrap()
        };
        let before = crate::layout_renderer::svg_parse_count();
        let initial = screencast(&mut page);
        assert_eq!(&initial.rgba[..4], [255, 0, 0, 255]);
        assert_eq!(crate::layout_renderer::svg_parse_count(), before + 1);
        let builds = page.vm().layout_pass_observability_for_test().1;

        let viewport = screenshot(&mut page, RendererScreenshotRegion::Viewport);
        assert_eq!(viewport.rgba, initial.rgba);
        assert_eq!(crate::layout_renderer::svg_parse_count(), before + 1);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);

        let clip = RendererScreenshotRegion::PageClip(RendererScreenshotClip {
            x: 0.0,
            y: 120.0,
            width: 20.0,
            height: 20.0,
            scale: 2.0,
        });
        let clipped = screenshot(&mut page, clip);
        assert_eq!((clipped.width, clipped.height), (40, 40));
        assert_eq!(&clipped.rgba[..4], [0, 0, 255, 255]);
        assert_eq!(crate::layout_renderer::svg_parse_count(), before + 2);
        assert_eq!(screencast(&mut page).rgba, initial.rgba);
        assert_eq!(crate::layout_renderer::svg_parse_count(), before + 2);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);

        page.vm_mut()
            .eval("document.getElementById('shape').setAttribute('fill','lime')")
            .unwrap();
        let updated = screenshot(&mut page, RendererScreenshotRegion::Viewport);
        assert_eq!(&updated.rgba[..4], [0, 255, 0, 255]);
        // Settle the pending style work from the mutation before measuring scroll.
        run_opportunity(&mut page, &loader).await.unwrap();
        let changed_builds = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut().eval("scrollTo(0,100)").unwrap();
        let full = screenshot(&mut page, RendererScreenshotRegion::FullDocument);
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            changed_builds
        );

        drop(session);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while page.vm().has_screencast_paint_for_test() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the last consumer releases paint after explicit screenshots");

        let fresh = screenshot(&mut page, RendererScreenshotRegion::FullDocument);
        assert_eq!((full.width, full.height), (fresh.width, fresh.height));
        assert_eq!(full.rgba, fresh.rgba);
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            changed_builds + 1
        );
        assert!(!page.vm().has_screencast_paint_for_test());
    });
}

#[test]
fn realtime_screenshot_defers_lazy_image_admission_until_resumed() {
    run_page_vm_large_stack_async_test("realtime-screenshot-lazy-image", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-screenshot-lazy-image").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 96,
            inner_height: 64,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        page.vm_mut().eval(r#"
            document.body.style.cssText = 'margin:0;height:2200px';
            document.body.innerHTML = '<img id=probe loading=lazy ' +
                'style="position:absolute;top:2000px" width=20 height=20 ' +
                'src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">';
        "#).unwrap();
        let image = page
            .vm()
            .document_runtime
            .get_element_by_id("probe")
            .unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let session = RendererScreencastPaintSession::default();
        assert!(matches!(
            page.capture_screencast_frame(crate::runtime::RendererCaptureScreencastFrameRequest {
                paint_session: Some(session.clone()),
                ..viewport_screencast_request(None)
            })
            .unwrap(),
            Reply::Captured(_)
        ));
        page.vm_mut().eval("probe.style.top = '0px'").unwrap();
        assert!(!host.borrow().has_pending_image_load_event_for_test(image));
        assert!(page.vm().has_screencast_paint_for_test());
        let builds = page.vm().layout_pass_observability_for_test().1;

        let target = page.vm().devtools_target();
        let pause = target.pause_ref();
        let agent = page.devtools_agent_token();
        pause.configure_page_route(RendererTurnOutputJournal::new(
            RendererOutputStreamIdentity::new_page(
                RendererOwnerLocalHostId::new_for_testing(1),
                PageId::new_for_testing(1),
                agent,
            ),
        ));
        target
            .outbound_route(agent, DevToolsSessionKey::Primary)
            .route_notification(
                &serde_json::json!({"method":"Debugger.paused","params":{"callFrames":[]}}).into(),
            );
        assert!(pause.enter_pause().is_some());
        let request = RendererCaptureScreenshotRequest::viewport_png();
        let paused = page.capture_screenshot(request.clone());
        pause.leave_pause();
        assert!(matches!(
            paused.unwrap(),
            RendererCaptureScreenshotReply::Captured(_)
        ));
        assert!(!host.borrow().has_pending_image_load_event_for_test(image));
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);

        assert!(matches!(
            page.capture_screenshot(request).unwrap(),
            RendererCaptureScreenshotReply::Captured(_)
        ));
        assert!(host.borrow().has_pending_image_load_event_for_test(image));
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
    });
}

#[test]
fn realtime_screencast_reuses_published_scroll_and_matches_fresh_pixels() {
    run_page_vm_large_stack_async_test("realtime-screencast-scroll", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-screencast-scroll").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 160,
            inner_height: 120,
            outer_width: 160,
            outer_height: 120,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        page.vm_mut().eval(r#"
            document.documentElement.style.margin = '0';
            document.body.style.cssText = 'margin:0;height:850px;background:white';
            document.body.innerHTML = `
                <div style="position:fixed;top:3px;right:3px;width:10px;height:10px;background:red"></div>
                <div id="overlay" style="position:fixed;top:37px;left:8px;width:35px;height:47px;background:magenta;clip:rect(1px,24px,40px,3px);z-index:1;opacity:.6"></div>
                <div id="scroller" style="position:absolute;top:58px;left:4px;width:70px;height:36px;overflow:auto;transform:translateX(.5px)">
                    <div style="position:sticky;top:0;height:8px;background:blue"></div>
                    <div style="height:130px;background:linear-gradient(yellow,green)"><span style="background:cyan">Text <span style="display:inline-block;width:8px;height:8px;background:black"></span> wraps</span></div>
                </div>
                <iframe id="frame" style="position:absolute;left:85px;top:50px;width:65px;height:45px;border:0"></iframe>
            `;
            const child = frame.contentDocument;
            child.documentElement.style.margin = '0';
            child.body.style.cssText = 'margin:0;height:350px;background:linear-gradient(orange,purple)';
            child.body.textContent = 'Child';
        "#).unwrap();

        let session = RendererScreencastPaintSession::default();
        let capture = |page: &mut PageVm, session: &RendererScreencastPaintSession, known| {
            let Reply::Captured(frame) = page
                .capture_screencast_frame(crate::runtime::RendererCaptureScreencastFrameRequest {
                    paint_session: Some(session.clone()),
                    ..viewport_screencast_request(known)
                })
                .unwrap()
            else {
                panic!("changed scroll state must produce a frame");
            };
            (
                moli_image::decode_png(&frame.image.bytes).unwrap().rgba,
                frame.visual_state,
            )
        };
        let (initial_pixels, mut known) = capture(&mut page, &session, None);
        let builds = page.vm().layout_pass_observability_for_test().1;
        let scroll_steps = [
            "scrollTo(0,18);scroller.scrollTop=9;frame.contentWindow.scrollTo(0,11)",
            "scrollTo(0,42);scroller.scrollTop=27;frame.contentWindow.scrollTo(0,25)",
            "scrollTo(0,0);scroller.scrollTop=0;frame.contentWindow.scrollTo(0,0)",
        ];
        let mut captured = Vec::new();
        for step in scroll_steps {
            page.vm_mut().eval(step).unwrap();
            let (pixels, state) = capture(&mut page, &session, Some(known));
            captured.push(pixels);
            known = state;
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        }
        assert_ne!(captured[0], initial_pixels);
        assert_ne!(captured[0], captured[1]);
        assert_eq!(captured[2], initial_pixels);

        let other_session = RendererScreencastPaintSession::default();
        capture(&mut page, &other_session, None);
        drop(session);
        capture(&mut page, &other_session, None);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        drop(other_session);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while page.vm().has_screencast_paint_for_test() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the last subscription releases paint while the page is idle");
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        assert!(host.borrow().has_retained_layout_for_test());

        let next_session = RendererScreencastPaintSession::default();
        capture(&mut page, &next_session, None);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        drop(next_session);

        for (step, pixels) in scroll_steps.into_iter().zip(captured) {
            page.vm_mut().eval(step).unwrap();
            let RendererCaptureScreenshotReply::Captured(fresh) = page
                .capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())
                .unwrap()
            else {
                panic!("fresh screenshot");
            };
            assert_eq!(moli_image::decode_png(&fresh.bytes).unwrap().rgba, pixels);
        }

        let session = RendererScreencastPaintSession::default();
        let (before_style, known) = capture(&mut page, &session, None);
        page.vm_mut()
            .eval("overlay.style.clip='rect(3px,31px,24px,13px)';overlay.style.zIndex='-1'")
            .unwrap();
        let (after_style, _) = capture(&mut page, &session, Some(known));
        assert_ne!(before_style, after_style);
        drop(session);
        let RendererCaptureScreenshotReply::Captured(fresh) = page
            .capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())
            .unwrap()
        else {
            panic!("fresh screenshot after the clip and stacking order changed");
        };
        assert_eq!(
            moli_image::decode_png(&fresh.bytes).unwrap().rgba,
            after_style
        );
    });
}
