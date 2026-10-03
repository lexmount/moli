use super::*;

#[test]
fn legacy_event_initializers_preserve_creation_state_and_convert_before_dispatch_guard() {
    let mut vm = new_parsed_test_vm(
        "https://legacy-event-init.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm.eval(include_str!("legacy_event_init.js")).expect(
        "legacy event initializers should preserve native Event state and WebIDL conversion",
    );
    assert_eq!(result, "true");
}

#[test]
fn legacy_mouse_initializer_converts_nullable_interfaces_and_short_before_mutation() {
    let mut vm = new_parsed_test_vm(
        "https://legacy-mouse-init.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        "globalThis.legacyMouseNativeRelatedTarget = document.implementation.createHTMLDocument('').createElement('select')",
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "legacyMouseNativeRelatedTarget").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        assert!(value.is_proxy(), "fixture must exercise a native Proxy");
        let object = v8::Local::<v8::Object>::try_from(value).unwrap();
        assert!(crate::web_api_interfaces::EventTarget::is_instance(
            scope, object
        ));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            "(() => { const event = new MouseEvent('before'); event.initMouseEvent('after', false, false, null, 0, 0, 0, 0, 0, false, false, false, false, 0, legacyMouseNativeRelatedTarget); return event.relatedTarget === legacyMouseNativeRelatedTarget; })()",
        )
        .unwrap(),
        "true",
    );
    let result = vm
        .eval(include_str!("legacy_mouse_event_init.js"))
        .expect("legacy MouseEvent conversion matrix should execute");
    if result != "true" {
        let failures = vm
            .eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .expect("legacy MouseEvent failures should be available");
        panic!("legacy MouseEvent conversion failures: {failures}");
    }
}
