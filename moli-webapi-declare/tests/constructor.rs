use std::pin::pin;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{
    WebApiIntrinsicKind, WebApiIntrinsicLookup, get_private_object,
    install_web_api_intrinsic_resolver, set_private_value, v8str,
};
use moli_webapi_declare::{
    capture_web_api_constructor_intrinsics, initialize_web_api_constructor_receiver,
    web_api_constructor_with_deferred_prototype,
};

moli_webapi_declare::declare_web_api_interfaces! {
    TestThing;
}

fn intrinsic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    kind: WebApiIntrinsicKind,
) -> WebApiIntrinsicLookup<'s> {
    if name != "TestThing" {
        return WebApiIntrinsicLookup::Unmanaged;
    }
    let global = scope.get_current_context().global(scope);
    WebApiIntrinsicLookup::Managed(get_private_object(
        scope,
        global,
        match kind {
            WebApiIntrinsicKind::Constructor => "test.constructor",
            WebApiIntrinsicKind::Prototype => "test.prototype",
        },
    ))
}

fn install<'s>(scope: &mut v8::PinScope<'s, '_>) {
    capture_web_api_constructor_intrinsics(scope).unwrap();
    let global = scope.get_current_context().global(scope);
    let template = v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
        TestThing,
        constructor
    ))
    .data(v8str(scope, "native callback data").into())
    .length(2)
    .build(scope);
    template.set_class_name(v8str(scope, "TestThing"));
    let native = template.get_function(scope).unwrap();
    let prototype = native.get(scope, v8str(scope, "prototype").into()).unwrap();
    let prototype = v8::Local::<v8::Object>::try_from(prototype).unwrap();
    set_private_value(scope, global, "test.constructor", native.into());
    set_private_value(scope, global, "test.prototype", prototype.into());
    install_web_api_intrinsic_resolver(scope, intrinsic);
    let public = web_api_constructor_with_deferred_prototype(scope, native).unwrap();
    prototype
        .set(scope, v8str(scope, "constructor").into(), public.into())
        .unwrap();
    global
        .set(scope, v8str(scope, "TestThing").into(), public.into())
        .unwrap();
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    assert!(args.is_construct_call());
    let Some(converted) = args.get(0).to_string(scope) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "TestThing") {
        return;
    }
    // A second native initialization must not invoke the author's getter twice.
    assert!(initialize_web_api_constructor_receiver(
        scope,
        args.this(),
        "TestThing"
    ));
    for (name, value) in [
        ("converted", converted.into()),
        ("originalNewTarget", args.new_target()),
        ("callbackData", args.data()),
        (
            "argumentCount",
            v8::Integer::new(scope, args.length()).into(),
        ),
    ] {
        args.this()
            .define_own_property(
                scope,
                v8str(scope, name).into(),
                value,
                v8::PropertyAttribute::NONE,
            )
            .unwrap();
    }
    rv.set(args.this().into());
}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}

#[test]
fn constructor_defers_prototype_get_and_preserves_native_callback_information() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    assert!(
        eval(
            scope,
            r#"(() => {
                const log = [];
                const prototype = Object.prototype;
                const N = new Proxy(function() {}, {get(target, key, receiver) {
                    if (key === 'prototype') { log.push('prototype'); return prototype; }
                    return Reflect.get(target, key, receiver);
                }});
                const argument = {toString() { log.push('conversion'); return 'value'; }};
                const value = Reflect.construct(TestThing, [argument, 1, 2], N);
                if (log.join(',') !== 'conversion,prototype'
                    || Object.getPrototypeOf(value) !== prototype
                    || value.converted !== 'value' || value.originalNewTarget !== N
                    || value.callbackData !== 'native callback data' || value.argumentCount !== 3
                    || TestThing.length !== 2 || TestThing.name !== 'TestThing'
                    || TestThing.prototype.constructor !== TestThing) return false;
                log.length = 0;
                const sentinel = {};
                try { Reflect.construct(TestThing, [{toString() { throw sentinel; }}], N); }
                catch (error) { if (error !== sentinel) return false; }
                if (log.length !== 0) return false;
                const badN = new Proxy(function() {}, {get(target, key) {
                    if (key === 'prototype') throw sentinel;
                    return Reflect.get(target, key);
                }});
                try { Reflect.construct(TestThing, [argument], badN); return false; }
                catch (error) { if (error !== sentinel) return false; }
                const nativeConstruct = Reflect.construct;
                Reflect.construct = () => { throw Error('public Reflect'); };
                Object.prototype.get = () => { throw Error('inherited trap'); };
                Object.prototype.construct = () => { throw Error('inherited construct'); };
                const C = TestThing;
                TestThing = {};
                const ordinary = new C('normal');
                return Object.getPrototypeOf(ordinary) === C.prototype
                    && ordinary.originalNewTarget === C
                    && nativeConstruct(C, ['again']).converted === 'again';
            })()"#,
        )
        .is_true()
    );
}

#[test]
fn constructor_uses_new_target_realm_and_keeps_reentrant_invocations_independent() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let first = v8::Context::new(scope, Default::default());
    let second = v8::Context::new(scope, Default::default());
    let token = v8str(scope, "same-origin constructor test");
    for context in [first, second] {
        context.set_security_token(token.into());
        install(&mut v8::ContextScope::new(scope, context));
    }
    let foreign = second.global(scope);
    let scope = &mut v8::ContextScope::new(scope, first);
    first
        .global(scope)
        .set(scope, v8str(scope, "other").into(), foreign.into())
        .unwrap();
    assert!(
        eval(
            scope,
            r#"(() => {
                const foreignPrototype = other.TestThing.prototype;
                const foreignTarget = other.Function();
                foreignTarget.prototype = 7;
                const bound = foreignTarget.bind(null);
                const proxiedBound = new Proxy(bound, {});
                const boundProxy = new Proxy(foreignTarget, {}).bind(null);
                for (const N of [foreignTarget, bound, proxiedBound, boundProxy]) {
                    const value = Reflect.construct(TestThing, ['x'], N);
                    if (Object.getPrototypeOf(value) !== foreignPrototype
                        || value.originalNewTarget !== N) return false;
                }
                const C = TestThing;
                other.TestThing = {prototype: {poisoned: true}};
                if (Object.getPrototypeOf(Reflect.construct(C, ['x'], foreignTarget))
                    !== foreignPrototype) return false;
                let reads = 0, nested;
                const prototype = Object.create(null);
                const N = new Proxy(function() {}, {get(target, key, receiver) {
                    if (key === 'prototype') {
                        if (++reads === 1) nested = Reflect.construct(C, ['nested'], N);
                        return prototype;
                    }
                    return Reflect.get(target, key, receiver);
                }});
                const value = Reflect.construct(C, ['outer'], N);
                if (reads !== 2 || Object.getPrototypeOf(value) !== prototype
                    || Object.getPrototypeOf(nested) !== prototype
                    || value.converted !== 'outer' || nested.converted !== 'nested') return false;
                for (const primitive of [true, false]) {
                    let revoke;
                    const pair = Proxy.revocable(function() {}, {get(target, key, receiver) {
                        if (key === 'prototype') { revoke(); return primitive ? 1 : prototype; }
                        return Reflect.get(target, key, receiver);
                    }});
                    revoke = pair.revoke;
                    try {
                        const instance = Reflect.construct(C, ['x'], pair.proxy);
                        if (primitive || Object.getPrototypeOf(instance) !== prototype) return false;
                    } catch (error) { if (!primitive || !(error instanceof TypeError)) return false; }
                }
                return true;
            })()"#,
        )
        .is_true()
    );
}
