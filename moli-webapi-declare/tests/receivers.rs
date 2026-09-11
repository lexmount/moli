use std::pin::pin;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{get_private_value, set_private_value, v8str};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const BRAND: &str = "__receiverTestBrand";
const OTHER_BRAND: &str = "__receiverTestOtherBrand";
const PROMISE: &str = "__receiverTestPromise";

fn has_brand<'s>(scope: &mut v8::PinScope<'s, '_>, receiver: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, receiver, BRAND).is_some()
}

fn has_other_brand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, OTHER_BRAND).is_some()
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let brand = v8::Boolean::new(scope, true);
    set_private_value(scope, args.this(), BRAND, brand.into());
    let resolver = v8::PromiseResolver::new(scope).unwrap();
    resolver.resolve(scope, args.this().into()).unwrap();
    let promise = resolver.get_promise(scope);
    set_private_value(scope, args.this(), PROMISE, promise.into());
}

fn callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    // This observable conversion must never precede the receiver check.
    if args.length() > 0 && args.get(0).to_string(scope).is_none() {
        return;
    }
    rv.set(args.data());
}

fn promise_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if args.length() > 0 && args.get(0).to_string(scope).is_none() {
        return;
    }
    rv.set(get_private_value(scope, args.this(), PROMISE).unwrap());
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = interfaces::NativeSample, constructor_callback = constructor, receiver = has_brand, enumerable)]
struct NativeSample {
    #[webapi(method, callback = callback, length = 1, data = 7)]
    method: (),
    #[webapi(alias = "method")]
    alias: (),
    #[webapi(accessor_property, getter = callback, setter = callback, data = 9, setter_data = 11)]
    value: (),
    #[webapi(method, callback = promise_callback, returns_promise)]
    load: (),
    #[webapi(accessor_property, getter = promise_callback, setter = callback, returns_promise)]
    ready: (),
    #[webapi(method, callback = callback, receiver = has_other_brand, data = 13)]
    other: (),
    #[webapi(static_method, callback = callback, data = 17)]
    static_value: (),
    #[webapi(static_method, callback = callback, returns_promise)]
    static_promise: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "PlainInterface", receiver = has_brand)]
struct PlainInterface {
    #[webapi(method, callback = callback, data = 7)]
    method: (),
    #[webapi(accessor_property, getter = callback, setter = callback, data = 9)]
    value: (),
    #[webapi(accessor_property, getter = promise_callback, returns_promise)]
    ready: (),
}

#[derive(WebApiObject)]
#[webapi(interface = interfaces::PlainInterface, receiver = has_brand)]
struct PlainObject {
    #[webapi(slot = BRAND)]
    brand: bool,
    #[webapi(method, callback = callback, data = 7)]
    method: (),
    #[webapi(accessor_property, getter = callback, setter = callback, data = 9)]
    value: (),
    #[webapi(accessor_property, getter = promise_callback, returns_promise)]
    ready: (),
}

fn run_script<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}

fn install<'s>(scope: &mut v8::PinScope<'s, '_>) {
    let template = NativeSample::build(scope);
    let constructor = template.get_function(scope).unwrap();
    let global = scope.get_current_context().global(scope);
    global
        .set(
            scope,
            v8str(scope, "NativeSample").into(),
            constructor.into(),
        )
        .unwrap();
    let key = v8::Boolean::new(scope, true);
    let other = v8::Object::new(scope);
    set_private_value(scope, other, OTHER_BRAND, key.into());
    global
        .set(scope, v8str(scope, "other").into(), other.into())
        .unwrap();
}

const ASSERTIONS: &str = r#"
    function assert(ok, message) { if (!ok) throw new Error(message); }
    function throwsTypeError(fn) {
        try { fn(); } catch (error) { return error instanceof TypeError; }
        return false;
    }
"#;

