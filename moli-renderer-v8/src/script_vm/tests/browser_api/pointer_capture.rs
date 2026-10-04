use super::*;

#[tokio::test]
async fn pointer_capture_arguments_use_native_receiver_and_webidl_conversion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://pointer-capture-arguments.test/",
        &loader,
    );
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame = document.body.appendChild(document.createElement('iframe'));
        frame.id = 'child'; frame.srcdoc = '<head></head><body></body>'; void frame.contentWindow;
        globalThis.nativeCaptureReceiver = document.implementation.createHTMLDocument('').createElement('select');
        'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeCaptureReceiver").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        assert!(value.is_proxy(), "fixture must exercise a native Proxy");
        let object = v8::Local::<v8::Object>::try_from(value).unwrap();
        assert!(crate::web_api_interfaces::Element::is_instance(
            scope, object
        ));
        Ok(())
    })
    .unwrap();
    let source = format!(
        "{}; globalThis.__captureResults = __pointerCaptureProbe([globalThis, document.getElementById('child').contentWindow]); __captureResults.complete",
        include_str!("pointer_capture.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__captureResults.errors,failures:__captureResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}

#[test]
fn pointer_capture_queries_pending_target_before_native_dispatch() {
    let mut vm = new_rendered_test_vm(
        "https://pointer-capture-pending-query.test/",
        r#"<html><body><div id="first" style="position:absolute;left:40px;top:40px;width:120px;height:120px">first</div><div id="second" style="position:absolute;left:220px;top:40px;width:120px;height:120px">second</div></body></html>"#,
    );
    let source = format!(
        "{}; globalThis.__captureResults = __installPointerCaptureStateProbe(document.getElementById('first'), document.getElementById('second')); 'ready'",
        include_str!("pointer_capture.js"),
    );
    vm.eval(&source).unwrap();
    vm.publish_layout_for_test().unwrap();
    for (kind, x, button) in [
        ("mousedown", 80.0, 0),
        ("mousemove", 82.0, -1),
        ("mousemove", 84.0, -1),
        ("mousemove", 86.0, -1),
        ("mouseup", 86.0, 0),
    ] {
        vm.dispatch_mouse_event_at_point(x, 80.0, kind, button, None, 0.0, 0.0)
            .unwrap();
    }
    assert_eq!(vm.eval("__captureResults.errors.length === 0 && __captureResults.rows.every(row=>Object.values(row.checks).every(value=>value===true))").unwrap(), "true", "{}", vm.eval("JSON.stringify(__captureResults)").unwrap());
    assert_eq!(
        vm.eval("__captureResults.events.join('|')").unwrap(),
        "pointerdown@first|gotpointercapture@first|pointermove@first|lostpointercapture@first|gotpointercapture@second|pointermove@second|lostpointercapture@second|pointermove@first|pointerup@first"
    );
}

fn new_pointer_activity_test_vm(
    kind: &str,
    chorded: bool,
    contacts: usize,
) -> StandaloneScriptVmHarness {
    let mut vm = new_rendered_test_vm(
        "https://pointer-activity.test/",
        r#"<html><body><div id="first" style="position:absolute;left:40px;top:40px;width:120px;height:120px">first</div><div id="second" style="position:absolute;left:220px;top:40px;width:120px;height:120px">second</div></body></html>"#,
    );
    vm.eval(&format!(
        "{}; globalThis.__expectedContacts = {contacts}; globalThis.__activityResults = __installPointerActivityProbe(document.getElementById('first'), document.getElementById('second'), '{kind}', {chorded}); 'ready'",
        include_str!("pointer_activity.js"),
    ))
    .unwrap();
    vm
}

fn assert_pointer_activity_probe(vm: &mut StandaloneScriptVmHarness) {
    assert_eq!(
        vm.eval("globalThis.__activityFinished = __activityResults.finish(); __activityFinished.complete")
            .unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__activityFinished)").unwrap()
    );
}

#[test]
fn hovering_pointer_capture_keeps_native_pointer_ids_after_button_release() {
    for (kind, id) in [("mouse", 1), ("mouse", 17), ("pen", 5)] {
        let mut vm = new_pointer_activity_test_vm(kind, false, 1);
        let pointer = crate::runtime::RendererPointerEventProperties {
            pointer_id: id,
            pointer_type: kind.to_owned(),
            ..Default::default()
        };
        for (event, button, buttons, x) in [
            ("mousemove", -1, 0, 80.0),
            ("mousedown", 0, 1, 80.0),
            ("mousemove", -1, 1, 82.0),
            ("mouseup", 0, 0, 82.0),
            ("mousemove", -1, 0, 84.0),
        ] {
            vm.dispatch_mouse_event_at_point_with_pointer(
                x,
                80.0,
                event,
                button,
                Some(buttons),
                1,
                0.0,
                0.0,
                pointer.clone(),
            )
            .unwrap();
        }
        assert_pointer_activity_probe(&mut vm);
    }
}

#[test]
fn chorded_mouse_capture_ends_only_when_the_last_button_is_released() {
    let mut vm = new_pointer_activity_test_vm("mouse", true, 1);
    for (phase, event, button, buttons, x) in [
        ("hover", "mousemove", -1, 0, 80.0),
        ("first-press", "mousedown", 0, 1, 80.0),
        ("second-press", "mousedown", 2, 3, 80.0),
        ("first-release", "mouseup", 0, 2, 80.0),
        ("move", "mousemove", -1, 2, 82.0),
        ("last-release", "mouseup", 2, 0, 82.0),
        ("after-end-hover", "mousemove", -1, 0, 84.0),
    ] {
        vm.eval(&format!("globalThis.__inputPhase = '{phase}'"))
            .unwrap();
        vm.dispatch_mouse_event_at_point(x, 80.0, event, button, Some(buttons), 0.0, 0.0)
            .unwrap();
    }
    assert_pointer_activity_probe(&mut vm);
}

#[test]
fn ended_and_cancelled_touch_contacts_are_retired_after_capture_events() {
    for contacts in [1, 2] {
        for end in ["touchend", "touchcancel"] {
            let mut vm = new_pointer_activity_test_vm("touch", false, contacts);
            let points = (0..contacts)
                .map(|id| crate::runtime::RendererTouchPoint {
                    id: id as i32,
                    x: 80.0 + 20.0 * id as f64,
                    y: 80.0,
                })
                .collect::<Vec<_>>();
            vm.dispatch_touch_event_at_points(&points, "touchstart", false)
                .unwrap();
            vm.dispatch_touch_event_at_points(&points, "touchmove", false)
                .unwrap();
            vm.dispatch_touch_event_at_points(&[], end, false).unwrap();
            assert_pointer_activity_probe(&mut vm);
        }
    }
}

#[test]
fn ending_one_touch_contact_preserves_capture_for_the_other_contact() {
    let mut vm = new_pointer_activity_test_vm("touch", false, 2);
    let points = [
        crate::runtime::RendererTouchPoint {
            id: 11,
            x: 80.0,
            y: 80.0,
        },
        crate::runtime::RendererTouchPoint {
            id: 12,
            x: 100.0,
            y: 80.0,
        },
    ];
    vm.dispatch_touch_event_at_points(&points, "touchstart", false)
        .unwrap();
    vm.dispatch_touch_event_at_points(&points, "touchmove", false)
        .unwrap();
    vm.dispatch_touch_event_at_points(&points[..1], "touchend", false)
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
                const [ended, remaining] = __activityResults.events.filter(e => e.type === 'pointerdown').map(e => e.id);
                const first = document.getElementById('first');
                let error;
                try { first.releasePointerCapture(ended); } catch (e) { error = e; }
                return error instanceof DOMException && error.name === 'NotFoundError'
                    && !first.hasPointerCapture(ended) && first.hasPointerCapture(remaining);
            })()"#,
        )
        .unwrap(),
        "true"
    );
    vm.dispatch_touch_event_at_points(&[], "touchend", false)
        .unwrap();
    assert_pointer_activity_probe(&mut vm);
}

