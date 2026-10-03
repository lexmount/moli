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
