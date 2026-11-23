#![cfg(feature = "devtools")]

use super::realtime_opportunity::run_opportunity;
use super::*;
use crate::runtime::{
    RendererCaptureScreencastFrameReply as Reply, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererScreencastPaintSession,
};
use moli_page_types::LayoutPolicy;

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
            let RendererCaptureScreenshotReply::Captured(image) = page
                .capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())
                .unwrap()
            else {
                panic!("fresh screenshot");
            };
            moli_image::decode_png(&image.bytes).unwrap().rgba
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
fn realtime_screencast_releases_paint_when_the_document_becomes_empty() {
    run_page_vm_large_stack_async_test("realtime-screencast-empty", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-screencast-empty").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval("document.body.textContent = 'Visible text'")
            .unwrap();

        let session = RendererScreencastPaintSession::default();
        let _ = page
            .capture_screencast_frame(crate::runtime::RendererCaptureScreencastFrameRequest {
                paint_session: Some(session.clone()),
                ..viewport_screencast_request(None)
            })
            .unwrap();
        assert!(page.vm().has_screencast_paint_for_test());

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
                <div id="scroller" style="position:absolute;top:58px;left:4px;width:70px;height:36px;overflow:auto;transform:translateX(.5px)">
                    <div style="position:sticky;top:0;height:8px;background:blue"></div>
                    <div style="height:130px;background:linear-gradient(yellow,green)">Text</div>
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
    });
}
