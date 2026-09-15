use std::pin::pin;

use super::*;

#[test]
fn failed_child_eval_capture_never_exposes_the_intrinsic_eval() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let source = v8str(
        scope,
        r#"Object.defineProperty({}, "eval", {
          configurable: true,
          get() { throw new Error("blocked eval read"); }
        })"#,
    );
    let window = v8::Script::compile(scope, source, None)
        .and_then(|script| crate::script_execution::execute_compiled_script(scope, script))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("Window-like object with a throwing eval getter");
    let eval_key = v8str(scope, "eval");
    let intrinsic_eval = context
        .global(scope)
        .get(scope, eval_key.into())
        .expect("intrinsic eval");
    set_private_value(scope, window, WINDOW_INTRINSIC_EVAL_SLOT, intrinsic_eval);

    let error = {
        let try_catch = pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        evaluate_child_window_expression(&mut scope, window, DomHandle::new(1), "1 + 1")
            .expect_err("the throwing eval getter must abort before mutation")
    };
    assert_eq!(error.to_string(), "failed to capture child Window eval");

    let descriptor = window
        .get_own_property_descriptor(scope, eval_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("eval accessor descriptor");
    assert!(
        descriptor
            .get(scope, v8str(scope, "get").into())
            .is_some_and(|value| value.is_function()),
        "the original accessor must remain installed"
    );
}

#[test]
fn child_eval_error_restores_the_wrapper_before_returning() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let window = v8::Object::new(scope);
    let eval_key = v8str(scope, "eval");
    let intrinsic_eval = context
        .global(scope)
        .get(scope, eval_key.into())
        .expect("intrinsic eval");
    let wrapper = v8::Script::compile(
        scope,
        v8str(scope, "(function childEvalWrapper() {})"),
        None,
    )
    .and_then(|script| crate::script_execution::execute_compiled_script(scope, script))
    .expect("child eval wrapper");
    assert_eq!(
        window.define_own_property(
            scope,
            eval_key.into(),
            wrapper,
            v8::PropertyAttribute::DONT_ENUM,
        ),
        Some(true)
    );
    set_private_value(scope, window, WINDOW_INTRINSIC_EVAL_SLOT, intrinsic_eval);

    evaluate_child_window_expression(
        scope,
        window,
        DomHandle::new(1),
        "throw new Error('child eval failure')",
    )
    .expect_err("the author expression should fail");

    assert!(
        window
            .get(scope, eval_key.into())
            .is_some_and(|value| value.strict_equals(wrapper)),
        "the child eval wrapper must be restored on the error path"
    );
}
