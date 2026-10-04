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
