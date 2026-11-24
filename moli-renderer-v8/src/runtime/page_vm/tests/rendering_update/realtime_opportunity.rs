use super::*;
use moli_page_types::LayoutPolicy;

pub(super) async fn run_opportunity(
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

#[cfg(feature = "molio")]
#[test]
fn realtime_viewport_requests_display_ticks_until_it_is_idle() {
    use crate::runtime::RendererViewportSubscription;
    use std::{sync::Arc, time::Instant};
    use tokio::sync::{Notify, watch};

    run_page_vm_large_stack_async_test("realtime-viewport-display-demand", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/viewport").unwrap(),
        );
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        let (updates, _frames) = watch::channel(None);
        let (display_tick_requests, mut requests) = watch::channel(None);
        page.subscribe_viewport_frames(RendererViewportSubscription {
            surface: Default::default(),
            viewport_revision: 4,
            initial_pointer_position: None,
            updates,
            cursor: watch::Sender::new(None),
            display_tick_requests,
            resource_wake: Arc::new(Notify::new()),
        })
        .unwrap();
        assert_eq!(*requests.borrow_and_update(), Some(4));
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(*requests.borrow_and_update(), None);

        let now = Instant::now();
        page.accept_viewport_display_tick(4, now);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(!requests.has_changed().unwrap());

        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        host.borrow().set_realtime_opportunity_clock_for_test(now);
        for script in [
            "requestAnimationFrame(() => {})",
            "document.body.style.background = 'red'",
        ] {
            page.vm_mut().eval(script).unwrap();
            assert_eq!(*requests.borrow_and_update(), Some(4));
            let fallback = page.next_rendering_opportunity_deadline_for_test().unwrap();
            assert!(fallback > now);
            page.accept_viewport_display_tick(3, now);
            assert_eq!(
                page.next_rendering_opportunity_deadline_for_test(),
                Some(fallback)
            );
            page.accept_viewport_display_tick(4, now);
            assert_eq!(
                page.next_rendering_opportunity_deadline_for_test(),
                Some(now)
            );
            assert!(
                !requests.has_changed().unwrap(),
                "rescheduling keeps demand active"
            );

            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(*requests.borrow_and_update(), None);
        }
    });
}

#[cfg(feature = "molio")]
#[test]
fn realtime_history_push_state_leaves_frame_scheduler_idle() {
    run_page_vm_large_stack_async_test("realtime-history-state", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/static").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );

        page.vm_mut()
            .eval("history.pushState(null, '', '/pushed')")
            .unwrap();
        let navigation = page.vm().viewport_navigation();
        assert_eq!(navigation.url.path(), "/pushed");
        assert!(navigation.can_go_back);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_lazy_image_csp_timeout_settles_frame_and_recovers() {
    run_page_vm_large_stack_async_test("realtime-lazy-image-csp-budget", || async move {
        use crate::v8_execution_watchdog::{V8ExecutionWatchdog, V8ExecutionWatchdogKind};

        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for report_only_first in [false, true] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/realtime-lazy-image-budget").unwrap(),
            );
            page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
            page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 800,
                inner_height: 600,
                device_pixel_ratio: 1.0,
                ..Default::default()
            }))
            .unwrap();
            if report_only_first {
                page.vm_mut()
                    .set_response_content_security_report_only_policies(&["img-src 'none'".into()]);
            }
            page.vm_mut()
                .eval(
                    r#"
                window.trace = [];
                const frame = document.createElement('iframe');
                document.body.append(frame);
                const child = frame.contentDocument;
                const policy = child.createElement('meta');
                policy.httpEquiv = 'Content-Security-Policy';
                policy.content = "img-src 'none'";
                child.head.append(policy);
                child.addEventListener('securitypolicyviolation', event => {
                    trace.push(event.disposition + ':' + event.effectiveDirective);
                    const end = performance.now() + 600;
                    while (performance.now() < end) {}
                    trace.push('listener finished');
                });
                child.addEventListener('securitypolicyviolation', () => trace.push('next listener'));
            "#,
                )
                .unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();

            page.vm_mut()
                .eval(
                    r#"
                requestAnimationFrame(() => {
                    const image = child.createElement('img');
                    image.loading = 'lazy';
                    image.width = image.height = 32;
                    image.src = '/fixture.png';
                    child.body.append(image);
                    trace.push('rAF finished');
                });
            "#,
                )
                .unwrap();
            let budget = V8ExecutionWatchdog::override_timeout_for_test(
                V8ExecutionWatchdogKind::LifecycleEvent,
                Duration::from_millis(100),
            );
            let error = run_opportunity(&mut page, &loader).await.unwrap_err();
            drop(budget);
            assert!(
                error
                    .to_string()
                    .contains("Realtime lazy image admission exceeded 100ms and was terminated"),
                "{error}"
            );
            let disposition = if report_only_first {
                "report"
            } else {
                "enforce"
            };
            let interrupted_trace = format!("rAF finished|{disposition}:img-src");
            assert_eq!(
                page.vm_mut().eval("trace.join('|')").unwrap(),
                interrupted_trace
            );
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );

            page.vm_mut()
                .eval("requestAnimationFrame(() => trace.push('recovered'))")
                .unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut().eval("trace.join('|')").unwrap(),
                format!("{interrupted_trace}|recovered")
            );
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}