fn new_touch_contact_test_vm() -> StandaloneScriptVmHarness {
    new_rendered_test_vm(
        "https://touch-contact-updates.test/",
        r#"<html><body><div id="first" style="position:absolute;left:40px;top:40px;width:120px;height:120px">first</div><div id="second" style="position:absolute;left:220px;top:40px;width:120px;height:120px">second</div></body></html>"#,
    )
}

#[test]
fn contact_updates_preserve_existing_touches_and_emit_only_changed_contacts() {
    for first_id in [0, 11] {
        let mut vm = new_touch_contact_test_vm();
        vm.eval(
            r#"
            globalThis.__touchChanges = [];
            for (const type of ['pointerdown','pointermove','pointerup','touchstart','touchmove','touchend']) {
                document.addEventListener(type, e => __touchChanges.push({
                    type, target:e.target.id,
                    touches:e.touches ? Array.from(e.touches, t => t.identifier) : null,
                    changed:e.changedTouches ? Array.from(e.changedTouches, t => t.identifier) : null
                }));
            }
            'ready'
            "#,
        ).unwrap();
        let first = crate::runtime::RendererTouchPoint {
            id: first_id,
            x: 80.0,
            y: 80.0,
        };
        let second = crate::runtime::RendererTouchPoint {
            id: first_id + 1,
            x: 260.0,
            y: 80.0,
        };
        vm.dispatch_touch_contact_updates(&[first], "touchstart")
            .unwrap();
        vm.eval("__touchChanges.length = 0").unwrap();
        vm.dispatch_touch_contact_updates(&[second, first], "touchstart")
            .unwrap();
        assert_eq!(
            vm.eval("__touchChanges.map(e=>e.type+'@'+e.target).join('|')")
                .unwrap(),
            "pointerdown@second|touchstart@second"
        );
        assert_eq!(
            vm.eval("JSON.stringify(__touchChanges.find(e=>e.type==='touchstart').touches)")
                .unwrap(),
            format!("[{first_id},{}]", first_id + 1)
        );
        vm.eval("__touchChanges.length = 0").unwrap();
        vm.dispatch_touch_contact_updates(&[second], "touchmove")
            .unwrap();
        assert_eq!(vm.eval("__touchChanges.length").unwrap(), "0");
        vm.dispatch_touch_contact_updates(
            &[
                crate::runtime::RendererTouchPoint { x: 84.0, ..first },
                second,
            ],
            "touchmove",
        )
        .unwrap();
        assert_eq!(
            vm.eval("__touchChanges.map(e=>e.type+'@'+e.target).join('|')")
                .unwrap(),
            "pointermove@first|touchmove@first"
        );
        assert_eq!(
            vm.eval("JSON.stringify(__touchChanges.find(e=>e.type==='touchmove').changed)")
                .unwrap(),
            format!("[{first_id}]")
        );
        vm.eval("__touchChanges.length = 0").unwrap();
        vm.dispatch_touch_contact_updates(&[], "touchend").unwrap();
        assert_eq!(
            vm.eval("__touchChanges.map(e=>e.type+'@'+e.target).join('|')")
                .unwrap(),
            "pointerup@first|touchend@first|pointerup@second|touchend@second"
        );
        assert_eq!(vm.eval("JSON.stringify(__touchChanges.filter(e=>e.type==='touchend').map(e=>[e.touches,e.changed]))").unwrap(),
            format!("[[[{}],[{first_id}]],[[],[{}]]]", first_id + 1, first_id + 1));
    }
}

