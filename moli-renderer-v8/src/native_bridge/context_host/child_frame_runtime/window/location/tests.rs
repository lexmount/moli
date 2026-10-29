use super::*;

fn interrupt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    scope.terminate_execution();
}

#[test]
fn failed_location_initialization_does_not_return_an_unwrapped_target() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let target = new_cross_origin_location_proxy_target(scope).unwrap();
    assert_eq!(
        target.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
        Some(true)
    );

    assert!(initialize_cross_origin_location_proxy(scope, target).is_none());
    assert!(get_private_value(scope, target, CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT).is_none());
}

#[test]
fn interrupted_location_initialization_can_retry_without_publishing_partial_identity() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let backing = new_cross_origin_location_proxy_target(scope).unwrap();
    let global = context.global(scope);
    assert_eq!(
        global.set(scope, v8str(scope, "backing").into(), backing.into()),
        Some(true)
    );
    let interrupt = v8::FunctionTemplate::new(scope, interrupt)
        .get_function(scope)
        .unwrap();
    assert_eq!(
        global.set(scope, v8str(scope, "interrupt").into(), interrupt.into()),
        Some(true)
    );
    // Interrupt a real V8 define-property operation after the first key. The
    // proxy is only a test input; production Location targets are plain objects.
    let source = v8str(
        scope,
        "new Proxy(backing, { defineProperty(target, key, descriptor) { \
         if (key === 'replace') interrupt(); \
         return Reflect.defineProperty(target, key, descriptor); } })",
    );
    let script = v8::Script::compile(scope, source, None).unwrap();
    let target = crate::script_execution::execute_compiled_script(scope, script).unwrap();
    let target = v8::Local::<v8::Object>::try_from(target).unwrap();
    {
        v8::tc_scope!(let scope, scope);
        assert!(initialize_cross_origin_location_proxy(scope, target).is_none());
        assert!(scope.has_terminated());
        assert!(!scope.can_continue());
        scope.cancel_terminate_execution();
    }
    assert_eq!(
        backing.has_own_property(scope, v8str(scope, "href").into()),
        Some(true)
    );
    // V8 may finish the current definition before reporting termination, but
    // later keys and the completed Location identity must remain unpublished.
    assert_eq!(
        backing.has_own_property(scope, v8str(scope, "then").into()),
        Some(false)
    );
    assert!(get_private_value(scope, backing, CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT).is_none());

    let location = initialize_cross_origin_location_proxy(scope, backing).unwrap();
    assert!(v8::Local::<v8::Proxy>::try_from(v8::Local::<v8::Value>::from(location)).is_ok());
    let identity =
        get_private_value(scope, backing, CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT).unwrap();
    assert!(identity.strict_equals(location.into()));
}
