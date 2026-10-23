use super::*;

#[test]
fn native_exception_constructors_and_factories_use_v8_api_wrappers() {
    let mut vm = new_storage_test_vm("https://native-error-wrapper.test/");
    vm.eval(
        r#"(() => {
      let failure;
      try { structuredClone(Symbol()); } catch(error) { failure = error; }
      globalThis.nativeErrorWrappers = [
        new DOMException('message'), new QuotaExceededError('full'),
        new WebSocketError('closed'), new RTCError({errorDetail:'dtls-failure'}),
        new OverconstrainedError('width'), new WebTransportError(), failure,
        structuredClone(new DOMException('copy')), structuredClone(new QuotaExceededError('copy'))
      ];
    })()"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeErrorWrappers");
        let values =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let mut wrappers = Vec::new();
        for index in 0..values.length() {
            let object =
                v8::Local::<v8::Object>::try_from(values.get_index(scope, index).unwrap()).unwrap();
            wrappers.push(object.is_api_wrapper());
        }
        assert_eq!(wrappers, vec![true; 9]);

        // Native facade/host contexts can throw without a full interface registry.
        let bare = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, bare);
        let fallback = crate::context_bootstrap::build_dom_exception_object(
            scope,
            "standalone host failure",
            "DataCloneError",
        );
        assert!(fallback.is_api_wrapper());
        let key = crate::util::v8str(scope, "nativeFallbackException");
        let global = bare.global(scope);
        assert_eq!(
            global.create_data_property(scope, key.into(), fallback.into()),
            Some(true)
        );
        let source = crate::util::v8str(scope, "Error.isError(nativeFallbackException)");
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(
            crate::script_execution::execute_compiled_script(scope, script)
                .unwrap()
                .is_true()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn native_error_brand_uses_v8_classification_across_window_realms() {
    let mut vm = new_parsed_test_vm(
        "https://native-error-brand.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(&format!(
            r#"(() => {{
            {fixture}
            const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
            return JSON.stringify({{main:nativeErrorBrandProbe(window),child:nativeErrorBrandProbe(child)}});
        }})()"#,
            fixture = include_str!("../../../tests/fixtures/native-error-brand.js"),
        ))
        .expect("native Error.isError probe should complete");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(result[realm]["failures"], serde_json::json!([]), "{result}");
        assert_eq!(
            result[realm]["rows"].as_array().unwrap().len(),
            84,
            "{result}"
        );
    }
}

#[test]
fn native_exception_stacks_ignore_mutated_javascript_capture_helpers() {
    let mut vm = new_parsed_test_vm(
        "https://dom-exception-native-stacks.test/",
        "<!doctype html><body></body>",
    );
    let actual = vm
        .eval(&format!(
            r#"(() => {{
        {fixture}
        const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
        return JSON.stringify({{
            main: domExceptionStackBoundariesProbe(window),
            child: domExceptionStackBoundariesProbe(child),
        }});
    }})()"#,
            fixture = include_str!("../../../tests/fixtures/dom-exception-stack-boundaries.js"),
        ))
        .expect("native exception stack boundaries should evaluate");
    let actual: serde_json::Value = serde_json::from_str(&actual).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(actual[realm]["failures"], serde_json::json!([]), "{actual}");
        assert_eq!(
            actual[realm]["rows"].as_array().unwrap().len(),
            37,
            "{actual}"
        );
    }
}
