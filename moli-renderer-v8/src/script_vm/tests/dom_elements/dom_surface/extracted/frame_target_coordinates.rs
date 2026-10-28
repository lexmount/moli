use super::*;

fn frame_target_vm(scale: f64) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(
        "https://frame-target-coordinates.test/",
        "<!doctype html><body style='margin:0'><div id=outside style='position:fixed;inset:0'></div><iframe id=frame style='position:absolute;left:100px;top:120px;width:200px;height:200px;margin:0;border:0;padding:0;transform-origin:0 0'></iframe>",
    );
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 800,
        screen_height: 600,
        screen_avail_width: 800,
        screen_avail_height: 600,
        ..Default::default()
    }))
    .unwrap();
    vm.eval(&format!(r#"
        frame.style.transform = 'scale({scale})';
        globalThis.child = frame.contentDocument;
        child.documentElement.style.cssText = 'margin:0;padding:0';
        child.body.style.cssText = 'margin:0;padding:0';
        child.body.innerHTML = '<div id=source style="position:absolute;left:0;top:0;width:80px;height:80px"></div><div id=capture style="position:absolute;left:80px;top:0;width:80px;height:80px"></div>';
        globalThis.records = []; globalThis.phase = 'down';
        const point = p => ({{clientX:p.clientX,clientY:p.clientY,screenX:p.screenX,screenY:p.screenY}});
        for (const [doc, receiver] of [[document,'root'],[child,'child']]) {{
          for (const type of ['pointerdown','gotpointercapture','pointerrawupdate','pointermove','pointerup','lostpointercapture','mousedown','mousemove','mouseup','click','dragstart','dragenter','dragover','drop','touchstart','touchmove','touchend']) {{
            doc.addEventListener(type, event => {{
              records.push({{type,receiver,phase,target:event.target.id,...point(event),
                changed:Array.from(event.changedTouches || [], point),
                touches:Array.from(event.touches || [], point)}});
              if (type === 'dragstart') {{
                event.dataTransfer.setData('text/plain','payload');
                event.dataTransfer.effectAllowed = 'copy';
              }}
              if (['dragenter','dragover','drop'].includes(type)) event.preventDefault();
            }}, true);
          }}
        }}
    "#)).unwrap();
    publish_layout_for_test(&mut vm);
    vm
}

fn frame_records(vm: &mut StandaloneScriptVmHarness) -> Vec<serde_json::Value> {
    serde_json::from_str(&vm.eval("JSON.stringify(records)").unwrap()).unwrap()
}

fn assert_frame_coordinates(point: &serde_json::Value, client: (f64, f64), root: (f64, f64)) {
    for (field, expected) in [
        ("clientX", client.0),
        ("clientY", client.1),
        ("screenX", root.0),
        ("screenY", root.1),
    ] {
        assert!(
            (point[field].as_f64().unwrap() - expected).abs() < 0.001,
            "{field}={expected}: {point}"
        );
    }
}

#[test]
fn iframe_capture_coordinates_follow_receiver_inside_and_outside_the_frame() {
    for scale in [1.0, 0.5] {
        for target in ["source", "capture"] {
            let mut vm = frame_target_vm(scale);
            vm.eval(&format!("child.getElementById('source').onpointerdown = e => child.getElementById('{target}').setPointerCapture(e.pointerId)")).unwrap();
            let passes = vm.layout_pass_observability_for_test().1;
            let inside = (100.0 + 20.0 * scale, 120.0 + 30.0 * scale);
            vm.dispatch_mouse_event_at_point(inside.0, inside.1, "mousedown", 0, Some(1), 0.0, 0.0)
                .unwrap();
            for (phase, root, event, button, buttons) in [
                ("inside", inside, "mousemove", -1, 1),
                ("outside", (40.0, 50.0), "mousemove", -1, 1),
                ("up", (40.0, 50.0), "mouseup", 0, 0),
            ] {
                vm.eval(&format!("phase='{phase}'")).unwrap();
                vm.dispatch_mouse_event_at_point(
                    root.0,
                    root.1,
                    event,
                    button,
                    Some(buttons),
                    0.0,
                    0.0,
                )
                .unwrap();
            }
            let records = frame_records(&mut vm);
            for (phase, root, types) in [
                (
                    "inside",
                    inside,
                    &[
                        "gotpointercapture",
                        "pointerrawupdate",
                        "pointermove",
                        "mousemove",
                    ][..],
                ),
                (
                    "outside",
                    (40.0, 50.0),
                    &["pointerrawupdate", "pointermove", "mousemove"][..],
                ),
                (
                    "up",
                    (40.0, 50.0),
                    &["pointerup", "mouseup", "lostpointercapture", "click"][..],
                ),
            ] {
                for event_type in types {
                    let event = records
                        .iter()
                        .find(|e| e["type"] == *event_type && e["phase"] == phase)
                        .unwrap_or_else(|| {
                            panic!("{scale}/{target}: missing {phase}/{event_type}: {records:?}")
                        });
                    assert_eq!(event["receiver"], "child");
                    assert_eq!(event["target"], target);
                    assert_frame_coordinates(
                        event,
                        ((root.0 - 100.0) / scale, (root.1 - 120.0) / scale),
                        root,
                    );
                }
            }
            assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        }
    }
}

#[test]
fn nested_iframe_capture_uses_the_published_frame_chain() {
    let mut vm = frame_target_vm(0.5);
    vm.eval(r#"
        child.body.innerHTML = '<iframe id="inner" style="position:absolute;left:30px;top:40px;width:100px;height:100px;border:0;padding:0;transform-origin:0 0;transform:scale(.5)"></iframe>';
        globalThis.nested = child.getElementById('inner').contentDocument;
        nested.body.style.cssText = 'margin:0';
        nested.body.innerHTML = '<div id="target" style="width:80px;height:80px"></div>';
        const target = nested.getElementById('target');
        target.onpointerdown = e => target.setPointerCapture(e.pointerId);
        for (const type of ['gotpointercapture','pointermove','lostpointercapture']) {
            target.addEventListener(type, e => records.push({type,phase,
                clientX:e.clientX,clientY:e.clientY,screenX:e.screenX,screenY:e.screenY}));
        }
        globalThis.movement = document.createElement('style');
        document.head.append(movement);
        records.length = 0;
    "#).unwrap();
    publish_layout_for_test(&mut vm);
    let passes = vm.layout_pass_observability_for_test().1;
    vm.dispatch_mouse_event_at_point(120.0, 147.5, "mousedown", 0, Some(1), 0.0, 0.0)
        .unwrap();
    // Mutating iframe attributes explicitly retires its published viewport.
    // Use CSSOM so a retained snapshot exists until the explicit publication.
    vm.eval("phase='move';movement.sheet.insertRule('#frame {left:300px !important;top:320px !important}',0)")
        .unwrap();
    vm.dispatch_mouse_event_at_point(40.0, 50.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .unwrap();
    let records = frame_records(&mut vm);
    for event_type in ["gotpointercapture", "pointermove"] {
        let event = records.iter().find(|e| e["type"] == event_type).unwrap();
        assert_frame_coordinates(event, (-300.0, -360.0), (40.0, 50.0));
    }
    assert_eq!(vm.layout_pass_observability_for_test().1, passes);

    publish_layout_for_test(&mut vm);
    vm.eval("phase='up'").unwrap();
    vm.dispatch_mouse_event_at_point(40.0, 50.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .unwrap();
    let records = frame_records(&mut vm);
    let event = records
        .iter()
        .find(|e| e["type"] == "lostpointercapture")
        .unwrap();
    assert_frame_coordinates(event, (-1100.0, -1160.0), (40.0, 50.0));
}

#[test]
fn iframe_capture_transfer_during_raw_update_converts_each_receiver() {
    let mut vm = frame_target_vm(1.0);
    vm.eval(r#"
        const source=child.getElementById('source');
        source.onpointerdown=e=>source.setPointerCapture(e.pointerId);
        source.addEventListener('pointerrawupdate',e=>outside.setPointerCapture(e.pointerId),{once:true});
    "#).unwrap();
    vm.dispatch_mouse_event_at_point(120.0, 150.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .unwrap();
    vm.eval("phase='move'").unwrap();
    vm.dispatch_mouse_event_at_point(120.0, 150.0, "mousemove", -1, Some(1), 0.0, 0.0)
        .unwrap();
    let records = frame_records(&mut vm);
    for (event_type, receiver, client) in [
        ("pointerrawupdate", "child", (20.0, 30.0)),
        ("lostpointercapture", "child", (20.0, 30.0)),
        ("gotpointercapture", "root", (120.0, 150.0)),
        ("pointermove", "root", (120.0, 150.0)),
        ("mousemove", "root", (120.0, 150.0)),
    ] {
        let event = records
            .iter()
            .find(|e| e["type"] == event_type && e["receiver"] == receiver && e["phase"] == "move")
            .unwrap_or_else(|| panic!("missing {event_type}/{receiver}: {records:?}"));
        assert_frame_coordinates(event, client, (120.0, 150.0));
    }
}

#[test]
fn cross_frame_drag_start_uses_source_frame_coordinates() {
    for source_is_child in [false, true] {
        let mut vm = frame_target_vm(0.5);
        vm.eval(if source_is_child {
            "child.getElementById('source').draggable=true"
        } else {
            "outside.draggable=true"
        })
        .unwrap();
        let (start, end) = if source_is_child {
            ((110.0, 135.0), (20.0, 30.0))
        } else {
            ((20.0, 30.0), (110.0, 135.0))
        };
        vm.dispatch_mouse_event_at_point(start.0, start.1, "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        vm.dispatch_mouse_event_at_point(end.0, end.1, "mousemove", -1, Some(1), 0.0, 0.0)
            .unwrap();
        vm.dispatch_mouse_event_at_point(end.0, end.1, "mouseup", 0, Some(0), 0.0, 0.0)
            .unwrap();
        let records = frame_records(&mut vm);
        for event_type in ["dragstart", "dragover", "drop"] {
            let event = records
                .iter()
                .find(|e| e["type"] == event_type)
                .unwrap_or_else(|| panic!("missing {event_type}: {records:?}"));
            let in_child = if event_type == "dragstart" {
                source_is_child
            } else {
                !source_is_child
            };
            assert_eq!(event["receiver"], if in_child { "child" } else { "root" });
            // main starts native dragging at the press position; target drag
            // events use the subsequent move/release position.
            let root = if event_type == "dragstart" {
                start
            } else {
                end
            };
            let client = if in_child {
                ((root.0 - 100.0) / 0.5, (root.1 - 120.0) / 0.5)
            } else {
                root
            };
            assert_frame_coordinates(event, client, root);
        }
    }
}

#[test]
fn iframe_touch_points_and_capture_keep_their_target_frame_coordinates() {
    use crate::runtime::RendererTouchPoint;
    for ids in [vec![0], vec![3, 4]] {
        let mut vm = frame_target_vm(0.5);
        let start = ids
            .iter()
            .enumerate()
            .map(|(i, id)| RendererTouchPoint {
                id: *id,
                x: 110.0 + i as f64 * 40.0,
                y: 135.0,
            })
            .collect::<Vec<_>>();
        vm.dispatch_touch_event_at_points(&start, "touchstart", false)
            .unwrap();
        let end = ids
            .iter()
            .enumerate()
            .map(|(i, id)| RendererTouchPoint {
                id: *id,
                x: 40.0 + i as f64 * 20.0,
                y: 50.0,
            })
            .collect::<Vec<_>>();
        vm.dispatch_touch_event_at_points(&end, "touchmove", false)
            .unwrap();
        vm.dispatch_touch_event_at_points(&end, "touchend", ids == [0])
            .unwrap();
        let records = frame_records(&mut vm);
        for (event_type, roots) in [
            ("touchstart", &start),
            ("touchmove", &end),
            ("touchend", &end),
        ] {
            let event = records.iter().find(|e| e["type"] == event_type).unwrap();
            assert_eq!(event["receiver"], "child");
            let changed = event["changed"].as_array().unwrap();
            assert_eq!(changed.len(), ids.len());
            for (touch, root) in changed.iter().zip(roots) {
                assert_frame_coordinates(
                    touch,
                    ((root.x - 100.0) / 0.5, (root.y - 120.0) / 0.5),
                    (root.x, root.y),
                );
            }
            let touches = event["touches"].as_array().unwrap();
            assert_eq!(
                touches.len(),
                if event_type == "touchend" {
                    0
                } else {
                    ids.len()
                }
            );
            for (touch, root) in touches.iter().zip(roots) {
                assert_frame_coordinates(
                    touch,
                    ((root.x - 100.0) / 0.5, (root.y - 120.0) / 0.5),
                    (root.x, root.y),
                );
            }
        }
        for event in records.iter().filter(|e| {
            matches!(
                e["type"].as_str(),
                Some(
                    "gotpointercapture"
                        | "pointermove"
                        | "pointerup"
                        | "lostpointercapture"
                        | "click"
                )
            )
        }) {
            assert_eq!(event["receiver"], "child");
            let root = (
                event["screenX"].as_f64().unwrap(),
                event["screenY"].as_f64().unwrap(),
            );
            assert_frame_coordinates(
                event,
                ((root.0 - 100.0) / 0.5, (root.1 - 120.0) / 0.5),
                root,
            );
        }
    }
}

#[test]
fn external_drag_uses_the_hit_target_frame_coordinates() {
    for scale in [1.0, 0.5] {
        let mut vm = frame_target_vm(scale);
        let root = (100.0 + 20.0 * scale, 120.0 + 30.0 * scale);
        for event_type in ["dragenter", "dragover", "drop"] {
            vm.dispatch_drag_event_at_point(
                root.0,
                root.1,
                event_type,
                crate::runtime::RendererDragData {
                    items: Vec::new(),
                    files: Vec::new(),
                    directories: Vec::new(),
                    drag_operations_mask: 1,
                },
                0,
            )
            .unwrap();
        }
        let records = frame_records(&mut vm);
        for event_type in ["dragenter", "dragover", "drop"] {
            let event = records.iter().find(|e| e["type"] == event_type).unwrap();
            assert_eq!(event["receiver"], "child");
            assert_frame_coordinates(event, (20.0, 30.0), root);
        }
    }
}
