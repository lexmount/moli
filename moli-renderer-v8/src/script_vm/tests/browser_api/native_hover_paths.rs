use super::*;

#[test]
fn native_hover_boundaries_preserve_common_ancestors_and_target_realms() {
    for context in [
        "root",
        "child",
        "nested",
        "shadow-open",
        "shadow-closed",
        "slotted",
    ] {
        for poison in ["ordinary", "deleted", "replaced", "getter"] {
            for pointer_type in ["mouse", "pen"] {
                for modifiers in [0, 15] {
                    let mut vm = new_storage_html_test_vm("https://native-hover-path.test/");
                    vm.eval(&format!(
                        "{}; globalThis.__hover=__installHoverPathProbe('{context}','{poison}','{pointer_type}',{modifiers}); 'ready'",
                        include_str!("native_hover_paths.js")
                    ))
                    .unwrap();
                    vm.publish_layout_for_test().unwrap();
                    for (phase, x) in [
                        ("reset", 540.0),
                        ("deep", 140.0),
                        ("parent", 60.0),
                        ("child", 140.0),
                        ("peer", 360.0),
                        ("exit", 540.0),
                    ] {
                        vm.eval(&format!("__hover.prepare('{phase}'); 'ready'"))
                            .unwrap();
                        vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                            x,
                            110.0,
                            "mousemove",
                            -1,
                            Some(0),
                            0,
                            0.0,
                            0.0,
                            crate::runtime::RendererPointerEventProperties {
                                pointer_id: 17,
                                pointer_type: pointer_type.to_owned(),
                                pressure: 0.0,
                                ..Default::default()
                            },
                            modifiers,
                        )
                        .unwrap_or_else(|error| {
                            panic!("{context}, {poison}, {pointer_type}, modifiers {modifiers}, {phase}: {error}")
                        });
                    }
                    assert_eq!(
                        vm.eval(
                            "globalThis.__hoverFinished=__hover.finish(); __hoverFinished.complete"
                        )
                        .unwrap(),
                        "true",
                        "{context}, {poison}, {pointer_type}, modifiers {modifiers}: {}",
                        vm.eval("JSON.stringify(__hoverFinished.checks.filter(c=>!c.pass))")
                            .unwrap()
                    );
                }
            }
        }
    }
}
