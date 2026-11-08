use std::pin::pin;

#[test]
fn backtrack_limits_bound_rejections_and_preserve_normal_matches() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = v8::String::new(scope, "^(a+)+$").unwrap();
    let matching = v8::String::new(scope, &"a".repeat(64)).unwrap();
    let rejecting = v8::String::new(scope, &format!("{}!", "a".repeat(64))).unwrap();
    for limit in [1, 1_000, 1_000_000] {
        let regexp = v8::RegExp::new_with_backtrack_limit(
            scope,
            source,
            v8::RegExpCreationFlags::UNICODE_SETS,
            limit,
        )
        .unwrap();
        // Without the explicit limit, this rejection requires exponential
        // backtracking. Do not assume an exact count for an optimizable match.
        assert!(regexp.exec(scope, rejecting).unwrap().is_null());
        if limit == 1_000_000 {
            assert!(!regexp.exec(scope, matching).unwrap().is_null());
        }
    }
    let unbounded = v8::RegExp::new(scope, source, v8::RegExpCreationFlags::UNICODE_SETS).unwrap();
    assert!(!unbounded.exec(scope, matching).unwrap().is_null());
}

#[test]
fn invalid_backtrack_limits_are_rejected_without_a_process_abort() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let try_catch = pin!(v8::TryCatch::new(scope));
    let scope = &mut try_catch.init();
    let source = v8::String::new(scope, "a").unwrap();
    for limit in [0, u32::MAX] {
        assert!(
            v8::RegExp::new_with_backtrack_limit(
                scope,
                source,
                v8::RegExpCreationFlags::empty(),
                limit,
            )
            .is_none()
        );
        assert!(!scope.has_caught());
    }
    let invalid_source = v8::String::new(scope, "[").unwrap();
    assert!(
        v8::RegExp::new_with_backtrack_limit(
            scope,
            invalid_source,
            v8::RegExpCreationFlags::UNICODE_SETS,
            1_000,
        )
        .is_none()
    );
    assert!(scope.has_caught());
}
