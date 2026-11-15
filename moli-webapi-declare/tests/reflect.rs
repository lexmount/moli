use moli_webapi_declare::{capture_web_api_reflect_set, web_api_reflect_set};

#[test]
fn captured_reflect_set_preserves_receiver_semantics_after_namespace_replacement() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    assert!(web_api_reflect_set(scope).is_none());
    assert_eq!(capture_web_api_reflect_set(scope), Some(()));
    let original = web_api_reflect_set(scope).unwrap();
    let source = v8::String::new(
        scope,
        "Reflect.set = () => { throw Error('author set'); }; globalThis.Reflect = null;",
    )
    .unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap();
    assert_eq!(capture_web_api_reflect_set(scope), Some(()));
    assert!(original.strict_equals(web_api_reflect_set(scope).unwrap().into()));
    let target = v8::Object::new(scope);
    let receiver = v8::Object::new(scope);
    let key = v8::String::new(scope, "value").unwrap();
    let value = v8::Integer::new(scope, 42);
    let undefined = v8::undefined(scope);
    let result = original
        .call(
            scope,
            undefined.into(),
            &[target.into(), key.into(), value.into(), receiver.into()],
        )
        .unwrap();
    assert!(result.is_true());
    assert_eq!(target.has_own_property(scope, key.into()), Some(false));
    assert_eq!(
        receiver.get(scope, key.into()).unwrap().int32_value(scope),
        Some(42)
    );
}

#[test]
fn reflect_set_capture_is_scoped_to_the_current_realm() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let first = v8::Context::new(scope, Default::default());
    let original = {
        let scope = &mut v8::ContextScope::new(scope, first);
        assert_eq!(capture_web_api_reflect_set(scope), Some(()));
        web_api_reflect_set(scope).unwrap()
    };
    let second = v8::Context::new(scope, Default::default());
    {
        let scope = &mut v8::ContextScope::new(scope, second);
        assert!(web_api_reflect_set(scope).is_none());
        assert_eq!(capture_web_api_reflect_set(scope), Some(()));
        assert!(!original.strict_equals(web_api_reflect_set(scope).unwrap().into()));
    }
    let scope = &mut v8::ContextScope::new(scope, first);
    assert!(original.strict_equals(web_api_reflect_set(scope).unwrap().into()));
}
