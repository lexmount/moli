use super::*;

#[test]
fn realtime_ro_depth_loop_broadcasts_deeper_targets_and_defers_skipped_targets() {
    run_page_vm_large_stack_async_test("realtime-ro-depth", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id="outer" style="width:100px"><div id="middle" style="width:80px"><div id="inner" style="width:60px;height:10px"></div></div></div>';
            window.order=[];window.calls=0;
            window.onerror=message=>{order.push(message);return true};
            window.observer=new ResizeObserver(entries=>{
                order.push(entries.map(e=>e.target.id+':'+e.contentRect.width).join(','));
                if(++calls===1){outer.style.width='110px';middle.style.width='90px'}
                else if(calls===2){inner.style.width='70px'}
            });
            observer.observe(outer);observer.observe(middle);observer.observe(inner);
            requestAnimationFrame(()=>order.push('raf'));
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(order)").unwrap(),
            r#"["raf","outer:100,middle:80,inner:60","middle:90","inner:70","ResizeObserver loop completed with undelivered notifications."]"#
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("String(calls)+'|'+order.at(-1)")
                .unwrap(),
            "4|outer:110"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_broadcast_uses_geometry_published_by_an_earlier_callback() {
    run_page_vm_large_stack_async_test("realtime-ro-broadcast-current", || async move {
        for force in [false, true] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            page.vm_mut().eval(&format!(r#"
                document.body.innerHTML='<div id=a style="width:100px;height:20px"></div><div id=b style="width:100px;height:20px"></div>';
                window.order=[];window.errors=0;window.onerror=()=>{{errors++;return true}};
                window.first=new ResizeObserver(()=>{{order.push('first');b.style.width='200px';if({force})void b.offsetWidth}});
                window.second=new ResizeObserver(entries=>order.push(entries[0].contentRect.width));
                first.observe(a);second.observe(b);
            "#)).unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([order,errors])")
                    .unwrap(),
                if force {
                    r#"[["first",200],0]"#
                } else {
                    r#"[["first",100],1]"#
                }
            );
            if !force {
                run_opportunity(&mut page, &loader).await.unwrap();
                assert_eq!(
                    page.vm_mut().eval("JSON.stringify(order)").unwrap(),
                    r#"["first",100,200]"#
                );
            }
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}

#[test]
fn realtime_ro_synchronous_geometry_does_not_consume_notification_or_repeat_layout() {
    run_page_vm_large_stack_async_test("realtime-ro-after-getter", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<style>#target{width:100px;height:20px}</style><div id=target></div>';
            window.sizes=[];window.observer=new ResizeObserver(entries=>sizes.push(entries[0].contentRect.width));observer.observe(target);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval("document.styleSheets[0].cssRules[0].style.width='240px';window.measured=target.offsetWidth").unwrap();
        let builds = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([measured,sizes])")
                .unwrap(),
            "[240,[100]]"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(sizes)").unwrap(),
            "[100,240]"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_constructor_order_and_same_box_reobserve_remain_stable() {
    run_page_vm_large_stack_async_test("realtime-ro-order", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px;padding:5px"></div>';
            window.order=[];
            window.first=new ResizeObserver(()=>{order.push('first');first.observe(target)});
            window.second=new ResizeObserver(()=>order.push('second'));
            second.observe(target);first.observe(target);first.disconnect();first.observe(target);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "first|second"
        );
        page.vm_mut()
            .eval("first.observe(target);second.observe(target,{box:'content-box'})")
            .unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none(),
            "same box reobserve cannot reset the reported size or arm a frame"
        );
        page.vm_mut()
            .eval("second.observe(target,{box:'border-box'})")
            .unwrap();
        assert_eq!(
            page.vm_mut().eval("second.takeRecords().length").unwrap(),
            "0"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "first|second|second"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_disconnect_and_unobserve_cancel_pending_work_and_active_records() {
    run_page_vm_large_stack_async_test("realtime-ro-cancel", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<div id=a style=width:100px></div><div id=b style=width:100px></div>';window.order=[]").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval("window.cancelled=new ResizeObserver(()=>order.push('cancelled'));cancelled.observe(a);cancelled.disconnect()").unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut()
            .eval("cancelled.observe(a);cancelled.unobserve(a)")
            .unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut().eval(r#"
            window.first=new ResizeObserver(()=>{order.push('first');second.unobserve(a);third.disconnect();order.push(second.takeRecords().length)});
            window.second=new ResizeObserver(entries=>order.push(entries.map(e=>e.target.id).join(',')));
            window.third=new ResizeObserver(()=>order.push('disconnected'));
            first.observe(a);second.observe(a);second.observe(b);third.observe(b);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("order.join('|')").unwrap(), "first|0|b");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_document_open_preserves_observer_and_reports_removed_and_new_targets() {
    run_page_vm_large_stack_async_test("realtime-ro-document-open", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=old style="width:100px;height:20px"></div>';
            window.log=[];window.observer=new ResizeObserver(entries=>log.push(entries.map(e=>[e.target.id,e.contentRect.width,e.contentRect.height])));observer.observe(old);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval(r#"
            requestAnimationFrame(()=>{
                document.open();document.write('<!doctype html><body><div id=new style="width:200px;height:30px"></div></body>');document.close();
                observer.observe(document.getElementById('new'));
            });
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(log)").unwrap(),
            r#"[[["old",100,20]],[["old",0,0],["new",200,30]]]"#
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_error_handler_mutations_refresh_geometry_without_restarting_depth() {
    run_page_vm_large_stack_async_test("realtime-ro-error-mutation", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.sizes=[];window.errors=0;
            window.onerror=()=>{errors++;target.style.width='300px';return true};
            window.observer=new ResizeObserver(entries=>{sizes.push(entries[0].contentRect.width);if(sizes.length===1)target.style.width='200px'});
            observer.observe(target);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let builds = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([sizes,errors,target.offsetWidth])")
                .unwrap(),
            "[[100],1,300]"
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            builds,
            "error callback geometry must already be current for the next phase"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([sizes,errors])")
                .unwrap(),
            "[[100,300],1]"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_error_microtask_is_current_for_io_and_capture_without_restarting_callbacks() {
    use crate::runtime::{RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest};

    run_page_vm_large_stack_async_test("realtime-ro-error-io-output", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 400,
            inner_height: 80,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        // Consume initialization before observing the scenario's one batch.
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .eval(
                r#"
                document.head.innerHTML='<style>html,body{margin:0;background:white}</style>';
                document.body.innerHTML='<div id=target style="position:absolute;left:0;top:0;width:100px;height:20px;background:red"></div>';
                window.events=[];window.sizes=[];window.errors=[];window.ioSizes=[];window.ticks=0;
                window.onerror=message=>{
                    errors.push(message);events.push('error');
                    target.style.width='300px';target.style.background='blue';
                    Promise.resolve().then(()=>{
                        events.push('error-microtask');
                        target.style.width='320px';target.style.background='lime';
                    });
                    return true;
                };
                window.ro=new ResizeObserver(entries=>{
                    const width=entries[0].contentRect.width;
                    sizes.push(width);events.push('ro:'+width);
                    if(sizes.length===1)target.style.width='200px';
                });
                window.io=new IntersectionObserver(entries=>{
                    for(const entry of entries){
                        ioSizes.push([entry.boundingClientRect.width,entry.intersectionRect.width]);
                        events.push('io:'+entry.boundingClientRect.width);
                    }
                });
                ro.observe(target);io.observe(target);
                requestAnimationFrame(()=>{ticks++;events.push('raf')});
                "#,
            )
            .unwrap();

        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([events,sizes,errors,ioSizes,ticks])")
                .unwrap(),
            r#"[["raf","ro:100","error","error-microtask"],[100],["ResizeObserver loop completed with undelivered notifications."],[],1]"#
        );
        let frame_builds = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );

        // Deliver the frame's owned sample before any screenshot or geometry
        // getter could repair stale post-error layout. IO delivery cannot build.
        assert!(
            page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver,
                &loader,
            )
            .await
            .unwrap()
        );
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([ioSizes,sizes,errors.length,ticks,events])")
                .unwrap(),
            r#"[[[320,320]],[100],1,1,["raf","ro:100","error","error-microtask","io:320"]]"#
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            frame_builds
        );
        let pending = page
            .current_realtime_rendering_opportunity()
            .expect("skipped RO remains due in the next batch");
        let deadline = page.next_rendering_opportunity_deadline_for_test();
        assert!(deadline.is_some());

        // Use the existing output route. This explicit paint may rebuild, but
        // cannot restart rAF/RO or consume the next batch's pending ticket.
        let RendererCaptureScreenshotReply::Captured(capture) = page
            .capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())
            .unwrap()
        else {
            panic!("current screen capture");
        };
        let image = moli_image::decode_png(&capture.bytes).unwrap();
        assert_eq!((image.width, image.height), (400, 80));
        let pixel = |x: usize, y: usize| {
            let offset = (y * image.width as usize + x) * 4;
            &image.rgba[offset..offset + 4]
        };
        assert_eq!(pixel(310, 10), [0, 255, 0, 255]);
        assert_eq!(pixel(330, 10), [255, 255, 255, 255]);
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([ioSizes,sizes,errors.length,ticks,events])")
                .unwrap(),
            r#"[[[320,320]],[100],1,1,["raf","ro:100","error","error-microtask","io:320"]]"#
        );
        assert_eq!(page.current_realtime_rendering_opportunity(), Some(pending));
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            deadline
        );
        let capture_builds = page.vm().layout_pass_observability_for_test().1;
        assert!(
            (frame_builds..=frame_builds + 1).contains(&capture_builds),
            "capture may reuse paint inputs or build one explicit pass"
        );

        // The next normal opportunity reports the skipped target once, using
        // current published geometry; it does not replay the finished phases.
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([ioSizes,sizes,errors.length,ticks,events])")
                .unwrap(),
            r#"[[[320,320]],[100,320],1,1,["raf","ro:100","error","error-microtask","io:320","ro:320"]]"#
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            capture_builds
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver,
            )
            .is_none()
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_native_button_value_change_schedules_resize_without_pseudo_state_change() {
    run_page_vm_large_stack_async_test("realtime-button-value-frame", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<input id="target" type="button" value="A" style="font-size:10px;line-height:10px;width:auto;min-width:0;padding:0;border:0;box-sizing:content-box">';
            // Establish the dirty value flag before testing an identical-value write.
            target.value='A';
            window.sizes=[];
            window.observer=new ResizeObserver(entries=>sizes.push(entries[0].contentRect.width));
            observer.observe(target);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([target.value,sizes])")
                .unwrap(),
            r#"["A",[40]]"#
        );
        assert!(page.current_realtime_rendering_opportunity().is_none());
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(page.vm().next_timeout_deadline().is_none());
        let builds = page.vm().layout_pass_observability_for_test().1;
        assert!(
            builds > 0,
            "the first RO delivery must have published real geometry"
        );

        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let target = host
            .borrow()
            .dom_host()
            .element_handle_by_id("target")
            .unwrap();
        let old_state = host
            .borrow()
            .retained_current_element_state(target)
            .expect("layout must have established the retained style world");
        let dom_version = host.borrow().dom_host().dom_version();

        // There is no DOM/style mutation, getter, rAF or capture to create an
        // unrelated opportunity after the initial RO demand has been consumed.
        page.vm_mut().eval("target.value='A'").unwrap();
        assert_eq!(host.borrow().dom_host().dom_version(), dom_version);
        assert_eq!(
            host.borrow().retained_current_element_state(target),
            Some(old_state)
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert_eq!(page.vm_mut().eval("JSON.stringify(sizes)").unwrap(), "[40]");

        assert_eq!(
            page.vm_mut()
                .eval("target.value='ABCDEFGHIJKLMNOPQRST';JSON.stringify([target.value,sizes])")
                .unwrap(),
            r#"["ABCDEFGHIJKLMNOPQRST",[40]]"#
        );
        assert!(host.borrow().dom_host().dom_version() > dom_version);
        assert_eq!(
            host.borrow().retained_current_element_state(target),
            Some(old_state),
            "the value must change without a checked, validity, direction or placeholder transition"
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            builds,
            "the setter must not synchronously lay out or deliver RO"
        );
        assert!(
            page.current_realtime_rendering_opportunity().is_some(),
            "a changed native button label must request a frame even when pseudo-state bits are unchanged"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_some()
        );

        // This helper only admits an already pending opportunity; it cannot
        // manufacture demand or repair the omission with a geometry getter.
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([target.value,sizes])")
                .unwrap(),
            r#"["ABCDEFGHIJKLMNOPQRST",[40,120]]"#
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
        assert!(page.current_realtime_rendering_opportunity().is_none());
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(page.vm().next_timeout_deadline().is_none());

        let changed_dom_version = host.borrow().dom_host().dom_version();
        page.vm_mut()
            .eval("target.value='ABCDEFGHIJKLMNOPQRST'")
            .unwrap();
        assert_eq!(host.borrow().dom_host().dom_version(), changed_dom_version);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(sizes)").unwrap(),
            "[40,120]"
        );
    });
}

#[test]
fn realtime_select_value_change_schedules_has_resize_like_selected_index() {
    run_page_vm_large_stack_async_test("realtime-select-value-frame", || async move {
        for (case, change, same_before, same_after) in [
            (
                "selectedIndex",
                "selectCtl.selectedIndex=1",
                "selectCtl.selectedIndex=0",
                "selectCtl.selectedIndex=1",
            ),
            (
                "value",
                "selectCtl.value='b'",
                "selectCtl.value='a'",
                "selectCtl.value='b'",
            ),
        ] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            assert_eq!(page.vm_mut().eval(r#"
                document.body.innerHTML='<style>option{color:rgb(1,2,3)}option:checked{color:rgb(4,5,6)}#target{width:100px;height:20px;padding:0;border:0}body:has(#optionB:checked) #target{width:200px}</style><select id=selectCtl><option id=optionA value=a>Alpha</option><option id=optionB value=b>Beta</option></select><div id=target></div>';
                selectCtl.selectedIndex=0;
                // Warm the style world before the initial RO publishes geometry.
                window.initialColors=[getComputedStyle(optionA).color,getComputedStyle(optionB).color];
                window.sizes=[];
                window.observer=new ResizeObserver(entries=>sizes.push(entries[0].contentRect.width));
                observer.observe(target);
                JSON.stringify(initialColors)
            "#).unwrap(), r#"["rgb(4, 5, 6)","rgb(1, 2, 3)"]"#);
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(sizes)").unwrap(),
                "[100]"
            );
            assert!(page.current_realtime_rendering_opportunity().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert!(page.vm().next_timeout_deadline().is_none());
            let builds = page.vm().layout_pass_observability_for_test().1;
            assert!(builds > 0);

            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            let a = host
                .borrow()
                .dom_host()
                .element_handle_by_id("optionA")
                .unwrap();
            let b = host
                .borrow()
                .dom_host()
                .element_handle_by_id("optionB")
                .unwrap();
            let old_a = host
                .borrow()
                .retained_current_element_state(a)
                .expect("warm option A state");
            let old_b = host
                .borrow()
                .retained_current_element_state(b)
                .expect("warm option B state");
            let version = host.borrow().dom_host().dom_version();
            page.vm_mut().eval(same_before).unwrap();
            assert_eq!(host.borrow().dom_host().dom_version(), version);
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(sizes)").unwrap(),
                "[100]"
            );

            // Do not read style/geometry, mutate attributes, request rAF, or
            // capture after this native state change. Only its own notification
            // may create the next opportunity and refresh the observer target.
            page.vm_mut().eval(change).unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval(
                        "JSON.stringify([selectCtl.value,optionA.selected,optionB.selected,sizes])"
                    )
                    .unwrap(),
                r#"["b",false,true,[100]]"#,
                "native peer selection: {case}"
            );
            assert!(host.borrow().dom_host().dom_version() > version);
            assert_ne!(host.borrow().retained_current_element_state(a), Some(old_a));
            assert_ne!(host.borrow().retained_current_element_state(b), Some(old_b));
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
            assert!(
                page.current_realtime_rendering_opportunity().is_some(),
                "the public {case} setter must request its own frame after changing selected peers"
            );
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_some()
            );

            // This only admits existing demand; it cannot create or repair it.
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(sizes)").unwrap(),
                "[100,200]",
                "{case}"
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
            assert_eq!(
                page.vm()
                    .layout_pass_observability_for_test()
                    .3
                    .unwrap()
                    .paint_operation_count,
                0
            );
            assert!(page.current_realtime_rendering_opportunity().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert!(page.vm().next_timeout_deadline().is_none());

            let changed_version = host.borrow().dom_host().dom_version();
            page.vm_mut().eval(same_after).unwrap();
            assert_eq!(host.borrow().dom_host().dom_version(), changed_version);
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(sizes)").unwrap(),
                "[100,200]"
            );
        }
    });
}

#[test]
fn realtime_file_input_commits_schedule_validity_resize_without_capture() {
    run_page_vm_large_stack_async_test("realtime-file-input-frame", || async move {
        for native in [false, true] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            page.vm_mut().eval(r#"
                document.body.innerHTML='<style>#target{width:100px;height:20px;padding:0;border:0}#upload:valid~#target{width:200px}</style><input id=upload type=file required><div id=target></div>';
                upload.value='';
                const transfer=new DataTransfer();
                transfer.items.add(new File(['abc'],'chosen.txt',{type:'text/plain',lastModified:7}));
                window.chosenFiles=transfer.files;
                window.emptyFiles=new DataTransfer().files;
                window.sizes=[];
                window.observer=new ResizeObserver(entries=>sizes.push(entries[0].contentRect.width));
                observer.observe(target);
            "#).unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(sizes)").unwrap(),
                "[100]"
            );
            assert!(page.current_realtime_rendering_opportunity().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert!(page.vm().next_timeout_deadline().is_none());
            let builds = page.vm().layout_pass_observability_for_test().1;
            assert!(
                builds > 0,
                "initial RO must establish a warm, current layout"
            );
            let upload = page
                .vm()
                .document_runtime
                .get_element_by_id("upload")
                .unwrap();
            let chosen = crate::dom::native::SelectedFile {
                bytes: b"abc".to_vec(),
                mime_type: "text/plain".to_owned(),
                name: "chosen.txt".to_owned(),
                last_modified: 7.0,
            };

            // No getter, rAF, screenshot or callback mutation can request or
            // repair the next frame after the initial observer demand is gone.
            if native {
                assert!(
                    page.vm_mut()
                        .set_file_input_files(upload, vec![chosen.clone()], false)
                        .unwrap()
                );
            } else {
                page.vm_mut().eval("upload.files=chosenFiles").unwrap();
            }
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
            assert!(
                page.current_realtime_rendering_opportunity().is_some(),
                "the file-state commit must create its own demand (native={native})"
            );
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_some()
            );
            // This helper admits only a previously requested opportunity.
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([upload.files.length,sizes])")
                    .unwrap(),
                "[1,[100,200]]",
                "the frame must revalidate validity-dependent style (native={native})"
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
            assert_eq!(
                page.vm()
                    .layout_pass_observability_for_test()
                    .3
                    .unwrap()
                    .paint_operation_count,
                0
            );
            assert!(page.current_realtime_rendering_opportunity().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );

            // These repeats use the same already committed payload and dirty
            // state, not an uninitialized input's first empty assignment.
            if native {
                assert!(
                    page.vm_mut()
                        .set_file_input_files(upload, vec![chosen], false)
                        .unwrap()
                );
            } else {
                page.vm_mut().eval("upload.files=chosenFiles").unwrap();
            }
            assert!(page.current_realtime_rendering_opportunity().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);

            if native {
                assert!(
                    page.vm_mut()
                        .set_file_input_files(upload, Vec::new(), false)
                        .unwrap()
                );
            } else {
                page.vm_mut().eval("upload.files=emptyFiles").unwrap();
            }
            assert!(page.current_realtime_rendering_opportunity().is_some());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_some()
            );
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([upload.files.length,sizes])")
                    .unwrap(),
                "[0,[100,200,100]]"
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 2);
            assert_eq!(
                page.vm()
                    .layout_pass_observability_for_test()
                    .3
                    .unwrap()
                    .paint_operation_count,
                0
            );
            assert!(page.current_realtime_rendering_opportunity().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert!(page.vm().next_timeout_deadline().is_none());
        }
    });
}