#[test]
fn ending_contact_updates_preserves_other_touches_and_their_capture() {
    for first_id in [0, 11] {
        let mut vm = new_touch_contact_test_vm();
        vm.eval(
            r#"
            globalThis.__touchEnds = [];
            globalThis.__touchPointers = {};
            document.addEventListener('pointerdown', e => __touchPointers[e.target.id] = e.pointerId);
            for (const type of ['touchmove', 'touchend']) {
                document.addEventListener(type, e => __touchEnds.push({
                    type, target:e.target.id,
                    touches:Array.from(e.touches, t => t.identifier),
                    targetTouches:Array.from(e.targetTouches, t => t.identifier),
                    changed:Array.from(e.changedTouches, t => t.identifier)
                }));
            }
            'ready'
            "#,
        ).unwrap();
        let first = crate::runtime::RendererTouchPoint {
            id: first_id,
            x: 80.0,
            y: 80.0,
        };
        let second = crate::runtime::RendererTouchPoint {
            id: first_id + 1,
            x: 260.0,
            y: 80.0,
        };
        vm.dispatch_touch_contact_updates(&[second, first], "touchstart")
            .unwrap();
        vm.dispatch_touch_contact_updates(&[first], "touchend")
            .unwrap();
        assert_eq!(
            vm.eval(
                r#"(() => {
                    const first = document.getElementById('first');
                    const second = document.getElementById('second');
                    let retired = false;
                    try { first.releasePointerCapture(__touchPointers.first); }
                    catch (e) { retired = e instanceof DOMException && e.name === 'NotFoundError'; }
                    return retired && !first.hasPointerCapture(__touchPointers.first)
                        && second.hasPointerCapture(__touchPointers.second);
                })()"#,
            )
            .unwrap(),
            "true"
        );
        let moved_second = crate::runtime::RendererTouchPoint { x: 264.0, ..second };
        vm.dispatch_touch_contact_updates(&[moved_second], "touchmove")
            .unwrap();
        vm.dispatch_touch_contact_updates(&[moved_second], "touchend")
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(__touchEnds)").unwrap(),
            format!(
                r#"[{{"type":"touchend","target":"first","touches":[{second_id}],"targetTouches":[],"changed":[{first_id}]}},{{"type":"touchmove","target":"second","touches":[{second_id}],"targetTouches":[{second_id}],"changed":[{second_id}]}},{{"type":"touchend","target":"second","touches":[],"targetTouches":[],"changed":[{second_id}]}}]"#,
                second_id = first_id + 1
            )
        );
        assert_eq!(
            vm.eval("document.getElementById('second').hasPointerCapture(__touchPointers.second)")
                .unwrap(),
            "false"
        );
    }
}

