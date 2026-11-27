#![cfg(feature = "devtools")]

use super::*;
use crate::runtime::{
    RendererDevToolsPageCommand, RendererPageCommand, RendererSyntheticWheelGesture,
};
use moli_page_types::LayoutPolicy;

fn surface(width: u32) -> crate::protocol_types::ViewportSurface {
    crate::protocol_types::ViewportSurface {
        inner_width: width,
        inner_height: 400,
        device_pixel_ratio: 1.0,
        ..Default::default()
    }
}

fn wheel(
    page: &mut PageVm,
    gesture: &RendererSyntheticWheelGesture,
    x: f64,
    dx: f64,
    dy: f64,
) -> anyhow::Result<()> {
    page.dispatch_renderer_page_command(RendererPageCommand::DevTools(
        RendererDevToolsPageCommand::DispatchSyntheticWheel {
            x,
            y: 40.0,
            delta_x: dx,
            delta_y: dy,
            end: false,
            intercept_drags: false,
            gesture: gesture.clone(),
        },
    ))?;
    Ok(())
}

fn install(page: &mut PageVm) {
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page.set_viewport_surface(Some(surface(600))).unwrap();
    page.vm_mut().eval(r#"
        document.body.style.cssText='margin:0;height:2400px';
        document.body.innerHTML='<div id="a" style="width:160px;height:120px;overflow:auto"><div style="width:100px;height:1800px">content</div></div><iframe id="f" style="position:absolute;left:300px;top:0;width:200px;height:180px;border:0"></iframe>';
        window.a=document.getElementById('a'); window.events=[];
        document.addEventListener('wheel',e=>events.push(e.deltaY));
    "#).unwrap();
}

#[test]
fn synthetic_wheel_removed_scroller_stays_invalid_after_reinsertion() {
    run_page_vm_large_stack_async_test("synthetic-wheel-reinsert", || async move {
        for (setup, detach, reinsert, gap) in [
            ("", "a.remove()", "document.body.prepend(a)", false),
            ("", "a.remove()", "document.body.prepend(a)", true),
            (
                "window.wrap=document.createElement('div');a.before(wrap);wrap.append(a)",
                "wrap.remove()",
                "document.body.prepend(wrap)",
                false,
            ),
            ("", "document.body.moveBefore(a,null)", "", false),
        ] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/wheel").unwrap(),
            );
            install(&mut page);
            page.vm_mut().eval(setup).unwrap();
            let gesture = RendererSyntheticWheelGesture::default();
            wheel(&mut page, &gesture, 40.0, 0.0, 50.0).unwrap();
            assert_eq!(page.vm_mut().eval("String(a.scrollTop)").unwrap(), "50");
            page.vm_mut().eval(detach).unwrap();
            if gap {
                wheel(&mut page, &gesture, 40.0, 0.0, 25.0).unwrap();
            }
            page.vm_mut().eval(reinsert).unwrap();
            wheel(&mut page, &gesture, 40.0, 0.0, 100.0).unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([a.scrollTop,scrollY])")
                    .unwrap(),
                "[50,0]",
                "{detach}, gap={gap}"
            );
            page.vm().wheel_input.borrow_mut().end_platform_sequence();
            wheel(&mut page, &gesture.next_sequence(), 40.0, 0.0, 40.0).unwrap();
            assert_eq!(
                page.vm_mut().eval("String(a.scrollTop)").unwrap(),
                "90",
                "new gesture must select a fresh scrollbox"
            );
        }
    });
}

