use super::*;
use crate::runtime::RendererTouchPoint;

const FRAME_MUTATIONS: [&str; 2] = [
    "frame.style.display='none'",
    "frame.style.left='300px';frame.style.top='320px'",
];

#[test]
fn pointerdown_frame_mutation_preserves_compat_mouse_coordinates() {
    for scale in [1.0, 0.5] {
        for mutation in FRAME_MUTATIONS {
            for cancel_focus in [false, true] {
                let mut vm = frame_target_vm(scale);
                vm.eval(&format!(
                    r#"
                    const source=child.getElementById('source');
                    source.onpointerdown=e=>{{
                        source.setPointerCapture(e.pointerId);
                        {mutation};
                    }};
                    if ({cancel_focus}) source.onmousedown=e=>e.preventDefault();
                "#
                ))
                .unwrap();
                let passes = vm.layout_pass_observability_for_test().1;
                let root = (100.0 + 20.0 * scale, 120.0 + 30.0 * scale);
                assert!(
                    vm.dispatch_mouse_event_at_point(
                        root.0,
                        root.1,
                        "mousedown",
                        0,
                        Some(1),
                        0.0,
                        0.0
                    )
                    .expect("pointerdown mutation must not abort compatibility dispatch")
                    .handled
                );
                // Focus scrolling is an independent default action allowed to
                // initialize layout. Cancel only that action when measuring
                // coordinate-dispatch layout work; pointerdown stays uncanceled
                // and compatibility mousedown must still be delivered.
                if cancel_focus {
                    assert_eq!(vm.layout_pass_observability_for_test().1, passes);
                }
                let records = frame_records(&mut vm);
                for event_type in ["pointerdown", "mousedown"] {
                    let event = records.iter().find(|e| e["type"] == event_type).unwrap();
                    assert_eq!(event["receiver"], "child");
                    assert_frame_coordinates(event, (20.0, 30.0), root);
                }
                assert_eq!(
                    vm.eval("String(source.hasPointerCapture(1))").unwrap(),
                    "true"
                );
            }
        }
    }
}

