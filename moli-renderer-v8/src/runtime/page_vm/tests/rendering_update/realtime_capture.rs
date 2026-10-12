use super::realtime_opportunity::run_opportunity;
use super::*;
use crate::runtime::RendererCaptureScreencastFrameReply as Reply;
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
        let Reply::Captured(first) = page
            .capture_screencast_frame(viewport_screencast_request(None))
            .unwrap()
        else {
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
            page.capture_screencast_frame(viewport_screencast_request(Some(
                first.visual_state.clone()
            )))
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
        let Reply::Captured(second) = page
            .capture_screencast_frame(viewport_screencast_request(Some(first.visual_state)))
            .unwrap()
        else {
            panic!("source change needs fresh pixels");
        };
        let image = moli_image::decode_png(&second.image.bytes).unwrap();
        assert_eq!(&image.rgba[..4], &[0, 0, 255, 255]);
        let after_capture = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page.capture_screencast_frame(viewport_screencast_request(Some(second.visual_state)))
                .unwrap(),
            Reply::Unchanged
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            after_capture
        );
    });
}
