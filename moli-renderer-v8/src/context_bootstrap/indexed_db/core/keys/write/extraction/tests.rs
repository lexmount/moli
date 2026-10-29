use super::*;

fn evaluate<'s>(scope: &mut v8::PinScope<'s, '_>, text: &'static str) -> v8::Local<'s, v8::Value> {
    let text = v8str(scope, text);
    let script = v8::Script::compile(scope, text, None).unwrap();
    crate::script_execution::execute_compiled_script(scope, script).unwrap()
}

fn interrupt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    scope.terminate_execution();
}

#[test]
fn index_exceptions_omit_only_the_affected_key_or_multientry_member() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = evaluate(
        scope,
        r#"(() => {
        Object.defineProperty(Array.prototype, '0', {configurable: true, get() {throw 41}});
        return { tags: [, 1, 1, 0, -0], valid: 9, get bad() {throw 42} };
    })()"#,
    );
    v8::tc_scope!(let scope, scope);
    let keys =
        extract_index_keys_from_value(scope, value, &KeyPath::String("tags".into()), true).unwrap();
    assert_eq!(
        keys,
        vec![Key::number(0.0).unwrap(), Key::number(1.0).unwrap()]
    );
    assert!(!scope.has_caught());
    for path in [
        KeyPath::String("bad".into()),
        KeyPath::Sequence(vec!["valid".into(), "bad".into()]),
    ] {
        assert!(
            extract_index_keys_from_value(scope, value, &path, false)
                .unwrap()
                .is_empty()
        );
        assert!(!scope.has_caught());
    }
    assert_eq!(
        extract_index_keys_from_value(scope, value, &KeyPath::String("valid".into()), false)
            .unwrap(),
        vec![Key::number(9.0).unwrap()]
    );
}

#[test]
fn inherited_multientry_values_remain_visible_but_inherited_key_paths_do_not() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = evaluate(
        scope,
        r#"(() => {
        Object.defineProperty(Array.prototype, '0', {configurable: true, get() {return 3}});
        return Object.assign(Object.create({inherited: 7}), {tags: [, 1]});
    })()"#,
    );
    assert_eq!(
        extract_index_keys_from_value(scope, value, &KeyPath::String("tags".into()), true).unwrap(),
        vec![Key::number(1.0).unwrap(), Key::number(3.0).unwrap()]
    );
    assert!(
        extract_index_keys_from_value(scope, value, &KeyPath::String("inherited".into()), false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn key_path_and_multientry_termination_reaches_the_execution_boundary() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    let callback = v8::Function::new(scope, interrupt).unwrap();
    assert_eq!(
        global.create_data_property(scope, v8str(scope, "interrupt").into(), callback.into()),
        Some(true)
    );
    let boundary = v8::Script::compile(scope, v8str(scope, "0"), None).unwrap();
    for mode in 0..4 {
        let value = evaluate(
            scope,
            r#"({get bad() {interrupt(); while (true) {}}, tags: Object.defineProperty([1], '0', {get() {interrupt(); while (true) {}}})})"#,
        );
        let object = value.to_object(scope).unwrap();
        let array = object.get(scope, v8str(scope, "tags").into()).unwrap();
        {
            v8::tc_scope!(let scope, scope);
            match mode {
                0 | 1 => {
                    let result = extract_index_keys_from_value(
                        scope,
                        value,
                        &KeyPath::String(if mode == 0 { "bad" } else { "tags" }.into()),
                        mode == 1,
                    );
                    assert!(
                        matches!(result, Err(KeyConversionError::Terminated)),
                        "mode {mode}: {result:?}"
                    );
                }
                2 => assert!(matches!(
                    extract_object_store_key_from_value(
                        scope,
                        value,
                        &KeyPath::String("bad".into())
                    ),
                    ObjectStoreKey::Failed
                )),
                _ => assert!(matches!(
                    crate::context_bootstrap::indexed_db::parse_idb_key(scope, array),
                    Err(crate::context_bootstrap::indexed_db::KeyConversionError::Terminated)
                )),
            }
            if !scope.has_terminated() {
                assert!(
                    scope.is_execution_terminating(),
                    "mode {mode} lost termination"
                );
                assert!(
                    crate::script_execution::execute_compiled_script(scope, boundary).is_none()
                );
            }
            assert!(
                scope.has_terminated(),
                "mode {mode} swallowed script termination"
            );
            assert!(!scope.can_continue());
            scope.cancel_terminate_execution();
        }
    }
}

#[test]
fn budgeted_index_keys_retain_only_output_and_reuse_duplicate_capacity() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = evaluate(
        scope,
        r#"({tags:[-0,0,new Date(0),'same','same','\ud800',new Uint8Array([1,2]),[1,'nested'],NaN]})"#,
    );
    let mut remaining = 8192u64;
    let keys = extract_index_keys_budgeted(
        scope,
        value,
        &KeyPath::String("tags".into()),
        true,
        &mut remaining,
    )
    .unwrap();
    assert_eq!(
        keys,
        vec![
            Key::number(0.0).unwrap(),
            Key::Date(0),
            Key::from("same"),
            Key::String(vec![0xd800]),
            Key::Binary(vec![1, 2]),
            Key::Array(vec![Key::number(1.0).unwrap(), Key::from("nested")]),
        ]
    );
    let bytes = keys.capacity() * std::mem::size_of::<Key>()
        + keys.iter().map(Key::heap_bytes).sum::<usize>();
    assert_eq!(8192 - remaining, bytes as u64);
    drop(keys);
    remaining.refund(bytes as u64);
    assert_eq!(remaining, 8192);
    let value = evaluate(scope, "({tags:Array(2000).fill('same')})");
    let mut remaining = 2048u64;
    let keys = extract_index_keys_budgeted(
        scope,
        value,
        &KeyPath::String("tags".into()),
        true,
        &mut remaining,
    )
    .unwrap();
    assert_eq!(keys, [Key::from("same")]);
    assert_eq!(
        2048 - remaining,
        (keys.capacity() * std::mem::size_of::<Key>() + keys[0].heap_bytes()) as u64
    );
}

#[test]
fn invalid_or_throwing_index_conversion_refunds_partial_native_allocations() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    for code in [
        "({first:'a'.repeat(200),second:[1,,3]})",
        "({first:'a'.repeat(200),get second(){throw 42}})",
        "(() => { const cycle = ['a']; cycle.push(cycle); return {first:'a'.repeat(200),second:cycle}; })()",
    ] {
        let value = evaluate(scope, code);
        let mut remaining = 4096u64;
        let keys = extract_index_keys_budgeted(
            scope,
            value,
            &KeyPath::Sequence(vec!["first".into(), "second".into()]),
            false,
            &mut remaining,
        )
        .unwrap();
        assert!(keys.is_empty());
        assert_eq!(remaining, 4096);
    }
}

#[test]
fn index_key_budget_failure_stops_before_later_getters_and_is_not_an_invalid_key() {
    let mut remaining = 4096u64;
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = evaluate(
        scope,
        r#"(() => {
        globalThis.visits = 0;
        const tags = ['x'.repeat(65536)];
        Object.defineProperty(tags, '1', {get(){visits++; return 'later'}});
        return {tags};
    })()"#,
    );
    let error = extract_index_keys_budgeted(
        scope,
        value,
        &KeyPath::String("tags".into()),
        true,
        &mut remaining,
    )
    .unwrap_err();
    assert!(matches!(error, KeyConversionError::Backend(_)));
    assert_eq!(evaluate(scope, "visits").integer_value(scope), Some(0));
    assert_eq!(remaining, 4096);
}