#[test]
fn contact_updates_on_move_can_start_an_additional_contact() {
    let mut vm = new_pointer_activity_test_vm("touch", false, 2);
    let first = crate::runtime::RendererTouchPoint {
        id: 0,
        x: 80.0,
        y: 80.0,
    };
    let second = crate::runtime::RendererTouchPoint {
        id: 11,
        x: 100.0,
        y: 80.0,
    };
    vm.dispatch_touch_contact_updates(&[first], "touchstart")
        .unwrap();
    vm.dispatch_touch_contact_updates(&[second], "touchmove")
        .unwrap();
    assert_eq!(
        vm.eval("__activityResults.events.filter(e=>e.type==='pointerdown').length")
            .unwrap(),
        "2"
    );
    vm.dispatch_touch_event_at_points(&[first, second], "touchmove", false)
        .unwrap();
    vm.dispatch_touch_contact_updates(&[], "touchend").unwrap();
    assert_pointer_activity_probe(&mut vm);
}

#[test]
fn cancelling_contacts_notifies_each_original_touch_target() {
    let mut vm = new_touch_contact_test_vm();
    vm.eval("globalThis.__cancelledTouches = []; document.addEventListener('touchcancel', e => __cancelledTouches.push([e.target.id,e.touches.length,e.targetTouches.length,Array.from(e.changedTouches,t=>t.identifier)])); 'ready'").unwrap();
    let points = [
        crate::runtime::RendererTouchPoint {
            id: 0,
            x: 80.0,
            y: 80.0,
        },
        crate::runtime::RendererTouchPoint {
            id: 11,
            x: 260.0,
            y: 80.0,
        },
    ];
    vm.dispatch_touch_contact_updates(&points, "touchstart")
        .unwrap();
    vm.dispatch_touch_contact_updates(&[], "touchcancel")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__cancelledTouches)").unwrap(),
        "[[\"first\",0,0,[0,11]],[\"second\",0,0,[0,11]]]"
    );
    vm.dispatch_touch_contact_updates(&points[..1], "touchstart")
        .unwrap();
    vm.dispatch_touch_contact_updates(&[], "touchend").unwrap();
    assert!(vm.active_touch_points.is_empty());
}

