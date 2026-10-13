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

#[test]
fn realtime_input_clicks_transparent_checkbox_over_later_static_label() {
    run_page_vm_large_stack_async_test("realtime-input-todo-checkbox", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-todo-checkbox").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML=`<style>
                html,body { margin:0 }
                ul { margin:0; padding:0; list-style:none; width:200px }
                li { position:relative }
                .toggle { appearance:none; border:none; position:absolute;
                    top:0; bottom:0; width:40px; height:40px; margin:auto 0; opacity:0 }
                label { display:block; padding:15px 15px 15px 60px; line-height:24px }
                .completed label { text-decoration:line-through }
            </style><ul id=todos></ul>`;
        "#,
            )
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval(r#"
            todos.innerHTML='<li id=row><div><input id=toggle class=toggle type=checkbox><label>A</label></div></li>';
            window.events=[];
            for(const name of ['click','input','change'])
                toggle.addEventListener(name,e=>events.push(e.type+':'+e.target.id+':'+e.target.checked));
            toggle.addEventListener('change',()=>row.classList.toggle('completed',toggle.checked));
        "#).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval(
                    r#"
                const rect=toggle.getBoundingClientRect();
                [rect.width,rect.height,document.elementFromPoint(20,20).id].join(':')
            "#
                )
                .unwrap(),
            "40:40:toggle"
        );
        for checked in [true, false] {
            mouse(&mut page, "mousedown", 20.0, 20.0, 0.0);
            mouse(&mut page, "mouseup", 20.0, 20.0, 0.0);
            assert_eq!(
                page.vm_mut()
                    .eval("String(toggle.checked)+':'+row.classList.contains('completed')")
                    .unwrap(),
                format!("{checked}:{checked}")
            );
        }
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "click:toggle:true|input:toggle:true|change:toggle:true|click:toggle:false|input:toggle:false|change:toggle:false"
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
fn realtime_synchronous_hits_refresh_scroll_transform_and_clip_without_callbacks() {
    run_page_vm_large_stack_async_test("realtime-current-hit-without-callbacks", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-current-hit").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut().eval(r#"
            document.documentElement.style.margin='0';document.body.style.margin='0';
            document.body.innerHTML=`
              <div style="position:absolute;left:0;top:0;width:400px;height:400px;background:white"></div>
              <div id=clipper style="position:absolute;left:20px;top:20px;width:100px;height:100px;overflow:hidden;padding:0;border:0">
                <div style="position:relative;width:100px;height:300px">
                  <div id=target style="position:absolute;left:10px;top:140px;width:40px;height:30px;background:red"></div>
                </div>
              </div>`;
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("String(document.elementFromPoint(40,30)===target)")
                .unwrap(),
            "false"
        );
        let before = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut()
            .eval(
                r#"
            window.lifecycle=[];
            window.ro=new ResizeObserver(()=>lifecycle.push('ro'));ro.observe(clipper);
            window.io=new IntersectionObserver(()=>lifecycle.push('io'));io.observe(target);
            requestAnimationFrame(()=>lifecycle.push('raf'));
        "#,
            )
            .unwrap();
        let pending = page
            .current_realtime_rendering_opportunity()
            .expect("pending lifecycle work");
        // Each dirty read must enter through hit testing itself. No separate
        // size getter, screenshot or rendering opportunity repairs the tree.
        assert_eq!(page.vm_mut().eval(r#"
            (()=>{
                const hit=(x,y)=>document.elementFromPoint(x,y)===target;
                clipper.scrollTop=140;
                const scrolled=hit(40,30);
                target.style.transform='translateX(50px)';
                const oldPoint=hit(40,30), transformed=hit(90,30);
                clipper.style.width='50px';
                const clipped=hit(90,30), repeated=hit(90,30);
                return JSON.stringify([scrolled,oldPoint,transformed,clipped,repeated,lifecycle.slice()]);
            })()
        "#).unwrap(), "[true,false,true,false,false,[]]");
        assert_eq!(page.vm().layout_pass_observability_for_test().1, before + 3);
        assert_eq!(page.current_realtime_rendering_opportunity(), Some(pending));
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(lifecycle)").unwrap(),
            "[]"
        );
        assert_eq!(
            page.vm_mut()
                .eval("String(io.takeRecords().length)")
                .unwrap(),
            "0",
            "synchronous hits must not sample intersections"
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver
            )
            .is_none(),
            "IO notification waits for rendering-phase sampling"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, before + 3);
        let metrics = page.vm().layout_pass_observability_for_test().3.unwrap();
        assert_eq!(metrics.paint_event_count, 0);
        assert_eq!(metrics.paint_operation_count, 0);
    });
}
