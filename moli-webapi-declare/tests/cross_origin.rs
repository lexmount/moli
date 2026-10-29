use std::pin::pin;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{get_private_value, set_private_value, v8str};
use moli_webapi_declare::{WebApiCrossOrigin, WebApiCrossOriginDeclaration};

const VALUE: &str = "__crossOriginTestValue";

fn receiver<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, object, VALUE).is_some()
}

fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, object, VALUE)
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<()> {
    let value = value.to_string(scope)?;
    set_private_value(scope, object, VALUE, value.into());
    Some(())
}

fn method<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    rv.set(args.this().into());
}

// Fields are declaration metadata; no runtime object carries this state.
#[allow(dead_code)]
#[derive(WebApiCrossOrigin)]
#[webapi(plain, receiver = receiver)]
struct Members {
    #[webapi(method, length = 1, callback = method)]
    operation: (),
    #[webapi(accessor_property, getter = get, setter = set)]
    value: (),
    #[webapi(accessor_property, setter = set)]
    write_only: (),
}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}

#[test]
fn native_members_share_templates_but_keep_functions_in_the_accessing_context() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let a = v8::Context::new(scope, Default::default());
    let b = v8::Context::new(scope, Default::default());
    let member = Members::cross_origin_property("operation").unwrap();
    let function_a = {
        let scope = &mut v8::ContextScope::new(scope, a);
        let object = v8::Object::new(scope);
        let value = v8str(scope, "initial");
        set_private_value(scope, object, VALUE, value.into());
        let function = member.get(scope, object).unwrap();
        assert!(function.strict_equals(member.get(scope, object).unwrap()));
        let global = a.global(scope);
        global
            .set(scope, v8str(scope, "object").into(), object.into())
            .unwrap();
        for (name, value) in [
            ("method", function),
            ("methodDescriptor", member.descriptor(scope).unwrap().into()),
            (
                "descriptor",
                Members::cross_origin_property("value")
                    .unwrap()
                    .descriptor(scope)
                    .unwrap()
                    .into(),
            ),
            (
                "writeOnly",
                Members::cross_origin_property("writeOnly")
                    .unwrap()
                    .descriptor(scope)
                    .unwrap()
                    .into(),
            ),
        ] {
            global.set(scope, v8str(scope, name).into(), value).unwrap();
        }
        assert!(eval(scope, r#"
            (() => {
              function throwsTypeError(callback) { try { callback(); } catch (e) { return e instanceof TypeError; } return false; }
              if (Object.getPrototypeOf(method) !== Function.prototype || method.name !== 'operation' || method.length !== 1 || Object.hasOwn(method, 'prototype')) return false;
              if (method.call(object) !== object || !throwsTypeError(() => method.call({})) || !throwsTypeError(() => new method())) return false;
              if (methodDescriptor.writable || methodDescriptor.enumerable || !methodDescriptor.configurable) return false;
              if (descriptor.get.name !== 'get value' || descriptor.get.length !== 0 || descriptor.set.name !== 'set value' || descriptor.set.length !== 1 || descriptor.enumerable || !descriptor.configurable) return false;
              if (descriptor.get.call(object) !== 'initial' || !throwsTypeError(() => descriptor.get.call({}))) return false;
              let conversions = 0;
              if (!throwsTypeError(() => descriptor.set.call({}, {toString() { conversions++; return 'bad'; }})) || conversions !== 0) return false;
              const exception = new Error('conversion');
              try { descriptor.set.call(object, {toString() { throw exception; }}); return false; } catch (e) { if (e !== exception) return false; }
              descriptor.set.call(object, 'updated');
              return descriptor.get.call(object) === 'updated' && writeOnly.get === undefined && writeOnly.set.name === 'set writeOnly';
            })()
        "#).is_true());
        v8::Global::new(scope, function)
    };
    let scope = &mut v8::ContextScope::new(scope, b);
    let object = v8::Object::new(scope);
    let function_b = member.get(scope, object).unwrap();
    let function_a = v8::Local::new(scope, &function_a);
    assert!(!function_a.strict_equals(function_b));
    let function_b = v8::Local::<v8::Object>::try_from(function_b).unwrap();
    assert!(
        function_b
            .get_prototype(scope)
            .unwrap()
            .strict_equals(eval(scope, "Function.prototype"))
    );
}

#[test]
fn native_member_failure_propagates_script_termination() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let member = Members::cross_origin_property("value").unwrap();
    let object = v8::Object::new(scope);
    let value = v8str(scope, "initial");
    set_private_value(scope, object, VALUE, value.into());
    let input = v8::Object::new(scope);
    {
        v8::tc_scope!(let scope, scope);
        scope.terminate_execution();
        assert!(member.set(scope, object, input.into()).is_none());
        assert!(
            scope.has_terminated(),
            "termination must reach the execution boundary"
        );
        assert!(!scope.can_continue());
        scope.cancel_terminate_execution();
    }
    assert!(member.set(scope, object, input.into()).is_some());
}

#[test]
fn isolate_template_cache_does_not_retain_an_accessing_context() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let weak = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let object = v8::Object::new(scope);
        assert!(
            Members::cross_origin_property("operation")
                .unwrap()
                .get(scope, object)
                .is_some()
        );
        v8::Weak::new(scope, context)
    };
    isolate.low_memory_notification();
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    assert!(weak.to_local(scope).is_none());
}
