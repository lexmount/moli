use std::pin::pin;

use moli_v8_test_util::ensure_v8;
use moli_webapi_declare::{
    WebApiObject, implements_interface, initialize_web_api_object, register_web_api_interfaces,
    web_api_object_type,
};

#[derive(WebApiObject)]
#[webapi(interface = "TestBase", allow_empty)]
struct Base {}

#[derive(WebApiObject)]
#[webapi(interface = "TestDerived", parent = "TestBase", allow_empty)]
struct Derived {}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct Record {
    value: u32,
}

#[derive(WebApiObject)]
#[webapi(interface = "TestBase", unbranded, allow_empty)]
struct PrototypeMembers {}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}

#[test]
fn declarations_brand_instances_and_preserve_derived_identity_across_realms() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    register_web_api_interfaces(
        scope,
        [("TestBase", None), ("TestDerived", Some("TestBase"))],
    )
    .unwrap();
    let first = v8::Context::new(scope, Default::default());
    let second = v8::Context::new(scope, Default::default());
    let object = {
        let scope = &mut v8::ContextScope::new(scope, first);
        let object = Base::new().bind(scope).unwrap();
        Derived::new().initialize(scope, object).unwrap();
        Base::new().initialize(scope, object).unwrap();
        Record::new(7).initialize(scope, object).unwrap();
        assert_eq!(
            web_api_object_type(scope, object).unwrap().name(),
            "TestDerived"
        );
        assert!(implements_interface(scope, object, "TestBase"));
        assert!(implements_interface(scope, object, "TestDerived"));
        assert!(!implements_interface(scope, object, "Unrelated"));
        assert!(initialize_web_api_object(scope, object, "Unrelated").is_err());
        object
    };
    let scope = &mut v8::ContextScope::new(scope, second);
    assert!(implements_interface(scope, object, "TestBase"));
    let key = v8::String::new(scope, "real").unwrap();
    second
        .global(scope)
        .set(scope, key.into(), object.into())
        .unwrap();
    eval(
        scope,
        "Object.setPrototypeOf(real, null); real.__moliWebApiType = 'Unrelated'",
    );
    assert_eq!(
        web_api_object_type(scope, object).unwrap().name(),
        "TestDerived"
    );
    assert!(implements_interface(scope, object, "TestBase"));
}

#[test]
fn identity_is_own_private_and_never_invokes_author_code() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let real = Base::new().bind(scope).unwrap();
    let key = v8::String::new(scope, "real").unwrap();
    context
        .global(scope)
        .set(scope, key.into(), real.into())
        .unwrap();
    assert_eq!(
        eval(scope, "Reflect.ownKeys(real).length").uint32_value(scope),
        Some(0)
    );
    for source in [
        "({__moliWebApiType: 0})",
        "({[Symbol('__moliWebApiType')]: 0})",
        "Object.create(real)",
        "new Proxy(real, {get() { throw Error('trap'); }, getPrototypeOf() { throw Error('trap'); }})",
        "(() => { const {proxy, revoke} = Proxy.revocable(real, {}); revoke(); return proxy; })()",
    ] {
        let object = v8::Local::<v8::Object>::try_from(eval(scope, source)).unwrap();
        assert_eq!(web_api_object_type(scope, object), None, "{source}");
        assert!(!implements_interface(scope, object, "TestBase"), "{source}");
    }
    let record = Record::new(7).bind(scope).unwrap();
    let prototype = PrototypeMembers::new().bind(scope).unwrap();
    assert_eq!(web_api_object_type(scope, record), None);
    assert_eq!(web_api_object_type(scope, prototype), None);
}

#[test]
fn inheritance_registration_is_atomic_and_rejects_conflicts_and_cycles() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let object = Derived::new().bind(scope).unwrap();
    register_web_api_interfaces(
        scope,
        [("TestDerived", Some("TestBase")), ("TestBase", None)],
    )
    .unwrap();
    register_web_api_interfaces(scope, [("TestDerived", Some("TestBase"))]).unwrap();
    assert!(register_web_api_interfaces(scope, [("TestDerived", None)]).is_err());
    assert!(register_web_api_interfaces(scope, [("A", Some("B")), ("B", Some("A"))]).is_err());
    assert!(implements_interface(scope, object, "TestBase"));
    // A rejected batch has not left conflicting declarations behind.
    register_web_api_interfaces(scope, [("A", None), ("B", Some("A"))]).unwrap();
}

#[derive(moli_webapi_declare::WebApiFunctionTemplate)]
#[webapi(name = "NativeConstructor", constructor_callback = native_constructor)]
struct NativeConstructor {}

fn native_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let key = v8::String::new(scope, "capturedReceiver").unwrap();
    scope
        .get_current_context()
        .global(scope)
        .set(scope, key.into(), args.this().into());
    if args.get(0).is_true() {
        let message = v8::String::new(scope, "construction failed").unwrap();
        let exception = v8::Exception::type_error(scope, message);
        scope.throw_exception(exception);
    } else if args.get(0).is_object() {
        rv.set(args.get(0));
    }
}

