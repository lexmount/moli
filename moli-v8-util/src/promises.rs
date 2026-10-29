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