#[test]
fn touch_listener_cleanup_runs_microtasks_between_changed_contacts() {
    let mut vm = new_touch_contact_test_vm();
    vm.eval("globalThis.__touchTurn = []; document.addEventListener('touchstart', e => { __touchTurn.push('event@'+e.target.id); queueMicrotask(()=>__touchTurn.push('microtask@'+e.target.id)); }); 'ready'").unwrap();
    let points = [
        crate::runtime::RendererTouchPoint {
            id: 11,
            x: 260.0,
            y: 80.0,
        },
        crate::runtime::RendererTouchPoint {
            id: 0,
            x: 80.0,
            y: 80.0,
        },
    ];
    vm.dispatch_touch_contact_updates(&points, "touchstart")
        .unwrap();
    assert_eq!(
        vm.eval("__touchTurn.join('|')").unwrap(),
        "event@first|microtask@first|event@second|microtask@second"
    );
}

#[test]
fn cancelled_pointerdown_suppresses_compatibility_mouse_events_until_last_release() {
    let mut vm = new_pointer_activity_test_vm("mouse", true, 1);
    vm.eval(
        r#"
        globalThis.__compatibilityMouseEvents = [];
        const first = document.getElementById('first');
        first.addEventListener('pointerdown', event => event.preventDefault());
        for (const type of ['mousedown', 'mouseup', 'mousemove']) {
            first.addEventListener(type, event => __compatibilityMouseEvents.push(`${type}:${event.buttons}`));
        }
        'ready'
        "#,
    )
    .unwrap();
    for (event, button, buttons) in [
        ("mousedown", 0, 1),
        ("mousedown", 2, 3),
        ("mouseup", 2, 1),
        ("mousemove", -1, 1),
        ("mouseup", 0, 0),
        ("mousemove", -1, 0),
    ] {
        vm.dispatch_mouse_event_at_point(80.0, 80.0, event, button, Some(buttons), 0.0, 0.0)
            .unwrap();
    }
    assert_eq!(
        vm.eval("__compatibilityMouseEvents.join('|')").unwrap(),
        "mousemove:0"
    );
}

