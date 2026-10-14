use super::realtime_opportunity::run_opportunity;
use super::*;
use crate::runtime::{
    RendererCaptureScreencastFrameReply as CastReply, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererPrintToPdfReply, RendererPrintToPdfRequest,
    RendererScreenshotPurpose,
};
use moli_page_types::LayoutPolicy;

async fn fixture(loader: &crate::network::ResourceRequestClient) -> PageVm {
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        loader,
        Url::parse("https://example.com/realtime-print").unwrap(),
    );
    page.layout_configuration.policy = LayoutPolicy::Realtime;
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 320,
        inner_height: 100,
        device_pixel_ratio: 1.0,
        ..Default::default()
    }))
    .unwrap();
    // Realtime viewport notifications belong to the next rendering
    // opportunity, not to an immediately ready legacy environment task.
    run_opportunity(&mut page, loader).await.unwrap();
    page.vm_mut().eval(r#"
        document.head.innerHTML=`<style>
          html,body { margin:0; background:white }
          #target,iframe { position:absolute; left:0; top:0; width:40px; height:40px; border:0 }
          #target { background:red } iframe { top:50px }
          @media print { #target,iframe { left:160px; width:80px } #target { background:lime } }
        </style>`;
        document.body.innerHTML='<div id=target></div><iframe id=frame></iframe>';
        frame.contentDocument.head.innerHTML='<style>html,body{margin:0}body{height:40px;background:red}@media print{body{background:blue}}</style>';
        window.events=[];window.ticks=0;
        window.ro=new ResizeObserver(es=>events.push('ro:'+es[0].contentRect.width));ro.observe(target);
        window.io=new IntersectionObserver(es=>events.push('io:'+es[0].boundingClientRect.width));io.observe(target);
        matchMedia('print').onchange=()=>events.push('media');
        frame.contentWindow.matchMedia('print').onchange=()=>events.push('child-media');
        window.fontSheet=new CSSStyleSheet();
        fontSheet.replaceSync('@media screen{@font-face{font-family:ScreenOnly;src:local(ScreenOnly)}}@media print{@font-face{font-family:PrintOnly;src:local(PrintOnly)}}');
        document.adoptedStyleSheets=[fontSheet];
        window.liveFonts=document.fonts;window.screenFace=[...liveFonts][0];
    "#).unwrap();
    run_opportunity(&mut page, loader).await.unwrap();
    assert!(
        page.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::IntersectionObserver,
            loader,
        )
        .await
        .unwrap()
    );
    assert_eq!(
        page.vm_mut().eval("events.join('|')").unwrap(),
        "ro:40|io:40"
    );
    page
}

fn print_request() -> RendererCaptureScreenshotRequest {
    RendererCaptureScreenshotRequest {
        purpose: RendererScreenshotPurpose::Print {
            print_background: true,
        },
        ..RendererCaptureScreenshotRequest::viewport_png()
    }
}

fn screen_token(page: &PageVm) -> crate::runtime::RendererVisualStateToken {
    page.vm().visual_state_token(
        page.document_lifecycle.identity(),
        moli_layout::PaintViewport::new(320, 100, 1.0),
        [255; 4],
    )
}

fn assert_screen_state(page: &mut PageVm) {
    assert_eq!(page.vm_mut().eval("JSON.stringify([[...liveFonts].map(face=>face.family),liveFonts.has(screenFace),document.fonts===liveFonts])").unwrap(), r#"[["ScreenOnly"],true,true]"#);
    assert_eq!(
        page.vm_mut()
            .eval(
                r#"JSON.stringify([
        target.offsetWidth,frame.offsetWidth,frame.contentWindow.innerWidth,
        frame.contentDocument.body.offsetWidth,getComputedStyle(target).width,
        frame.contentWindow.getComputedStyle(frame.contentDocument.body).backgroundColor,
        matchMedia('print').matches,frame.contentWindow.matchMedia('print').matches,
        events.join('|')
    ])"#
            )
            .unwrap(),
        r#"[40,40,40,40,"40px","rgb(255, 0, 0)",false,false,"ro:40|io:40"]"#
    );
}

#[test]
fn realtime_print_preserves_screen_observers_and_screencast_without_an_extra_frame() {
    run_page_vm_large_stack_async_test("realtime-print-isolation", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = fixture(&loader).await;
        let CastReply::Captured(first) = page
            .capture_screencast_frame(viewport_screencast_request(None))
            .unwrap()
        else {
            panic!("screen capture");
        };
        // Screen publication asks existing IO targets to be checked. Settle
        // this genuine screen work before attributing any work to printing.
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_screen_state(&mut page);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        let builds = page.vm().layout_pass_observability_for_test().1;
        let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
        assert_eq!(
            screen_token(&page),
            first.visual_state,
            "clean frame and reads preserve the captured token"
        );

        let RendererCaptureScreenshotReply::Captured(print) =
            page.capture_screenshot(print_request()).unwrap()
        else {
            panic!("print capture");
        };
        assert_eq!(
            screen_token(&page),
            first.visual_state,
            "printing preserves the captured screen token"
        );
        let image = moli_image::decode_png(&print.bytes).unwrap();
        let pixel = |x: usize, y: usize| {
            let offset = (y * image.width as usize + x) * 4;
            &image.rgba[offset..offset + 4]
        };
        assert_eq!(pixel(20, 20), [255, 255, 255, 255]);
        assert_eq!(pixel(230, 20), [0, 255, 0, 255]);
        assert_eq!(
            pixel(230, 70),
            [0, 0, 255, 255],
            "child uses print media too"
        );
        assert_screen_state(&mut page);
        assert_eq!(
            screen_token(&page),
            first.visual_state,
            "screen reads after printing preserve the token"
        );
        assert_eq!(
            page.vm().layout_snapshot_cache_observability_for_test().2,
            publishes
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        for selector in [
            PageSelectedTaskTestSelector::RenderingUpdate,
            PageSelectedTaskTestSelector::IntersectionObserver,
        ] {
            assert!(
                page.claim_exact_selected_page_task_for_test(selector)
                    .is_none()
            );
        }
        assert!(matches!(
            page.capture_screencast_frame(viewport_screencast_request(Some(first.visual_state)))
                .unwrap(),
            CastReply::Unchanged
        ));
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
    });
}

