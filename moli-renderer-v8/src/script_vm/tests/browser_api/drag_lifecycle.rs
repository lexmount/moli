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
fn native_drag_keeps_legacy_mouse_hover_separate_from_pointer_suppression() {
    for context in [
        "root",
        "child",
        "nested",
        "shadow-open",
        "shadow-closed",
        "slotted",
    ] {
        for (pointer_type, pointer_id) in [("mouse", 17), ("pen", 5)] {
            for capture in [false, true] {
                for mode in ["cancel", "drop", "escape"] {
                    for poison in ["ordinary", "getter"] {
                        let mut vm = new_rendered_test_vm(
                            "https://drag-hover.test/",
                            "<html><body></body></html>",
                        );
                        vm.eval(&format!("{};globalThis.__dragHover=__installDragHoverProbe('{context}','{pointer_type}',{capture},'{mode}','{poison}',15);'ready'", include_str!("drag_hover_state.js"))).unwrap();
                        vm.publish_layout_for_test().unwrap();
                        for (phase, event, x, button, buttons) in [
                            ("hover", "mousemove", 80.0, -1, 0),
                            ("down", "mousedown", 80.0, 0, 1),
                            ("start", "mousemove", 100.0, 0, 1),
                            ("drag", "mousemove", 520.0, 0, 1),
                        ] {
                            vm.eval(&format!("__dragHover.prepare('{phase}');'ready'"))
                                .unwrap();
                            dispatch_drag_lifecycle_input(
                                &mut vm,
                                pointer_type,
                                pointer_id,
                                (phase, event, x, button, buttons, 15),
                            );
                        }
                        if mode == "escape" {
                            vm.dispatch_key_event(
                                "keydown", "Escape", "Escape", "", 0, false, false,
                            )
                            .unwrap();
                        }
                        for (phase, event, x, button) in [
                            ("up", "mouseup", 520.0, 0),
                            ("after", "mousemove", 540.0, -1),
                        ] {
                            vm.eval(&format!("__dragHover.prepare('{phase}');'ready'"))
                                .unwrap();
                            dispatch_drag_lifecycle_input(
                                &mut vm,
                                pointer_type,
                                pointer_id,
                                (phase, event, x, button, 0, 15),
                            );
                        }
                        assert_eq!(vm.eval("globalThis.__hoverFinished=__dragHover.finish();__hoverFinished.complete").unwrap(), "true",
                            "{context}/{pointer_type}/{capture}/{mode}/{poison}: {}", vm.eval("JSON.stringify(__hoverFinished)").unwrap());
                    }
                }
            }
        }
    }
}

#[test]
fn native_hover_keeps_mouse_and_pen_pointer_targets_independent() {
    let mut vm = new_rendered_test_vm("https://hover-pointers.test/", "<html><body></body></html>");
    vm.eval("document.body.innerHTML='<div id=a style=\"position:fixed;left:40px;top:40px;width:140px;height:140px\"></div><div id=b style=\"position:fixed;left:300px;top:40px;width:140px;height:140px\"></div>'; globalThis.pointerRows=[]; globalThis.mouseRows=[]; for(const node of document.querySelectorAll('div')) { for(const type of ['pointerover','pointerenter','pointerout','pointerleave','mouseover','mouseenter','mouseout','mouseleave']) { node.addEventListener(type,e=>{ e.stopPropagation(); const row=[e.type,e.target.id,e.relatedTarget?.id||null]; if(e.type.startsWith('pointer')) pointerRows.push([e.pointerType,...row]); else mouseRows.push(row); }); }} 'ready'").unwrap();
    vm.publish_layout_for_test().unwrap();
    for (pointer_type, pointer_id, x) in [
        ("mouse", 17, 80.0),
        ("pen", 5, 320.0),
        ("mouse", 17, 80.0),
        ("pen", 5, 320.0),
    ] {
        dispatch_drag_lifecycle_input(
            &mut vm,
            pointer_type,
            pointer_id,
            ("hover", "mousemove", x, -1, 0, 0),
        );
    }
    assert_eq!(
        vm.eval("JSON.stringify(pointerRows)").unwrap(),
        "[[\"mouse\",\"pointerover\",\"a\",null],[\"mouse\",\"pointerenter\",\"a\",null],[\"pen\",\"pointerover\",\"b\",null],[\"pen\",\"pointerenter\",\"b\",null]]"
    );
    assert_eq!(
        vm.eval("JSON.stringify(mouseRows.slice(-4))").unwrap(),
        "[[\"mouseout\",\"a\",\"b\"],[\"mouseleave\",\"a\",\"b\"],[\"mouseover\",\"b\",\"a\"],[\"mouseenter\",\"b\",\"a\"]]"
    );
    vm.eval("pointerRows=[];mouseRows=[];'ready'").unwrap();
    dispatch_drag_lifecycle_input(
        &mut vm,
        "mouse",
        17,
        ("hover", "mousemove", 320.0, -1, 0, 0),
    );
    assert_eq!(
        vm.eval("JSON.stringify(pointerRows)").unwrap(),
        "[[\"mouse\",\"pointerout\",\"a\",\"b\"],[\"mouse\",\"pointerleave\",\"a\",\"b\"],[\"mouse\",\"pointerover\",\"b\",\"a\"],[\"mouse\",\"pointerenter\",\"b\",\"a\"]]"
    );
    assert_eq!(vm.eval("JSON.stringify(mouseRows)").unwrap(), "[]");
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

#[test]
fn cancel_drag_command_completes_dragend_microtasks_before_returning() {
    let mut vm = new_rendered_test_vm(
        "https://drag-cancel-command.test/",
        "<html><body></body></html>",
    );
    vm.eval(&format!(
        "{}; globalThis.__dragResults=__installDragLifecycleProbe('drop',false,0);\n\
        globalThis.cancelReady=false; document.addEventListener('dragend', () => {{\n\
          Promise.resolve().then(() => {{cancelReady=true;}});\n\
        }}); 'ready'",
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
    vm.clear_active_drag_data_transfer().unwrap();
    // Entering a realm is not a task checkpoint. Read the value directly so an
    // eval() boundary cannot accidentally run the pending microtask for us.
    vm.with_default_context_scope(|scope, _| {
        let key = v8::String::new(scope, "cancelReady").unwrap();
        let global = scope.get_current_context().global(scope);
        assert!(global.get(scope, key.into()).unwrap().boolean_value(scope));
        Ok(())
    })
    .unwrap();
}
