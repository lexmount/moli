use moli_webidl::{Context, WebIdlDictionary};

#[derive(WebIdlDictionary)]
#[webidl(prefix = "LongLongInit")]
struct Init {
    #[webidl(default = 0)]
    value: i64,
    #[webidl(converter = "long_long", default = 0)]
    explicit: i64,
}

#[test]
fn long_long_derive_wraps_and_preserves_conversion_errors() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    for (expression, expected) in [
        ("-17.9", -17),
        ("NaN", 0),
        ("Infinity", 0),
        ("undefined", 0),
        ("2**63", i64::MIN),
        ("2**64", 0),
        ("-(2**63)-2048", i64::MAX - 2047),
    ] {
        let source = v8::String::new(
            scope,
            &format!("({{value:{expression},explicit:{expression}}})"),
        )
        .unwrap();
        let value = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let init =
            moli_webidl::parse_dictionary::<Init>(scope, value, Context::argument("Test", 1))
                .unwrap()
                .unwrap();
        assert_eq!(init.value, expected, "{expression}");
        assert_eq!(init.explicit, expected, "{expression}");
    }
    for source in [
        "({value:Symbol(),explicit:0})",
        "({value:0,explicit:1n})",
        "({get explicit(){throw 42},value:0})",
    ] {
        v8::tc_scope!(let scope, scope);
        let source = v8::String::new(scope, source).unwrap();
        let value = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(
            moli_webidl::parse_dictionary::<Init>(scope, value, Context::argument("Test", 1))
                .is_err()
        );
    }
}

#[test]
fn buffer_source_row_writes_preserve_padding_and_reject_out_of_bounds() {
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

#[derive(WebIdlDictionary)]
#[webidl(prefix = "Plane")]
struct Plane {
    #[webidl(required)]
    offset: u32,
}
#[derive(WebIdlDictionary)]
#[webidl(prefix = "Planes")]
struct Planes {
    #[webidl(sequence, converter = "dictionary")]
    layout: Option<Vec<Plane>>,
}
#[test]
fn sequence_dictionary_conversion_observes_iteration_and_required_members() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    for (source, expected) in [
        ("({layout:[{offset:3},{offset:7}]})", Some(vec![3, 7])),
        ("({layout:[]})", Some(vec![])),
        ("({})", None),
    ] {
        let source = v8::String::new(scope, source).unwrap();
        let value = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let planes =
            moli_webidl::parse_dictionary::<Planes>(scope, value, Context::argument("Test", 1))
                .unwrap()
                .unwrap();
        assert_eq!(
            planes
                .layout
                .map(|list| list.into_iter().map(|p| p.offset).collect::<Vec<_>>()),
            expected
        );
    }
    for source in ["({layout:[null]})", "({layout:[{}]})", "({layout:{}})"] {
        let source = v8::String::new(scope, source).unwrap();
        let value = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(
            moli_webidl::parse_dictionary::<Planes>(scope, value, Context::argument("Test", 1))
                .is_err()
        );
    }
}
