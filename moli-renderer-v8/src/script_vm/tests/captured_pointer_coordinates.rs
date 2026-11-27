use super::*;

async fn captured_child_vm(nested: bool, scaled: bool) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_html_test_vm("https://captured-pointer-coordinates.test/");
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Lazy);
    vm.eval(r#"
        document.body.style.margin='0';
        document.body.innerHTML='<div id=outside style="position:absolute;left:0;top:0;width:50px;height:50px"></div>';
        globalThis.frame=document.createElement('iframe');
        frame.style.cssText='position:absolute;left:100px;top:60px;width:200px;height:120px;border:0';
        frame.srcdoc='<style>html,body{margin:0}</style><div id=captured style="position:absolute;left:20px;top:20px;width:40px;height:40px"></div>';
        document.body.append(frame);'ready'
    "#).unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "capture outer frame setup").await;
    if nested {
        vm.eval(r#"
            globalThis.inner=frame.contentDocument.createElement('iframe');
            inner.style.cssText='position:absolute;left:10px;top:10px;width:100px;height:80px;border:0';
            inner.srcdoc='<style>html,body{margin:0}</style><div id=captured style="position:absolute;left:20px;top:20px;width:40px;height:40px"></div>';
            frame.contentDocument.body.append(inner);'ready'
        "#).unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "capture inner frame setup")
            .await;
    }
    if scaled {
        vm.eval("frame.style.transform='scale(2)';frame.style.transformOrigin='0 0';'scaled'")
            .unwrap();
    }
    vm.eval(&format!(r#"
        globalThis.captureDocument={}.contentDocument;
        globalThis.captured=captureDocument.getElementById('captured');
        globalThis.events=[];globalThis.transitions=[];
        captured.addEventListener('pointerdown',event=>captured.setPointerCapture(event.pointerId));
        for(const type of ['gotpointercapture','lostpointercapture'])
            captured.addEventListener(type,event=>transitions.push([type,event.target.id]));
        for(const type of ['pointerdown','pointermove','pointerup'])
            captureDocument.addEventListener(type,event=>events.push([type,event.target.id,event.clientX,event.clientY]));
        outside.addEventListener('pointermove',event=>events.push([event.type,event.target.id,event.clientX,event.clientY]));
        'installed'
    "#,if nested { "inner" } else { "frame" })).unwrap();
    vm.publish_layout_for_test().unwrap();
    vm
}

fn dispatch(vm: &mut StandaloneScriptVmHarness, kind: &str, x: f64, y: f64, buttons: i32) {
    let result = vm
        .dispatch_mouse_event_at_point(
            x,
            y,
            kind,
            if kind == "mousemove" { -1 } else { 0 },
            Some(buttons),
            0.0,
            0.0,
        )
        .unwrap();
    assert!(result.handled);
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_scroll_reprojects_nested_frame_pointer_coordinates() {
    let mut vm = captured_child_vm(true, false).await;
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Realtime);
    vm.eval("document.body.style.height='2000px';frame.contentDocument.body.style.height='2000px'")
        .unwrap();
    // Establish :hover before taking the baseline so the next mousemove
    // keeps the same style inputs while exercising the reprojected frame.
    dispatch(&mut vm, "mousemove", 140.0, 100.0, 0);
    assert_eq!(
        vm.eval("events.length=0;frame.getBoundingClientRect().top")
            .unwrap(),
        "60"
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("scrollTo(0,40);frame.getBoundingClientRect().top")
            .unwrap(),
        "20"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    dispatch(&mut vm, "mousemove", 140.0, 60.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["pointermove","captured",30,30]]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);

    // Scroll only the outer iframe's document. The inner iframe moves in
    // its parent viewport while the top-level viewport stays at rest.
    assert_eq!(
        vm.eval("events.length=0;scrollTo(0,0);frame.getBoundingClientRect().top")
            .unwrap(),
        "60"
    );
    assert_eq!(
        vm.eval(
            "frame.contentWindow.scrollTo(0,15);\
             JSON.stringify([scrollY,frame.getBoundingClientRect().top,\
             frame.contentWindow.scrollY,inner.getBoundingClientRect().top])",
        )
        .unwrap(),
        "[0,60,15,-5]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    dispatch(&mut vm, "mousemove", 140.0, 85.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["pointermove","captured",30,30]]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
}

async fn assert_removed_pointer_target_preserves_frame_coordinates(
    pointer_type: &str,
    scaled: bool,
) {
    let mut vm = new_storage_html_test_vm("https://removed-pointer-coordinates.test/");
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Lazy);
    vm.eval(r#"
        document.body.style.margin='0';
        globalThis.frame=document.createElement('iframe');
        frame.style.cssText='position:absolute;left:100px;top:60px;width:200px;height:120px;border:0';
        frame.srcdoc='<style>html,body{margin:0}</style><div id=container style="position:absolute;left:20px;top:20px;width:80px;height:80px"><div id=target style="width:40px;height:40px"></div></div>';
        document.body.append(frame);'ready'
    "#).unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "removed pointer target setup")
        .await;
    if scaled {
        vm.eval("frame.style.transform='scale(2)';frame.style.transformOrigin='0 0';'scaled'")
            .unwrap();
    }
    let (mouse_type, buttons) = match pointer_type {
        "pointerdown" => ("mousedown", 1),
        "pointerup" => ("mouseup", 0),
        _ => unreachable!(),
    };
    vm.eval(&format!(
        r#"
        globalThis.target=frame.contentDocument.getElementById('target');
        globalThis.container=frame.contentDocument.getElementById('container');
        globalThis.events=[];globalThis.hadCapture=null;
        target.addEventListener('{pointer_type}',event=>{{
            events.push([event.type,event.target.id,event.clientX,event.clientY]);
            hadCapture=target.hasPointerCapture(event.pointerId);
            target.remove();
            frame.style.left='300px';frame.style.top='200px';
            frame.style.transform='scale(3)';
        }});
        container.addEventListener('{mouse_type}',event=>
            events.push([event.type,event.target.id,event.clientX,event.clientY]));
        'installed'
    "#
    ))
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    let (x, y) = if scaled {
        (160.0, 120.0)
    } else {
        (130.0, 90.0)
    };
    if pointer_type == "pointerup" {
        dispatch(&mut vm, "mousedown", x, y, 1);
    }
    let builds = vm.layout_pass_observability_for_test().1;
    dispatch(&mut vm, mouse_type, x, y, buttons);
    let events: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
    assert_eq!(
        events,
        serde_json::json!([
            [pointer_type, "target", 30, 30],
            [mouse_type, "container", 30, 30],
        ]),
        "pointer_type={pointer_type}, scaled={scaled}"
    );
    assert_eq!(vm.eval("String(hadCapture)").unwrap(), "false");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        builds,
        "retargeting must use geometry sampled before the callback moves and scales the frame"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn pointer_down_removal_preserves_compat_mouse_frame_coordinates() {
    assert_removed_pointer_target_preserves_frame_coordinates("pointerdown", false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn pointer_up_removal_preserves_compat_mouse_frame_coordinates() {
    assert_removed_pointer_target_preserves_frame_coordinates("pointerup", false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn scaled_frame_pointer_removal_preserves_compat_mouse_coordinates() {
    for pointer_type in ["pointerdown", "pointerup"] {
        assert_removed_pointer_target_preserves_frame_coordinates(pointer_type, true).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_child_pointer_uses_published_frame_coordinates_outside_its_bounds() {
    let mut vm = captured_child_vm(false, false).await;
    let builds = vm.layout_pass_observability_for_test().1;
    dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    vm.eval("frame.style.left='200px';frame.style.top='100px';'moved'")
        .unwrap();
    // Capture continues to use the published coordinates until a screenshot
    // publishes the moved frame.
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    vm.publish_layout_for_test().unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["pointerdown","captured",30,30],["pointermove","captured",-80,-40],["pointermove","captured",-80,-40],["pointermove","captured",-180,-80],["pointerup","captured",-180,-80],["pointermove","outside",20,20]]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(transitions)").unwrap(),
        r#"[["gotpointercapture","captured"],["lostpointercapture","captured"]]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}

#[tokio::test(flavor = "current_thread")]
async fn captured_pointer_composes_nested_and_transformed_frame_coordinates() {
    for (nested, scaled, down, expected) in [
        (true, false, (140.0, 100.0), (-90, -50)),
        (false, true, (160.0, 120.0), (-40, -20)),
        (true, true, (180.0, 140.0), (-50, -30)),
    ] {
        let mut vm = captured_child_vm(nested, scaled).await;
        let builds = vm.layout_pass_observability_for_test().1;
        dispatch(&mut vm, "mousedown", down.0, down.1, 1);
        dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
        dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
        let facts: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
        assert_eq!(
            facts,
            serde_json::json!([
                ["pointerdown", "captured", 30, 30],
                ["pointermove", "captured", expected.0, expected.1],
                ["pointerup", "captured", expected.0, expected.1],
            ]),
            "nested={nested}, scaled={scaled}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_pointer_remains_deliverable_without_an_invertible_frame_mapping() {
    for style in ["display:none", "transform:scale(0)"] {
        let mut vm = captured_child_vm(false, false).await;
        dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
        dispatch(&mut vm, "mousemove", 131.0, 91.0, 1);
        let (document, fallback_sample) = {
            let host = vm._context_host.borrow();
            let frame = host.child_browsing_context_handles_in_document_order()[0];
            let document = host.child_browsing_context_document_handle(frame).unwrap();
            let sample = host.published_frame_geometry().unwrap();
            assert!(sample.root_to_viewport(&host, document).is_some());
            (document, sample)
        };
        vm.eval(&format!("frame.style.cssText+=';{style}';'unprojectable'"))
            .unwrap();
        vm.publish_layout_for_test().unwrap();
        {
            let host = vm._context_host.borrow();
            assert!(fallback_sample.root_to_viewport(&host, document).is_some());
            assert!(
                host.input_frame_transform(document, Some(&fallback_sample))
                    .is_none()
            );
        }
        let builds = vm.layout_pass_observability_for_test().1;
        dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
        dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
        let facts: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
        for row in &facts.as_array().unwrap()[2..] {
            assert_eq!(row[1], "captured");
            assert!(row[2].as_f64().is_some_and(f64::is_finite));
            assert!(row[3].as_f64().is_some_and(f64::is_finite));
        }
        assert_eq!(facts.as_array().unwrap().len(), 4);
        assert_eq!(
            vm.eval("JSON.stringify(transitions)").unwrap(),
            r#"[["gotpointercapture","captured"],["lostpointercapture","captured"]]"#
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn raw_update_release_preserves_pressed_mouse_frame_route() {
    let mut vm = captured_child_vm(false, false).await;
    vm.eval(
        r#"
        globalThis.rawCoordinates=[];
        captured.addEventListener('pointerrawupdate',event=>{
            rawCoordinates.push([event.clientX,event.clientY]);
            captured.releasePointerCapture(event.pointerId);
            frame.style.left='300px';
        });
        captureDocument.addEventListener('mousemove',event=>
            events.push([event.type,event.target.id,event.clientX,event.clientY]));
        outside.addEventListener('pointerup',()=>events.push(['unexpected outer release']));
        'installed'
    "#,
    )
    .unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    assert_eq!(
        vm.eval("JSON.stringify(rawCoordinates)").unwrap(),
        "[[-80,-40]]"
    );
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["pointerdown","captured",30,30],["pointermove","",-80,-40],["mousemove","",-80,-40]]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    vm.publish_layout_for_test().unwrap();
    dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(events.slice(3))").unwrap(),
        r#"[["pointerup","",-280,-40],["pointermove","outside",20,20]]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}

#[tokio::test(flavor = "current_thread")]
async fn raw_update_capture_transfer_requires_active_document() {
    let mut vm = captured_child_vm(false, false).await;
    vm.eval(r#"
        globalThis.second=document.createElement('iframe');
        second.style.cssText='position:absolute;left:400px;top:200px;width:200px;height:120px;border:0';
        second.srcdoc='<style>html,body{margin:0}</style><div id=next style="width:40px;height:40px"></div>';
        document.body.append(second);'ready'
    "#).unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "capture transfer target setup")
        .await;
    vm.eval(r#"
        globalThis.next=second.contentDocument.getElementById('next');
        globalThis.replacement=captureDocument.createElement('div');
        replacement.id='replacement';
        captureDocument.body.append(replacement);
        globalThis.captureAttempts=[];
        captured.addEventListener('pointerrawupdate',event=>{
            next.setPointerCapture(event.pointerId);
            replacement.setPointerCapture(event.pointerId);
            captureAttempts.push([
                next.hasPointerCapture(event.pointerId),replacement.hasPointerCapture(event.pointerId)
            ]);
        });
        next.addEventListener('pointerdown',event=>{
            next.setPointerCapture(event.pointerId);
            captureAttempts.push([next.hasPointerCapture(event.pointerId)]);
        });
        replacement.addEventListener('gotpointercapture',event=>
            transitions.push([event.type,event.target.id]));
        'installed'
    "#).unwrap();
    vm.publish_layout_for_test().unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["pointerdown","captured",30,30],["pointermove","replacement",-80,-40]]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(transitions)").unwrap(),
        r#"[["gotpointercapture","captured"],["lostpointercapture","captured"],["gotpointercapture","replacement"]]"#
    );
    dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
    dispatch(&mut vm, "mousedown", 425.0, 220.0, 1);
    assert_eq!(
        vm.eval("JSON.stringify(captureAttempts)").unwrap(),
        "[[false,true],[true]]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
}

#[tokio::test(flavor = "current_thread")]
async fn capture_transitions_inherit_pointer_geometry_before_compatibility_mouse_geometry() {
    let mut vm = captured_child_vm(false, false).await;
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Realtime);
    vm.eval(
        r#"
        globalThis.captureSamples=[];
        for(const type of ['gotpointercapture','pointerup','mouseup','lostpointercapture'])
            captured.addEventListener(type,event=>{
                captureSamples.push([type,event.clientX,event.clientY,event.screenX,event.screenY,
                    event.button,event.buttons,event.timeStamp]);
                if(type==='gotpointercapture') {
                    frame.style.left='200px';
                    frame.getBoundingClientRect();
                }
            });
        'installed'
    "#,
    )
    .unwrap();
    dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
    dispatch(&mut vm, "mouseup", 130.0, 90.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(captureSamples.map(row=>row.slice(0,7)))")
            .unwrap(),
        r#"[["gotpointercapture",30,30,130,90,0,0],["pointerup",30,30,130,90,0,0],["mouseup",-70,30,130,90,0,0],["lostpointercapture",30,30,130,90,0,0]]"#
    );
    assert_eq!(vm.eval("String(captureSamples[0][7]===captureSamples[1][7] && captureSamples[1][7]===captureSamples[3][7])").unwrap(), "true");
}

#[tokio::test(flavor = "current_thread")]
async fn touch_coordinates_share_the_frame_projection_and_release_restores_hit_routing() {
    let mut vm = captured_child_vm(false, true).await;
    vm.eval(r#"
        globalThis.touchSamples=[];
        captured.addEventListener('pointerdown',event=>captured.releasePointerCapture(event.pointerId));
        captured.addEventListener('touchstart',event=>{
            const point=event.changedTouches[0];
            touchSamples.push([point.clientX,point.clientY,point.screenX,point.screenY,point.pageX,point.pageY]);
        });
        outside.addEventListener('pointerup',event=>
            events.push([event.type,event.target.id,event.clientX,event.clientY]));
        'installed'
    "#).unwrap();
    vm.dispatch_touch_event_at_point(160.0, 120.0, "touchstart", false)
        .unwrap();
    vm.dispatch_touch_event_at_point(20.0, 20.0, "touchmove", false)
        .unwrap();
    vm.dispatch_touch_event_at_point(20.0, 20.0, "touchend", false)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(touchSamples)").unwrap(),
        "[[30,30,160,120,30,30]]"
    );
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["pointerdown","captured",30,30],["pointermove","outside",20,20],["pointerup","outside",20,20]]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn captured_touch_uses_the_last_published_frame_geometry() {
    for (touch, pointer) in [
        ("touchmove", "pointermove"),
        ("touchend", "pointerup"),
        ("touchcancel", "pointercancel"),
    ] {
        for publish in [false, true] {
            let mut vm = captured_child_vm(false, false).await;
            vm.eval(&format!(
            r#"
            const style = document.createElement('style');
            style.textContent = 'iframe[data-moved] {{ margin-left: 100px }}';
            document.head.append(style);
            globalThis.samples = [];
            for (const type of ['{touch}', '{pointer}']) {{
                captured.addEventListener(type, event => {{
                    const point = event.changedTouches?.[0] ?? event;
                    samples.push([event.type, point.clientX, point.clientY, point.screenX, point.screenY]);
                }});
            }}
            "#,
            ))
            .unwrap();
            vm.dispatch_touch_event_at_point(130.0, 90.0, "touchstart", false)
                .unwrap();
            let builds = vm.layout_pass_observability_for_test().1;
            vm.eval("frame.setAttribute('data-moved', '')").unwrap();
            if publish {
                vm.publish_layout_for_test().unwrap();
            }
            vm.dispatch_touch_event_at_point(140.0, 100.0, touch, false)
                .unwrap();

            let x = if publish { -60 } else { 40 };
            let samples = vm.eval("JSON.stringify(samples)").unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&samples).unwrap(),
                serde_json::json!([[pointer, x, 40, 140, 100], [touch, x, 40, 140, 100]]),
                "{touch}, publish={publish}"
            );
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                builds + u64::from(publish)
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn capture_projects_through_a_frame_box_without_a_child_layout_tree() {
    let mut vm = captured_child_vm(false, false).await;
    dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
    vm.eval("frame.style.cssText+=';width:0;height:0;border:5px solid transparent';'collapsed'")
        .unwrap();
    vm.publish_layout_for_test().unwrap();
    {
        let host = vm._context_host.borrow();
        let frame = host.child_browsing_context_handles_in_document_order()[0];
        let document = host.child_browsing_context_document_handle(frame).unwrap();
        assert!(
            host.with_latest_layout_tree_for_document(document, |_| ())
                .is_none()
        );
    }
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(events.slice(1))").unwrap(),
        r#"[["pointermove","captured",-85,-45],["pointerup","captured",-85,-45]]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn tap_mousemove_keeps_child_coordinates_after_mouseover_invalidates_layout() {
    let mut vm = captured_child_vm(false, false).await;
    vm.eval(
        r#"
        globalThis.tapSamples=[];
        captured.addEventListener('mouseover',event=>{
            tapSamples.push([event.type,event.clientX,event.clientY,event.screenX,event.screenY]);
            frame.style.left='200px';
        }, {once:true});
        captured.addEventListener('mousemove',event=>
            tapSamples.push([event.type,event.clientX,event.clientY,event.screenX,event.screenY]));
        'installed'
    "#,
    )
    .unwrap();
    vm.dispatch_touch_event_at_point(130.0, 90.0, "touchstart", false)
        .unwrap();
    vm.dispatch_touch_event_at_point(130.0, 90.0, "touchend", false)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(tapSamples)").unwrap(),
        r#"[["mouseover",30,30,130,90],["mousemove",30,30,130,90]]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn capture_transfers_in_a_live_child_document_without_published_geometry() {
    let mut vm = captured_child_vm(true, false).await;
    dispatch(&mut vm, "mousedown", 140.0, 100.0, 1);
    dispatch(&mut vm, "mousemove", 141.0, 101.0, 1);
    vm.eval(
        r#"
        globalThis.replacement=captureDocument.createElement('div');
        replacement.id='replacement';
        captureDocument.body.append(replacement);
        globalThis.captureTransfer=null;
        captured.addEventListener('pointermove',event=>{
            replacement.setPointerCapture(event.pointerId);
            captureTransfer=replacement.hasPointerCapture(event.pointerId);
        }, {once:true});
        replacement.addEventListener('gotpointercapture',event=>
            transitions.push([event.type,event.target.id]));
        frame.style.display='none';
        'hidden'
    "#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    assert_eq!(
        vm.eval("String(inner.contentDocument === captureDocument && replacement.isConnected)")
            .unwrap(),
        "true"
    );
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    assert_eq!(vm.eval("String(captureTransfer)").unwrap(), "true");
    dispatch(&mut vm, "mousemove", 21.0, 21.0, 1);
    assert_eq!(
        vm.eval("JSON.stringify(transitions)").unwrap(),
        r#"[["gotpointercapture","captured"],["lostpointercapture","captured"],["gotpointercapture","replacement"]]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn lost_touch_capture_checks_the_previous_document_before_cross_frame_move() {
    let mut vm = captured_child_vm(false, false).await;
    vm.eval(
        r#"
        globalThis.recapture=[];
        captured.addEventListener('pointermove',event=>captured.releasePointerCapture(event.pointerId),
            {once:true});
        captured.addEventListener('lostpointercapture',event=>{
            outside.setPointerCapture(event.pointerId);
            const outsideCaptured=outside.hasPointerCapture(event.pointerId);
            captured.setPointerCapture(event.pointerId);
            recapture.push([outsideCaptured,captured.hasPointerCapture(event.pointerId)]);
        }, {once:true});
        'installed'
    "#,
    )
    .unwrap();
    vm.dispatch_touch_event_at_point(130.0, 90.0, "touchstart", false)
        .unwrap();
    vm.dispatch_touch_event_at_point(131.0, 91.0, "touchmove", false)
        .unwrap();
    vm.dispatch_touch_event_at_point(20.0, 20.0, "touchmove", false)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(recapture)").unwrap(),
        "[[false,true]]"
    );
    assert_eq!(
        vm.eval("JSON.stringify(events.at(-1))").unwrap(),
        r#"["pointermove","outside",20,20]"#
    );
    vm.dispatch_touch_event_at_point(21.0, 21.0, "touchmove", false)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(transitions)").unwrap(),
        r#"[["gotpointercapture","captured"],["lostpointercapture","captured"],["gotpointercapture","captured"]]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn touch_cancel_cleans_up_capture_when_the_original_frame_retires() {
    let mut vm = captured_child_vm(false, false).await;
    vm.eval(
        r#"
        globalThis.captureLog = [];
        captured.addEventListener('pointermove', event =>
            captured.releasePointerCapture(event.pointerId));
        captured.addEventListener('pointercancel', () => captureLog.push('retired cancel'));
        outside.addEventListener('pointermove', event =>
            outside.setPointerCapture(event.pointerId), {once:true});
        outside.addEventListener('gotpointercapture', event => {
            captureLog.push('got');
            outside.releasePointerCapture(event.pointerId);
        });
        outside.addEventListener('lostpointercapture', () => captureLog.push('lost'));
        "#,
    )
    .unwrap();
    for (x, y, kind) in [
        (130.0, 90.0, "touchstart"),
        (131.0, 91.0, "touchmove"),
        (20.0, 20.0, "touchmove"),
        (21.0, 21.0, "touchmove"),
    ] {
        vm.dispatch_touch_event_at_point(x, y, kind, false).unwrap();
    }
    assert_eq!(vm.eval("JSON.stringify(captureLog)").unwrap(), r#"["got"]"#);
    let document = {
        let host = vm._context_host.borrow();
        let frame = host.child_browsing_context_handles_in_document_order()[0];
        host.child_browsing_context_document_handle(frame).unwrap()
    };
    vm.eval("frame.remove()").unwrap();
    {
        let host = vm._context_host.borrow();
        assert!(host.input_owner_for_document(document).is_none());
        assert!(host.pointer_is_known(2));
    }

    vm.dispatch_touch_event_at_point(21.0, 21.0, "touchcancel", false)
        .unwrap();

    assert_eq!(
        vm.eval("JSON.stringify(captureLog)").unwrap(),
        r#"["got","lost"]"#
    );
    assert!(!vm._context_host.borrow().pointer_is_known(2));
    assert!(vm.active_touch_points.is_empty());
    assert!(vm.active_touch_pointer_handles.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn frame_navigation_retires_the_capture_route_and_old_projection() {
    let mut vm = captured_child_vm(false, false).await;
    dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
    dispatch(&mut vm, "mousemove", 131.0, 91.0, 1);
    let (frame, old_document, old_sample) = {
        let host = vm._context_host.borrow();
        let frame = host.child_browsing_context_handles_in_document_order()[0];
        let document = host.child_browsing_context_document_handle(frame).unwrap();
        (frame, document, host.published_frame_geometry().unwrap())
    };
    vm.eval("frame.srcdoc='<body>replacement</body>'").unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "replace captured frame owner")
        .await;
    {
        let host = vm._context_host.borrow();
        let replacement = host.child_browsing_context_document_handle(frame).unwrap();
        assert_ne!(old_document, replacement);
        assert!(old_sample.root_to_viewport(&host, old_document).is_none());
        assert!(old_sample.root_to_viewport(&host, replacement).is_none());
    }
    vm.publish_layout_for_test().unwrap();
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(events.slice(2))").unwrap(),
        r#"[["pointermove","outside",20,20]]"#
    );
}
