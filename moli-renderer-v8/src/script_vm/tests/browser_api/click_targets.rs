use super::*;

fn assert_native_click_targets(in_child: bool) {
    for pointer_type in ["mouse", "pen"] {
        for (button, mask) in [(0, 1), (1, 4), (2, 2)] {
            for mode in [
                "same",
                "siblings",
                "nested-common",
                "down-child-up-parent",
                "down-parent-up-child",
                "root-common",
                "capture-source",
                "capture-sibling",
                "capture-parent",
                "release-before-up",
                "release-on-up",
                "switch-on-up",
                "cancel-pointerdown",
                "cancel-mousedown",
                "mouseup-reparent-up",
                "mouseup-reparent-down",
                "mouseup-reparent-both",
                "same-up-reparent",
                "poison-parent-getters",
                "common-label",
            ] {
                let mut vm = new_storage_html_test_vm("https://click-target.test/audit.html");
                let points = vm
                    .eval(&format!(
                        "{}; globalThis.__clickResult=__installClickTargetProbe('{mode}',{button},{in_child}); JSON.stringify({{down:__clickResult.down,up:__clickResult.up}})",
                        include_str!("click_targets.js")
                    ))
                    .unwrap();
                let points: serde_json::Value = serde_json::from_str(&points).unwrap();
                vm.publish_layout_for_test().unwrap();
                for (event, position, buttons) in [
                    ("mousemove", &points["down"], 0),
                    ("mousedown", &points["down"], mask),
                    ("mousemove", &points["up"], mask),
                    ("mouseup", &points["up"], 0),
                ] {
                    if event == "mouseup" {
                        vm.eval("__clickResult.prepareUp(); 'ready'").unwrap();
                    }
                    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                        position[0].as_f64().unwrap(),
                        position[1].as_f64().unwrap(),
                        event,
                        if event == "mousemove" { -1 } else { button },
                        Some(buttons),
                        i32::from(event != "mousemove"),
                        0.0,
                        0.0,
                        crate::runtime::RendererPointerEventProperties {
                            pointer_id: 17,
                            pointer_type: pointer_type.to_owned(),
                            pressure: if buttons == 0 { 0.0 } else { 0.5 },
                            ..Default::default()
                        },
                        0,
                    )
                    .unwrap();
                }
                assert_eq!(
                    vm.eval(&format!(
                        "globalThis.__clickFinished=__clickResult.finish('{pointer_type}'); __clickFinished.complete"
                    ))
                    .unwrap(),
                    "true",
                    "{mode}, {pointer_type}, button {button}, child {in_child}: {}",
                    vm.eval("JSON.stringify(__clickFinished.checks.filter(c=>!c.pass))")
                        .unwrap()
                );
            }
        }
    }
}

#[test]
fn native_click_targets_use_release_capture_or_current_common_ancestor() {
    assert_native_click_targets(false);
}

#[test]
fn native_click_targets_use_the_child_document_tree_and_realm() {
    assert_native_click_targets(true);
}

