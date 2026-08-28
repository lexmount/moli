//! Realm-local function cache for HTML's CrossOriginPropertyDescriptorMap.
//!
//! The cache owner is V8's hidden extras-binding object for the accessing
//! Context. Values therefore have exactly Realm lifetime, are traced entirely
//! by V8, and cannot survive a stable WindowProxy being rebound to a new
//! Context. No Rust host/global cache participates in wrapper ownership.

use crate::util::{get_private_value, set_private_value, v8str};

pub(super) fn realm_local_cross_origin_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache_slot: &'static str,
    name: &'static str,
    length: i32,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<v8::Local<'s, v8::Function>> {
    let accessing_context = scope
        .get_incumbent_context()
        .unwrap_or_else(|| scope.get_current_context());
    let accessing_context = v8::Global::new(scope, accessing_context);
    let accessing_context = v8::Local::new(scope, &accessing_context);
    let accessing_scope = &mut v8::ContextScope::new(scope, accessing_context);
    let cache = accessing_context.get_extras_binding_object(accessing_scope);
    if let Some(function) = get_private_value(accessing_scope, cache, cache_slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        return Some(function);
    }

    let function = v8::Function::builder(callback)
        .length(length)
        .build(accessing_scope)?;
    function.set_name(v8str(accessing_scope, name));
    set_private_value(accessing_scope, cache, cache_slot, function.into());
    Some(function)
}
