use moli_v8_util::v8str;

type MemberCallback = for<'s, 'i> fn(
    &mut v8::PinScope<'s, 'i>,
    v8::FunctionCallbackArguments<'s>,
    v8::ReturnValue<'s>,
);

pub fn throw_illegal_invocation(scope: &mut v8::PinScope<'_, '_>) {
    let message = v8str(scope, "Illegal invocation");
    let exception = v8::Exception::type_error(scope, message);
    scope.throw_exception(exception);
}

/// Like Blink's ExceptionToRejectPromiseScope, this covers both receiver checks
/// and exceptions thrown by the implementation (including argument conversion).
/// It does not wrap successful results, preserving cached Promise identity.
pub fn invoke_promise_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
    callback: MemberCallback,
) {
    v8::tc_scope!(let scope, scope);
    callback(scope, args, rv);
    // Termination is not a catchable WebIDL exception. Leave it to V8.
    if !scope.can_continue() {
        return;
    }
    let Some(exception) = scope.exception() else {
        return;
    };
    scope.reset();
    // The callback's current realm supplies both TypeError and Promise, even
    // when its receiver or caller belongs to a different realm.
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    if resolver.reject(scope, exception).is_some() {
        rv.set(resolver.get_promise(scope).into());
    }
}
