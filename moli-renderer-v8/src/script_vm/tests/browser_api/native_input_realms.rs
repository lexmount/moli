use super::*;

#[test]
fn input_hit_test_policy_can_bypass_css_or_inert_separately() {
    use crate::native_bridge::element::{
        InputHitTestMode, InputHitTestPolicy, input_surface_hit_test,
    };
    use moli_layout::LayoutPoint;

    let vm = new_rendered_test_vm(
        "https://hit-test-policy.test/",
        r#"<!doctype html><style>
        .box { position: absolute; inset: 0; width: 100px; height: 100px; }
        </style>
        <div id="bottom" class="box"></div>
        <div id="css" class="box" style="z-index: 1; pointer-events: none"></div>
        <div id="inert" class="box" style="z-index: 2" inert></div>"#,
    );
    for (ignore_pointer_events_none, ignore_inert, expected) in [
        (false, false, "bottom"),
        (true, false, "css"),
        (false, true, "inert"),
        (true, true, "inert"),
    ] {
        let expected_handle = vm.element_handle_by_id_for_test(expected).unwrap();
        let host = vm._context_host.borrow();
        for mode in [InputHitTestMode::Dom, InputHitTestMode::Mouse] {
            let hit = input_surface_hit_test(
                &host,
                host.document_handle(),
                LayoutPoint::new(25.0, 25.0),
                InputHitTestPolicy {
                    ignore_pointer_events_none,
                    ignore_inert,
                    ..InputHitTestPolicy::INTERACTIVE
                },
                mode,
            )
            .unwrap();
            assert_eq!(hit.target, Some(expected_handle), "expected {expected}");
        }
    }
}

#[test]
fn input_hit_test_policy_keeps_viewport_clipping_independent_of_fallback() {
    use crate::native_bridge::element::{
        InputHitTestMode, InputHitTestPolicy, input_surface_hit_test,
    };
    use moli_layout::LayoutPoint;
    use moli_page_types::LayoutPolicy;

    for layout_policy in [LayoutPolicy::Mock, LayoutPolicy::Lazy] {
        let mut vm = new_parsed_test_vm(
            "https://hit-test-viewport-policy.test/",
            "<!doctype html><html id='root'><body><button>Target</button></body></html>",
        );
        vm.set_layout_policy(layout_policy);
        vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 64,
            inner_height: 64,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))
        .unwrap();
        if layout_policy.uses_real_layout() {
            publish_layout_for_test(&mut vm);
        }
        let root = vm.element_handle_by_id_for_test("root").unwrap();
        for (inert, x, use_viewport_fallback, expected) in [
            (false, 64.0, false, None),
            (true, 10.0, false, None),
            (true, 10.0, true, Some(root)),
            (true, 64.0, true, None),
        ] {
            vm.eval(&format!("document.documentElement.inert = {inert}"))
                .unwrap();
            let host = vm._context_host.borrow();
            let hit = input_surface_hit_test(
                &host,
                host.document_handle(),
                LayoutPoint::new(x, 10.0),
                InputHitTestPolicy {
                    use_viewport_fallback,
                    ..InputHitTestPolicy::INTERACTIVE
                },
                InputHitTestMode::Dom,
            )
            .unwrap();
            assert_eq!(
                hit.target, expected,
                "{layout_policy:?}, inert={inert}, x={x}, fallback={use_viewport_fallback}"
            );
        }
    }
}

