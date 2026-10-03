use super::*;

fn new_frame_drag_test_vm(origin: &str, mode: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_rendered_test_vm(
        "https://frame-drag.test/",
        r#"<html><body style="margin:0;touch-action:none"><div id="outside" style="position:absolute;left:40px;top:40px;width:120px;height:120px;touch-action:none"></div><iframe id="child" style="position:absolute;left:220px;top:40px;width:160px;height:160px;border:0"></iframe></body></html>"#,
    );
    vm.eval(
        r#"
        const childWindow = document.getElementById('child').contentWindow;
        childWindow.document.documentElement.style.cssText = 'margin:0;touch-action:none';
        childWindow.document.body.style.cssText = 'margin:0;touch-action:none';
        childWindow.document.body.innerHTML = '<div id="inside" style="position:absolute;left:0;top:0;width:50px;height:100px;touch-action:none"></div><div id="second" style="position:absolute;left:70px;top:0;width:60px;height:100px;touch-action:none"></div>';
        'ready'
        "#,
    )
    .unwrap();
    vm.eval(&format!(
        "{}; globalThis.__frameResults = __installFrameDragProbe('{origin}', '{mode}'); 'ready'",
        include_str!("frame_drag.js")
    ))
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    vm
}

fn dispatch_frame_input(
    vm: &mut StandaloneScriptVmHarness,
    pointer_type: &str,
    pointer_id: i32,
    phase: &str,
    input: (&str, f64, i32, i32),
) {
    let (event, x, button, buttons) = input;
    vm.eval(&format!("globalThis.__framePhase = '{phase}'; 'ready'"))
        .unwrap();
    vm.dispatch_mouse_event_at_point_with_pointer(
        x,
        80.0,
        event,
        button,
        Some(buttons),
        1,
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

fn assert_frame_probe(vm: &mut StandaloneScriptVmHarness) {
    assert_eq!(
        vm.eval("globalThis.__frameFinished = __frameResults.finish(); __frameFinished.complete")
            .unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__frameFinished)").unwrap()
    );
}

#[test]
fn held_mouse_and_pen_stay_in_the_initiating_child_frame() {
    for (pointer_type, pointer_id) in [("mouse", 17), ("pen", 5)] {
        for (origin, modes) in [
            (
                "child",
                &[
                    "plain",
                    "release",
                    "capture",
                    "chorded",
                    "cancelled",
                    "remove",
                    "cover",
                    "reposition",
                ][..],
            ),
            ("parent", &["release", "capture", "cancelled"][..]),
        ] {
            for mode in modes {
                let mut vm = new_frame_drag_test_vm(origin, mode);
                let start = if origin == "child" { 240.0 } else { 80.0 };
                dispatch_frame_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "hover",
                    ("mousemove", start, -1, 0),
                );
                dispatch_frame_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "down",
                    ("mousedown", start, 0, 1),
                );
                if *mode == "chorded" {
                    dispatch_frame_input(
                        &mut vm,
                        pointer_type,
                        pointer_id,
                        "chord-down",
                        ("mousedown", start, 2, 3),
                    );
                }
                let held_buttons = if *mode == "chorded" { 3 } else { 1 };
                dispatch_frame_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "within-move",
                    ("mousemove", 310.0, -1, held_buttons),
                );
                if ["remove", "cover", "reposition"].contains(mode) {
                    vm.eval("__frameResults.mutate(); 'ready'").unwrap();
                    vm.publish_layout_for_test().unwrap();
                }
                if *mode == "cover" {
                    dispatch_frame_input(
                        &mut vm,
                        pointer_type,
                        pointer_id,
                        "covered-within-move",
                        ("mousemove", 310.0, -1, 1),
                    );
                }
                dispatch_frame_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "outside-move",
                    ("mousemove", 80.0, -1, held_buttons),
                );
                if *mode == "chorded" {
                    dispatch_frame_input(
                        &mut vm,
                        pointer_type,
                        pointer_id,
                        "chord-release",
                        ("mouseup", 80.0, 0, 2),
                    );
                    dispatch_frame_input(
                        &mut vm,
                        pointer_type,
                        pointer_id,
                        "held-after-release",
                        ("mousemove", 100.0, -1, 2),
                    );
                }
                dispatch_frame_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "up",
                    ("mouseup", 80.0, if *mode == "chorded" { 2 } else { 0 }, 0),
                );
                dispatch_frame_input(
                    &mut vm,
                    pointer_type,
                    pointer_id,
                    "post-up-hover",
                    ("mousemove", 80.0, -1, 0),
                );
                assert_frame_probe(&mut vm);
            }
        }
    }
}

#[tokio::test]
async fn frame_drag_follows_the_replacement_document_after_navigation() {
    for (pointer_type, pointer_id) in [("mouse", 17), ("pen", 5)] {
        let mut vm = new_frame_drag_test_vm("child", "navigate");
        for (phase, input) in [
            ("down", ("mousedown", 240.0, 0, 1)),
            ("within-move", ("mousemove", 310.0, -1, 1)),
        ] {
            dispatch_frame_input(&mut vm, pointer_type, pointer_id, phase, input);
        }
        vm.eval("__frameResults.mutate(); 'navigation-started'")
            .unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "frame drag navigation").await;
        assert_eq!(vm.eval("__frameResults.state.navigated").unwrap(), "true");
        vm.publish_layout_for_test().unwrap();
        for (phase, input) in [
            ("outside-move", ("mousemove", 80.0, -1, 1)),
            ("up", ("mouseup", 80.0, 0, 0)),
            ("post-up-hover", ("mousemove", 80.0, -1, 0)),
        ] {
            dispatch_frame_input(&mut vm, pointer_type, pointer_id, phase, input);
        }
        assert_frame_probe(&mut vm);
    }
}

#[test]
fn a_neutral_move_releases_frame_drag_without_a_matching_mouseup() {
    let mut vm = new_frame_drag_test_vm("child", "plain");
    for (phase, input) in [
        ("down", ("mousedown", 240.0, 0, 1)),
        ("neutral", ("mousemove", 80.0, -1, 0)),
        ("new-down", ("mousedown", 80.0, 0, 1)),
    ] {
        dispatch_frame_input(&mut vm, "mouse", 17, phase, input);
    }
    assert_eq!(
        vm.eval("JSON.stringify(__frameResults.events.filter(e => ['neutral','new-down'].includes(e.phase) && ['pointermove','pointerdown'].includes(e.type)).map(e => [e.document,e.target,e.buttons]))").unwrap(),
        r#"[["parent","outside",0],["parent","outside",1]]"#
    );
}

#[test]
fn reattaching_an_iframe_does_not_restore_its_previous_drag_ownership() {
    let mut vm = new_frame_drag_test_vm("child", "remove");
    for (phase, input) in [
        ("down", ("mousedown", 240.0, 0, 1)),
        ("within-move", ("mousemove", 310.0, -1, 1)),
    ] {
        dispatch_frame_input(&mut vm, "mouse", 17, phase, input);
    }
    vm.eval(
        r#"
        const retainedFrame = document.getElementById('child');
        __frameResults.mutate();
        document.body.appendChild(retainedFrame);
        void retainedFrame.contentWindow;
        'reattached'
        "#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    for (phase, input) in [
        ("outside-move", ("mousemove", 80.0, -1, 1)),
        ("up", ("mouseup", 80.0, 0, 0)),
        ("post-up-hover", ("mousemove", 80.0, -1, 0)),
    ] {
        dispatch_frame_input(&mut vm, "mouse", 17, phase, input);
    }
    assert_frame_probe(&mut vm);
}
