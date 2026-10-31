use moli_webidl::Context;

#[test]
fn buffer_source_partial_writes_preserve_view_bounds_and_reject_overflow() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    for source in [
        "new DataView(new ArrayBuffer(8),2,4)",
        "new DataView(new SharedArrayBuffer(8),2,4)",
    ] {
        let source = v8::String::new(scope, source).unwrap();
        let value = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let buffer = moli_webidl::convert::<moli_webidl::AllowSharedBufferSource>(
            scope,
            value,
            Context::argument("Test", 1),
        )
        .unwrap();
        assert!(buffer.write_bytes(scope, &[7, 7, 7, 7]));
        assert!(buffer.write_bytes_at(scope, 1, &[1, 2]));
        assert_eq!(buffer.to_vec(scope), vec![7, 1, 2, 7]);
        assert!(!buffer.write_bytes_at(scope, 3, &[1, 2]));
        assert!(!buffer.write_bytes_at(scope, usize::MAX, &[1]));
        assert!(buffer.write_bytes_at(scope, 4, &[]));
        assert_eq!(buffer.to_vec(scope), vec![7, 1, 2, 7]);
    }
}
