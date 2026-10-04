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
