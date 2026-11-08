use moli_webidl::{Context, WebIdlDictionary};

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

#[test]
fn dictionary_sequence_converts_each_member_before_advancing_the_iterator() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = v8::String::new(
        scope,
        r#"
      globalThis.reads = [];
      ({
        get layout() {
          reads.push('layout');
          return {
            [Symbol.iterator]() {
              reads.push('iterator');
              let index = 0;
              return {
                next() {
                  reads.push('next' + index);
                  if (index++ !== 0) return { done: true };
                  return { value: { get offset() { reads.push('offset'); return 3; } } };
                }
              };
            }
          };
        }
      })
    "#,
    )
    .unwrap();
    let value = v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap();
    let parsed =
        moli_webidl::parse_dictionary::<Planes>(scope, value, Context::argument("Test", 1))
            .unwrap()
            .unwrap();
    assert_eq!(parsed.layout.unwrap()[0].offset, 3);
    let source = v8::String::new(scope, "JSON.stringify(reads)").unwrap();
    let reads = v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap();
    assert_eq!(
        reads.to_rust_string_lossy(scope),
        r#"["layout","iterator","next0","offset","next1"]"#
    );
}
