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
