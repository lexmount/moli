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

#[test]
fn realtime_css_animations_sample_delayed_layout_and_keep_forward_fill() {
    use std::time::Duration;

    run_page_vm_large_stack_async_test("realtime-css-animations", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/animation").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        run_opportunity(&mut page, &loader).await.unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        page.vm_mut()
            .eval(
                r#"
                document.body.innerHTML = `<style>
                body { margin: 0 }
                #notice { height: 0; overflow: hidden; animation: expand .4s 1s linear forwards }
                #footer { position: fixed; bottom: -72px; height: 72px;
                          animation: slide .4s 1s linear forwards }
                @keyframes expand { from { height: 0; margin-bottom: 0 }
                                    to { height: 40px; margin-bottom: 16px } }
                @keyframes slide { from { bottom: -72px } to { bottom: 0 } }
                </style><div id=notice>Notice</div><div id=main></div><div id=footer>Footer</div>`;
                window.geometry = () => JSON.stringify([
                    notice.offsetHeight, main.offsetTop, getComputedStyle(footer).bottom
                ]);
                "#,
            )
            .unwrap();
        let initial = page.next_rendering_opportunity_deadline_for_test().unwrap();
        let clock = |now| host.borrow().set_realtime_opportunity_clock_for_test(now);
        clock(initial);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("geometry()").unwrap(),
            r#"[0,0,"-72px"]"#
        );

        let start = page.next_rendering_opportunity_deadline_for_test().unwrap();
        assert_eq!(start, initial + Duration::from_secs(1));
        clock(start + Duration::from_millis(200));
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("geometry()").unwrap(),
            r#"[20,28,"-36px"]"#
        );

        clock(start + Duration::from_millis(450));
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("geometry()").unwrap(),
            r#"[40,56,"0px"]"#
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
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
fn realtime_geometry_compares_styles_to_the_screen_publication() {
    use style::servo::restyle_damage::ServoRestyleDamage;

    run_page_vm_large_stack_async_test("realtime-equivalent-style", || async move {
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
    .sameA { width: 120px }
    .sameB { width: 120px }
    .repaint { width: 120px; color: red; background-color: blue }
    .grow { width: 200px }
    .pseudo { width: 200px }
    .pseudo::before { content: "x"; display: block; height: 24px }
  </style>
  <div id="probe" class="sameA">
    <div style="display:contents"><div style="height:10px"></div></div>
  </div>
  <div id="neighbor" class="sameA"></div>`;
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
                .eval("probe.className = 'sameB'; String(probe.offsetWidth)")
                .unwrap(),
            "120",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let probe = page
            .vm()
            .document_runtime
            .get_element_by_id("probe")
            .unwrap();
        let neighbor = page
            .vm()
            .document_runtime
            .get_element_by_id("neighbor")
            .unwrap();

        assert_eq!(
            page.vm_mut()
                .eval("probe.className = 'repaint'; getComputedStyle(probe).color")
                .unwrap(),
            "rgb(255, 0, 0)",
        );
        assert_eq!(
            host.borrow().observed_screen_style_damage_for_test(),
            ServoRestyleDamage::REPAINT,
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetWidth)").unwrap(),
            "120"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert!(
            host.borrow()
                .observed_screen_style_damage_for_test()
                .is_empty()
        );

        page.vm_mut()
            .eval(
                "probe.className = 'grow'; neighbor.className = 'repaint'; \
                 getComputedStyle(probe).color; getComputedStyle(neighbor).color",
            )
            .unwrap();
        assert!(
            host.borrow()
                .screen_style_damage_for_node_for_test(probe)
                .unwrap()
                .contains(ServoRestyleDamage::RELAYOUT)
        );
        assert_eq!(
            host.borrow()
                .screen_style_damage_for_node_for_test(neighbor),
            Some(ServoRestyleDamage::REPAINT),
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetWidth)").unwrap(),
            "200",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
        assert_eq!(
            host.borrow().screen_style_damage_for_node_for_test(probe),
            None
        );
        assert_eq!(
            host.borrow()
                .screen_style_damage_for_node_for_test(neighbor),
            None
        );

        page.vm_mut()
            .eval("probe.className = 'pseudo'; getComputedStyle(probe).color")
            .unwrap();
        assert_eq!(
            host.borrow().observed_screen_style_damage_for_test(),
            ServoRestyleDamage::reconstruct(),
        );
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetHeight)").unwrap(),
            "34",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 3);
    });
}

#[test]
fn realtime_equivalent_parent_styles_only_resolve_affected_elements() {
    run_page_vm_large_stack_async_test("realtime-exact-style-targets", || async move {
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
    #parent.sameA, #parent.sameB { width: 120px; font-size: 10px }
    #parent.inherit { font-size: 20px }
    #parent .item { width: 2em; height: 1px }
    #parent.wide #child { width: 60px }
  </style>
  <div id="parent" class="sameA"><div id="child" class="item"></div></div>`;
const probe = document.getElementById('parent');
probe.insertAdjacentHTML('beforeend', '<div class="item"></div>'.repeat(799));
"#,
            )
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        assert_eq!(
            page.vm_mut().eval("String(child.offsetWidth)").unwrap(),
            "20"
        );
        let passes = page.vm().layout_pass_observability_for_test().1;
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let resolutions = host.borrow().element_style_resolutions_for_test();
        assert_eq!(
            page.vm_mut()
                .eval("probe.className = 'sameB'; String(child.offsetWidth)")
                .unwrap(),
            "20",
        );
        assert_eq!(
            host.borrow().element_style_resolutions_for_test(),
            resolutions + 1
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);

        assert_eq!(
            page.vm_mut()
                .eval("probe.className = 'sameB wide'; String(child.offsetWidth)")
                .unwrap(),
            "60",
        );
        assert_eq!(
            page.vm_mut()
                .eval(
                    "probe.className = 'sameB wide inherit'; \
                     getComputedStyle(probe).fontSize; String(child.nextElementSibling.offsetWidth)",
                )
                .unwrap(),
            "40",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
    });
}

