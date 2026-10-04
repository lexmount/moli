use super::*;

fn assert_compatibility_mouse_targets(in_child: bool) {
    for pointer_type in ["mouse", "pen"] {
        for captured in [false, true] {
            for trigger in ["down", "move", "up"] {
                for mode in [
                    "keep",
                    "remove-target",
                    "remove-parent",
                    "shadow-child-open",
                    "shadow-child-closed",
                    "shadow-host-open",
                    "shadow-host-closed",
                    "nested-shadow-host",
                    "slotted-target",
                    "slotted-slot",
                    "slotted-slot-parent",
                    "reparent-same-document",
                    "remove-reinsert",
                    "adopt-detached-document",
                    "remove-and-throw",
                    "poison-native-relations",
                    "remove-document-element",
                ] {
                    let mut vm =
                        new_storage_html_test_vm("https://compat-mouse-target.test/audit.html");
                    vm.eval(&format!(
                        "{}; globalThis.__compatResult=__installCompatMouseTargetProbe('{mode}','{trigger}',{captured},{in_child}); 'ready'",
                        include_str!("compat_mouse_targets.js")
                    )).unwrap();
                    vm.publish_layout_for_test().unwrap();
                    for (event, phase, x, buttons) in [
                        ("mousemove", "hover", 80.0, 0),
                        ("mousedown", "down", 80.0, 1),
                        ("mousemove", "move", 85.0, 1),
                        ("mouseup", "up", 85.0, 0),
                    ] {
                        vm.eval(&format!("__compatResult.prepare('{phase}'); 'ready'"))
                            .unwrap();
                        vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                            x,
                            80.0,
                            event,
                            if event == "mousemove" { -1 } else { 0 },
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
                        .unwrap_or_else(|error| {
                            panic!("{mode}, {trigger}, {pointer_type}, capture {captured}, child {in_child}, phase {phase}: {error}")
                        });
                        // This case verifies the single mapped event. Later
                        // input would require a new render after tree removal.
                        if phase == trigger {
                            break;
                        }
                    }
                    assert_eq!(
                        vm.eval(&format!(
                            "globalThis.__compatFinished=__compatResult.finish('{pointer_type}'); __compatFinished.complete"
                        ))
                        .unwrap(),
                        "true",
                        "{mode}, {trigger}, {pointer_type}, capture {captured}, child {in_child}: {}",
                        vm.eval("JSON.stringify(__compatFinished.checks.filter(c=>!c.pass))")
                            .unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn compatibility_mouse_targets_follow_the_preceding_pointer_path_after_removal() {
    assert_compatibility_mouse_targets(false);
}

#[test]
fn compatibility_mouse_targets_preserve_child_document_identity_and_realm() {
    assert_compatibility_mouse_targets(true);
}
