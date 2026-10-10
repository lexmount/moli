use moli_webidl::Context;

#[test]
fn array_buffer_conversion_preserves_identity_and_rejects_detached_resizable_buffers() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let other = v8::Context::new(scope, Default::default());
    for owner in [context, other] {
        for resizable in [false, true] {
            for detached in [false, true] {
                let buffer = {
                    let scope = &mut v8::ContextScope::new(scope, owner);
                    let source = if resizable {
                        "new ArrayBuffer(2, {maxByteLength: 4})"
                    } else {
                        "new ArrayBuffer(2)"
                    };
                    let source = v8::String::new(scope, source).unwrap();
                    let value = v8::Script::compile(scope, source, None)
                        .unwrap()
                        .run(scope)
                        .unwrap();
                    v8::Local::<v8::ArrayBuffer>::try_from(value).unwrap()
                };
                if detached {
                    assert_eq!(buffer.detach(None), Some(true));
                }
                assert_eq!(buffer.was_detached(), detached);
                assert_eq!(buffer.is_resizable_by_user_javascript(), resizable);
                let scope = &mut v8::ContextScope::new(scope, context);
                // Author properties cannot affect the native conversion.
                let key = v8::String::new(scope, "resizable").unwrap();
                let value = v8::Boolean::new(scope, !resizable);
                assert_eq!(
                    buffer.create_data_property(scope, key.into(), value.into()),
                    Some(true)
                );
                let converted = moli_webidl::convert::<v8::Local<v8::ArrayBuffer>>(
                    scope,
                    buffer.into(),
                    Context::argument("Test.buffer", 1),
                );
                if resizable {
                    assert!(converted.is_err(), "resizable buffer, detached={detached}");
                } else {
                    assert!(converted.unwrap().strict_equals(buffer.into()));
                }
            }
        }
    }
}