#[test]
fn cssom_equivalent_parent_style_reuses_cached_descendant_styles_in_all_layout_modes() {
    run_page_vm_large_stack_async_test("cssom-exact-style-targets", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in [
            LayoutPolicy::Mock,
            LayoutPolicy::Lazy,
            LayoutPolicy::Realtime,
        ] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/cssom").unwrap(),
            );
            page.vm_mut().set_layout_policy(policy);
            assert_eq!(
                page.vm_mut().eval(r#"
document.body.innerHTML = '<style>.one, .two { color: blue }</style><div id="root" class="one"></div>';
root.innerHTML = '<span></span>'.repeat(800);
function allBlue() {
  return String(Array.from(root.children).every(el => getComputedStyle(el).color === 'rgb(0, 0, 255)'));
}
allBlue()
"#).unwrap(),
                "true",
            );
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            let resolutions = host.borrow().element_style_resolutions_for_test();
            let passes = page.vm().layout_pass_observability_for_test().1;
            assert_eq!(
                page.vm_mut()
                    .eval("root.className = 'two'; allBlue()")
                    .unwrap(),
                "true",
            );
            assert_eq!(
                host.borrow().element_style_resolutions_for_test(),
                resolutions + 1,
                "{policy:?}"
            );
            assert_eq!(
                page.vm().layout_pass_observability_for_test().1,
                passes,
                "{policy:?}"
            );
        }
    });
}

