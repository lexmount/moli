use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

async fn drain_font_tasks(
    page: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> usize {
    let mut count = 0;
    while page
        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FontLoading, loader)
        .await
        .unwrap()
    {
        count += 1;
        assert!(count < 16, "font readiness must not spin");
    }
    count
}

#[test]
fn realtime_font_task_can_prepare_initial_resize_before_frame_callbacks() {
    run_page_vm_large_stack_async_test("realtime-initial-font-readiness", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-font-readiness").unwrap(),
        );
        page.vm_mut()
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut().eval(r#"
            globalThis.events=[];
            const frame=document.createElement('iframe');
            frame.style.cssText='width:300px;height:100px;border:0';
            document.body.append(frame);
            globalThis.child=frame.contentWindow;
            child.addEventListener('resize',()=>events.push('resize:'+child.innerWidth));
            child.visualViewport.addEventListener('resize',()=>events.push('visual:'+child.innerWidth));
            globalThis.ready=child.document.fonts.ready;
            ready.then(()=>events.push('ready'));
            requestAnimationFrame(()=>events.push('raf'));
        "#).unwrap();
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "");
        assert_eq!(
            page.vm_mut().eval("child.document.fonts.status").unwrap(),
            "loaded"
        );
        assert_eq!(
            page.vm_mut().eval("child.document.readyState").unwrap(),
            "complete"
        );
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 2);
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "ready");
        let builds = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            builds, 1,
            "root/child readiness share current screen geometry"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "ready|resize:300|visual:300|raf"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );

        page.vm_mut().eval("events=[];child.frameElement.style.width='360px';requestAnimationFrame(()=>events.push('raf'))").unwrap();
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 0);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "raf");
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "raf|resize:360|visual:360"
        );
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
    });
}

#[test]
fn realtime_font_ready_waits_for_document_completion_and_refreshes_resolved_reads() {
    run_page_vm_large_stack_async_test("realtime-font-ready-reads", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/font-ready-reads").unwrap(),
        );
        page.vm_mut()
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Loading)
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML='<div id=target style="width:100px;height:20px"></div>';
            globalThis.ready=false;globalThis.promise=document.fonts.ready;
            promise.then(()=>ready=true);
        "#,
            )
            .unwrap();
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 1);
        assert_eq!(page.vm_mut().eval("String(ready)").unwrap(), "false");
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        page.vm_mut()
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
            .unwrap();
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 1);
        assert_eq!(page.vm_mut().eval("String(ready)").unwrap(), "true");
        let builds = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut()
            .eval("target.style.width='240px';globalThis.same=promise===document.fonts.ready")
            .unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([same,target.offsetWidth,promise===document.fonts.ready])")
                .unwrap(),
            "[true,240,true]"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 0);
    });
}

#[test]
fn realtime_initial_font_task_does_not_require_a_javascript_font_set() {
    run_page_vm_large_stack_async_test("realtime-font-native-initialization", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/font-native-initialization").unwrap(),
        );
        page.vm_mut()
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        drain_font_tasks(&mut page, &loader).await;
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .eval(
                r#"
            globalThis.events=[];globalThis.frame=document.createElement('iframe');
            frame.style.cssText='width:320px;height:100px;border:0';document.body.append(frame);
            frame.contentWindow.addEventListener('resize',()=>events.push('resize'));
            requestAnimationFrame(()=>events.push('raf'));
        "#,
            )
            .unwrap();
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 1);
        assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "");
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "resize|raf"
        );
    });
}

#[test]
fn realtime_font_task_preserves_open_promises_but_retires_detached_documents() {
    run_page_vm_large_stack_async_test("realtime-font-owner", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/font-owner").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut().eval("globalThis.calls=0;globalThis.promise=document.fonts.ready;promise.then(()=>calls++)").unwrap();
        let stale = page
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FontLoading)
            .unwrap();
        page.vm_mut()
            .eval(
                "document.open();document.write('<!doctype html><div>new</div>');document.close()",
            )
            .unwrap();
        page.vm_mut()
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
            .unwrap();
        page.run_claimed_selected_page_task_for_test(stale, &loader)
            .await
            .unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "0");
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 1);
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([calls,promise===document.fonts.ready])")
                .unwrap(),
            "[1,true]"
        );

        page.vm_mut()
            .eval(
                r#"
            globalThis.childReady=0;globalThis.frame=document.createElement('iframe');
            document.body.append(frame);frame.contentDocument.fonts.ready.then(()=>childReady++);
        "#,
            )
            .unwrap();
        let stale = page
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::FontLoading)
            .unwrap();
        page.vm_mut().eval("frame.remove();globalThis.replacement=document.createElement('iframe');document.body.append(replacement);replacement.contentDocument.fonts.ready.then(()=>childReady+=10)").unwrap();
        page.run_claimed_selected_page_task_for_test(stale, &loader)
            .await
            .unwrap();
        assert_eq!(page.vm_mut().eval("String(childReady)").unwrap(), "0");
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 1);
        assert_eq!(page.vm_mut().eval("String(childReady)").unwrap(), "10");
    });
}

#[test]
fn realtime_font_readiness_failure_stays_pending_without_automatic_retry() {
    run_page_vm_large_stack_async_test("realtime-font-failure", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/font-failure").unwrap(),
        );
        page.vm_mut()
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval("globalThis.ready=false;document.fonts.ready.then(()=>ready=true)")
            .unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        host.borrow().simulate_active_layout_pass_for_test(true);
        let error = page
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::FontLoading,
                &loader,
            )
            .await
            .unwrap_err();
        host.borrow().simulate_active_layout_pass_for_test(false);
        assert_eq!(
            error.downcast_ref::<moli_layout::LayoutError>(),
            Some(&moli_layout::LayoutError::ReentrantLayoutPass)
        );
        assert_eq!(page.vm_mut().eval("String(ready)").unwrap(), "false");
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 0);
        page.vm_mut()
            .eval("document.body.style.width='400px'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let builds = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 1);
        assert_eq!(page.vm_mut().eval("String(ready)").unwrap(), "true");
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
    });
}

#[test]
fn realtime_font_resolution_allows_geometry_reads_from_a_then_getter() {
    run_page_vm_large_stack_async_test("realtime-font-resolution-reentry", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/font-resolution-reentry").unwrap(),
        );
        page.vm_mut()
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML='<div id=target style="width:100px"></div>';
            globalThis.events=[];globalThis.fonts=document.fonts;
            Object.defineProperty(fonts,'then',{configurable:true,get(){
              delete fonts.then;
              target.style.width='240px';events.push(target.offsetWidth);
              return undefined;
            }});
            fonts.ready.then(value=>events.push(value===fonts));
        "#,
            )
            .unwrap();
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)").unwrap(), "[]");
        assert_eq!(drain_font_tasks(&mut page, &loader).await, 1);
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(events)").unwrap(),
            "[240,true]"
        );
        let builds = page.vm().layout_pass_observability_for_test().1;
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn legacy_font_readiness_keeps_its_existing_cost_and_delivery_path() {
    run_page_vm_large_stack_async_test("legacy-font-readiness", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/legacy-font-readiness").unwrap(),
            );
            page.vm_mut().set_layout_policy(policy);
            page.vm_mut()
                .eval("globalThis.ready=false;document.fonts.ready.then(()=>ready=true)")
                .unwrap();
            assert_eq!(page.vm_mut().eval("String(ready)").unwrap(), "true");
            assert_eq!(drain_font_tasks(&mut page, &loader).await, 0);
            assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}