#[test]
fn realtime_print_success_and_failure_preserve_pending_animation_work() {
    run_page_vm_large_stack_async_test("realtime-print-pending", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = fixture(&loader).await;
        page.vm_mut()
            .eval("requestAnimationFrame(()=>ticks++)")
            .unwrap();
        let ticket = page.current_realtime_rendering_opportunity().unwrap();
        let deadline = page.next_rendering_opportunity_deadline_for_test();
        let builds = page.vm().layout_pass_observability_for_test().1;
        page.capture_screenshot(print_request()).unwrap();
        assert_eq!(page.current_realtime_rendering_opportunity(), Some(ticket));
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            deadline
        );
        assert_eq!(page.vm_mut().eval("String(ticks)").unwrap(), "0");

        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        host.borrow().simulate_active_layout_pass_for_test(true);
        let error = page.capture_screenshot(print_request()).unwrap_err();
        host.borrow().simulate_active_layout_pass_for_test(false);
        assert_eq!(
            error.downcast_ref::<moli_layout::LayoutError>(),
            Some(&moli_layout::LayoutError::ReentrantLayoutPass)
        );
        assert_eq!(page.current_realtime_rendering_opportunity(), Some(ticket));
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            deadline
        );
        assert_screen_state(&mut page);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(ticks)").unwrap(), "1");
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert_screen_state(&mut page);
    });
}

#[test]
fn realtime_native_pdf_preserves_screen_state_and_pending_animation_work() {
    run_page_vm_large_stack_async_test("realtime-native-pdf-isolation", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = fixture(&loader).await;
        page.vm_mut()
            .eval("requestAnimationFrame(()=>ticks++)")
            .unwrap();
        let ticket = page.current_realtime_rendering_opportunity().unwrap();
        let deadline = page.next_rendering_opportunity_deadline_for_test();
        let token = screen_token(&page);
        let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
        let builds = page.vm().layout_pass_observability_for_test().1;

        let RendererPrintToPdfReply::Printed(bytes) =
            page.print_to_pdf(Default::default()).unwrap()
        else {
            panic!("native PDF capture");
        };
        assert!(
            !lopdf::Document::load_mem(&bytes)
                .unwrap()
                .get_pages()
                .is_empty()
        );
        let mut invalid = RendererPrintToPdfRequest::default();
        invalid.options.width = 0.0;
        assert!(matches!(
            page.print_to_pdf(invalid).unwrap(),
            RendererPrintToPdfReply::Failed(_)
        ));

        assert_eq!(page.current_realtime_rendering_opportunity(), Some(ticket));
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            deadline
        );
        assert_eq!(page.vm_mut().eval("String(ticks)").unwrap(), "0");
        assert_eq!(screen_token(&page), token);
        assert_screen_state(&mut page);
        assert_eq!(
            page.vm().layout_snapshot_cache_observability_for_test().2,
            publishes
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 2);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(ticks)").unwrap(), "1");
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 2);
        assert_screen_state(&mut page);
    });
}

#[test]
fn realtime_print_respects_explicit_live_media_and_live_changes_invalidate_screencast() {
    run_page_vm_large_stack_async_test("realtime-print-live-media", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = fixture(&loader).await;
        let CastReply::Captured(first) = page
            .capture_screencast_frame(viewport_screencast_request(None))
            .unwrap()
        else {
            panic!("screen capture");
        };
        for (media, x, color) in [
            ("screen", 20, [255, 0, 0, 255]),
            ("print", 230, [0, 255, 0, 255]),
        ] {
            let mut emulation = page.document_settings.emulated_media.clone();
            emulation.media = Some(media.to_owned());
            page.set_emulated_media(&emulation).unwrap();
            let RendererCaptureScreenshotReply::Captured(print) =
                page.capture_screenshot(print_request()).unwrap()
            else {
                panic!("explicit-media print capture");
            };
            let image = moli_image::decode_png(&print.bytes).unwrap();
            let offset = (20 * image.width as usize + x) * 4;
            assert_eq!(&image.rgba[offset..offset + 4], color);
            assert_eq!(
                page.vm_mut()
                    .eval("String(matchMedia('print').matches)")
                    .unwrap(),
                (media == "print").to_string()
            );
        }
        let CastReply::Captured(changed) = page
            .capture_screencast_frame(viewport_screencast_request(Some(
                first.visual_state.clone(),
            )))
            .unwrap()
        else {
            panic!("live media change must invalidate the screen token");
        };
        assert_ne!(changed.visual_state, first.visual_state);
    });
}
