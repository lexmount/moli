use super::*;

#[test]
fn mouse_release_over_inert_document_clears_capture_and_compatibility_state() {
    use moli_page_types::LayoutPolicy;

    for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
        for release_x in [55.0, -20.0] {
            let mut vm = new_parsed_test_vm(
                "https://inert-mouse-release.test/",
                r#"<!doctype html><button id="target" style="position:absolute;left:30px;top:30px;width:100px;height:100px">Press</button>"#,
            );
            vm.set_layout_policy(policy);
            vm.eval(
                r#"
                const target = document.getElementById('target');
                window.inputEvents = [];
                window.captureOnExit = null;
                window.lostCaptures = 0;
                target.addEventListener('lostpointercapture', () => lostCaptures++);
                for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup']) {
                    target.addEventListener(type, event => inputEvents.push(event.type));
                }
                target.addEventListener('pointerdown', event => {
                    window.pressedPointer = event.pointerId;
                    target.setPointerCapture(pressedPointer);
                    event.preventDefault();
                }, {once: true});
                target.addEventListener('pointerout', () => {
                    target.setPointerCapture(pressedPointer);
                    window.captureOnExit = target.hasPointerCapture(pressedPointer);
                }, {once: true});
                "#,
            )
            .unwrap();
            publish_layout_for_test(&mut vm);

            vm.dispatch_mouse_event_at_point(55.0, 55.0, "mousedown", 0, Some(1), 0.0, 0.0)
                .unwrap();
            vm.dispatch_mouse_event_at_point(55.0, 55.0, "mousemove", -1, Some(1), 0.0, 0.0)
                .unwrap();
            // Withdraw pending capture so the release reaches hit testing,
            // while the old active capture still needs its lost event.
            vm.eval("target.releasePointerCapture(pressedPointer); document.documentElement.inert = true")
                .unwrap();
            vm.dispatch_mouse_event_at_point(release_x, 55.0, "mouseup", 0, Some(0), 0.0, 0.0)
                .unwrap();
            assert_eq!(
                vm.eval(
                    r#"
                    document.documentElement.inert = false;
                    target.setPointerCapture(pressedPointer);
                    JSON.stringify([captureOnExit, target.hasPointerCapture(pressedPointer),
                                    lostCaptures, inputEvents.splice(0)])
                    "#,
                )
                .unwrap(),
                r#"[false,false,1,["pointerdown"]]"#,
                "{policy:?}, release x={release_x}",
            );

            vm.dispatch_mouse_event_at_point(55.0, 55.0, "mousedown", 0, Some(1), 0.0, 0.0)
                .unwrap();
            vm.dispatch_mouse_event_at_point(55.0, 55.0, "mouseup", 0, Some(0), 0.0, 0.0)
                .unwrap();
            assert_eq!(
                vm.eval("JSON.stringify(inputEvents)").unwrap(),
                r#"["pointerdown","mousedown","pointerup","mouseup"]"#,
                "{policy:?}, release x={release_x}",
            );
        }
    }
}

#[test]
fn mouse_input_in_an_inert_child_viewport_targets_its_document_root() {
    use moli_page_types::LayoutPolicy;

    for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
        let mut vm = new_storage_html_test_vm("https://inert-child-mouse-input.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            document.body.style.margin = '0';
            const frame = document.createElement('iframe');
            frame.style.cssText = 'position:absolute;left:65px;top:45px;width:320px;height:240px;border:0';
            document.body.appendChild(frame);
            const inner = frame.contentDocument;
            inner.body.innerHTML = '<button>child</button>';
            inner.documentElement.inert = true;
            window.inputEvents = [];
            for (const [doc, source] of [[document, 'parent'], [inner, 'child']]) {
                for (const type of ['mousedown', 'mouseup', 'click']) {
                    doc.addEventListener(type, event => inputEvents.push([
                        source, event.type, event.target.nodeName, event.clientX, event.clientY
                    ]));
                }
            }
            "#,
        )
        .unwrap();
        publish_layout_for_test(&mut vm);

        vm.dispatch_mouse_event_at_point(110.0, 105.0, "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        vm.dispatch_mouse_event_at_point(110.0, 105.0, "mouseup", 0, Some(0), 0.0, 0.0)
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(inputEvents)").unwrap(),
            r#"[["child","mousedown","HTML",45,60],["child","mouseup","HTML",45,60],["child","click","HTML",45,60]]"#,
            "{policy:?}",
        );

        vm.eval("inputEvents.length = 0; frame.inert = true")
            .unwrap();
        vm.dispatch_mouse_event_at_point(110.0, 105.0, "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        vm.dispatch_mouse_event_at_point(110.0, 105.0, "mouseup", 0, Some(0), 0.0, 0.0)
            .unwrap();
        assert_eq!(
            vm.eval("inputEvents.length === 3 && inputEvents.every(([source, , tag]) => source === 'parent' && tag !== 'IFRAME')")
                .unwrap(),
            "true",
            "{policy:?}",
        );
    }
}

#[test]
fn mouse_boundary_events_leave_and_reenter_after_an_inert_hit_miss() {
    let mut vm = new_rendered_test_vm(
        "https://inert-mouse-boundary.test/",
        r#"<!doctype html><style>body { margin: 0; min-height: 100vh; }</style>
        <button id="target" style="position:absolute;left:30px;top:30px;width:100px;height:100px"></button>"#,
    );
    vm.eval(
        r#"
        window.boundaries = [];
        const target = document.getElementById('target');
        for (const type of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter',
                            'pointerout', 'pointerleave', 'mouseout', 'mouseleave']) {
            target.addEventListener(type, event => boundaries.push(event.type));
        }
    "#,
    )
    .unwrap();

    vm.dispatch_mouse_event_at_point(55.0, 55.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .unwrap();
    let enter = r#"["pointerover","pointerenter","mouseover","mouseenter"]"#;
    assert_eq!(
        vm.eval("JSON.stringify(boundaries.splice(0))").unwrap(),
        enter
    );

    vm.eval("document.documentElement.inert = true").unwrap();
    vm.dispatch_mouse_event_at_point(300.0, 100.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(boundaries.splice(0))").unwrap(),
        r#"["pointerout","pointerleave","mouseout","mouseleave"]"#,
    );

    vm.eval("document.documentElement.inert = false").unwrap();
    vm.dispatch_mouse_event_at_point(55.0, 55.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .unwrap();
    assert_eq!(vm.eval("JSON.stringify(boundaries)").unwrap(), enter);
}

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
                        vm.dispatch_mouse_input(
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
                            MouseDispatchCompletion::FinishTurn,
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
                vm.dispatch_mouse_input(
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
                    MouseDispatchCompletion::FinishTurn,
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