fn new_pointer_document_test_vm() -> StandaloneScriptVmHarness {
    let mut vm = new_rendered_test_vm(
        "https://pointer-document.test/",
        r#"<html><body><div id="outside" style="position:absolute;left:40px;top:40px;width:120px;height:120px">outside</div><iframe id="child" style="position:absolute;left:220px;top:40px;width:120px;height:120px;border:0"></iframe></body></html>"#,
    );
    vm.eval(
        r#"
        const childWindow = document.getElementById('child').contentWindow;
        childWindow.document.documentElement.style.cssText = 'margin:0;touch-action:none';
        childWindow.document.body.style.cssText = 'margin:0;touch-action:none';
        childWindow.document.body.innerHTML = '<div id="inside" style="width:120px;height:120px">inside</div>';
        'ready'
        "#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    vm
}

fn install_pointer_document_probe(
    vm: &mut StandaloneScriptVmHarness,
    child_origin: bool,
    foreign_methods: bool,
    mode: &str,
) {
    vm.eval(&format!(
        r#"{};
        const outside = document.getElementById('outside');
        const inside = childWindow.document.getElementById('inside');
        const first = {first}, second = {second};
        const methodRealm = {realm};
        globalThis.__documentResults = __installPointerDocumentProbe(first, second, methodRealm, '{mode}');
        'ready'"#,
        include_str!("pointer_document.js"),
        first = if child_origin { "inside" } else { "outside" },
        second = if child_origin { "outside" } else { "inside" },
        realm = if child_origin != foreign_methods { "childWindow" } else { "globalThis" },
    ))
    .unwrap();
}

fn assert_pointer_document_probe(vm: &mut StandaloneScriptVmHarness) {
    assert_eq!(
        vm.eval("globalThis.__documentFinished = __documentResults.finish(); __documentFinished.complete")
            .unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__documentFinished)").unwrap()
    );
}

#[test]
fn mouse_and_pen_capture_follow_the_native_event_document_across_realms() {
    for (pointer_type, pointer_id) in [("mouse", 17), ("pen", 5)] {
        for child_origin in [false, true] {
            for foreign_methods in [false, true] {
                for mode in ["capture", "move"] {
                    let mut vm = new_pointer_document_test_vm();
                    install_pointer_document_probe(&mut vm, child_origin, foreign_methods, mode);
                    let (start_x, end_x) = if child_origin {
                        (260.0, 80.0)
                    } else {
                        (80.0, 260.0)
                    };
                    for (phase, event, x, button, buttons) in [
                        ("hover", "mousemove", start_x, -1, 0),
                        ("down", "mousedown", start_x, 0, 1),
                        ("cross-document-move", "mousemove", end_x, -1, 1),
                        ("up", "mouseup", end_x, 0, 0),
                    ] {
                        vm.eval(&format!("globalThis.__inputPhase = '{phase}'; 'ready'"))
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
                    assert_pointer_document_probe(&mut vm);
                }
            }
        }
    }
}

#[test]
fn touch_capture_changes_active_document_only_after_native_delivery() {
    for child_origin in [false, true] {
        for foreign_methods in [false, true] {
            for mode in ["capture", "move"] {
                let mut vm = new_pointer_document_test_vm();
                install_pointer_document_probe(&mut vm, child_origin, foreign_methods, mode);
                let (start_x, end_x) = if child_origin {
                    (260.0, 80.0)
                } else {
                    (80.0, 260.0)
                };
                for (phase, event, x) in [
                    ("down", "touchstart", start_x),
                    ("cross-document-move", "touchmove", end_x),
                    ("up", "touchend", end_x),
                ] {
                    vm.eval(&format!("globalThis.__inputPhase = '{phase}'; 'ready'"))
                        .unwrap();
                    vm.dispatch_touch_event_at_point(x, 80.0, event, false)
                        .unwrap();
                }
                assert_pointer_document_probe(&mut vm);
            }
        }
    }
}

#[test]
fn concurrent_touch_contacts_keep_independent_active_documents() {
    let mut vm = new_pointer_document_test_vm();
    vm.eval(
        r#"
        const outside = document.getElementById('outside');
        const inside = childWindow.document.getElementById('inside');
        globalThis.__contactChecks = [];
        globalThis.__contactIds = [];
        for (const [index, own, other] of [[0, outside, inside], [1, inside, outside]]) {
            own.addEventListener('pointerdown', event => {
                __contactIds[index] = event.pointerId;
                own.setPointerCapture(event.pointerId);
                other.setPointerCapture(event.pointerId);
                __contactChecks.push(own.hasPointerCapture(event.pointerId) && !other.hasPointerCapture(event.pointerId));
                if (index === 1) {
                    own.setPointerCapture(__contactIds[0]);
                    __contactChecks.push(other.hasPointerCapture(__contactIds[0]) && !own.hasPointerCapture(__contactIds[0]));
                }
            });
        }
        'ready'
        "#,
    )
    .unwrap();
    let first = crate::runtime::RendererTouchPoint {
        id: 11,
        x: 80.0,
        y: 80.0,
    };
    let second = crate::runtime::RendererTouchPoint {
        id: 12,
        x: 260.0,
        y: 80.0,
    };
    vm.dispatch_touch_event_at_points(&[first], "touchstart", false)
        .unwrap();
    vm.dispatch_touch_event_at_points(&[second], "touchstart", false)
        .unwrap();
    vm.dispatch_touch_event_at_points(&[first, second], "touchmove", false)
        .unwrap();
    vm.dispatch_touch_event_at_points(&[first], "touchend", false)
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
                const [ended, remaining] = __contactIds;
                let endedError;
                try { inside.setPointerCapture(ended); } catch (error) { endedError = error.name; }
                outside.setPointerCapture(remaining);
                return __contactChecks.length === 3 && __contactChecks.every(Boolean)
                    && endedError === 'NotFoundError' && !outside.hasPointerCapture(remaining)
                    && inside.hasPointerCapture(remaining);
            })()"#,
        )
        .unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify({ids:__contactIds, checks:__contactChecks, firstCaptured:outside.hasPointerCapture(__contactIds[0]), secondCaptured:inside.hasPointerCapture(__contactIds[1])})")
            .unwrap()
    );
    vm.dispatch_touch_event_at_points(&[], "touchend", false)
        .unwrap();
    assert_eq!(
        vm.eval("!outside.hasPointerCapture(__contactIds[0]) && !inside.hasPointerCapture(__contactIds[1])")
            .unwrap(),
        "true"
    );
}