#[test]
fn synthetic_wheel_does_not_scroll_a_rebuilt_box() {
    run_page_vm_large_stack_async_test("synthetic-wheel-rebuilt-box", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        for rebuild in [
            "a.style.display='flow-root'",
            "a.style.listStylePosition='inside'",
            "a.style.listStylePosition='inside'; void a.offsetWidth; a.style.listStylePosition='outside'; void a.offsetWidth",
            "a.style.display='flow-root'; void a.offsetWidth",
            "a.style.display='flow-root'; void a.offsetWidth; a.style.display='block'; void a.offsetWidth",
        ] {
            install(&mut page);
            page.vm().wheel_input.borrow_mut().end_platform_sequence();
            if rebuild.contains("listStylePosition") {
                page.vm_mut().eval("a.style.display='list-item'").unwrap();
            }
            let gesture = RendererSyntheticWheelGesture::default();
            wheel(&mut page, &gesture, 40.0, 0.0, 50.0).unwrap();
            assert_eq!(page.vm_mut().eval("String(a.scrollTop)").unwrap(), "50");

            page.vm_mut().eval(rebuild).unwrap();
            wheel(&mut page, &gesture, 40.0, 0.0, 100.0).unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([a.scrollTop,scrollY])")
                    .unwrap(),
                "[50,0]",
                "{rebuild}"
            );

            page.vm().wheel_input.borrow_mut().end_platform_sequence();
            wheel(&mut page, &gesture.next_sequence(), 40.0, 0.0, 40.0).unwrap();
            assert_eq!(page.vm_mut().eval("String(a.scrollTop)").unwrap(), "90");
        }
    });
}

#[test]
fn synthetic_wheel_adopted_scroller_does_not_follow_old_gesture() {
    run_page_vm_large_stack_async_test("synthetic-wheel-adoption", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 40.0, 0.0, 50.0).unwrap();
        page.vm_mut()
            .eval("f.contentDocument.body.append(a)")
            .unwrap();
        wheel(&mut page, &gesture, 40.0, 0.0, 100.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([a.scrollTop,scrollY])")
                .unwrap(),
            "[50,0]"
        );
    });
}

#[test]
fn synthetic_wheel_uses_one_scroller_for_both_axes_and_keeps_viewport_at_boundary() {
    run_page_vm_large_stack_async_test("synthetic-wheel-vector", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        page.vm_mut().eval(r#"
          document.body.innerHTML='<div id=outer style="width:160px;height:150px;overflow-x:auto;overflow-y:hidden"><div id=inner style="width:600px;height:140px;overflow-y:auto;overflow-x:hidden"><div style="height:1800px">text</div></div></div>';
        "#).unwrap();
        wheel(
            &mut page,
            &RendererSyntheticWheelGesture::default(),
            40.0,
            40.0,
            50.0,
        )
        .unwrap();
        assert_eq!(page.vm_mut().eval("JSON.stringify([outer.scrollLeft,outer.scrollTop,inner.scrollLeft,inner.scrollTop,scrollY])").unwrap(),"[0,0,0,50,0]");
        page.vm().wheel_input.borrow_mut().end_platform_sequence();
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 400.0, 0.0, -50.0).unwrap();
        assert_eq!(page.vm_mut().eval("String(scrollY)").unwrap(), "0");
        wheel(&mut page, &gesture, 400.0, 0.0, 100.0).unwrap();
        assert_eq!(page.vm_mut().eval("String(scrollY)").unwrap(), "100");
    });
}

#[test]
fn synthetic_wheel_skips_resized_outside_updates_and_rejects_initial_outside_anchor() {
    run_page_vm_large_stack_async_test("synthetic-wheel-resize", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 500.0, 0.0, 50.0).unwrap();
        page.set_viewport_surface(Some(surface(300))).unwrap();
        wheel(&mut page, &gesture, 500.0, 0.0, 100.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([scrollY,events])")
                .unwrap(),
            "[50,[50]]"
        );
        let error = wheel(
            &mut page,
            &RendererSyntheticWheelGesture::default(),
            500.0,
            0.0,
            100.0,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("Position out of bounds"),
            "{error}"
        );
        let repeat = gesture.next_sequence();
        wheel(&mut page, &repeat, 500.0, 0.0, 75.0).unwrap();
        page.set_viewport_surface(Some(surface(600))).unwrap();
        wheel(&mut page, &repeat, 500.0, 0.0, 25.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([scrollY,events])")
                .unwrap(),
            "[75,[50,25]]"
        );
    });
}

