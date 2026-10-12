use super::*;
use moli_page_types::DocumentActivity;

fn binding_count(page: &PageVm) -> usize {
    let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
    crate::observer_runtime::callback_binding_count_for_test(&mut host.borrow_mut())
}

#[test]
fn realtime_ro_document_order_is_independent_of_target_and_callback_realms() {
    run_page_vm_large_stack_async_test("realtime-ro-document-order", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame></iframe>';window.order=[]").unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            child.document.body.innerHTML='<div id=target style="width:80px;height:20px"></div>';
            window.childTarget=child.document.getElementById('target');
            window.childObserver=new child.ResizeObserver(function(entries,observer){
                order.push('child:'+entries[0].contentRect.width+':'+(this===observer&&entries instanceof Array));
                target.style.width='200px';
            });
            window.rootObserver=new ResizeObserver(child.Function('entries','observer',
                "parent.order.push('root:'+entries[0].contentRect.width+':'+(this===observer&&entries instanceof Array&&entries[0] instanceof ResizeObserverEntry))"));
            window.crossTargetObserver=new ResizeObserver(entries=>order.push('cross:'+entries[0].contentRect.width));
            childObserver.observe(childTarget);rootObserver.observe(target);crossTargetObserver.observe(childTarget);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "root:100:true|cross:80|child:80:true"
        );
        let builds = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "200"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "root:100:true|cross:80|child:80:true|root:200:true"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_callbacks_checkpoint_mutations_and_report_exceptions_in_the_callback_realm() {
    run_page_vm_large_stack_async_test("realtime-ro-callback-checkpoint", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame></iframe>';window.order=[]").unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            window.onerror=()=>{order.push('wrong-error-realm');return true};
            child.onerror=child.Function("parent.order.push('child-error');return true");
            window.mutation=new MutationObserver(()=>order.push('mutation'));
            mutation.observe(target,{attributes:true});
            window.first=new ResizeObserver(child.Function(
                "parent.order.push('first');parent.target.setAttribute('data-probe','changed');Promise.resolve().then(()=>parent.order.push('microtask'));throw Error('RO callback failure')"));
            window.second=new ResizeObserver(()=>order.push('second'));
            first.observe(target);second.observe(target);
        "#).unwrap();
        assert_eq!(page.vm_mut().eval("String(order.length)").unwrap(), "0");
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "first|child-error|mutation|microtask|second"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_parent_callback_retirement_skips_child_stage_and_retired_callback_realm() {
    run_page_vm_large_stack_async_test("realtime-ro-detach-in-callback", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame></iframe>';window.order=[]").unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            window.first=new ResizeObserver(()=>{order.push('first');frame.remove()});
            window.retiredRealm=new ResizeObserver(child.Function("parent.order.push('retired-realm')"));
            window.retiredDocument=new child.ResizeObserver(()=>order.push('retired-document'));
            window.last=new ResizeObserver(()=>order.push('last'));
            first.observe(target);retiredRealm.observe(target);retiredDocument.observe(target);last.observe(target);
        "#).unwrap();
        assert_eq!(binding_count(&page), 4);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("order.join('|')").unwrap(), "first|last");
        assert_eq!(binding_count(&page), 2);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_active_wrapper_survives_gc_and_disconnect_releases_its_cycle() {
    run_page_vm_large_stack_async_test("realtime-ro-gc", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.widths=[];window.stop=false;
            (()=>{let observer=new ResizeObserver(entries=>{
                widths.push(entries[0].contentRect.width);if(stop)observer.disconnect();
            });observer.observe(target)})();
            window.inactive=new ResizeObserver(()=>inactive.disconnect());
        "#,
            )
            .unwrap();
        assert_eq!(binding_count(&page), 2);
        page.vm_mut().eval("delete window.inactive").unwrap();
        page.vm_mut()
            .renderer_document_isolate_ops()
            .collect_renderer_document_isolate_garbage()
            .unwrap();
        assert_eq!(
            binding_count(&page),
            1,
            "only active observations are native GC roots"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .renderer_document_isolate_ops()
            .collect_renderer_document_isolate_garbage()
            .unwrap();
        assert_eq!(binding_count(&page), 1);
        page.vm_mut()
            .eval("stop=true;target.style.width='200px'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(widths)").unwrap(),
            "[100,200]"
        );
        page.vm_mut()
            .renderer_document_isolate_ops()
            .collect_renderer_document_isolate_garbage()
            .unwrap();
        assert_eq!(
            binding_count(&page),
            0,
            "disconnect releases even a callback/observer cycle"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_isolated_realm_retirement_cancels_the_last_frame_demand() {
    run_page_vm_large_stack_async_test("realtime-ro-isolated-retirement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval("document.body.innerHTML='<div id=target style=width:100px;height:20px></div>'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let session =
            moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("realtime-ro-isolated"));
        let context = page
            .vm_mut()
            .ensure_isolated_world_for_owner(Some(&session), "ro-realm", false)
            .unwrap();
        page.vm_mut().exec_in_execution_context(context,
            "window.observer=new ResizeObserver(()=>document.body.dataset.retired='called');observer.observe(document.getElementById('target'))").unwrap();
        assert_eq!(binding_count(&page), 1);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_some()
        );
        assert!(
            page.vm_mut()
                .detach_runtime_inspector_session(Some("realtime-ro-isolated"))
        );
        assert_eq!(binding_count(&page), 0);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert_eq!(
            page.vm_mut()
                .eval("String(document.body.dataset.retired===undefined)")
                .unwrap(),
            "true"
        );
    });
}

#[test]
fn realtime_ro_hidden_page_detach_releases_observer_and_callback_roots_immediately() {
    run_page_vm_large_stack_async_test("realtime-ro-hidden-detach", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<div id=target style=width:100px;height:20px></div><iframe id=frame></iframe>';window.order=[]").unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .set_document_activity_for_bootstrap(DocumentActivity::new(false, false));
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            window.childObserver=new child.ResizeObserver(()=>order.push('retired-observer'));
            window.rootObserver=new ResizeObserver(child.Function("parent.order.push('retired-callback')"));
            childObserver.observe(target);rootObserver.observe(target);
        "#).unwrap();
        assert_eq!(binding_count(&page), 2);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut().eval("frame.remove()").unwrap();
        assert_eq!(binding_count(&page), 0);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut()
            .set_document_activity_for_bootstrap(DocumentActivity::default());
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(order.length)").unwrap(), "0");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_watchdog_failure_stops_renewal_and_accepts_explicit_recovery() {
    run_page_vm_large_stack_async_test("realtime-ro-watchdog", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.calls=[];window.hang=true;
            window.first=new ResizeObserver(()=>{if(hang)while(true){}});
            window.second=new ResizeObserver(entries=>calls.push(entries[0].contentRect.width));
            first.observe(target);second.observe(target);
        "#,
            )
            .unwrap();
        let budget = crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::LifecycleEvent,
            Duration::from_millis(100),
        );
        let error = run_opportunity(&mut page, &loader).await.unwrap_err();
        drop(budget);
        assert!(
            error
                .to_string()
                .contains("ResizeObserver lifecycle exceeded 100ms and was terminated"),
            "{error}"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert_eq!(page.vm_mut().eval("JSON.stringify(calls)").unwrap(), "[]");
        page.vm_mut()
            .eval("hang=false;target.style.width='200px'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(calls)").unwrap(),
            "[200]"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_failed_broadcast_keeps_pending_checks_for_a_later_requested_frame() {
    run_page_vm_large_stack_async_test("realtime-ro-failed-pending", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            window.order=[];window.hang=true;
            window.first=new ResizeObserver(()=>{if(hang)while(true){}});
            window.second=new ResizeObserver(entries=>order.push(entries[0].contentRect.width));
            first.observe(target);second.observe(target);
        "#,
            )
            .unwrap();
        let budget = crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::LifecycleEvent,
            Duration::from_millis(100),
        );
        let error = run_opportunity(&mut page, &loader).await.unwrap_err();
        drop(budget);
        assert!(
            error
                .to_string()
                .contains("ResizeObserver lifecycle exceeded 100ms and was terminated"),
            "{error}"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert_eq!(page.vm_mut().eval("JSON.stringify(order)").unwrap(), "[]");
        let builds = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut()
            .eval("hang=false;requestAnimationFrame(()=>order.push('frame'))")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(order)").unwrap(),
            "[\"frame\",100]",
            "new frame demand must recover unreported observations without a source mutation"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}