#[test]
fn disconnected_child_capture_dispatches_lost_to_the_active_child_document() {
    for remove_during_got in [false, true] {
        let mut vm = new_pointer_document_test_vm();
        vm.eval(&format!(
            r#"
            const outside = document.getElementById('outside');
            const inside = childWindow.document.getElementById('inside');
            globalThis.__lostChecks = [];
            globalThis.__rootLost = 0;
            document.addEventListener('lostpointercapture', () => __rootLost++);
            childWindow.document.addEventListener('lostpointercapture', event => {{
                outside.setPointerCapture(event.pointerId);
                __lostChecks.push(event.target === childWindow.document
                    && !outside.hasPointerCapture(event.pointerId));
            }});
            inside.addEventListener('pointerdown', event => inside.setPointerCapture(event.pointerId));
            inside.addEventListener('gotpointercapture', event => {{
                if ({remove_during_got}) inside.remove();
            }});
            'ready'
            "#,
        ))
        .unwrap();
        vm.dispatch_mouse_event_at_point(260.0, 80.0, "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        vm.dispatch_mouse_event_at_point(262.0, 80.0, "mousemove", -1, Some(1), 0.0, 0.0)
            .unwrap();
        if !remove_during_got {
            vm.eval("inside.remove(); 'ready'").unwrap();
        }
        vm.dispatch_mouse_event_at_point(264.0, 80.0, "mousemove", -1, Some(1), 0.0, 0.0)
            .unwrap();
        assert_eq!(
            vm.eval("__rootLost === 0 && __lostChecks.length === 1 && __lostChecks.every(Boolean)")
                .unwrap(),
            "true",
            "{}",
            vm.eval("JSON.stringify({root:__rootLost, checks:__lostChecks})")
                .unwrap()
        );
    }
}

#[test]
fn hovering_capture_boundaries_follow_capture_transitions_and_implicit_release() {
    for context in [
        "root",
        "child",
        "nested",
        "shadow-open",
        "shadow-closed",
        "slotted",
    ] {
        for pointer_type in ["mouse", "pen"] {
            for mode in ["implicit", "drag", "explicit", "raw"] {
                for poison in ["ordinary", "getter"] {
                    let mut vm = new_rendered_test_vm(
                        "https://capture-boundaries.test/",
                        "<html><body></body></html>",
                    );
                    vm.eval(&format!("{};globalThis.__capture=__installCaptureBoundaryProbe('{context}','{pointer_type}','{mode}','{poison}',15);'ready'", include_str!("pointer_capture_boundaries.js"))).unwrap();
                    vm.publish_layout_for_test().unwrap();
                    let mut inputs = vec![
                        ("hover", "mousemove", 80.0, -1, 0),
                        ("down", "mousedown", 80.0, 0, 1),
                    ];
                    if mode != "implicit" {
                        inputs.extend([
                            ("start", "mousemove", 100.0, -1, 1),
                            ("cross", "mousemove", 520.0, -1, 1),
                        ]);
                        if mode == "explicit" {
                            inputs.push(("flush", "mousemove", 522.0, -1, 1));
                        }
                    }
                    inputs.push((
                        "up",
                        "mouseup",
                        if mode == "implicit" { 80.0 } else { 520.0 },
                        0,
                        0,
                    ));
                    inputs.push((
                        "after",
                        "mousemove",
                        if mode == "implicit" { 82.0 } else { 540.0 },
                        -1,
                        0,
                    ));
                    for (phase, event, x, button, buttons) in inputs {
                        vm.eval(&format!("__capture.prepare('{phase}');'ready'"))
                            .unwrap();
                        vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                            x,
                            80.0,
                            event,
                            button,
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
                            15,
                        )
                        .unwrap();
                    }
                    assert_eq!(vm.eval("globalThis.__captureFinished=__capture.finish();__captureFinished.complete").unwrap(),"true","{context}/{pointer_type}/{mode}/{poison}: {}",vm.eval("JSON.stringify(__captureFinished.checks.filter(c=>!c.pass))").unwrap());
                }
            }
        }
    }
}

