use crate::embedding::realm_runtime;

pub(super) fn streams_owner_terminating(scope: &mut v8::PinScope<'_, '_>) -> bool {
    realm_runtime(scope).is_some_and(|callbacks| (callbacks.owner_terminating)(scope))
}

pub(super) fn structured_clone_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    (realm_runtime(scope)?.structured_clone_value)(scope, value)
}