#[test]
fn realtime_style_reuse_preserves_lazy_pseudo_coverage() {
    run_page_vm_large_stack_async_test("realtime-lazy-pseudo-coverage", || async move {
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
    .sameA, .sameB { width: 120px }
    li::marker { color: blue; content: attr(data-label) ' ' }
    li.red::marker { color: red }
  </style>
  <div id="probe" class="sameA"><ol><li id="item" data-label="A">Item</li></ol></div>`;
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
                .eval("probe.className = 'sameB'; String(probe.offsetWidth)")
                .unwrap(),
            "120",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);

        // The equivalent parent update leaves the child's lazy pseudo cache
        // subject to the canonical style path's inherited invalidation.
        page.vm_mut().eval("getComputedStyle(item).color").unwrap();
        assert_eq!(
            page.vm_mut()
                .eval(
                    "item.dataset.label = 'B'; probe.offsetWidth; \
                 String(getComputedStyle(item, '::marker').content.includes('B'))",
                )
                .unwrap(),
            "true",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            page.vm_mut()
                .eval(
                    "item.className = 'red'; getComputedStyle(item).color; \
                 probe.offsetWidth; getComputedStyle(item, '::marker').color",
                )
                .unwrap(),
            "rgb(255, 0, 0)",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
    });
}

#[test]
fn realtime_style_reuse_expires_when_canonical_styles_are_lost() {
    run_page_vm_large_stack_async_test("realtime-style-cache-loss", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut()
            .eval("document.body.innerHTML='<style>.sameA,.sameB{width:120px}</style><div id=probe class=sameA></div>'")
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetWidth)").unwrap(),
            "120"
        );
        let passes = page.vm().layout_pass_observability_for_test().1;
        let probe = page
            .vm()
            .document_runtime
            .get_element_by_id("probe")
            .unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();

        // Simulate losing an entry that was used to produce the frozen tree.
        host.borrow()
            .retire_computed_style_for_inactive_handle(probe);
        page.vm_mut().eval("getComputedStyle(probe).color").unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(
            page.vm_mut()
                .eval("probe.className = 'sameB'; String(probe.offsetWidth)")
                .unwrap(),
            "120",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
    });
}

#[test]
fn named_container_change_reaches_cssom_descendants_past_an_unchanged_parent() {
    run_page_vm_large_stack_async_test("cssom-named-container-invalidation", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/cssom").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"
document.body.innerHTML = `
  <style>
    #container { container-type: inline-size; width: 240px }
    #container.named { container-name: card }
    #probe { width: 100px; color: blue }
    @container card (min-width: 200px) { #probe { width: 60px; color: red } }
  </style>
  <div id="container"><div id="middle"><div id="probe"></div></div></div>`;
String(probe.offsetWidth) + '/' + getComputedStyle(probe).color
"#
                )
                .unwrap(),
            "100/rgb(0, 0, 255)",
        );
        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"
container.className = 'named';
getComputedStyle(container).containerName;
getComputedStyle(middle).color;
getComputedStyle(probe).color + '/' + probe.offsetWidth
"#
                )
                .unwrap(),
            "rgb(255, 0, 0)/60",
        );
    });
}

#[test]
fn realtime_style_reuse_expires_after_a_temporary_container_size() {
    run_page_vm_large_stack_async_test("realtime-style-container-restore", || async move {
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
    #container { container-type: inline-size; width: 150px }
    #probe { width: attr(data-width type(<length>), 120px); color: blue }
    @container (min-width: 200px) { #probe { width: 60px; color: red } }
  </style>
  <div id="container"><div id="probe"></div></div>`;
"#,
            )
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetWidth)").unwrap(),
            "120"
        );
        let passes = page.vm().layout_pass_observability_for_test().1;
        let container = page
            .vm()
            .document_runtime
            .get_element_by_id("container")
            .unwrap();
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let document = host.borrow().document_handle();
        let temporary = moli_selector::QueryContainerSizes::from([(
            container,
            euclid::default::Size2D::new(Some(app_units::Au::from_px(240)), None),
        )]);
        let original = host
            .borrow()
            .replace_layout_container_sizes(document, temporary)
            .unwrap();
        assert_eq!(
            page.vm_mut().eval("getComputedStyle(probe).color").unwrap(),
            "rgb(255, 0, 0)",
        );
        host.borrow()
            .replace_layout_container_sizes(document, original)
            .unwrap();
        assert_eq!(
            page.vm_mut().eval("String(probe.offsetWidth)").unwrap(),
            "120"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(
            page.vm_mut()
                .eval("probe.dataset.width = '140px'; String(probe.offsetWidth)")
                .unwrap(),
            "140",
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
    });
}

#[test]
fn realtime_same_css_text_observer_record_keeps_layout_current() {
    run_page_vm_large_stack_async_test("realtime-cssom-same-value-observed", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut()
            .eval(
                "document.body.innerHTML='<div id=target></div>'; target.style.cssText='width: 120px;'",
            )
            .unwrap();
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "120"
        );
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        assert!(host.borrow().realtime_screen_layout_is_current());
        let passes = page.vm().layout_pass_observability_for_test().1;
        let resolutions = host.borrow().element_style_resolutions_for_test();

        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"
const observer = new MutationObserver(() => {});
observer.observe(target, { attributes: true, attributeOldValue: true });
target.style.cssText = target.style.cssText;
JSON.stringify(observer.takeRecords().map(record => [record.attributeName, record.oldValue]))
"#,
                )
                .unwrap(),
            r#"[["style","width: 120px;"]]"#
        );
        assert!(host.borrow().realtime_screen_layout_is_current());
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "120"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(
            host.borrow().element_style_resolutions_for_test(),
            resolutions
        );

        page.vm_mut()
            .eval("target.style.cssText = 'width: 150px;'")
            .unwrap();
        assert!(!host.borrow().realtime_screen_layout_is_current());
        assert_eq!(
            page.vm_mut().eval("String(target.offsetWidth)").unwrap(),
            "150"
        );
    });
}

#[test]
fn realtime_same_css_text_resolves_urls_against_the_new_base() {
    run_page_vm_large_stack_async_test("realtime-cssom-base-change", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        let images = page
            .vm_mut()
            .eval(
                r#"
const base = document.head.appendChild(document.createElement('base'));
base.href = '/a/';
const target = document.body.appendChild(document.createElement('div'));
target.style.cssText = 'background-image: url(image.png);';
const original = getComputedStyle(target).backgroundImage;
void target.offsetWidth;
base.href = '/b/';
const afterBase = getComputedStyle(target).backgroundImage;
void target.offsetWidth;

const observer = new MutationObserver(() => {});
observer.observe(target, {attributes: true, attributeOldValue: true});
target.style.cssText = target.style.cssText;
[original, afterBase, getComputedStyle(target).backgroundImage].join('|')
"#,
            )
            .unwrap();
        assert_eq!(
            images,
            concat!(
                r#"url("https://example.com/a/image.png")|"#,
                r#"url("https://example.com/a/image.png")|"#,
                r#"url("https://example.com/b/image.png")"#
            )
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
        for policy in [LayoutPolicy::Mock, LayoutPolicy::Lazy] {
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