#[test]
fn removed_capture_target_before_pointerup_preserves_uncaptured_click_target() {
    for pointer_type in ["mouse", "pen"] {
        for remove_event in ["pointerenter", "gotpointercapture"] {
            let mut vm = new_rendered_test_vm(
                "https://capture-removal-click.test/",
                r#"<html><body><div id="source" style="position:absolute;left:40px;top:40px;width:120px;height:120px"></div><div id="capture" style="position:absolute;left:220px;top:40px;width:120px;height:120px"></div><div id="peer" style="position:absolute;left:500px;top:40px;width:120px;height:120px"></div></body></html>"#,
            );
            vm.eval(&format!(r#"
                const source=document.getElementById('source'), capture=document.getElementById('capture');
                globalThis.clickTargets=[]; globalThis.lostTargets=[];
                source.addEventListener('pointerdown',event=>capture.setPointerCapture(event.pointerId));
                capture.addEventListener('{remove_event}',()=>capture.remove());
                document.addEventListener('click',event=>clickTargets.push(event.target===document.body));
                document.addEventListener('lostpointercapture',event=>lostTargets.push(event.target===document));
                'ready'
            "#)).unwrap();
            for (event, x, button, buttons) in [
                ("mousemove", 80.0, -1, 0),
                ("mousedown", 80.0, 0, 1),
                ("mouseup", 520.0, 0, 0),
            ] {
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
                        pointer_id: 17,
                        pointer_type: pointer_type.to_owned(),
                        pressure: if buttons == 0 { 0.0 } else { 0.5 },
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            assert_eq!(
                vm.eval("JSON.stringify(clickTargets)").unwrap(),
                "[true]",
                "{pointer_type}/{remove_event}"
            );
            assert_eq!(
                vm.eval("JSON.stringify(lostTargets)").unwrap(),
                if remove_event == "gotpointercapture" {
                    "[true]"
                } else {
                    "[]"
                },
                "{pointer_type}/{remove_event}"
            );
        }
    }
}