#[test]
fn native_constructors_brand_successful_receivers_and_replacement_objects() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let template = NativeConstructor::build(scope);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let constructor = template.get_function(scope).unwrap();
    let key = v8::String::new(scope, "NativeConstructor").unwrap();
    context
        .global(scope)
        .set(scope, key.into(), constructor.into());
    for source in [
        "new NativeConstructor()",
        "new (class Derived extends NativeConstructor {})()",
        "new NativeConstructor({replacement: true})",
        "NativeConstructor({replacement: true})",
    ] {
        let object = v8::Local::<v8::Object>::try_from(eval(scope, source)).unwrap();
        assert!(
            implements_interface(scope, object, "NativeConstructor"),
            "{source}"
        );
    }
    let failed = eval(
        scope,
        "try { new NativeConstructor(true); } catch (e) { if (e.message !== 'construction failed') throw e; } capturedReceiver",
    );
    let failed = v8::Local::<v8::Object>::try_from(failed).unwrap();
    assert_eq!(web_api_object_type(scope, failed), None);
}

#[test]
fn only_explicitly_registered_native_proxies_share_target_identity() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let target = Base::new().bind(scope).unwrap();
    let handler = v8::Object::new(scope);
    let native = v8::Proxy::new(scope, target, handler).unwrap();
    let native_object = v8::Local::<v8::Object>::from(native);
    assert_eq!(web_api_object_type(scope, native_object), None);
    moli_webapi_declare::register_web_api_proxy(scope, native).unwrap();
    assert!(implements_interface(scope, native_object, "TestBase"));
    initialize_web_api_object(scope, native_object, "TestBase").unwrap();
    let impostor = v8::Proxy::new(scope, target, handler).unwrap();
    assert_eq!(web_api_object_type(scope, impostor.into()), None);
    let outer_handler = v8::Object::new(scope);
    let outer = v8::Proxy::new(scope, native_object, outer_handler).unwrap();
    assert_eq!(web_api_object_type(scope, outer.into()), None);
    assert!(moli_webapi_declare::register_web_api_proxy(scope, outer).is_err());
    native.revoke();
    assert_eq!(web_api_object_type(scope, native_object), None);
}

#[derive(moli_webapi_declare::WebApiFunctionTemplate)]
#[webapi(name = "TestBase", receiver = "TestBase")]
struct CheckedBaseTemplate {
    #[webapi(method, callback = convert_argument)]
    convert: (),
    #[webapi(method, callback = convert_argument, returns_promise)]
    convert_async: (),
}

fn convert_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(value) = args.get(0).to_string(scope) {
        rv.set(value.into());
    }
}

#[test]
fn generated_interface_receivers_accept_subtypes_and_reject_forgery_before_conversion() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let template = CheckedBaseTemplate::build(scope);
    let context = v8::Context::new(scope, Default::default());
    let other = v8::Context::new(scope, Default::default());
    let object = {
        let scope = &mut v8::ContextScope::new(scope, other);
        register_web_api_interfaces(scope, [("TestDerived", Some("TestBase"))]).unwrap();
        Derived::new().bind(scope).unwrap()
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let constructor = template.get_function(scope).unwrap();
    let global = context.global(scope);
    let constructor_key = v8::String::new(scope, "TestBase").unwrap();
    let object_key = v8::String::new(scope, "real").unwrap();
    global.set(scope, constructor_key.into(), constructor.into());
    global.set(scope, object_key.into(), object.into());
    assert!(eval(scope, r#"
        (() => {
          let conversions = 0;
          const input = {toString() { conversions++; return 'converted'; }};
          const method = TestBase.prototype.convert;
          Object.setPrototypeOf(real, null);
          if (method.call(real, input) !== 'converted') return false;
          for (const fake of [{}, Object.create(TestBase.prototype), Object.create(real), new Proxy(real, {})]) {
            try { method.call(fake, input); return false; }
            catch (error) { if (!(error instanceof TypeError)) return false; }
          }
          return conversions === 1;
        })()
    "#).is_true());
    let promise = eval(
        scope,
        "TestBase.prototype.convertAsync.call({}, {toString() { throw Error('must not convert'); }})",
    );
    let promise = v8::Local::<v8::Promise>::try_from(promise).unwrap();
    assert_eq!(promise.state(), v8::PromiseState::Rejected);
    let reason = promise.result(scope);
    let reason = v8::Local::<v8::Object>::try_from(reason).unwrap();
    let name = v8::String::new(scope, "name").unwrap();
    assert_eq!(
        reason
            .get(scope, name.into())
            .unwrap()
            .to_string(scope)
            .unwrap()
            .to_rust_string_lossy(scope),
        "TypeError"
    );
}
