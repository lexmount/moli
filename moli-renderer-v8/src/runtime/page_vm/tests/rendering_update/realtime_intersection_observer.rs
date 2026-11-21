use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;
use std::time::{Duration, Instant};

fn realtime_page(loader: &crate::network::ResourceRequestClient) -> PageVm {
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        loader,
        Url::parse("https://example.com/realtime-intersection").unwrap(),
    );
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page
}

async fn notify(page: &mut PageVm, loader: &crate::network::ResourceRequestClient) {
    assert!(
        page.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::IntersectionObserver,
            loader,
        )
        .await
        .unwrap(),
        "IO has its own ready notification source"
    );
}

#[test]
fn realtime_io_delay_resumes_at_the_earliest_due_sample_without_polling() {
    run_page_vm_large_stack_async_test("realtime-io-delay", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        run_opportunity(&mut page, &loader).await.unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let clock = |now| host.borrow().set_realtime_opportunity_clock_for_test(now);
        clock(Instant::now());
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=target style="position:fixed;left:-200px;width:100px;height:20px"></div>';
            window.events=[];
            window.short=new IntersectionObserver(([entry])=>events.push('short:'+entry.isIntersecting),{delay:80});
            window.long=new IntersectionObserver(([entry])=>events.push('long:'+entry.isIntersecting),{delay:120});
            short.observe(target);long.observe(target);
        "#).unwrap();
        let initial = page.next_rendering_opportunity_deadline_for_test().unwrap();
        clock(initial);
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "short:false|long:false"
        );

        clock(initial + Duration::from_millis(20));
        page.vm_mut().eval("target.style.left='0px'").unwrap();
        let early = page.next_rendering_opportunity_deadline_for_test().unwrap();
        let short_due = initial + Duration::from_millis(80);
        let long_due = initial + Duration::from_millis(120);
        assert!(early < short_due);
        clock(early);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("String(short.takeRecords().length+long.takeRecords().length)")
                .unwrap(),
            "0",
            "the early frame must not queue records before either delay expires",
        );
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(short_due)
        );
        let builds = page.vm().layout_pass_observability_for_test().1;

        clock(early);
        page.vm_mut()
            .eval("window.rafCalls=0;requestAnimationFrame(()=>++rafCalls)")
            .unwrap();
        let animation_tick = page.next_rendering_opportunity_deadline_for_test().unwrap();
        assert!(animation_tick < short_due, "an IO wait must not stall rAF");
        clock(animation_tick);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(rafCalls)").unwrap(), "1");
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(short_due)
        );

        page.admit_rendering_opportunity_at_for_test(short_due - Duration::from_nanos(1));
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate
            )
            .is_none()
        );
        clock(short_due);
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "short:false|long:false|short:true"
        );
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(long_due)
        );

        clock(long_due);
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "short:false|long:false|short:true|long:true"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_io_delay_cancellation_keeps_sampled_records_and_resets_registration_time() {
    run_page_vm_large_stack_async_test("realtime-io-delay-cancel", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for cancel in ["io.unobserve(target)", "io.disconnect()"] {
            let mut page = realtime_page(&loader);
            run_opportunity(&mut page, &loader).await.unwrap();
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            let clock = |now| host.borrow().set_realtime_opportunity_clock_for_test(now);
            clock(Instant::now());
            page.vm_mut().eval(r#"
                document.body.innerHTML='<div id=target style="position:fixed;left:-200px;width:100px;height:20px"></div><div id=fresh style="width:40px;height:20px"></div>';
                window.events=[];
                window.io=new IntersectionObserver(entries=>{
                    for(const entry of entries) events.push(entry.target.id+':'+entry.isIntersecting);
                },{delay:160});
                io.observe(target);
            "#).unwrap();
            let initial = page.next_rendering_opportunity_deadline_for_test().unwrap();
            clock(initial);
            run_opportunity(&mut page, &loader).await.unwrap();
            notify(&mut page, &loader).await;
            assert_eq!(
                page.vm_mut().eval("events.join('|')").unwrap(),
                "target:false"
            );

            clock(initial + Duration::from_millis(20));
            page.vm_mut()
                .eval("target.style.left='0px';io.observe(fresh)")
                .unwrap();
            let early = page.next_rendering_opportunity_deadline_for_test().unwrap();
            let due = initial + Duration::from_millis(160);
            assert!(early < due);
            clock(early);
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.next_rendering_opportunity_deadline_for_test(),
                Some(due)
            );

            page.vm_mut().eval(cancel).unwrap();
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            notify(&mut page, &loader).await;
            assert_eq!(
                page.vm_mut().eval("events.join('|')").unwrap(),
                "target:false|fresh:true"
            );

            page.vm_mut()
                .eval("io.disconnect();io.observe(target)")
                .unwrap();
            let observed_again = page.next_rendering_opportunity_deadline_for_test().unwrap();
            assert!(observed_again < due);
            clock(observed_again);
            run_opportunity(&mut page, &loader).await.unwrap();
            notify(&mut page, &loader).await;
            assert_eq!(
                page.vm_mut().eval("events.join('|')").unwrap(),
                "target:false|fresh:true|target:true"
            );
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}

#[test]
fn realtime_io_delay_retires_the_observers_deadline_with_its_child_realm() {
    run_page_vm_large_stack_async_test("realtime-io-delay-retirement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=target style="position:fixed;left:-200px;width:100px;height:20px"></div><iframe id=frame></iframe>';
            window.events=[];
        "#).unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let clock = |now| host.borrow().set_realtime_opportunity_clock_for_test(now);
        clock(Instant::now());
        page.vm_mut()
            .eval(
                r#"
            window.io=new frame.contentWindow.IntersectionObserver(
                ([entry])=>events.push('child:'+entry.isIntersecting),{delay:160});
            io.observe(target);
        "#,
            )
            .unwrap();
        let first = page.next_rendering_opportunity_deadline_for_test().unwrap();
        clock(first);
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "child:false"
        );

        clock(first + Duration::from_millis(20));
        page.vm_mut().eval("target.style.left='0px'").unwrap();
        let before_due = page.next_rendering_opportunity_deadline_for_test().unwrap();
        clock(before_due);
        run_opportunity(&mut page, &loader).await.unwrap();
        let old_deadline = first + Duration::from_millis(160);
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(old_deadline)
        );
        page.vm_mut().eval(r#"
            frame.remove();
            window.rootObserver=new IntersectionObserver(([entry])=>events.push('root:'+entry.isIntersecting));
            rootObserver.observe(target);
        "#).unwrap();
        let root_tick = page.next_rendering_opportunity_deadline_for_test().unwrap();
        assert!(root_tick < old_deadline);
        clock(root_tick);
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "child:false|root:true"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.admit_rendering_opportunity_at_for_test(old_deadline);
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate
            )
            .is_none()
        );
    });
}