#[test]
fn native_auxclick_resolves_its_target_after_contextmenu_tree_mutation() {
    for in_child in [false, true] {
        for pointer_type in ["mouse", "pen"] {
            for captured in [false, true] {
                let mode = if captured {
                    "capture-sibling"
                } else {
                    "siblings"
                };
                let mut vm = new_storage_html_test_vm("https://aux-click-target.test/audit.html");
                let points = vm.eval(&format!(
                    "{}; globalThis.__clickResult=__installClickTargetProbe('{mode}',2,{in_child}); JSON.stringify({{down:__clickResult.down,up:__clickResult.up}})",
                    include_str!("click_targets.js")
                )).unwrap();
                vm.eval(&format!(
                    "globalThis.__clickDoc={}; __clickDoc.defaultView.addEventListener('contextmenu',()=>__clickDoc.getElementById('other').appendChild(__clickDoc.getElementById('destination')),{{once:true}}); 'ready'",
                    if in_child { "document.querySelector('iframe').contentDocument" } else { "document" }
                )).unwrap();
                let points: serde_json::Value = serde_json::from_str(&points).unwrap();
                vm.publish_layout_for_test().unwrap();
                for (event, position, buttons) in [
                    ("mousemove", &points["down"], 0),
                    ("mousedown", &points["down"], 2),
                    ("mousemove", &points["up"], 2),
                    ("mouseup", &points["up"], 0),
                ] {
                    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                        position[0].as_f64().unwrap(),
                        position[1].as_f64().unwrap(),
                        event,
                        if event == "mousemove" { -1 } else { 2 },
                        Some(buttons),
                        i32::from(event != "mousemove"),
                        0.0,
                        0.0,
                        crate::runtime::RendererPointerEventProperties {
                            pointer_id: 17,
                            pointer_type: pointer_type.to_owned(),
                            pressure: if buttons == 0 { 0.0 } else { 0.5 },
                            ..Default::default()
                        },
                        0,
                    )
                    .unwrap();
                }
                let actual = vm.eval(&format!(
                    "JSON.stringify(__clickResult.finish('{pointer_type}').rows.filter(e=>e.type==='contextmenu'||e.type==='auxclick').map(e=>[e.type,e.target,e.button,e.buttons,e.trusted,e.realm,e.ownerRealm]))"
                )).unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
                    serde_json::json!([
                        ["contextmenu", "destination", 2, 0, true, true, true],
                        [
                            "auxclick",
                            if captured { "destination" } else { "body" },
                            2,
                            0,
                            true,
                            true,
                            true
                        ],
                    ]),
                    "child {in_child}, {pointer_type}, captured {captured}"
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ChordedInputEncoding {
    Explicit,
    InferButtons,
    InferButton,
}

fn assert_native_chorded_click_targets(context: &str) {
    for pointer_type in ["mouse", "pen"] {
        for pair in [[0, 1], [1, 0], [0, 2], [2, 0], [1, 2], [2, 1]] {
            for reverse in [false, true] {
                for mode in ["same", "split", "capture-peer"] {
                    for encoding in [
                        ChordedInputEncoding::Explicit,
                        ChordedInputEncoding::InferButtons,
                        ChordedInputEncoding::InferButton,
                    ] {
                        let mut vm =
                            new_storage_html_test_vm("https://chorded-click.test/audit.html");
                        let steps = vm.eval(&format!(
                            "{}; globalThis.__chord=__installChordedClickProbe('{context}','{pointer_type}',[{},{}],{reverse},'{mode}','getter',0); JSON.stringify(__chord.steps)",
                            include_str!("chorded_click.js"), pair[0], pair[1],
                        )).unwrap();
                        let steps: Vec<serde_json::Value> = serde_json::from_str(&steps).unwrap();
                        vm.publish_layout_for_test().unwrap();
                        for step in steps {
                            vm.eval(&format!("__chord.prepare({}); 'ready'", step["phase"]))
                                .unwrap();
                            let event = step["event"].as_str().unwrap();
                            let button = if matches!(encoding, ChordedInputEncoding::InferButton) {
                                -1
                            } else {
                                step["button"].as_i64().unwrap() as i32
                            };
                            let buttons = step["buttons"].as_i64().unwrap() as i32;
                            vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                                step["x"].as_f64().unwrap(),
                                80.0,
                                event,
                                button,
                                (!matches!(encoding, ChordedInputEncoding::InferButtons))
                                    .then_some(buttons),
                                i32::from(event != "mousemove"),
                                0.0,
                                0.0,
                                crate::runtime::RendererPointerEventProperties {
                                    pointer_id: 17,
                                    pointer_type: pointer_type.to_owned(),
                                    pressure: if buttons == 0 { 0.0 } else { 0.5 },
                                    ..Default::default()
                                },
                                0,
                            )
                            .unwrap();
                        }
                        assert_eq!(
                            vm.eval("globalThis.__chordFinished=__chord.finish(); __chordFinished.complete")
                                .unwrap(),
                            "true",
                            "{context}, {pointer_type}, pair {pair:?}, reverse {reverse}, {mode}, {encoding:?}: {}",
                            vm.eval("JSON.stringify(__chordFinished.checks.filter(c=>!c.pass))")
                                .unwrap()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn native_chorded_clicks_pair_each_button_press_in_the_root_document() {
    assert_native_chorded_click_targets("root");
}

#[test]
fn native_chorded_clicks_pair_each_button_press_in_a_child_document() {
    assert_native_chorded_click_targets("child");
}

#[test]
fn native_chorded_clicks_pair_each_button_press_in_a_nested_document() {
    assert_native_chorded_click_targets("nested");
}

#[test]
fn native_chorded_clicks_pair_each_button_press_in_an_open_shadow_tree() {
    assert_native_chorded_click_targets("shadow-open");
}

#[test]
fn native_chorded_clicks_pair_each_button_press_in_a_closed_shadow_tree() {
    assert_native_chorded_click_targets("shadow-closed");
}

#[test]
fn native_chorded_clicks_pair_each_button_press_in_a_slotted_tree() {
    assert_native_chorded_click_targets("slotted");
}

#[test]
fn native_clicks_do_not_reuse_presses_after_a_move_reports_no_held_buttons() {
    let mut vm = new_storage_html_test_vm("https://chorded-reset.test/audit.html");
    vm.eval(
        r#"
        document.body.innerHTML='<div style="position:fixed;left:0;top:0;width:200px;height:200px"></div>';
        globalThis.activations=[];
        for(const type of ['click','auxclick']) window.addEventListener(type,event=>activations.push(event.type));
        'ready'
        "#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    for (event, button, buttons) in [
        ("mousedown", 0, 1),
        ("mousedown", 1, 5),
        ("mousemove", -1, 0),
        ("mouseup", 1, 0),
        ("mouseup", 0, 0),
    ] {
        vm.dispatch_mouse_event_at_point(80.0, 80.0, event, button, Some(buttons), 0.0, 0.0)
            .unwrap();
    }
    assert_eq!(vm.eval("JSON.stringify(activations)").unwrap(), "[]");
}
