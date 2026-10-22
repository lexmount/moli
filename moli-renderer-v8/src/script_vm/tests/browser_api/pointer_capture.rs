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

fn new_touch_contact_test_vm() -> StandaloneScriptVmHarness {
    new_rendered_test_vm(
        "https://touch-contact-updates.test/",
        r#"<html><body><div id="first" style="position:absolute;left:40px;top:40px;width:120px;height:120px">first</div><div id="second" style="position:absolute;left:220px;top:40px;width:120px;height:120px">second</div></body></html>"#,
    )
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
        vm.dispatch_touch_event_at_points(&[second, first], "touchstart", false)
            .unwrap();
        vm.dispatch_touch_event_at_points(&[first], "touchend", false)
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
        vm.dispatch_touch_event_at_points(&[moved_second], "touchmove", false)
            .unwrap();
        vm.dispatch_touch_event_at_points(&[moved_second], "touchend", false)
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