#[test]
fn realtime_io_samples_after_raf_ro_and_notifies_in_a_separate_task() {
    run_page_vm_large_stack_async_test("realtime-io-stages", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.events=[];
            window.io=new IntersectionObserver(entries=>events.push('io:'+entries[0].boundingClientRect.width));
            io.observe(target);
            window.ro=new ResizeObserver(()=>{events.push('ro');target.style.width='240px';ro.disconnect()});
            ro.observe(target);
            requestAnimationFrame(()=>{events.push('raf');Promise.resolve().then(()=>events.push('microtask'))});
        "#).unwrap();
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "");
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "raf|microtask|ro"
        );
        let builds = page.vm().layout_pass_observability_for_test().1;
        // Delivery is of the sampled record; it must not silently resample.
        page.vm_mut().eval("target.style.width='360px'").unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "raf|microtask|ro|io:240"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver
            )
            .is_none()
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(page.vm().next_timeout_deadline().is_none());
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
    });
}

#[test]
fn realtime_io_take_records_consumes_samples_without_layout_or_delivery() {
    run_page_vm_large_stack_async_test("realtime-io-records", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.events=[];window.io=new IntersectionObserver(()=>events.push('io'));io.observe(target);
        "#).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("String(io.takeRecords().length)")
                .unwrap(),
            "0"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("String(io.takeRecords().length)")
                .unwrap(),
            "1"
        );
        notify(&mut page, &loader).await;
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_io_observe_after_clean_frame_and_scroll_progress_without_capture() {
    run_page_vm_large_stack_async_test("realtime-io-scroll", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=scroller style="width:200px;height:100px;overflow:auto"><div style="height:300px"></div><div id=sentinel style="height:20px"></div></div>';
            window.events=[];
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let builds = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut().eval("window.io=new IntersectionObserver(entries=>events.push(entries[0].isIntersecting),{root:scroller});io.observe(sentinel)").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "false");
        page.vm_mut().eval("scroller.scrollTop=240").unwrap();
        // A scroll event may precede the due frame in the RenderingUpdate FIFO.
        while page
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate,
                &loader,
            )
            .await
            .unwrap()
        {}
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "false|true"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_io_later_observer_records_remain_accessible_during_notification() {
    run_page_vm_large_stack_async_test("realtime-io-reentry", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.events=[];
            window.first=new IntersectionObserver(()=>{
                events.push('first:'+second.takeRecords().length);
                Promise.resolve().then(()=>events.push('microtask'));
            });
            window.second=new IntersectionObserver(()=>events.push('second'));
            first.observe(target);second.observe(target);
        "#,
            )
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "first:1|microtask"
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver
            )
            .is_none()
        );
    });
}

