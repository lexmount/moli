/// Follow GetFunctionRealm's bound and Proxy targets without author traps.
/// A bound Proxy's creation context need not be its target function's realm.
/// Revoked targets and unavailable contexts return None without an exception;
/// callers that require a realm must report that failure themselves.
pub fn callable_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut callable: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Context>> {
    loop {
        if let Ok(proxy) = v8::Local::<v8::Proxy>::try_from(callable) {
            if proxy.is_revoked() {
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

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use moli_v8_test_util::ensure_v8;

    use super::callable_relevant_context;
    use crate::v8str;

    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }

    #[test]
    fn callable_realms_follow_bound_proxy_targets_without_author_traps() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Context::new(scope, Default::default());
        let foreign = v8::Context::new(scope, Default::default());
        let function = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            eval(scope, "(function Foreign() {})")
        };
        let scope = &mut v8::ContextScope::new(scope, local);
        local
            .global(scope)
            .set(scope, v8str(scope, "foreignFunction").into(), function)
            .unwrap();
        let values = eval(
            scope,
            r#"(() => {
                globalThis.realmTraps = 0;
                const proxy = new Proxy(foreignFunction, {
                    getPrototypeOf() { ++realmTraps; return Function.prototype; },
                    get(target, key, receiver) {
                        ++realmTraps;
                        return Reflect.get(target, key, receiver);
                    }
                });
                const bound = Function.prototype.bind.call(proxy, null);
                const values = [foreignFunction, proxy, bound, new Proxy(bound, {}),
                    Function.prototype.bind.call(new Proxy(bound, {}), null)];
                realmTraps = 0;
                return values;
            })()"#,
        );
        let values = v8::Local::<v8::Array>::try_from(values).unwrap();
        for index in 0..values.length() {
            let value = values.get_index(scope, index).unwrap();
            assert_eq!(callable_relevant_context(scope, value), Some(foreign));
        }
        assert_eq!(eval(scope, "realmTraps").int32_value(scope), Some(0));
    }

    #[test]
    fn callable_realms_return_none_for_revoked_targets_without_an_exception() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let values = eval(
            scope,
            r#"(() => {
                const pair = Proxy.revocable(function() {}, {});
                const bound = pair.proxy.bind(null);
                pair.revoke();
                return [pair.proxy, bound, new Proxy(bound, {})];
            })()"#,
        );
        let values = v8::Local::<v8::Array>::try_from(values).unwrap();
        for index in 0..values.length() {
            let value = values.get_index(scope, index).unwrap();
            v8::tc_scope!(let scope, scope);
            assert!(callable_relevant_context(scope, value).is_none());
            assert!(!scope.has_caught());
        }
    }
}