#[test]
fn synthetic_wheel_round_cannot_follow_a_replaced_document() {
    run_page_vm_large_stack_async_test("synthetic-wheel-document-replacement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        let old_gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &old_gesture, 40.0, 0.0, 50.0).unwrap();
        assert_eq!(page.vm_mut().eval("String(a.scrollTop)").unwrap(), "50");

        page.vm_mut()
            .eval("document.open(); document.write('<body></body>'); document.close();")
            .unwrap();
        install(&mut page);
        wheel(&mut page, &old_gesture, 40.0, 0.0, 50.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([a.scrollTop,events])")
                .unwrap(),
            "[0,[]]",
        );

        let next_round = old_gesture.next_sequence();
        wheel(&mut page, &next_round, 40.0, 0.0, 40.0).unwrap();
        assert_eq!(page.vm_mut().eval("String(a.scrollTop)").unwrap(), "40");
    });
}

#[test]
fn synthetic_wheel_survives_unrelated_mutations() {
    run_page_vm_large_stack_async_test("synthetic-wheel-stable-box", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 40.0, 0.0, 50.0).unwrap();
        page.vm_mut().eval("const unrelated=document.createElement('div');document.body.append(unrelated);unrelated.remove();a.style.height='140px';void a.offsetWidth").unwrap();
        wheel(&mut page, &gesture, 40.0, 0.0, 25.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([a.scrollTop,scrollY])")
                .unwrap(),
            "[75,0]"
        );
    });
}

#[test]
fn synthetic_wheel_actual_phase_expires_without_an_ack_for_filtered_updates() {
    run_page_vm_large_stack_async_test("wheel-phase-expiry", || async move {
        use crate::wheel_input::{WheelInput, WheelPhase, WheelSource};
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        page.vm_mut().eval("window.block=false;events=[];addEventListener('wheel',e=>{if(block)e.preventDefault();events.push([e.cancelable,e.defaultPrevented])},{passive:false})").unwrap();
        let now = std::time::Instant::now();
        for (ms, block) in [(0, false), (20, false), (620, true), (640, true)] {
            page.vm_mut()
                .eval(&format!("window.block={block}"))
                .unwrap();
            let input = WheelInput {
                position: (550., 40.),
                delta: (0., 25.),
                modifiers: 0,
                timestamp: now + std::time::Duration::from_millis(ms),
                phase: WheelPhase::Unspecified,
                source: WheelSource::Platform,
            };
            // The caller deliberately discards completion; mode is committed in
            // the renderer before any protocol workflow can consume its reply.
            let _ = page
                .vm_mut()
                .dispatch_wheel_input(input, Default::default())
                .unwrap();
            if ms == 0 {
                let command = RendererSyntheticWheelGesture::default();
                page.validate_synthetic_wheel_anchor(550., 40., &command)
                    .unwrap();
                page.set_viewport_surface(Some(surface(300))).unwrap();
                // A producer End filtered before Aura is not a platform End.
                page.dispatch_synthetic_wheel(550., 40., 0., 0., true, false, command)
                    .unwrap();
                page.set_viewport_surface(Some(surface(600))).unwrap();
            }
        }
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify(events.filter(Array.isArray))")
                .unwrap(),
            "[[true,false],[false,false],[true,true],[true,true]]"
        );
        assert_eq!(page.vm_mut().eval("String(scrollY)").unwrap(), "50");
        let deadline = page.next_action_window_deadline().unwrap();
        page.vm().wheel_input.borrow_mut().expire(deadline);
        assert!(page.next_action_window_deadline().is_none());
    });
}

#[test]
fn synthetic_wheel_platform_end_without_coordinates_preserves_queue_mode() {
    run_page_vm_large_stack_async_test("wheel-coordinate-free-end", || async move {
        use crate::wheel_input::{WheelInput, WheelPhase, WheelSource};
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        page.vm_mut().eval("events=[];window.block=false;addEventListener('wheel',e=>{if(block)e.preventDefault();events.push([e.cancelable,e.defaultPrevented])},{passive:false})").unwrap();
        for (phase, position, delta, block) in [
            (WheelPhase::Began, (550., 40.), (0., 25.), false),
            (WheelPhase::Ended, (f64::NAN, f64::NAN), (0., 0.), true),
            (WheelPhase::Changed, (550., 40.), (0., 25.), true),
            (WheelPhase::Cancelled, (f64::NAN, f64::NAN), (0., 0.), true),
            (WheelPhase::Began, (550., 40.), (0., 25.), true),
        ] {
            page.vm_mut()
                .eval(&format!("window.block={block}"))
                .unwrap();
            let input = WheelInput {
                position,
                delta,
                modifiers: 0,
                timestamp: std::time::Instant::now(),
                phase,
                source: WheelSource::Platform,
            };
            page.vm_mut()
                .dispatch_wheel_input(input, Default::default())
                .unwrap();
        }
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify(events.filter(Array.isArray))")
                .unwrap(),
            "[[true,false],[false,false],[true,true]]"
        );
        assert_eq!(page.vm_mut().eval("String(scrollY)").unwrap(), "50");
    });
}

