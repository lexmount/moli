use super::realtime_opportunity::run_opportunity;
use super::*;
use crate::runtime::{RendererPageCommand, RendererPointerEventProperties};
use moli_page_types::LayoutPolicy;

fn mouse(page: &mut PageVm, event: &str, x: f64, y: f64, delta: f64) {
    page.dispatch_renderer_page_command(RendererPageCommand::DispatchMouseEventAtPoint {
        x,
        y,
        event_name: event.to_owned(),
        button: if event == "wheel" { -1 } else { 0 },
        buttons: Some(i32::from(event == "mousedown")),
        click_count: 1,
        delta_x: 0.0,
        delta_y: delta,
        pointer: RendererPointerEventProperties::default(),
        modifiers: 0,
    })
    .unwrap();
}

#[test]
fn realtime_input_wheels_finish_individually_and_honor_canceled_default_actions() {
    run_page_vm_large_stack_async_test("realtime-input-wheel", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-input").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=scroller style="position:absolute;left:0;top:0;width:200px;height:100px;overflow:auto"><div style="height:600px"></div></div>';
            window.events=[];
            window.mo=new MutationObserver(()=>events.push('mutation'));
            mo.observe(scroller,{attributes:true});
            scroller.addEventListener('wheel',event=>{
                events.push('wheel:'+event.deltaY+':'+scroller.scrollTop);
                scroller.setAttribute('data-delta',event.deltaY);
                Promise.resolve().then(()=>events.push('microtask:'+scroller.scrollTop));
                if(event.deltaY<0)event.preventDefault();
            },{passive:false});
        "#).unwrap();
        for (index, delta) in [30.0, -20.0, 60.0].into_iter().enumerate() {
            mouse(&mut page, "wheel", 10.0, 10.0, delta);
            assert!(
                page.next_action_window_deadline().is_none(),
                "Realtime never admits the one-second window"
            );
            assert_eq!(
                page.vm_mut().eval("String(events.length)").unwrap(),
                ((index + 1) * 3).to_string(),
                "each command returns after its handler, default action and microtasks"
            );
        }
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "wheel:30:0|mutation|microtask:30|wheel:-20:30|mutation|microtask:30|wheel:60:30|mutation|microtask:90"
        );
        assert_eq!(
            page.vm_mut().eval("String(scroller.scrollTop)").unwrap(),
            "90"
        );
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
fn legacy_input_modes_keep_the_wheel_window_and_batch_checkpoint() {
    run_page_vm_large_stack_async_test("legacy-input-window", || async move {
        for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/legacy-input").unwrap(),
            );
            page.vm_mut().set_layout_policy(policy);
            page.vm_mut()
                .eval(
                    r#"
                document.body.innerHTML='<div style="width:200px;height:200px"></div>';
                window.events=[];addEventListener('wheel',event=>{
                    events.push('wheel:'+event.deltaY);
                    Promise.resolve().then(()=>events.push('microtask:'+event.deltaY));
                },true);
            "#,
                )
                .unwrap();
            mouse(&mut page, "wheel", 10.0, 10.0, 30.0);
            let deadline = page.next_action_window_deadline().expect("legacy window");
            mouse(&mut page, "wheel", 10.0, 10.0, -20.0);
            assert_eq!(page.next_action_window_deadline(), Some(deadline));
            assert_eq!(page.vm_mut().eval("events.join('|')").unwrap(), "");
            page.flush_page_action_window(moli_action_window::ActionBarrier::Explicit)
                .unwrap();
            assert_eq!(
                page.vm_mut().eval("events.join('|')").unwrap(),
                "wheel:30|wheel:-20|microtask:30|microtask:-20"
            );
            assert!(page.next_action_window_deadline().is_none());
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}

#[test]
fn realtime_input_clicks_new_transparent_controls_and_uses_their_current_position() {
    run_page_vm_large_stack_async_test("realtime-input-checkbox", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-checkbox").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval("document.body.innerHTML='<div style=height:100px></div>'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval(r#"
            window.toggle=document.createElement('input');toggle.type='checkbox';
            toggle.style.cssText='position:fixed;left:24px;top:24px;width:24px;height:24px;margin:0;opacity:0';
            document.body.append(toggle);window.clicks=0;
            toggle.addEventListener('click',()=>{clicks++;toggle.style.left='124px'});
        "#).unwrap();
        for (x, expected) in [(30.0, "true:1"), (30.0, "true:1"), (130.0, "false:2")] {
            mouse(&mut page, "mousedown", x, 30.0, 0.0);
            mouse(&mut page, "mouseup", x, 30.0, 0.0);
            assert_eq!(
                page.vm_mut()
                    .eval("String(toggle.checked)+':'+clicks")
                    .unwrap(),
                expected
            );
        }
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
