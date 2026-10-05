use moli_webidl::{AllowSharedBufferSource, Context};
fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}
#[test]
fn borrowed_buffer_sources_copy_late_and_write_only_the_native_range() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    for shared in [false, true] {
        let value = eval(
            scope,
            if shared {
                "globalThis.storage=new SharedArrayBuffer(6); new Uint8Array(storage).set([1,2,3,4,5,6]); new DataView(storage,1,3)"
            } else {
                "globalThis.storage=new ArrayBuffer(6); new Uint8Array(storage).set([1,2,3,4,5,6]); new DataView(storage,1,3)"
            },
        );
        let source = moli_webidl::convert::<AllowSharedBufferSource>(
            scope,
            value,
            Context::argument("Test.buffer", 1),
        )
        .unwrap();
        assert_eq!(source.byte_length(), 3);
        eval(scope, "new Uint8Array(storage)[2]=9");
        assert_eq!(source.to_vec(scope), [2, 9, 4]);
        assert!(!source.write_bytes(scope, &[7, 8, 9, 10]));
        assert_eq!(source.to_vec(scope), [2, 9, 4]);
        assert!(source.write_bytes(scope, &[7, 8]));
        assert!(source.write_bytes(scope, &[]));
        assert_eq!(
            eval(scope, "Array.from(new Uint8Array(storage)).join(',')")
                .to_rust_string_lossy(scope),
            "1,7,8,4,5,6"
        );
        if !shared {
            let buffer = v8::Local::<v8::ArrayBuffer>::try_from(eval(scope, "storage")).unwrap();
            assert_eq!(buffer.detach(None), Some(true));
            assert_eq!(source.byte_length(), 0);
            assert!(source.to_vec(scope).is_empty());
            assert!(!source.write_bytes(scope, &[1]));
        }
    }
}
#[test]
fn borrowed_buffer_sources_reject_forged_proxies_and_resizable_storage() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    for expression in [
        "{}",
        "Object.create(ArrayBuffer.prototype)",
        "new Proxy(new Uint8Array(2),{})",
        "new ArrayBuffer(2,{maxByteLength:4})",
        "new Uint8Array(new ArrayBuffer(2,{maxByteLength:4}))",
        "new SharedArrayBuffer(2,{maxByteLength:4})",
        "new DataView(new SharedArrayBuffer(2,{maxByteLength:4}))",
    ] {
        let value = eval(scope, expression);
        assert!(
            moli_webidl::convert::<AllowSharedBufferSource>(
                scope,
                value,
                Context::argument("Test.buffer", 1)
            )
            .is_err(),
            "{expression}"
        );
    }
    for view in [false, true] {
        let value = eval(
            scope,
            if view {
                "globalThis.resizable=new ArrayBuffer(2,{maxByteLength:4}); new DataView(resizable)"
            } else {
                "globalThis.resizable=new ArrayBuffer(2,{maxByteLength:4}); resizable"
            },
        );
        let buffer = v8::Local::<v8::ArrayBuffer>::try_from(eval(scope, "resizable")).unwrap();
        buffer.detach(None).unwrap();
        assert!(
            moli_webidl::convert::<AllowSharedBufferSource>(
                scope,
                value,
                Context::argument("Test.buffer", 1)
            )
            .is_err(),
            "detached resizable view={view}"
        );
    }
}
