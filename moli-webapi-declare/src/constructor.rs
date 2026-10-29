//! Opt-in constructors whose argument conversion precedes prototype lookup.
//!
//! A private NewTarget lets V8 allocate a receiver without reading author
//! properties. The callback selects its prototype after argument conversion.

use moli_v8_util::{
    callable_relevant_context, get_private_value, global_constructor_prototype,
    new_null_prototype_object, private_key, throw_type_error, v8str,
};

const NEW_TARGET: &str = "moli.webidl.deferredNewTarget";

/// Wrap only callbacks that finish allocation with
/// `initialize_web_api_constructor_receiver` after argument conversion.
/// Pass an intrinsic Reflect.construct captured before author script runs.
pub fn web_api_constructor_with_deferred_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
    reflect_construct: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::Proxy>> {
    let trap = v8::Function::builder(construct_trap)
        .data(reflect_construct.into())
        .length(3)
        .build(scope)?;
    let handler = new_null_prototype_object(scope);
    handler
        .define_own_property(
            scope,
            v8str(scope, "construct").into(),
            trap.into(),
            v8::PropertyAttribute::NONE,
        )?
        .then_some(())?;
    v8::Proxy::new(scope, constructor.into(), handler)
}

fn construct_trap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let construct =
        v8::Local::<v8::Function>::try_from(args.data()).expect("captured Reflect.construct");
    let Some(new_target) = v8::Function::new(scope, private_constructor) else {
        return;
    };
    let prototype = new_null_prototype_object(scope);
    let Some(key) = private_key(scope, NEW_TARGET) else {
        return;
    };
    if prototype.set_private(scope, key, args.get(2)) != Some(true)
        || new_target.define_own_property(
            scope,
            v8str(scope, "prototype").into(),
            prototype.into(),
            v8::PropertyAttribute::READ_ONLY
                | v8::PropertyAttribute::DONT_ENUM
                | v8::PropertyAttribute::DONT_DELETE,
        ) != Some(true)
    {
        return;
    }
    if let Some(result) = construct.call(
        scope,
        v8::undefined(scope).into(),
        &[args.get(0), args.get(1), new_target.into()],
    ) {
        rv.set(result);
    }
}

fn private_constructor(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_>,
) {
}

fn deferred_new_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let prototype = receiver.get_prototype(scope)?;
    let prototype = v8::Local::<v8::Object>::try_from(prototype).ok()?;
    get_private_value(scope, prototype, NEW_TARGET)
}

/// Read before initializing the receiver if a callback needs the original
/// NewTarget. Native callback arguments and data remain otherwise unchanged.
pub fn web_api_constructor_new_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> v8::Local<'s, v8::Value> {
    deferred_new_target(scope, args.this()).unwrap_or_else(|| args.new_target())
}

/// Finish allocation after overload resolution and WebIDL argument conversion.
/// Each invocation owns its temporary prototype record. Reentrant construction
/// cannot replace it, and selecting the real prototype releases that record.
pub fn initialize_web_api_constructor_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    default_interface_name: &str,
) -> bool {
    let Some(new_target) = deferred_new_target(scope, receiver) else {
        return true;
    };
    let Ok(target) = v8::Local::<v8::Object>::try_from(new_target) else {
        return false;
    };
    let Some(prototype) = target.get(scope, v8str(scope, "prototype").into()) else {
        return false;
    };
    let prototype = if let Ok(prototype) = v8::Local::<v8::Object>::try_from(prototype) {
        prototype
    } else {
        let Some(context) = callable_relevant_context(scope, new_target) else {
            throw_type_error(scope, "WebIDL constructor realm is unavailable");
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let Some(prototype) = global_constructor_prototype(scope, default_interface_name) else {
            throw_type_error(scope, "WebIDL constructor prototype is unavailable");
            return false;
        };
        prototype
    };
    receiver.set_prototype(scope, prototype.into()) == Some(true)
}
