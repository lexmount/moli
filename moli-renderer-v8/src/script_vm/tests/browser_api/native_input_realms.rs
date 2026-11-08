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
                            false,
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

#[test]
fn native_mouse_release_followups_use_target_document_intrinsics() {
    for poison in ["ordinary", "deleted", "replaced", "getter"] {
        for (button, buttons, followup, click_count) in [
            (0, 1, "click", 1),
            (0, 1, "click", 2),
            (1, 4, "auxclick", 1),
            (2, 2, "contextmenu", 1),
        ] {
            let mut vm = new_storage_html_test_vm("https://native-input-realm.test/");
            vm.eval(r#"
              document.body.style.margin = '0';
              const frame = document.createElement('iframe');
              frame.style.cssText = 'position:fixed;left:0;top:0;width:240px;height:220px;border:0';
              document.body.appendChild(frame);
              const owner = frame.contentWindow;
              const Mouse = owner.MouseEvent;
              const Pointer = owner.PointerEvent;
              const node = owner.document.createElement('div');
              node.style.cssText = 'position:fixed;left:50px;top:50px;width:100px;height:100px';
              owner.document.body.appendChild(node);
              globalThis.releaseRows = [];
              globalThis.releaseConstructorReads = 0;
              for (const type of ['mousedown', 'mouseup', 'click', 'dblclick', 'auxclick', 'contextmenu']) {
                node.addEventListener(type, event => releaseRows.push([
                  type, [Mouse.prototype, Pointer.prototype].includes(Object.getPrototypeOf(event)),
                  event instanceof Mouse, event.view === owner, event.isTrusted,
                  event.target === node, event.currentTarget === node
                ]));
              }
              globalThis.releaseOwner = owner;
            "#).unwrap();
            vm.eval(&format!(r#"
              if ('{poison}' === 'deleted') delete releaseOwner.MouseEvent;
              if ('{poison}' === 'replaced') releaseOwner.MouseEvent = function () {{ releaseConstructorReads++; throw Error('public constructor'); }};
              if ('{poison}' === 'getter') Object.defineProperty(releaseOwner, 'MouseEvent', {{configurable: true, get() {{ releaseConstructorReads++; throw Error('public getter'); }}}});
            "#)).unwrap();
            vm.publish_layout_for_test().unwrap();
            for (event, held) in [("mousedown", buttons), ("mouseup", 0)] {
                vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                    80.0,
                    80.0,
                    event,
                    button,
                    Some(held),
                    click_count,
                    0.0,
                    0.0,
                    crate::runtime::RendererPointerEventProperties::default(),
                    0,
                    false,
                )
                .unwrap();
            }
            let mut expected_types = vec!["mousedown", "mouseup", followup];
            if click_count == 2 {
                expected_types.push("dblclick");
            }
            let expected = serde_json::Value::Array(
                expected_types
                    .into_iter()
                    .map(|name| serde_json::json!([name, true, true, true, true, true, true]))
                    .collect(),
            );
            let actual: serde_json::Value =
                serde_json::from_str(&vm.eval("JSON.stringify(releaseRows)").unwrap()).unwrap();
            assert_eq!(
                actual, expected,
                "{poison}, button {button}, count {click_count}"
            );
            assert_eq!(vm.eval("releaseConstructorReads").unwrap(), "0");
        }
    }
}