#[test]
fn template_receiver_checks_preserve_data_descriptors_and_precede_conversion() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    run_script(scope, ASSERTIONS);
    run_script(
        scope,
        r#"
        const proto = NativeSample.prototype;
        const descriptor = Object.getOwnPropertyDescriptor(proto, 'value');
        const real = new NativeSample();
        class Derived extends NativeSample {}
        assert(new Derived().method() === 7, 'native subclass');
        assert(real.value === 9 && real.method() === 7, 'callback data');
        assert(descriptor.set.call(real, 'ok') === 11, 'separate setter data');
        assert(descriptor.get.name === 'get value' && descriptor.set.length === 1, 'accessor metadata');
        assert(descriptor.enumerable && descriptor.configurable, 'accessor flags');
        assert(proto.method.length === 1 && /\[native code\]/.test(proto.method.toString()), 'native method');
        assert(proto.alias === proto.method, 'aliases retain checked function identity');
        assert(NativeSample.staticValue.call({}) === 17, 'static receiver is unchecked');
        assert(proto.other.call(other) === 13, 'field receiver overrides default');
        assert(throwsTypeError(() => proto.other.call(real)), 'override rejects default brand');
        let conversions = 0;
        const input = { toString() { conversions++; return 'x'; } };
        const bad = [proto, {}, Object.create(proto), new Proxy(real, {}), null, undefined, 7];
        const revoked = Proxy.revocable(real, {});
        revoked.revoke();
        bad.push(revoked.proxy);
        for (const receiver of bad) {
            assert(throwsTypeError(() => proto.method.call(receiver, input)), 'method rejects');
            assert(throwsTypeError(() => descriptor.get.call(receiver)), 'getter rejects');
            assert(throwsTypeError(() => descriptor.set.call(receiver, input)), 'setter rejects');
        }
        assert(conversions === 0, 'brand before conversion');
        const original = new Error('conversion');
        let thrown;
        try { descriptor.set.call(real, { toString() { throw original; } }); } catch (e) { thrown = e; }
        assert(thrown === original, 'valid setter preserves conversion exception');
        Object.setPrototypeOf(real, null);
        assert(proto.method.call(real) === 7, 'brand is independent of mutable prototype');
        let traps = 0;
        const proxy = new Proxy(real, {
            get() { traps++; }, getPrototypeOf() { traps++; }, has() { traps++; }
        });
        assert(throwsTypeError(() => proto.method.call(proxy)), 'proxy is not the native receiver');
        assert(traps === 0, 'brand check does not execute page code');
    "#,
    );
}

#[test]
fn promise_members_reject_all_synchronous_errors_and_preserve_success_identity() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    run_script(scope, ASSERTIONS);
    let result = run_script(
        scope,
        r#"(async () => {
        const proto = NativeSample.prototype;
        const descriptor = Object.getOwnPropertyDescriptor(proto, 'ready');
        const real = new NativeSample();
        assert(real.ready === real.ready && real.load() === real.ready, 'cached Promise identity');
        for (const receiver of [{}, proto, Object.create(proto), new Proxy(real, {}), null]) {
            for (const invoke of [() => descriptor.get.call(receiver), () => proto.load.call(receiver)]) {
                const promise = invoke();
                assert(promise instanceof Promise, 'invalid receiver must not throw synchronously');
                const reason = await promise.then(() => null, e => e);
                assert(reason instanceof TypeError, 'Promise rejects with TypeError');
            }
        }
        const original = new Error('conversion');
        const input = { toString() { throw original; } };
        for (const promise of [real.load(input), NativeSample.staticPromise(input)]) {
            assert(promise instanceof Promise, 'callback exception becomes Promise');
            assert(await promise.then(() => null, e => e) === original, 'preserve exception identity');
        }
        assert(throwsTypeError(() => descriptor.set.call({}, input)), 'Promise attribute setter still throws');
        assert(await real.ready === real, 'successful load result');
        return true;
    })()"#,
    );
    let promise = v8::Local::<v8::Promise>::try_from(result).unwrap();
    scope.perform_microtask_checkpoint();
    assert_eq!(
        promise.state(),
        v8::PromiseState::Fulfilled,
        "{}",
        promise
            .result(scope)
            .to_string(scope)
            .unwrap()
            .to_rust_string_lossy(scope)
    );
    assert!(promise.result(scope).is_true());
}

