use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

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

#[test]
fn realtime_io_projects_entries_in_callback_realm_not_observer_realm() {
    run_page_vm_large_stack_async_test("realtime-io-argument-realm", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(
            "document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame></iframe>';window.rows=[]"
        ).unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            window.rootIO=new IntersectionObserver(child.Function('entries','observer', `
                const entry=entries[0];
                parent.rows.push({
                    callbackRealm:window===parent.child,
                    observerRealm:Object.getPrototypeOf(observer)===parent.IntersectionObserver.prototype,
                    receiver:this===observer&&observer===parent.rootIO,
                    arrayRealm:Object.getPrototypeOf(entries)===Array.prototype,
                    entryRealm:Object.getPrototypeOf(entry)===IntersectionObserverEntry.prototype,
                    rectRealm:[entry.boundingClientRect,entry.intersectionRect,entry.rootBounds].every(
                        rect=>rect!==null&&DOMRectReadOnly.prototype.isPrototypeOf(rect)),
                    target:entry.target===parent.document.getElementById('target'),
                    count:entries.length,argc:arguments.length,width:entry.boundingClientRect.width
                });
            `));
            rootIO.observe(target);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("JSON.stringify(rows)").unwrap(), "[]");
        let builds = page.vm().layout_pass_observability_for_test().1;
        notify(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(rows)").unwrap(),
            r#"[{"callbackRealm":true,"observerRealm":true,"receiver":true,"arrayRealm":true,"entryRealm":true,"rectRealm":true,"target":true,"count":1,"argc":2,"width":100}]"#,
            "observer ownership and callback argument conversion use their distinct Realms"
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            builds,
            "delivery projects owned records without another geometry pass"
        );
    });
}
