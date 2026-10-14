use super::*;

async fn run_initial_child_viewport_environment_opportunity(
    page: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) {
    // Initial child layout publishes its viewport after this opportunity's
    // environment phase. This remaining demand must not be unfinished RO work.
    assert!(
        page.next_rendering_opportunity_deadline_for_test()
            .is_some()
    );
    {
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        assert!(host.has_realtime_environment_notifications());
        assert!(!crate::observer_runtime::has_pending_realtime_resize_observers(&host, None,));
    }
    let builds = page.vm().layout_pass_observability_for_test().1;
    run_opportunity(page, loader).await.unwrap();
    assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
    {
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        assert!(!host.has_realtime_environment_notifications());
        assert!(!crate::observer_runtime::has_pending_realtime_resize_observers(&host, None,));
    }
    assert!(
        page.next_rendering_opportunity_deadline_for_test()
            .is_none()
    );
}

#[test]
fn realtime_ro_child_write_to_deeper_parent_target_is_delivered_in_the_same_opportunity() {
    run_page_vm_large_stack_async_test("realtime-ro-cross-document-deeper", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(
            "document.body.innerHTML='<div id=outer style=width:100px;height:20px><div id=inner style=width:60px;height:10px></div></div><iframe id=frame></iframe>'",
        ).unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            child.document.body.innerHTML='<div id=target style="width:80px;height:10px"></div>';
            window.order=[];window.errors=[];window.ticks=0;
            window.onerror=message=>{errors.push(message);return true};
            child.onerror=child.Function('message',"parent.errors.push('child:'+message);return true");
            window.rootObserver=new ResizeObserver(entries=>{
                order.push('root:'+entries.map(entry=>entry.target.id+':'+entry.contentRect.width).join(','));
            });
            child.observer=new child.ResizeObserver(child.Function('entries',`
                parent.order.push('child:'+entries[0].contentRect.width);
                parent.document.getElementById('inner').style.width='70px';
            `));
            rootObserver.observe(outer);rootObserver.observe(inner);
            child.observer.observe(child.document.getElementById('target'));
            requestAnimationFrame(()=>{ticks++;order.push('raf')});
        "#).unwrap();

        // No getter or capture repairs the later child callback's parent write.
        // The parent depth remains outer's depth, so inner is still eligible.
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([order,errors,ticks])")
                .unwrap(),
            r#"[["raf","root:outer:100,inner:60","child:80","root:inner:70"],[],1]"#
        );
        run_initial_child_viewport_environment_opportunity(&mut page, &loader).await;
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([order,errors,ticks])")
                .unwrap(),
            r#"[["raf","root:outer:100,inner:60","child:80","root:inner:70"],[],1]"#
        );
    });
}

