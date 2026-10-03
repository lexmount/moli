use super::*;

fn dispatch_drag_init_input(
    vm: &mut StandaloneScriptVmHarness,
    pointer_type: &str,
    pointer_id: i32,
    phase: &str,
    input: (&str, f64, i32, i32),
) {
    let (event, x, button, buttons) = input;
    vm.eval(&format!("globalThis.__dragPhase = '{phase}'; 'ready'"))
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
            ..crate::runtime::RendererPointerEventProperties::default()
        },
    )
    .unwrap();
}

#[test]
fn native_mouse_and_pen_resolve_drag_source_and_consume_canceled_attempts() {
    for (pointer_type, pointer_id) in [("mouse", 17), ("pen", 5)] {
        for mode in [
            "plain",
            "false",
            "auto",
            "invalid",
            "case-true",
            "ancestor",
            "inner-draggable",
            "inner-false",
            "anchor",
            "anchor-no-href",
            "image",
            "svg",
            "shadow",
            "native-getters",
            "toggle-on",
            "toggle-off",
            "reparent-inner",
            "reparent-source",
            "replace-target",
            "remove-source",
            "cancel-mousedown",
            "cancel-pointerdown",
            "secondary",
            "chorded",
            "no-motion",
            "jitter-3",
            "threshold-4",
            "subthreshold-then-cross",
            "repeat-cancel",
        ] {
            let mut vm =
                new_rendered_test_vm("https://drag-init.test/", "<html><body></body></html>");
            vm.eval(&format!(
                "{}; globalThis.__dragResults = __installDragInitProbe('{mode}'); 'ready'",
                include_str!("drag_init.js")
            ))
            .unwrap();
            vm.publish_layout_for_test().unwrap();
            let mut button = if mode == "secondary" { 2 } else { 0 };
            let mut buttons = if mode == "secondary" { 2 } else { 1 };
            dispatch_drag_init_input(
                &mut vm,
                pointer_type,
                pointer_id,
                "hover",
                ("mousemove", 80.0, -1, 0),
            );
            dispatch_drag_init_input(
                &mut vm,
                pointer_type,
                pointer_id,
                "down",
                ("mousedown", 80.0, button, buttons),
            );
            if mode == "chorded" {
                dispatch_drag_init_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "chord-down",
                    ("mousedown", 80.0, 2, 3),
                );
                button = 2;
                buttons = 3;
            }
            if [
                "reparent-inner",
                "reparent-source",
                "replace-target",
                "remove-source",
            ]
            .contains(&mode)
            {
                vm.publish_layout_for_test().unwrap();
            }
            let below_threshold = if cfg!(target_os = "macos") {
                82.0
            } else {
                83.0
            };
            if mode == "subthreshold-then-cross" {
                dispatch_drag_init_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "below",
                    ("mousemove", below_threshold, button, buttons),
                );
                dispatch_drag_init_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "cross",
                    ("mousemove", 84.0, button, buttons),
                );
            } else {
                let x = match mode {
                    "no-motion" => 80.0,
                    "jitter-3" => below_threshold,
                    "threshold-4" => 84.0,
                    _ => 100.0,
                };
                dispatch_drag_init_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "move1",
                    ("mousemove", x, button, buttons),
                );
            }
            if mode == "repeat-cancel" {
                dispatch_drag_init_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "move2",
                    ("mousemove", 110.0, button, buttons),
                );
                dispatch_drag_init_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "move3",
                    ("mousemove", 120.0, button, buttons),
                );
            }
            dispatch_drag_init_input(
                &mut vm,
                pointer_type,
                pointer_id,
                "up",
                ("mouseup", 120.0, button, i32::from(mode == "chorded")),
            );
            if mode == "chorded" {
                dispatch_drag_init_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "final-up",
                    ("mouseup", 120.0, 0, 0),
                );
            }
            assert_eq!(
                vm.eval(
                    "globalThis.__dragFinished = __dragResults.finish(); __dragFinished.complete"
                )
                .unwrap(),
                "true",
                "{pointer_type}/{mode}: {}",
                vm.eval("JSON.stringify(__dragFinished)").unwrap()
            );
        }
    }
}