#[cfg(feature = "molio")]
#[test]
fn synthetic_wheel_and_viewport_wheel_share_phases_and_coordinate_free_end() {
    run_page_vm_large_stack_async_test("viewport-wheel-phases", || async move {
        use crate::runtime::{
            RendererViewportSubscription, RendererViewportWheelInput, RendererViewportWheelPhase,
        };
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        page.layout_configuration.policy = LayoutPolicy::Realtime;
        let (updates, frames) = tokio::sync::watch::channel(None);
        let (cursor, _cursors) = tokio::sync::watch::channel(None);
        page.subscribe_viewport_frames(RendererViewportSubscription {
            surface: surface(600),
            viewport_revision: 1,
            initial_pointer_position: None,
            updates,
            cursor,
            display_tick_requests: tokio::sync::watch::Sender::new(None),
            resource_wake: std::sync::Arc::new(tokio::sync::Notify::new()),
        })
        .unwrap();
        page.vm_mut().eval("window.block=false;events=[];addEventListener('wheel',e=>{if(block)e.preventDefault();events.push([e.cancelable,e.defaultPrevented])},{passive:false});const b=document.createElement('div');b.id='b';b.style.cssText='position:fixed;left:520px;top:0;width:100px;height:100px;overflow:auto;display:none';b.innerHTML='<div style=height:2000px>overlay</div>';document.body.append(b)").unwrap();
        page.update_viewport_frame();
        assert!(
            frames
                .borrow()
                .as_ref()
                .is_some_and(|frame| frame.snapshot.is_some())
        );
        let document = page.document_lifecycle.identity();
        let input = |phase, position, revision| RendererViewportWheelInput {
            document,
            viewport_revision: revision,
            position,
            buttons: 0,
            delta: (0., 25.),
            modifiers: 0,
            phase,
            precise: true,
        };
        page.dispatch_viewport_wheel(input(RendererViewportWheelPhase::Started, (550., 40.), 1))
            .unwrap();
        page.vm_mut().eval("block=true").unwrap();
        // Explicit Changed retains the mode selected by a real native Began.
        page.dispatch_viewport_wheel(input(RendererViewportWheelPhase::Moved, (550., 40.), 1))
            .unwrap();
        // Neither a stale geometry revision nor missing coordinates loses End.
        page.dispatch_viewport_wheel(input(
            RendererViewportWheelPhase::Ended,
            (f64::NAN, f64::NAN),
            99,
        ))
        .unwrap();
        page.vm_mut().eval("b.style.display='block'").unwrap();
        page.update_viewport_frame();
        page.dispatch_viewport_wheel(input(RendererViewportWheelPhase::Moved, (550., 40.), 1))
            .unwrap();
        // Wayland can include a final movement with End.
        page.dispatch_viewport_wheel(input(RendererViewportWheelPhase::Ended, (550., 40.), 1))
            .unwrap();
        page.dispatch_viewport_wheel(input(
            RendererViewportWheelPhase::Cancelled,
            (f64::NAN, f64::NAN),
            99,
        ))
        .unwrap();
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 550., 0., 25.).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify(events.filter(Array.isArray))")
                .unwrap(),
            "[[true,false],[false,false],[false,false],[false,false],[true,true]]"
        );
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([scrollY,b.scrollTop])")
                .unwrap(),
            "[50,50]"
        );
    });
}
