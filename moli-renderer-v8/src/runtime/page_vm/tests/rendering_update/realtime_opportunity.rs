use super::*;
use moli_page_types::LayoutPolicy;

async fn run_opportunity(
    page: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<()> {
    let deadline = page
        .next_rendering_opportunity_deadline_for_test()
        .expect("pending opportunity");
    page.admit_rendering_opportunity_at_for_test(deadline);
    let claimed = page
        .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
        .expect("due rendering task");
    assert_eq!(
        claimed.rendering_update_owner_and_kind().unwrap().1,
        RendererPageRenderingUpdateTaskKind::FrameLifecycle
    );
    page.run_claimed_selected_page_task_for_test(claimed, loader)
        .await?;
    Ok(())
}

#[test]
fn realtime_opportunity_coalesces_changes_and_updates_layout_without_capture() {
    run_page_vm_large_stack_async_test("realtime-opportunity-coalesce", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut()
            .eval("document.body.innerHTML='<div id=target style=width:100px;height:10px></div>'")
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        let first = page.current_realtime_rendering_opportunity().unwrap();
        let deadline = page.next_rendering_opportunity_deadline_for_test();
        page.vm_mut()
            .eval("for(let i=0;i<512;i++) target.style.width=(i+1)+'px'")
            .unwrap();
        assert_eq!(page.current_realtime_rendering_opportunity(), Some(first));
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            deadline,
            "repeated writes cannot postpone an armed opportunity"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        assert!(
            page.vm().next_timeout_deadline().is_none(),
            "frames are not Window timers"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 1);
        let metrics = page.vm().layout_pass_observability_for_test().3.unwrap();
        assert_eq!(
            metrics.reason,
            moli_layout::LayoutFlushReason::RenderingOpportunity
        );
        assert_eq!(metrics.paint_operation_count, 0);
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "512"
        );
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            1,
            "getter reuses the frame result"
        );
        assert!(page.current_realtime_rendering_opportunity().is_none());
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate
            )
            .is_none()
        );
    });
}

#[test]
fn realtime_synchronous_geometry_keeps_the_opportunity_but_avoids_another_build() {
    run_page_vm_large_stack_async_test("realtime-opportunity-after-getter", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut().eval("document.body.innerHTML='<style>#target{width:100px;height:10px}</style><div id=target></div>'").unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .eval("document.styleSheets[0].cssRules[0].style.width='240px'")
            .unwrap();
        let pending = page.current_realtime_rendering_opportunity().unwrap();
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "240"
        );
        let passes = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(page.current_realtime_rendering_opportunity(), Some(pending));
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut().eval("window.sheet=new CSSStyleSheet();sheet.replaceSync('#target{width:360px}');document.adoptedStyleSheets=[sheet]").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "360"
        );
        page.vm_mut()
            .eval("sheet.replaceSync('#target{width:480px}')")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "480"
        );
        page.vm_mut()
            .eval("document.adoptedStyleSheets=[sheet]")
            .unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none(),
            "unchanged adoption must not resubscribe frames"
        );
    });
}

#[test]
fn realtime_claimed_opportunity_cannot_consume_a_replacement_document_request() {
    run_page_vm_large_stack_async_test("realtime-opportunity-replacement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        let deadline = page.next_rendering_opportunity_deadline_for_test().unwrap();
        page.admit_rendering_opportunity_at_for_test(deadline);
        let old = page
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .unwrap();
        page.vm_mut().eval("document.open();document.write('<!doctype html><div id=target style=width:270px></div>');document.close()").unwrap();
        let replacement = page.current_realtime_rendering_opportunity().unwrap();
        page.run_claimed_selected_page_task_for_test(old, &loader)
            .await
            .unwrap();
        assert_eq!(
            page.current_realtime_rendering_opportunity(),
            Some(replacement)
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "270"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_empty_document_waits_for_new_input_without_retrying() {
    run_page_vm_large_stack_async_test("realtime-opportunity-empty", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval("document.documentElement.remove()")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        page.vm_mut().eval("document.append(document.createElement('html'));document.documentElement.innerHTML='<body><div id=target style=width:90px></div></body>'").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "90"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn legacy_layout_modes_do_not_schedule_rendering_opportunities() {
    run_page_vm_large_stack_async_test("legacy-without-opportunity", || async move {
        for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/legacy").unwrap(),
            );
            page.vm_mut().set_layout_policy(policy);
            page.vm_mut().eval("document.body.innerHTML='<style>div{width:100px}</style><div id=target></div>';target.style.width='200px';document.styleSheets[0].cssRules[0].style.height='20px'").unwrap();
            assert!(page.current_realtime_rendering_opportunity().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        }
    });
}

#[test]
fn realtime_failed_opportunity_reports_error_without_an_automatic_retry() {
    run_page_vm_large_stack_async_test("realtime-opportunity-failure", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut()
            .eval("document.body.innerHTML='<div id=target style=width:100px></div>'")
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        let deadline = page.next_rendering_opportunity_deadline_for_test().unwrap();
        page.admit_rendering_opportunity_at_for_test(deadline);
        let task = page.take_rendering_update_body_task_for_test().unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        host.borrow().simulate_active_layout_pass_for_test(true);
        let error = page
            .apply_selected_page_rendering_update_turn(task)
            .unwrap_err();
        host.borrow().simulate_active_layout_pass_for_test(false);
        assert_eq!(
            error.downcast_ref::<moli_layout::LayoutError>(),
            Some(&moli_layout::LayoutError::ReentrantLayoutPass)
        );
        assert!(page.current_realtime_rendering_opportunity().is_none());
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 0);
        page.vm_mut().eval("target.style.width='240px'").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "240"
        );
    });
}
