use super::*;

fn dispatch_drag_lifecycle_input(
    vm: &mut StandaloneScriptVmHarness,
    pointer_type: &str,
    pointer_id: i32,
    input: (&str, &str, f64, i32, i32, u8),
) {
    let (phase, event, x, button, buttons, modifiers) = input;
    vm.eval(&format!("globalThis.__dragPhase='{phase}'; 'ready'"))
        .unwrap();
    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
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
        modifiers,
    )
    .unwrap();
}

#[test]
fn native_drag_lifecycle_suppresses_input_and_releases_capture() {
    for (pointer_type, pointer_id) in [("mouse", 17), ("pen", 5)] {
        for capture in [false, true] {
            for mode in ["cancel", "drop", "reject", "escape"] {
                for modifiers in [0, 5, 8, 15] {
                    let mut vm = new_rendered_test_vm(
                        "https://drag-lifecycle.test/",
                        "<html><body></body></html>",
                    );
                    vm.eval(&format!(
                        "{}; globalThis.__dragResults=__installDragLifecycleProbe('{mode}',{capture},{modifiers}); 'ready'",
                        include_str!("drag_lifecycle.js")
                    ))
                    .unwrap();
                    vm.publish_layout_for_test().unwrap();
                    for input in [
                        ("hover", "mousemove", 80.0, -1, 0, modifiers),
                        ("down", "mousedown", 80.0, 0, 1, modifiers),
                        ("start", "mousemove", 100.0, 0, 1, modifiers),
                    ] {
                        dispatch_drag_lifecycle_input(&mut vm, pointer_type, pointer_id, input);
                    }
                    if mode == "escape" {
                        vm.eval("globalThis.__dragPhase='escape'; 'ready'").unwrap();
                        vm.dispatch_key_event("keydown", "Escape", "Escape", "", 0, false, false)
                            .unwrap();
                    } else {
                        for (phase, x) in [("a", 320.0), ("b", 520.0)] {
                            dispatch_drag_lifecycle_input(
                                &mut vm,
                                pointer_type,
                                pointer_id,
                                (phase, "mousemove", x, 0, 1, modifiers ^ 8),
                            );
                        }
                    }
                    dispatch_drag_lifecycle_input(
                        &mut vm,
                        pointer_type,
                        pointer_id,
                        (
                            "up",
                            "mouseup",
                            if mode == "escape" { 100.0 } else { 520.0 },
                            0,
                            0,
                            modifiers ^ 8,
                        ),
                    );
                    dispatch_drag_lifecycle_input(
                        &mut vm,
                        pointer_type,
                        pointer_id,
                        ("after", "mousemove", 540.0, -1, 0, 0),
                    );
                    assert_eq!(
                        vm.eval("globalThis.__dragFinished=__dragResults.finish(); __dragFinished.complete").unwrap(),
                        "true",
                        "{pointer_type}, {mode}, capture {capture}, modifiers {modifiers}: {}",
                        vm.eval("JSON.stringify(__dragFinished)").unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn native_drop_uses_the_drop_handlers_effect_and_suppresses_text_input() {
    let mut vm = new_rendered_test_vm("https://drag-effect.test/", "<html><body></body></html>");
    vm.eval(&format!(
        "{}; globalThis.__dragResults=__installDragLifecycleProbe('drop',false,0);\n\
        globalThis.endEffect=null; globalThis.keyEvents=[];\n\
        document.addEventListener('drop', e=>{{e.dataTransfer.dropEffect='link';}});\n\
        document.addEventListener('dragend', e=>{{endEffect=e.dataTransfer.dropEffect;}});\n\
        document.addEventListener('keydown', e=>{{keyEvents.push(e.key);}}); 'ready'",
        include_str!("drag_lifecycle.js")
    ))
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    for input in [
        ("hover", "mousemove", 80.0, -1, 0, 0),
        ("down", "mousedown", 80.0, 0, 1, 0),
        ("start", "mousemove", 100.0, 0, 1, 0),
    ] {
        dispatch_drag_lifecycle_input(&mut vm, "mouse", 17, input);
    }
    vm.eval("globalThis.edit=document.createElement('input'); edit.value='keep'; document.body.appendChild(edit); edit.focus(); 'ready'").unwrap();
    assert!(!vm.insert_text_into_active_control("ignored").unwrap());
    vm.dispatch_key_event("keydown", "x", "KeyX", "x", 0, false, true)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([edit.value,keyEvents])").unwrap(),
        "[\"keep\",[]]"
    );
    for input in [
        ("a", "mousemove", 320.0, 0, 1, 0),
        ("b", "mousemove", 520.0, 0, 1, 0),
        ("up", "mouseup", 520.0, 0, 0, 0),
    ] {
        dispatch_drag_lifecycle_input(&mut vm, "mouse", 17, input);
    }
    assert_eq!(vm.eval("endEffect").unwrap(), "link");
    vm.insert_text_into_active_control(" resumed").unwrap();
    assert_eq!(vm.eval("edit.value").unwrap(), "keep resumed");
}

#[test]
fn native_drag_cancels_the_source_when_first_movement_crosses_a_drop_zone() {
    let mut vm = new_rendered_test_vm("https://drag-crossing.test/", "<html><body></body></html>");
    vm.eval(&format!(
        "{}; globalThis.__dragResults=__installDragLifecycleProbe('drop',false,0);\n\
        globalThis.cancelTargets=[]; document.addEventListener('pointercancel', e=>{{\n\
          cancelTargets.push([e.target.id,e.clientX,e.clientY,e.pressure]);\n\
        }}); 'ready'",
        include_str!("drag_lifecycle.js")
    ))
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    for input in [
        ("hover", "mousemove", 80.0, -1, 0, 0),
        ("down", "mousedown", 80.0, 0, 1, 0),
        ("start", "mousemove", 320.0, 0, 1, 0),
    ] {
        dispatch_drag_lifecycle_input(&mut vm, "mouse", 17, input);
    }
    assert_eq!(
        vm.eval("JSON.stringify(cancelTargets)").unwrap(),
        "[[\"source\",320,80,0.5]]"
    );
    for input in [
        ("b", "mousemove", 520.0, 0, 1, 0),
        ("up", "mouseup", 520.0, 0, 0, 0),
    ] {
        dispatch_drag_lifecycle_input(&mut vm, "mouse", 17, input);
    }
    assert_eq!(vm.eval("JSON.stringify(__dragResults.finish().trace.filter(e=>e.type==='dragend').map(e=>[e.target,e.dropEffect]))").unwrap(), "[[\"source\",\"copy\"]]");
}
