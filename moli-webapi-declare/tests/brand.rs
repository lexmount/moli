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
#[webapi(interface = "TestDerived", allow_empty)]
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
