use crate::{registered_ecmascript_constructor, v8str};

/// Implements PromiseResolve(%Promise%, value) in the current realm.
pub fn ecmascript_promise_resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        let constructor = promise.get(scope, v8str(scope, "constructor").into())?;
        let global = scope.get_current_context().global(scope);
        let intrinsic = registered_ecmascript_constructor(scope, global, "Promise")?;
        if constructor.strict_equals(intrinsic.into()) {
            return Some(promise);
        }
    }
    let resolver = v8::PromiseResolver::new(scope)?;
    resolver.resolve(scope, value)?;
    Some(resolver.get_promise(scope))
}

/// Return a fresh native Promise resolved with the supplied value.
pub fn set_resolved_promise(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.resolve(scope, value);
    rv.set(promise.into());
}

/// Reject an existing native resolver using the current realm TypeError.
pub fn reject_promise_with_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let error = crate::v8_string(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}