#[test]
fn realtime_opportunity_keeps_cadence_skips_missed_ticks_and_stops_when_idle() {
    run_page_vm_large_stack_async_test("realtime-opportunity-cadence", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-cadence").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        run_opportunity(&mut page, &loader).await.unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let interval = std::time::Duration::from_nanos(16_666_667);
        let work = std::time::Duration::from_millis(4);
        let start = std::time::Instant::now();
        host.borrow().set_realtime_opportunity_clock_for_test(start);
        page.vm_mut()
            .eval("window.calls=0;requestAnimationFrame(function frame(){if(++calls<3)requestAnimationFrame(frame)})")
            .unwrap();
        let first = page.next_rendering_opportunity_deadline_for_test().unwrap();
        assert_eq!(first, start + interval);

        // Finish four milliseconds after the nominal first tick. Callback
        // work consumes the current interval instead of adding to it.
        host.borrow()
            .set_realtime_opportunity_clock_for_test(first + work);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "1");
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(first + interval),
            "callback and layout work must not add a whole wait interval"
        );

        // A delayed frame still calls the callback once. Skip the elapsed
        // ticks and preserve the cadence for the next available opportunity.
        host.borrow()
            .set_realtime_opportunity_clock_for_test(first + interval * 10 + work);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "2");
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(first + interval * 11)
        );
        host.borrow()
            .set_realtime_opportunity_clock_for_test(first + interval * 11);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "3");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );

        // A canceled last request leaves no scheduled work, and a new burst
        // starts from its own clock sample rather than a canceled future tick.
        let resumed = first + interval * 100;
        host.borrow()
            .set_realtime_opportunity_clock_for_test(resumed);
        page.vm_mut()
            .eval("window.raf=requestAnimationFrame(()=>calls++)")
            .unwrap();
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(resumed + interval)
        );
        page.vm_mut().eval("cancelAnimationFrame(raf)").unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        host.borrow()
            .set_realtime_opportunity_clock_for_test(resumed + work);
        page.vm_mut()
            .eval("requestAnimationFrame(()=>calls++)")
            .unwrap();
        assert_eq!(
            page.next_rendering_opportunity_deadline_for_test(),
            Some(resumed + work + interval)
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("String(calls)").unwrap(), "4");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
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
fn realtime_geometry_reuses_layout_after_detached_dom_mutations() {
    run_page_vm_large_stack_async_test("realtime-detached-dom", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut()
            .eval(
                r#"
document.body.innerHTML = '<div id="probe" style="width:120px"><div style="height:10px"></div></div>';
globalThis.detached = document.createElement('div');
detached.style.cssText = 'width:240px;height:8px;overflow:hidden';
globalThis.detachedText = detached.appendChild(document.createTextNode('initial'));
globalThis.scratch = document.createDocumentFragment();
globalThis.otherDocument = document.implementation.createHTMLDocument('inactive');
"#,
            )
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetHeight)").unwrap(),
            "10"
        );
        let passes = page.vm().layout_pass_observability_for_test().1;

        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"(() => {
    for (let i = 0; i < 12; ++i) {
        detached.setAttribute('data-revision', String(i));
        detached.style.width = (229 + i) + 'px';
        detachedText.data = String(i);
        scratch.replaceChildren(document.createElement('span'));
        otherDocument.body.setAttribute('data-revision', String(i));
        if (probe.offsetWidth !== 120) throw new Error('unexpected probe width');
    }
    return [detached.isConnected, otherDocument.body.isConnected, probe.offsetHeight].join('|');
})()"#,
                )
                .unwrap(),
            "false|true|10"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);

        assert_eq!(
            page.vm_mut()
                .eval("probe.appendChild(detached); [detached.offsetWidth, probe.offsetHeight].join('|')")
                .unwrap(),
            "240|18"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);

        assert_eq!(
            page.vm_mut()
                .eval("scratch.appendChild(detached); String(probe.offsetHeight)")
                .unwrap(),
            "10"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
    });
}

#[test]
fn realtime_geometry_uses_data_attribute_style_invalidation() {
    run_page_vm_large_stack_async_test("realtime-data-attributes", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut()
            .eval(
                r#"
document.body.innerHTML = `
  <style>
    #probe { width: 120px }
    #probe:has(> [data-wide]) { width: 200px }
    #attributeValue { width: attr(data-width type(<length>), 60px) }
  </style>
  <div id="probe"><span id="child"></span></div><div id="attributeValue"></div>`;
"#,
            )
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetWidth)").unwrap(),
            "120"
        );
        let passes = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"(() => {
                for (let i = 0; i < 12; ++i) {
                    child.dataset.unused = i;
                    if (probe.offsetWidth !== 120) throw new Error('unexpected width');
                }
                return child.dataset.unused;
            })()"#
                )
                .unwrap(),
            "11",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"
child.dataset.wide = '';
getComputedStyle(probe).color;
String(probe.offsetWidth)
"#
                )
                .unwrap(),
            "200",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"
child.removeAttribute('data-wide');
attributeValue.dataset.width = '180px';
[probe.offsetWidth, attributeValue.offsetWidth].join('|')
"#
                )
                .unwrap(),
            "120|180",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
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
