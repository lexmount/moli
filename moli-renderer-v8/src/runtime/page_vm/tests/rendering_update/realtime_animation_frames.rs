use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::{DocumentActivity, LayoutPolicy};

fn realtime_page(loader: &crate::network::ResourceRequestClient) -> PageVm {
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        loader,
        Url::parse("https://example.com/realtime-raf").unwrap(),
    );
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page
}

#[test]
fn realtime_raf_batches_callbacks_and_checkpoints_without_timer_or_extra_layout() {
    run_page_vm_large_stack_async_test("realtime-raf-batch", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        run_opportunity(&mut page, &loader).await.unwrap();
        let passes = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut()
            .eval(
                r#"
            window.order = []; window.timestamps = [];
            requestAnimationFrame(t => {
                order.push('first'); timestamps.push(t);
                cancelAnimationFrame(cancelled);
                requestAnimationFrame(() => order.push('nested'));
                Promise.resolve().then(() => {
                    order.push('microtask'); cancelAnimationFrame(microCancelled);
                    requestAnimationFrame(() => order.push('micro-nested'));
                });
            });
            window.cancelled = requestAnimationFrame(() => order.push('cancelled'));
            window.microCancelled = requestAnimationFrame(() => order.push('micro-cancelled'));
            requestAnimationFrame(t => { order.push('last'); timestamps.push(t); });
        "#,
            )
            .unwrap();
        assert!(page.vm().next_timeout_deadline().is_none());
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("JSON.stringify([order,timestamps[0]===timestamps[1],Number.isFinite(timestamps[0])])").unwrap(), "[[\"first\",\"microtask\",\"last\"],true,true]");
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "first|microtask|last|nested|micro-nested"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_writes_and_microtasks_are_consumed_by_the_same_layout_phase() {
    run_page_vm_large_stack_async_test("realtime-raf-layout", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval("document.body.innerHTML='<div id=target style=width:100px;height:10px></div>'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let passes = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut().eval("requestAnimationFrame(()=>{target.style.width='200px';Promise.resolve().then(()=>target.style.width='300px')})").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "300"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut().eval("requestAnimationFrame(()=>{target.style.width='400px';window.inCallback=target.offsetWidth})").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(inCallback)").unwrap(), "400");
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_cancellation_preserves_source_demand_and_timer_namespaces() {
    run_page_vm_large_stack_async_test("realtime-raf-cancel", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval("window.calls=0;window.timer=setTimeout(()=>calls+=100,5000);window.raf=requestAnimationFrame(()=>calls++);cancelAnimationFrame(raf)").unwrap();
        assert!(
            page.vm().next_timeout_deadline().is_some(),
            "rAF cancellation cannot remove a Window timer"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut().eval("clearTimeout(timer);window.raf=requestAnimationFrame(()=>calls++);clearTimeout(raf)").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "1");
        page.vm_mut().eval("document.body.style.width='250px';const id=requestAnimationFrame(()=>calls++);cancelAnimationFrame(id)").unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_some(),
            "cancelling the final callback must preserve unconsumed source changes"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([calls,document.body.offsetWidth])")
                .unwrap(),
            "[1,250]"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_uses_native_time_origin_and_reports_exceptions_before_continuing() {
    run_page_vm_large_stack_async_test("realtime-raf-errors-clock", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval(r#"
            window.order=[]; window.times=[];
            window.onerror=() => { order.push('error'); return true; };
            Object.defineProperty(window,'performance',{configurable:true,get(){throw Error('public performance was read')}});
            Date.now=()=>{throw Error('Date.now was read')};
            requestAnimationFrame(t=>{times.push(t); order.push('throw');Promise.resolve().then(()=>order.push('microtask'));throw Error('frame failure')});
            requestAnimationFrame(t=>{times.push(t); order.push('next')});
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("JSON.stringify([order,times.length,times[0]===times[1],Number.isFinite(times[0])])").unwrap(), "[[\"throw\",\"error\",\"microtask\",\"next\"],2,true,true]");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_child_stage_uses_document_order_and_target_window_time() {
    run_page_vm_large_stack_async_test("realtime-raf-child-order", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<iframe id=first></iframe><iframe id=second></iframe>';window.order=[];window.epochs=[]").unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "first").unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "second").unwrap();
        page.vm_mut().eval(r#"
            window.child=first.contentWindow; window.other=second.contentWindow;
            window.parentOrigin=performance.timeOrigin; window.childOrigin=child.performance.timeOrigin;
            child.requestAnimationFrame(child.Function('t',"parent.order.push('child');parent.epochs.push(t+performance.timeOrigin);requestAnimationFrame(()=>parent.order.push('child-nested'))"));
            other.requestAnimationFrame(other.Function('t',"parent.order.push('other')"));
            requestAnimationFrame(t=>{
                order.push('parent'); epochs.push(t+parentOrigin);
                child.requestAnimationFrame(function(t){order.push(this===window?'borrowed-parent-realm':'wrong-realm');epochs.push(t+childOrigin)});
            });
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "parent|child|borrowed-parent-realm|other"
        );
        assert_eq!(
            page.vm_mut()
                .eval("String(Math.max(...epochs)-Math.min(...epochs)<0.11)")
                .unwrap(),
            "true"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "parent|child|borrowed-parent-realm|other|child-nested"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_retirement_removes_target_and_callback_realms() {
    run_page_vm_large_stack_async_test("realtime-raf-retirement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval("document.body.innerHTML='<iframe id=frame></iframe>';window.order=[]")
            .unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut()
            .eval(
                r#"
            window.child=frame.contentWindow;
            requestAnimationFrame(child.Function("parent.order.push('retired-callback-realm')"));
            child.requestAnimationFrame(()=>order.push('retired-target'));
            requestAnimationFrame(()=>order.push('live'));
            frame.remove();
        "#,
            )
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("order.join('|')").unwrap(), "live");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_navigation_during_callback_does_not_run_old_keys_or_lose_new_callbacks() {
    run_page_vm_large_stack_async_test("realtime-raf-document-open", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            window.order=[];
            requestAnimationFrame(()=>{
                order.push('old-first');
                Promise.resolve().then(()=>{
                    document.open();document.write('<!doctype html><body>new</body>');document.close();
                    window.newHandle=requestAnimationFrame(()=>order.push('new'));
                    cancelAnimationFrame(oldHandle);
                });
            });
            window.oldHandle=requestAnimationFrame(()=>order.push('old-second'));
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("order.join('|')").unwrap(), "old-first");
        assert_eq!(
            page.vm_mut().eval("String(newHandle>oldHandle)").unwrap(),
            "true"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "old-first|new"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_receiver_is_captured_before_handle_conversion_navigates() {
    run_page_vm_large_stack_async_test("realtime-raf-receiver", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval(
                r#"
            window.order=[];
            requestAnimationFrame(()=>order.push('old'));
            cancelAnimationFrame({valueOf(){
                document.open();document.write('<!doctype html><body>new</body>');document.close();
                return requestAnimationFrame(()=>order.push('new'));
            }});
        "#,
            )
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("order.join('|')").unwrap(), "new");
        assert_eq!(
            page.vm_mut()
                .eval("try{requestAnimationFrame.call({},()=>{});'wrong'}catch(e){e.name}")
                .unwrap(),
            "TypeError"
        );
        assert_eq!(
            page.vm_mut()
                .eval("try{cancelAnimationFrame();'wrong'}catch(e){e.name}")
                .unwrap(),
            "TypeError"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_hidden_or_script_disabled_page_keeps_demand_without_polling() {
    run_page_vm_large_stack_async_test("realtime-raf-suspended", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .eval("window.calls=0;requestAnimationFrame(()=>calls++)")
            .unwrap();
        page.vm_mut()
            .set_document_activity_for_bootstrap(DocumentActivity::new(false, false));
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut()
            .eval("document.body.style.width='190px';requestAnimationFrame(()=>calls++)")
            .unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut()
            .set_document_activity_for_bootstrap(DocumentActivity::default());
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([calls,document.body.offsetWidth])")
                .unwrap(),
            "[2,190]"
        );
        page.vm_mut()
            .eval("requestAnimationFrame(()=>calls++)")
            .unwrap();
        page.vm_mut().set_script_execution_disabled(true);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut().set_script_execution_disabled(false);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "3");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_watchdog_failure_settles_running_state_and_allows_explicit_recovery() {
    run_page_vm_large_stack_async_test("realtime-raf-watchdog", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval("window.calls=0;requestAnimationFrame(()=>{while(true){}});requestAnimationFrame(()=>calls++)").unwrap();
        let budget = crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::LifecycleEvent,
            Duration::from_millis(100),
        );
        let error = run_opportunity(&mut page, &loader).await.unwrap_err();
        drop(budget);
        assert!(
            error
                .to_string()
                .contains("animation frame callback exceeded 100ms and was terminated"),
            "{error}"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut()
            .eval("requestAnimationFrame(()=>calls++)")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "2");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn legacy_raf_keeps_its_timer_registration_and_cancellation_routes() {
    run_page_vm_large_stack_async_test("legacy-raf-timers", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/legacy-raf").unwrap(),
            );
            page.vm_mut().set_layout_policy(policy);
            page.vm_mut()
                .eval("window.raf=requestAnimationFrame(()=>{})")
                .unwrap();
            assert!(page.vm().next_timeout_deadline().is_some());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            page.vm_mut().eval("cancelAnimationFrame(raf)").unwrap();
            assert!(page.vm().next_timeout_deadline().is_none());
        }
    });
}

#[test]
fn realtime_raf_document_replacement_cannot_reenter_a_running_opportunity() {
    run_page_vm_large_stack_async_test("realtime-raf-no-reentry", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        let task = page.current_realtime_rendering_opportunity().unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        assert!(
            host.borrow()
                .begin_realtime_rendering_opportunity(task.task_id(), task.owner().target())
                .unwrap()
        );
        page.vm_mut().eval("document.open();document.write('<!doctype html><body>new</body>');document.close();window.called=false;requestAnimationFrame(()=>called=true)").unwrap();
        assert!(
            page.current_realtime_rendering_opportunity().is_none(),
            "new owner must wait for the old body to unwind"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        host.borrow().finish_realtime_rendering_opportunity(
            task.task_id(),
            task.owner().target(),
            true,
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(called)").unwrap(), "true");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_hidden_child_document_open_releases_callback_roots_immediately() {
    run_page_vm_large_stack_async_test("realtime-raf-hidden-retirement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval("document.body.innerHTML='<iframe id=frame></iframe>';window.order=[]")
            .unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .set_document_activity_for_bootstrap(DocumentActivity::new(false, false));
        page.vm_mut().eval("window.child=frame.contentWindow;child.requestAnimationFrame(()=>order.push('retired'));child.requestAnimationFrame(()=>order.push('retired-again'))").unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        assert_eq!(host.borrow().realtime_animation_frame_count_for_test(), 2);
        page.vm_mut().eval("child.document.open();child.document.write('<!doctype html><body>new</body>');child.document.close()").unwrap();
        assert_eq!(
            host.borrow().realtime_animation_frame_count_for_test(),
            0,
            "hidden documents cannot retain retired callback roots until a future frame"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut()
            .set_document_activity_for_bootstrap(DocumentActivity::default());
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("order.length.toString()").unwrap(), "0");
    });
}

#[test]
fn realtime_raf_isolated_realm_retirement_preserves_default_world_callbacks() {
    run_page_vm_large_stack_async_test("realtime-raf-isolated-retirement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        run_opportunity(&mut page, &loader).await.unwrap();
        let session = moli_page_types::DevToolsSessionKey::from_wire_session_id(Some(
            "realtime-raf-isolated",
        ));
        let context = page
            .vm_mut()
            .ensure_isolated_world_for_owner(Some(&session), "raf-realm", false)
            .unwrap();
        page.vm_mut()
            .exec_in_execution_context(
                context,
                "requestAnimationFrame(()=>document.body.dataset.retired='called')",
            )
            .unwrap();
        page.vm_mut()
            .eval("window.called=false;requestAnimationFrame(()=>called=true)")
            .unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        assert_eq!(host.borrow().realtime_animation_frame_count_for_test(), 2);
        assert!(
            page.vm_mut()
                .detach_runtime_inspector_session(Some("realtime-raf-isolated"))
        );
        assert_eq!(host.borrow().realtime_animation_frame_count_for_test(), 1);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([called,document.body.dataset.retired===undefined])")
                .unwrap(),
            "[true,true]"
        );
        assert_eq!(host.borrow().realtime_animation_frame_count_for_test(), 0);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_raf_reports_to_callback_realm_instead_of_target_window() {
    run_page_vm_large_stack_async_test("realtime-raf-child-error", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval("document.body.innerHTML='<iframe id=frame></iframe>';window.order=[]")
            .unwrap();
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            window.child=frame.contentWindow;
            window.onerror=()=>{order.push('wrong-parent-error');return true};
            child.onerror=child.Function("parent.order.push('child-error');return true");
            requestAnimationFrame(child.Function("parent.order.push('throw');Promise.resolve().then(()=>parent.order.push('child-microtask'));throw Error('child frame failure')"));
            requestAnimationFrame(()=>order.push('next'));
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "throw|child-error|child-microtask|next"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}