#[test]
fn pointerup_frame_mutation_releases_capture_and_finishes_click_without_layout() {
    for scale in [1.0, 0.5] {
        for mutation in FRAME_MUTATIONS {
            let mut vm = frame_target_vm(scale);
            vm.eval("const source=child.getElementById('source');source.ondragstart=e=>e.preventDefault();source.onpointerdown=e=>source.setPointerCapture(e.pointerId)").unwrap();
            let root = (100.0 + 20.0 * scale, 120.0 + 30.0 * scale);
            vm.dispatch_mouse_event_at_point(root.0, root.1, "mousedown", 0, Some(1), 0.0, 0.0)
                .unwrap();
            vm.dispatch_mouse_event_at_point(root.0, root.1, "mousemove", -1, Some(1), 0.0, 0.0)
                .unwrap();
            vm.eval(&format!(
                "records=[];source.onpointerup=()=>{{{mutation};}}"
            ))
            .unwrap();
            let passes = vm.layout_pass_observability_for_test().1;
            assert!(
                vm.dispatch_mouse_event_at_point(root.0, root.1, "mouseup", 0, Some(0), 0.0, 0.0)
                    .expect("pointerup mutation must not bypass capture release")
                    .handled
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, passes);
            let records = frame_records(&mut vm);
            assert_eq!(
                records
                    .iter()
                    .map(|e| e["type"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["pointerup", "mouseup", "lostpointercapture", "click"]
            );
            for event in &records {
                assert_eq!(event["receiver"], "child");
                assert_frame_coordinates(event, (20.0, 30.0), root);
            }
            assert_eq!(
                vm.eval("String(source.hasPointerCapture(1))").unwrap(),
                "false"
            );
            let host = vm._context_host.borrow();
            assert!(!host.pointer_capture_is_active(1));
            assert!(host.active_pointer_capture_target(1).is_none());
        }
    }
}

#[test]
fn touchend_frame_mutation_finishes_every_contact_without_layout() {
    for ids in [vec![0], vec![3, 4]] {
        for mutation in FRAME_MUTATIONS {
            let mut vm = frame_target_vm(0.5);
            let points = ids
                .iter()
                .enumerate()
                .map(|(i, id)| RendererTouchPoint {
                    id: *id,
                    x: 110.0 + i as f64 * 40.0,
                    y: 135.0,
                })
                .collect::<Vec<_>>();
            vm.dispatch_touch_event_at_points(&points, "touchstart", false)
                .unwrap();
            vm.dispatch_touch_event_at_points(&points, "touchmove", false)
                .unwrap();
            vm.eval(&format!(
                "records=[];child.addEventListener('pointerup',()=>{{{mutation};}},{{once:true}})"
            ))
            .unwrap();
            let passes = vm.layout_pass_observability_for_test().1;
            assert!(
                vm.dispatch_touch_event_at_points(&points, "touchend", ids == [0])
                    .expect("first pointerup mutation must not abort remaining contacts")
                    .handled
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, passes);
            let records = frame_records(&mut vm);
            for (index, point) in points.iter().enumerate() {
                let pointer_id = point.id + 2;
                for event_type in ["pointerup", "lostpointercapture"] {
                    let event = records
                        .iter()
                        .find(|e| e["type"] == event_type && e["pointerId"] == pointer_id)
                        .unwrap();
                    assert_eq!(event["receiver"], "child");
                    assert_frame_coordinates(
                        event,
                        (20.0 + index as f64 * 80.0, 30.0),
                        (point.x, point.y),
                    );
                }
                let host = vm._context_host.borrow();
                assert!(!host.pointer_capture_is_active(pointer_id));
                assert!(host.active_pointer_capture_target(pointer_id).is_none());
            }
            let end = records.iter().find(|e| e["type"] == "touchend").unwrap();
            assert_eq!(end["touches"], serde_json::json!([]));
            let changed = end["changed"].as_array().unwrap();
            assert_eq!(changed.len(), points.len());
            for (index, (touch, point)) in changed.iter().zip(&points).enumerate() {
                assert_frame_coordinates(
                    touch,
                    (20.0 + index as f64 * 80.0, 30.0),
                    (point.x, point.y),
                );
            }
            assert!(vm.active_touch_points.is_empty());
            assert!(vm.active_touch_pointer_handles.is_empty());
            assert!(vm.active_touch_event_handle.is_none());
            assert!(vm.active_touch_pointer_handle.is_none());
            assert!(vm.active_touch_point.is_none());
            if ids == [0] {
                let click = records.iter().find(|e| e["type"] == "click").unwrap();
                assert_frame_coordinates(click, (20.0, 30.0), (110.0, 135.0));
            }
        }
    }
}

#[test]
fn raw_update_capture_transfer_preserves_hidden_sibling_coordinates_and_cleanup() {
    let mut vm = frame_target_vm(0.5);
    vm.eval(r#"
        const second=document.createElement('iframe');
        second.style.cssText='position:absolute;left:400px;top:50px;width:200px;height:200px;border:0;transform-origin:0 0;transform:scale(.5)';
        document.body.append(second);
        const other=second.contentDocument;
        other.body.innerHTML='<div id="other-target">target</div>';
        const otherTarget=other.getElementById('other-target');
        for(const type of ['gotpointercapture','pointermove','mousemove','pointerup','mouseup','lostpointercapture','click']) {
            other.addEventListener(type,e=>records.push({type,receiver:'other',phase,target:e.target.id,...point(e)}),true);
        }
        const source=child.getElementById('source');
        source.ondragstart=e=>e.preventDefault();
        source.onpointerdown=e=>source.setPointerCapture(e.pointerId);
        source.addEventListener('pointerrawupdate',e=>{
            otherTarget.setPointerCapture(e.pointerId);
            second.style.display='none';
        },{once:true});
    "#).unwrap();
    publish_layout_for_test(&mut vm);
    vm.dispatch_mouse_event_at_point(110.0, 135.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .unwrap();
    vm.eval("records=[];phase='move'").unwrap();
    let passes = vm.layout_pass_observability_for_test().1;
    vm.dispatch_mouse_event_at_point(110.0, 135.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .expect("hidden new capture receiver must use its sampled transform");
    assert_eq!(vm.layout_pass_observability_for_test().1, passes);
    let records = frame_records(&mut vm);
    for (event_type, receiver, client) in [
        ("pointerrawupdate", "child", (20.0, 30.0)),
        ("lostpointercapture", "child", (20.0, 30.0)),
        ("gotpointercapture", "other", (-580.0, 170.0)),
        ("pointermove", "other", (-580.0, 170.0)),
        ("mousemove", "other", (-580.0, 170.0)),
    ] {
        let event = records
            .iter()
            .find(|e| e["type"] == event_type && e["receiver"] == receiver)
            .unwrap();
        assert_frame_coordinates(event, client, (110.0, 135.0));
    }

    // The next command has a new published snapshot without the hidden frame.
    // Its coordinate fallback must still allow the captured pointer to end.
    publish_layout_for_test(&mut vm);
    vm.eval("records=[];phase='up'").unwrap();
    let passes = vm.layout_pass_observability_for_test().1;
    vm.dispatch_mouse_event_at_point(110.0, 135.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("a receiver absent from the input snapshot must not prevent cleanup");
    assert_eq!(vm.layout_pass_observability_for_test().1, passes);
    let records = frame_records(&mut vm);
    for event_type in ["pointerup", "mouseup", "lostpointercapture", "click"] {
        let event = records.iter().find(|e| e["type"] == event_type).unwrap();
        assert_eq!(event["receiver"], "other");
        assert_frame_coordinates(event, (110.0, 135.0), (110.0, 135.0));
    }
    assert_eq!(
        vm.eval("String(otherTarget.hasPointerCapture(1))").unwrap(),
        "false"
    );
    assert!(!vm._context_host.borrow().pointer_capture_is_active(1));
}
