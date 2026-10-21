use super::*;

#[test]
fn custom_event_detail_uses_native_prototype_state_and_derived_dictionary() {
    let mut vm = new_parsed_test_vm(
        "https://custom-event-init.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/custom-event-init.js"
        ))
        .expect("CustomEvent WebIDL matrix should complete");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn custom_event_detail_accepts_registered_native_receivers_and_preserves_proxy_payloads() {
    let mut vm = new_storage_html_test_vm("https://custom-event-native-proxy.test/");
    vm.eval(
        r#"
        globalThis.nativeDetail = document.implementation.createHTMLDocument('').createElement('select');
        globalThis.nativeCustomEvent = new Proxy(new CustomEvent('before', {detail: nativeDetail}), {});
        "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let payload_key = v8::String::new(scope, "nativeDetail").unwrap();
        let payload = global.get(scope, payload_key.into()).unwrap();
        assert!(payload.is_proxy(), "payload must exercise a native Proxy");
        let payload = v8::Local::<v8::Object>::try_from(payload).unwrap();
        assert!(crate::web_api_interfaces::Element::is_instance(
            scope, payload
        ));

        let event_key = v8::String::new(scope, "nativeCustomEvent").unwrap();
        let event = global.get(scope, event_key.into()).unwrap();
        let event = v8::Local::<v8::Proxy>::try_from(event).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, event).unwrap();
        assert!(crate::web_api_interfaces::CustomEvent::is_instance(
            scope,
            event.into()
        ));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              const get = Object.getOwnPropertyDescriptor(CustomEvent.prototype, 'detail').get;
              if (get.call(nativeCustomEvent) !== nativeDetail) return false;
              CustomEvent.prototype.initCustomEvent.call(nativeCustomEvent, 'after', true, false, nativeCustomEvent);
              if (get.call(nativeCustomEvent) !== nativeCustomEvent || nativeCustomEvent.type !== 'after') return false;
              try { get.call(new Proxy(nativeCustomEvent, {})); return false; }
              catch (error) { return error instanceof TypeError; }
            })()"#,
        )
        .unwrap(),
        "true",
    );
}
