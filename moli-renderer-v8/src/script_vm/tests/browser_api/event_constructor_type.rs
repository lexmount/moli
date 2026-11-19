use super::*;

#[test]
fn event_constructors_preserve_dom_string_code_units_and_conversion_order() {
    let mut vm = new_parsed_test_vm(
        "https://event-constructor-type.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!("event_constructor_type.js"))
        .expect("event constructors should preserve DOMString code units and conversion errors");
    assert_eq!(result, "true");
}

#[test]
fn native_value_event_factories_keep_renderer_templates_without_author_dictionary_reads() {
    let mut vm = new_parsed_test_vm(
        "https://native-value-event-factories.test/",
        "<!doctype html><main>ready</main>",
    );
    vm.eval(r#"
        globalThis.originalEventFactories = {MessageEvent, StorageEvent, ErrorEvent, PageTransitionEvent, KeyboardEvent};
        for (const name of Object.keys(originalEventFactories)) {
            delete globalThis[name];
            Object.defineProperty(globalThis, name, {configurable: true, get() {throw new Error('public event constructor');}});
        }
        Object.defineProperty(Object.prototype, 'composed', {configurable: true, get() {throw new Error('native event dictionary read');}});
        Object.defineProperty(Object.prototype, 'isComposing', {configurable: true, get() {throw new Error('keyboard dictionary read');}});
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let data = v8::Integer::new(scope, 7).into();
        let source = v8::null(scope).into();
        let ports = v8::Array::new(scope, 0);
        let message = crate::context_bootstrap::construct_original_message_event(
            scope,
            "message",
            data,
            "origin",
            &[0xd800],
            source,
            ports,
        )
        .unwrap();
        let storage = crate::context_bootstrap::construct_original_storage_event_utf16(
            scope,
            "storage",
            Some(&[0xd800]),
            None,
            Some(&[0xdfff]),
            "url",
            None,
        )
        .unwrap();
        let error_value = v8::Integer::new(scope, 9).into();
        let error = crate::context_bootstrap::construct_original_error_event(
            scope,
            "boom",
            "file",
            4,
            2,
            error_value,
        )
        .unwrap();
        let transition = crate::context_bootstrap::construct_original_page_transition_event(
            scope, "pageshow", true,
        )
        .unwrap();
        let keyboard = crate::native_bridge::element::construct_keyboard_event(
            scope, "keydown", "A", "KeyA", false, true, false, true, true, 0,
        )
        .unwrap();
        let global = scope.get_current_context().global(scope);
        for (name, interface, event) in [
            ("nativeMessageFactory", "MessageEvent", message),
            ("nativeStorageFactory", "StorageEvent", storage),
            ("nativeErrorFactory", "ErrorEvent", error),
            ("nativePageFactory", "PageTransitionEvent", transition),
            ("nativeKeyboardFactory", "KeyboardEvent", keyboard),
        ] {
            assert_eq!(
                event.get_constructor_name().to_rust_string_lossy(scope),
                interface
            );
            assert_eq!(
                moli_webapi_declare::web_api_object_type(scope, event)
                    .unwrap()
                    .name(),
                interface
            );
            let name = crate::util::v8str(scope, name);
            assert_eq!(
                global.create_data_property(scope, name.into(), event.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        delete Object.prototype.composed;
        delete Object.prototype.isComposing;
        for (const [event, Ctor] of [
            [nativeMessageFactory, originalEventFactories.MessageEvent],
            [nativeStorageFactory, originalEventFactories.StorageEvent],
            [nativeErrorFactory, originalEventFactories.ErrorEvent],
            [nativePageFactory, originalEventFactories.PageTransitionEvent],
            [nativeKeyboardFactory, originalEventFactories.KeyboardEvent],
        ]) {
            if (!(event instanceof Ctor) || !Number.isFinite(event.timeStamp)) return false;
        }
        return nativeMessageFactory.data === 7 && nativeMessageFactory.lastEventId === '\ud800'
            && Object.isFrozen(nativeMessageFactory.ports) && nativeMessageFactory.source === null
            && nativeStorageFactory.key === '\ud800' && nativeStorageFactory.newValue === '\udfff'
            && nativeErrorFactory.message === 'boom' && nativeErrorFactory.error === 9 && nativeErrorFactory.cancelable
            && nativePageFactory.persisted && nativePageFactory.bubbles && nativePageFactory.cancelable
            && !nativePageFactory.composed && nativePageFactory.isTrusted
            && nativeKeyboardFactory.key === 'A' && nativeKeyboardFactory.code === 'KeyA'
            && nativeKeyboardFactory.keyCode === 65 && !nativeKeyboardFactory.isComposing
            && nativeKeyboardFactory.repeat && nativeKeyboardFactory.ctrlKey && nativeKeyboardFactory.shiftKey
            && nativeKeyboardFactory.isTrusted;
    })()"#).unwrap(), "true");
}
