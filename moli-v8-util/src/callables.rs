use crate::throw_type_error;

/// ECMAScript GetFunctionRealm follows bound and Proxy targets without calling
/// author property traps. In particular, a bound Proxy's creation context need
/// not be the underlying function's realm.
pub fn callable_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut callable: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Context>> {
    loop {
        if let Ok(proxy) = v8::Local::<v8::Proxy>::try_from(callable) {
            if proxy.is_revoked() {
                throw_type_error(scope, "Cannot determine the realm of a revoked Proxy");
                return None;
            }
            callable = proxy.get_target(scope);
            continue;
        }
        if let Ok(function) = v8::Local::<v8::Function>::try_from(callable) {
            let target = function.get_bound_function(scope);
            if !target.is_undefined() {
                callable = target;
                continue;
            }
        }
        return v8::Local::<v8::Object>::try_from(callable)
            .ok()?
            .get_creation_context(scope);
    }
}
