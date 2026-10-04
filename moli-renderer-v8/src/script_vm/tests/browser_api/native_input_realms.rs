use super::*;

#[test]
fn native_mouse_boundary_and_wheel_events_use_target_document_intrinsics() {
    for context in ["root", "child", "nested", "shadow-open", "shadow-closed"] {
        for poison in ["ordinary", "deleted", "replaced", "getter"] {
            for pointer_type in ["mouse", "pen"] {
                for modifiers in [0, 15] {
                    let mut vm = new_storage_html_test_vm("https://native-input-realm.test/");
                    vm.eval(&format!(
                        "{}; globalThis.__mouseRealm=__installMouseWheelRealmProbe('{context}','{poison}','{pointer_type}',{modifiers}); 'ready'",
                        include_str!("native_input_realms.js")
                    ))
                    .unwrap();
                    vm.publish_layout_for_test().unwrap();
                    for (phase, x, event) in [
                        ("reset", 540.0, "mousemove"),
                        ("enter", 80.0, "mousemove"),
                        ("same", 185.0, "mousemove"),
                        ("peer-wheel", 185.0, "wheel"),
                        ("cross", 340.0, "mousemove"),
                        ("return", 80.0, "mousemove"),
                        ("source-wheel", 80.0, "wheel"),
                    ] {
                        vm.eval(&format!("__mouseRealm.prepare('{phase}'); 'ready'"))
                            .unwrap();
                        let wheel = event == "wheel";
                        vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                            x,
                            80.0,
                            event,
                            -1,
                            Some(0),
                            0,
                            if wheel { 4.0 } else { 0.0 },
                            if wheel { 7.0 } else { 0.0 },
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
                        vm.eval("globalThis.__mouseRealmFinished=__mouseRealm.finish(); __mouseRealmFinished.complete")
                            .unwrap(),
                        "true",
                        "{context}, {poison}, {pointer_type}, modifiers {modifiers}: {}",
                        vm.eval("JSON.stringify(__mouseRealmFinished.checks.filter(c=>!c.pass))")
                            .unwrap()
                    );
                }
            }
        }
    }
}