#[test]
fn mouse_release_over_inert_document_clears_capture_and_compatibility_state() {
    use moli_page_types::LayoutPolicy;

    for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
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

    for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
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
fn touch_input_in_an_inert_child_viewport_targets_its_document_root() {
    use moli_page_types::LayoutPolicy;

    for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
        let mut vm = new_storage_html_test_vm("https://inert-child-touch-input.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            const frame = document.createElement('iframe');
            frame.style.cssText = 'position:absolute;left:65px;top:45px;width:320px;height:240px;border:0';
            document.body.appendChild(frame);
            const inner = frame.contentDocument;
            inner.body.innerHTML = '<button>child</button>';
            window.inputEvents = [];
            for (const [doc, source] of [[document, 'parent'], [inner, 'child']]) {
                for (const type of ['pointerdown', 'touchstart', 'pointerup', 'touchend', 'click']) {
                    doc.addEventListener(type, event => {
                        const point = event.changedTouches?.[0] ?? event;
                        inputEvents.push([source, type, event.target.nodeName, point.clientX, point.clientY]);
                    });
                }
            }
            "#,
        )
        .unwrap();
        publish_layout_for_test(&mut vm);
        vm.eval("inner.documentElement.inert = true").unwrap();

        vm.dispatch_touch_event_at_point(110.0, 105.0, "touchstart", false)
            .unwrap();
        vm.dispatch_touch_event_at_points(&[], "touchend", false)
            .unwrap();
        let outside = vm
            .dispatch_touch_event_at_point(-1.0, 105.0, "touchstart", false)
            .unwrap();
        assert!(!outside.handled, "{policy:?}");
        assert_eq!(
            vm.eval("JSON.stringify(inputEvents)").unwrap(),
            concat!(
                r#"[["child","pointerdown","HTML",45,60],["child","touchstart","HTML",45,60],"#,
                r#"["child","pointerup","HTML",45,60],["child","touchend","HTML",45,60],"#,
                r#"["child","click","HTML",45,60]]"#,
            ),
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

#[test]
fn mouse_button_events_report_click_count_in_detail() {
    let mut vm = new_storage_html_test_vm("https://mouse-detail.test/");
    vm.eval(
        r#"
          document.body.style.margin = '0';
          const target = document.createElement('div');
          target.id = 't';
          target.style.cssText = 'position:fixed;left:0;top:0;width:120px;height:60px';
          document.body.appendChild(target);
          globalThis.detailLog = [];
          for (const type of ['mousemove', 'mousedown', 'mouseup', 'click', 'dblclick']) {
            target.addEventListener(type, event => detailLog.push(type + '|d' + event.detail));
          }
          globalThis.takeDetailLog = () => {
            const out = detailLog.join(',');
            detailLog = [];
            return out;
          };
        "#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();

    // Single click followed by double click (UI Events click count in detail).
    for (event, buttons, click_count) in [
        ("mousedown", 1, 1),
        ("mouseup", 0, 1),
        ("mousedown", 1, 2),
        ("mouseup", 0, 2),
    ] {
        vm.dispatch_mouse_input(
            50.0,
            20.0,
            event,
            0,
            Some(buttons),
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
    assert_eq!(
        vm.eval("takeDetailLog()").unwrap(),
        "mousedown|d1,mouseup|d1,click|d1,mousedown|d2,mouseup|d2,click|d2,dblclick|d2"
    );

    // mousemove must retain detail: 0 even if click_count is non-zero.
    vm.dispatch_mouse_input(
        55.0,
        20.0,
        "mousemove",
        -1,
        Some(0),
        2,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties::default(),
        0,
        false,
        MouseDispatchCompletion::FinishTurn,
    )
    .unwrap();
    assert_eq!(vm.eval("takeDetailLog()").unwrap(), "mousemove|d0");

    // Auxiliary mouse buttons also carry click_count on mousedown/mouseup.
    for (event, buttons) in [("mousedown", 4), ("mouseup", 0)] {
        vm.dispatch_mouse_input(
            50.0,
            20.0,
            event,
            1,
            Some(buttons),
            2,
            0.0,
            0.0,
            crate::runtime::RendererPointerEventProperties::default(),
            0,
            false,
            MouseDispatchCompletion::FinishTurn,
        )
        .unwrap();
    }
    assert_eq!(
        vm.eval("takeDetailLog()").unwrap(),
        "mousedown|d2,mouseup|d2"
    );

    // Zero click_count reports detail: 0 on mousedown/mouseup.
    for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
        vm.dispatch_mouse_input(
            50.0,
            20.0,
            event,
            0,
            Some(buttons),
            0,
            0.0,
            0.0,
            crate::runtime::RendererPointerEventProperties::default(),
            0,
            false,
            MouseDispatchCompletion::FinishTurn,
        )
        .unwrap();
    }
    assert!(
        vm.eval("takeDetailLog()")
            .unwrap()
            .starts_with("mousedown|d0,mouseup|d0")
    );
}