#[test]
fn realtime_io_cancellation_preserves_queued_records_but_stops_future_samples() {
    run_page_vm_large_stack_async_test("realtime-io-cancel", || async move {
        for cancel in ["io.unobserve(target)", "io.disconnect()"] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            page.vm_mut().eval(r#"
                document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
                window.events=[];window.io=new IntersectionObserver(entries=>events.push(entries.length));io.observe(target);
            "#).unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            page.vm_mut().eval(cancel).unwrap();
            notify(&mut page, &loader).await;
            assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "1");
            page.vm_mut().eval("target.remove()").unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            assert!(
                page.claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::IntersectionObserver
                )
                .is_none()
            );
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}

#[test]
fn realtime_io_separates_observer_target_and_callback_owners_and_retires_children() {
    run_page_vm_large_stack_async_test("realtime-io-owners", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame></iframe>';window.events=[]").unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            child.document.body.innerHTML='<div id=childTarget style="width:80px;height:20px"></div>';
            window.rootIO=new IntersectionObserver(child.Function('entries',
                "parent.events.push('root:'+entries[0].boundingClientRect.width+':'+(window===parent.child));Promise.resolve().then(()=>parent.events.push('microtask'))"));
            rootIO.observe(child.document.getElementById('childTarget'));
            window.childIO=new child.IntersectionObserver(entries=>events.push('child:'+entries[0].boundingClientRect.width));
            childIO.observe(target);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "root:80:true|microtask"
        );
        // Removing the child retires the still-queued child-owned notification,
        // even though its observed node and callback both belong to the root.
        page.vm_mut().eval("frame.remove()").unwrap();
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "root:80:true|microtask"
        );
    });
}

#[test]
fn realtime_io_claimed_notification_cannot_consume_replacement_document_work() {
    run_page_vm_large_stack_async_test("realtime-io-rewrite", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.events=[];window.io=new IntersectionObserver(()=>events.push('old'));io.observe(target);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let old = page
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver,
            )
            .unwrap();
        page.vm_mut().eval(r#"
            io.disconnect();io.takeRecords();
            document.open();document.write('<body><div id=newTarget style="width:120px;height:20px"></div></body>');document.close();
            window.newIO=new IntersectionObserver(()=>events.push('new'));newIO.observe(newTarget);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.run_claimed_selected_page_task_for_test(old, &loader)
            .await
            .unwrap();
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "");
        notify(&mut page, &loader).await;
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "new");
    });
}

#[test]
fn legacy_io_modes_keep_microtask_delivery_and_create_no_frame_or_io_tasks() {
    run_page_vm_large_stack_async_test("legacy-io-isolation", || async move {
        for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/legacy-intersection").unwrap(),
            );
            page.vm_mut().set_layout_policy(policy);
            page.vm_mut()
                .eval(
                    r#"
                document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
                window.calls=0;window.io=new IntersectionObserver(()=>calls++);io.observe(target);
            "#,
                )
                .unwrap();
            assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "1");
            assert!(
                page.claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::IntersectionObserver
                )
                .is_none()
            );
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        }
    });
}