#[test]
fn realtime_ro_cross_document_cycles_report_each_error_once_and_keep_io_current() {
    run_page_vm_large_stack_async_test("realtime-ro-cross-document-errors", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 400,
            inner_height: 160,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        page.vm_mut().eval(
            "document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame style=width:200px;height:40px;border:0></iframe>'",
        ).unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            child.document.body.innerHTML='<div id=target style="width:80px;height:10px"></div>';
            window.rootSizes=[];window.childSizes=[];window.errors=[];window.order=[];
            window.ioSizes=[];window.ticks=0;
            window.onerror=message=>{
                errors.push(['root',message]);order.push('root-error');
                target.style.width='130px';
                Promise.resolve().then(()=>{
                    order.push('root-error-microtask');target.style.width='140px';
                    child.document.getElementById('target').style.width='100px';
                });
                return true;
            };
            child.onerror=child.Function('message',`
                parent.errors.push(['child',message]);parent.order.push('child-error');
                parent.document.getElementById('target').style.width='150px';
                Promise.resolve().then(()=>{
                    parent.order.push('child-error-microtask');
                    parent.document.getElementById('target').style.width='160px';
                    document.getElementById('target').style.width='110px';
                });
                return true;
            `);
            window.rootObserver=new ResizeObserver(entries=>{
                rootSizes.push(entries[0].contentRect.width);
                order.push('root:'+entries[0].contentRect.width);
                if(rootSizes.length===1)target.style.width='110px';
            });
            child.observer=new child.ResizeObserver(child.Function('entries',`
                parent.childSizes.push(entries[0].contentRect.width);
                parent.order.push('child:'+entries[0].contentRect.width);
                if(parent.childSizes.length===1){
                    parent.document.getElementById('target').style.width='120px';
                    document.getElementById('target').style.width='90px';
                }
            `));
            window.io=new IntersectionObserver(entries=>{
                for(const entry of entries)ioSizes.push([entry.boundingClientRect.width,entry.intersectionRect.width]);
            });
            rootObserver.observe(target);io.observe(target);
            child.observer.observe(child.document.getElementById('target'));
            requestAnimationFrame(()=>{ticks++;order.push('raf')});
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([rootSizes,childSizes,errors,order,ticks,ioSizes])")
                .unwrap(),
            r#"[[100],[80],[["root","ResizeObserver loop completed with undelivered notifications."],["child","ResizeObserver loop completed with undelivered notifications."]],["raf","root:100","child:80","root-error","root-error-microtask","child-error","child-error-microtask"],1,[]]"#
        );
        let builds = page.vm().layout_pass_observability_for_test().1;
        assert!(
            page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver,
                &loader,
            )
            .await
            .unwrap()
        );
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(ioSizes)").unwrap(),
            "[[160,160]]"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);

        // Each final same-depth change remains due for the next opportunity.
        // Re-gathering after the errors must neither consume that demand nor
        // deliver either Document again at depth zero in the current frame.
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_some()
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify([rootSizes,childSizes,errors.length,ticks,ioSizes,order.slice(-2)])").unwrap(),
            r#"[[100,160],[80,110],2,1,[[160,160]],["root:160","child:110"]]"#
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_empty_window_controller_survives_gc_and_is_created_by_a_completed_opportunity() {
    run_page_vm_large_stack_async_test("realtime-ro-empty-window-controller", || async move {
        for (label, collect_constructor, warm_without_observers) in [
            ("new-window", false, false),
            ("collected-constructor", true, false),
            ("completed-empty-opportunity", false, true),
        ] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            page.vm_mut().eval(
                "document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame></iframe>'",
            ).unwrap();
            materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
            if collect_constructor {
                page.vm_mut().eval(
                    "window.unusedObserver=new ResizeObserver(()=>{});delete window.unusedObserver",
                ).unwrap();
                page.vm_mut()
                    .renderer_document_isolate_ops()
                    .collect_renderer_document_isolate_garbage()
                    .unwrap();
                let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
                assert_eq!(
                    crate::observer_runtime::callback_binding_count_for_test(
                        &mut host.borrow_mut()
                    ),
                    0,
                    "{label}: the controller is not a root for a dead observer"
                );
            }
            if warm_without_observers {
                run_opportunity(&mut page, &loader).await.unwrap();
                run_initial_child_viewport_environment_opportunity(&mut page, &loader).await;
            }
            page.vm_mut().eval(r#"
                window.child=frame.contentWindow;
                child.document.body.innerHTML='<div id=target style="width:80px;height:10px"></div>';
                window.parentSizes=[];window.childSizes=[];window.errors=[];window.ticks=0;
                window.onerror=message=>{errors.push(message);return true};
                child.observer=new child.ResizeObserver(child.Function('entries',`
                    parent.childSizes.push(entries[0].contentRect.width);
                    if(parent.childSizes.length===1){
                        parent.rootObserver=new parent.ResizeObserver(parent.Function('entries',
                            'parentSizes.push(entries[0].contentRect.width)'));
                        parent.rootObserver.observe(parent.document.getElementById('target'));
                    }
                `));
                child.observer.observe(child.document.getElementById('target'));
                requestAnimationFrame(()=>ticks++);
            "#).unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            let has_controller = collect_constructor || warm_without_observers;
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([parentSizes,childSizes,errors,ticks])")
                    .unwrap(),
                if has_controller {
                    r#"[[],[80],["ResizeObserver loop completed with undelivered notifications."],1]"#
                } else {
                    "[[100],[80],[],1]"
                },
                "{label}: only a controller already visited at depth Bottom defers the new observation"
            );
            if has_controller {
                run_opportunity(&mut page, &loader).await.unwrap();
                assert_eq!(
                    page.vm_mut()
                        .eval("JSON.stringify([parentSizes,childSizes,errors.length,ticks])")
                        .unwrap(),
                    "[[100],[80],1,1]",
                    "{label}"
                );
            } else {
                run_initial_child_viewport_environment_opportunity(&mut page, &loader).await;
                assert_eq!(
                    page.vm_mut()
                        .eval("JSON.stringify([parentSizes,childSizes,errors,ticks])")
                        .unwrap(),
                    "[[100],[80],[],1]",
                    "{label}: startup resize must not repeat observers or rAF"
                );
            }
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none(),
                "{label}"
            );
        }
    });
}