#[test]
fn object_and_template_declarations_use_the_same_receiver_policy() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    let constructor = PlainInterface::build(scope).get_function(scope).unwrap();
    global
        .define_own_property(
            scope,
            v8str(scope, "PlainInterface").into(),
            constructor.into(),
            v8::PropertyAttribute::DONT_ENUM,
        )
        .unwrap();
    let object = PlainObject::new(true).bind(scope).unwrap();
    global
        .set(scope, v8str(scope, "object").into(), object.into())
        .unwrap();
    run_script(scope, ASSERTIONS);
    run_script(
        scope,
        r#"
        for (const surface of [object, PlainInterface.prototype]) {
            assert(surface.method.call(object) === 7, 'valid receiver');
            assert(throwsTypeError(() => surface.method.call({})), 'invalid receiver');
            const value = Object.getOwnPropertyDescriptor(surface, 'value');
            assert(value.get.call(object) === 9, 'getter data');
            assert(throwsTypeError(() => value.set.call({})), 'invalid setter receiver');
            const promise = Object.getOwnPropertyDescriptor(surface, 'ready').get.call({});
            assert(promise instanceof Promise, 'invalid Promise getter receiver');
            promise.catch(() => {});
        }
    "#,
    );
    scope.perform_microtask_checkpoint();
}

#[test]
fn cross_realm_receiver_validation_uses_the_callee_error_and_promise_realm() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let child = v8::Context::new(scope, Default::default());
    let token = v8str(scope, "same-origin");
    child.set_security_token(token.into());
    {
        let scope = &mut v8::ContextScope::new(scope, child);
        install(scope);
        run_script(scope, "globalThis.real = new NativeSample()");
    }
    let context = v8::Context::new(scope, Default::default());
    context.set_security_token(token.into());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    let child_global = child.global(scope);
    context
        .global(scope)
        .set(scope, v8str(scope, "child").into(), child_global.into())
        .unwrap();
    run_script(scope, ASSERTIONS);
    let result = run_script(
        scope,
        r#"(async () => {
        assert(NativeSample.prototype.method.call(child.real) === 7, 'cross-realm receiver');
        assert(child.NativeSample.prototype.method.call(new NativeSample()) === 7, 'reverse cross-realm receiver');
        let caught;
        try { child.NativeSample.prototype.method.call({}); } catch (e) { caught = e; }
        assert(caught instanceof child.TypeError && !(caught instanceof TypeError), 'callee TypeError realm');
        const ready = Object.getOwnPropertyDescriptor(child.NativeSample.prototype, 'ready').get;
        for (const promise of [ready.call({}), child.NativeSample.prototype.load.call({})]) {
            assert(promise instanceof child.Promise && !(promise instanceof Promise), 'callee Promise realm');
            const error = await promise.catch(e => e);
            assert(error instanceof child.TypeError && !(error instanceof TypeError), 'rejection realm');
        }
        assert(ready.call(new NativeSample()) instanceof Promise, 'successful Promise is not rewrapped');
        return true;
    })()"#,
    );
    let promise = v8::Local::<v8::Promise>::try_from(result).unwrap();
    scope.perform_microtask_checkpoint();
    assert_eq!(
        promise.state(),
        v8::PromiseState::Fulfilled,
        "{}",
        promise
            .result(scope)
            .to_string(scope)
            .unwrap()
            .to_rust_string_lossy(scope)
    );
    assert!(promise.result(scope).is_true());
}

#[test]
fn promise_callbacks_do_not_convert_execution_termination_into_rejection() {
    fn terminate(
        scope: &mut v8::PinScope<'_, '_>,
        _args: v8::FunctionCallbackArguments<'_>,
        _rv: v8::ReturnValue<'_>,
    ) {
        scope.terminate_execution();
    }

    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    let terminate = v8::Function::new(scope, terminate).unwrap();
    context
        .global(scope)
        .set(scope, v8str(scope, "terminate").into(), terminate.into())
        .unwrap();
    let source = v8str(
        scope,
        // TerminateExecution requests an interrupt; reach a V8 interrupt check
        // before the conversion returns, instead of relying on a short call to
        // happen to consume the request.
        "new NativeSample().load({toString() { terminate(); for (;;) {} }})",
    );
    let script = v8::Script::compile(scope, source, None).unwrap();
    {
        v8::tc_scope!(let scope, scope);
        let result = script.run(scope);
        assert!(
            result.is_none(),
            "promise={:?}, caught={}, terminated={}, terminating={}",
            result.map(|value| value.is_promise()),
            scope.has_caught(),
            scope.has_terminated(),
            scope.is_execution_terminating()
        );
        assert!(scope.has_terminated());
        assert!(!scope.can_continue());
    }
    scope.cancel_terminate_execution();
    assert!(run_script(scope, "new NativeSample().method() === 7").is_true());
}

mod interfaces {
    moli_webapi_declare::declare_web_api_interfaces! {
        pub(super) NativeSample;
        pub(super) PlainInterface;
    }
}
