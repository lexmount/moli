#![allow(dead_code)] // Each integration-test binary uses its own subset of this fixture.

use moli_v8_util::{register_ecmascript_intrinsic, throw_type_error, v8_string, v8str};
use moli_webapi::{dom_exception, interfaces};
use moli_webapi_declare::WebApiInterfaceDescriptor;

pub fn illegal_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(scope, "Illegal constructor");
}

pub fn finish_template<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    interface: WebApiInterfaceDescriptor,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    moli_webapi_declare::capture_web_api_reflect_set(scope)
        .expect("standalone realm captures trusted Reflect.set before author script");
    interface.register(scope).unwrap();
    template.set_class_name(v8str(scope, interface.name()));
    template.read_only_prototype();
    let constructor = template.get_function(scope).unwrap();
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap();
    moli_webapi_declare::define_to_string_tag(scope, prototype, interface.name());
    let global = scope.get_current_context().global(scope);
    // A standalone realm uses the shared immutable intrinsic registry instead
    // of a Renderer resolver. Public-global replacement must not change brands.
    assert!(register_ecmascript_intrinsic(
        scope,
        global,
        interface.name(),
        constructor.into(),
        prototype,
    ));
    assert_eq!(
        global.set(
            scope,
            v8str(scope, interface.name()).into(),
            constructor.into()
        ),
        Some(true)
    );
}

pub fn install_errors(scope: &mut v8::PinScope<'_, '_>) {
    let exception =
        v8::FunctionTemplate::new(scope, dom_exception::dom_exception_constructor_callback);
    let error = v8::FunctionTemplate::new(scope, dom_exception::dom_error_constructor_callback);
    let quota = v8::FunctionTemplate::new(
        scope,
        dom_exception::quota_exceeded_error_constructor_callback,
    );
    let socket = v8::FunctionTemplate::new(
        scope,
        moli_webapi::websocket_values::websocket_error_constructor_callback,
    );
    let rtc = v8::FunctionTemplate::new(scope, dom_exception::rtc_error_constructor_callback);
    let transport = v8::FunctionTemplate::new(
        scope,
        dom_exception::web_transport_error_constructor_callback,
    );
    for template in [quota, socket, rtc, transport] {
        template.inherit(exception);
    }
    for (interface, template) in [
        (interfaces::DOMException::DESCRIPTOR, exception),
        (interfaces::DOMError::DESCRIPTOR, error),
        (interfaces::QuotaExceededError::DESCRIPTOR, quota),
        (interfaces::WebSocketError::DESCRIPTOR, socket),
        (interfaces::RTCError::DESCRIPTOR, rtc),
        (interfaces::WebTransportError::DESCRIPTOR, transport),
    ] {
        dom_exception::install_dom_exception_template_bindings(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
}

pub fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    v8::tc_scope!(let scope, scope);
    let result = v8::Script::compile(scope, v8_string(scope, source).unwrap(), None)
        .and_then(|script| script.run(scope));
    match result {
        Some(value) => value,
        None => panic!(
            "binding test failed: {}",
            scope.exception().unwrap().to_rust_string_lossy(scope)
        ),
    }
}

pub fn publish(scope: &mut v8::PinScope<'_, '_>, name: &str, value: v8::Local<'_, v8::Object>) {
    let global = scope.get_current_context().global(scope);
    assert_eq!(
        global.set(scope, v8_string(scope, name).unwrap().into(), value.into()),
        Some(true)
    );
}
