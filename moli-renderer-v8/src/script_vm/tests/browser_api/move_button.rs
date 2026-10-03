use super::*;

fn dispatch_move_button_input(
    vm: &mut StandaloneScriptVmHarness,
    pointer_type: &str,
    pointer_id: i32,
    input: (&str, &str, f64, i32, i32),
) {
    let (phase, event, x, button, buttons) = input;
    let state = serde_json::json!({"name":phase, "button":button, "buttons":buttons});
    vm.eval(&format!(
        "globalThis.__movePhase={state}; __moveExpected['{phase}']=__movePhase; 'ready'"
    ))
    .unwrap();
    vm.dispatch_mouse_event_at_point_with_pointer(
        x,
        80.0,
        event,
        button,
        Some(buttons),
        i32::from(event != "mousemove"),
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties {
            pointer_id,
            pointer_type: pointer_type.to_owned(),
            pressure: if buttons == 0 { 0.0 } else { 0.5 },
            ..Default::default()
        },
    )
    .unwrap();
}

#[test]
fn native_mouse_and_pen_movement_and_boundaries_preserve_button_transitions() {
    for (pointer_type, pointer_id) in [("mouse", 17), ("pen", 5)] {
        for (button, pressed) in [(0, 1), (1, 4), (2, 2), (3, 8), (4, 16)] {
            for capture in [false, true] {
                for carries_held_button in [false, true] {
                    let mut vm = new_rendered_test_vm(
                        "https://move-button.test/",
                        "<html><body></body></html>",
                    );
                    vm.eval(&format!(
                        "{}; globalThis.__moveExpected={{}}; globalThis.__moveResults=__installMoveButtonProbe({capture}); 'ready'",
                        include_str!("move_button.js")
                    ))
                    .unwrap();
                    vm.publish_layout_for_test().unwrap();
                    let second = if button == 0 { 2 } else { 0 };
                    let chorded = pressed | if second == 0 { 1 } else { 2 };
                    let movement_button = if carries_held_button { button } else { -1 };
                    for input in [
                        ("hover", "mousemove", 80.0, -1, 0),
                        ("down", "mousedown", 80.0, button, pressed),
                        ("held", "mousemove", 100.0, movement_button, pressed),
                        ("chord-down", "mousedown", 100.0, second, chorded),
                        ("chord-held", "mousemove", 110.0, movement_button, chorded),
                        ("chord-up", "mouseup", 110.0, second, pressed),
                        ("cross", "mousemove", 260.0, movement_button, pressed),
                        ("up", "mouseup", 260.0, button, 0),
                        ("after", "mousemove", 80.0, -1, 0),
                    ] {
                        dispatch_move_button_input(&mut vm, pointer_type, pointer_id, input);
                    }
                    assert_eq!(
                        vm.eval("globalThis.__moveFinished=__moveResults.finish(); __moveFinished.complete")
                            .unwrap(),
                        "true",
                        "{pointer_type}, button {button}, capture {capture}, held field {carries_held_button}: {}",
                        vm.eval("JSON.stringify(__moveFinished)").unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn native_touch_cancel_and_capture_events_cannot_be_canceled() {
    let mut vm = new_rendered_test_vm(
        "https://move-cancel.test/",
        "<html><body style='margin:0'><div id='target' style='width:200px;height:200px;touch-action:none'></div></body></html>",
    );
    vm.eval(
        r#"
        globalThis.cancelRows=[];
        const target=document.getElementById('target');
        for (const type of ['gotpointercapture','pointercancel','lostpointercapture']) {
            target.addEventListener(type,event=>{
                event.preventDefault();
                cancelRows.push([event.type,event.cancelable,event.defaultPrevented,event.bubbles,event.composed,event.isTrusted]);
            });
        }
        'ready'
        "#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    for event in ["touchstart", "touchmove", "touchcancel"] {
        vm.dispatch_touch_event_at_point(80.0, 80.0, event, false)
            .unwrap();
    }
    let rows: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(cancelRows)").unwrap()).unwrap();
    assert_eq!(
        rows,
        serde_json::json!([
            ["gotpointercapture", false, false, true, true, true],
            ["pointercancel", false, false, true, true, true],
            ["lostpointercapture", false, false, true, true, true]
        ])
    );
}
