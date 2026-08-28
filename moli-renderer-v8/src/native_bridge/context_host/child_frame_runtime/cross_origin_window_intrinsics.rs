//! Realm-local adapters for cross-origin Window internal methods.
//!
//! V8 137 rejects foreign globals at the security access check before its
//! prototype/extensibility operations reach the Window interceptors. Keep the
//! stable global proxy identity and adapt only that narrow observable surface;
//! every other target delegates to the original V8 intrinsic.

use super::window::window_access_is_allowed;
use crate::{
    definitions::define_get_set_property,
    util::{throw_type_error, v8str},
};

pub(crate) fn install_cross_origin_window_internal_method_intrinsics<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> anyhow::Result<()> {
    let object = intrinsic_object(scope, global, "Object")
        .ok_or_else(|| anyhow::anyhow!("failed to resolve Object intrinsic"))?;
    let reflect = intrinsic_object(scope, global, "Reflect")
        .ok_or_else(|| anyhow::anyhow!("failed to resolve Reflect intrinsic"))?;

    replace_intrinsic_method(
        scope,
        object,
        "setPrototypeOf",
        2,
        cross_origin_window_object_set_prototype_of_callback,
    )?;
    replace_intrinsic_method(
        scope,
        reflect,
        "setPrototypeOf",
        2,
        cross_origin_window_reflect_set_prototype_of_callback,
    )?;
    replace_intrinsic_method(
        scope,
        object,
        "preventExtensions",
        1,
        cross_origin_window_object_prevent_extensions_callback,
    )?;
    replace_intrinsic_method(
        scope,
        reflect,
        "preventExtensions",
        1,
        cross_origin_window_reflect_prevent_extensions_callback,
    )?;

    let object_prototype = object
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("failed to resolve Object.prototype"))?;
    let proto_key = v8str(scope, "__proto__");
    let proto_descriptor = object_prototype
        .get_own_property_descriptor(scope, proto_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("failed to resolve Object.prototype.__proto__"))?;
    let proto_getter = proto_descriptor
        .get(scope, v8str(scope, "get").into())
        .ok_or_else(|| anyhow::anyhow!("failed to resolve Object.prototype.__proto__ getter"))?;
    let proto_setter = proto_descriptor
        .get(scope, v8str(scope, "set").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("failed to resolve Object.prototype.__proto__ setter"))?;
    let proto_setter = build_intrinsic_wrapper(
        scope,
        proto_setter,
        "set __proto__",
        1,
        cross_origin_window_legacy_proto_setter_callback,
    )
    .ok_or_else(|| anyhow::anyhow!("failed to build Object.prototype.__proto__ setter"))?;
    define_get_set_property(
        scope,
        object_prototype,
        proto_key.into(),
        proto_getter,
        proto_setter.into(),
        v8::PropertyAttribute::DONT_ENUM,
        "Object.prototype.__proto__",
    )
}

fn intrinsic_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    global
        .get(scope, v8str(scope, name).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn replace_intrinsic_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    name: &'static str,
    length: i32,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> anyhow::Result<()> {
    let key = v8str(scope, name);
    let original = owner
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("failed to resolve intrinsic method `{name}`"))?;
    let replacement = build_intrinsic_wrapper(scope, original, name, length, callback)
        .ok_or_else(|| anyhow::anyhow!("failed to build intrinsic method `{name}`"))?;
    owner
        .define_own_property(
            scope,
            key.into(),
            replacement.into(),
            v8::PropertyAttribute::DONT_ENUM,
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow::anyhow!("failed to replace intrinsic method `{name}`"))
}

fn build_intrinsic_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Function>,
    name: &'static str,
    length: i32,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<v8::Local<'s, v8::Function>> {
    let replacement = v8::Function::builder(callback)
        .data(original.into())
        .length(length)
        .build(scope)?;
    replacement.set_name(v8str(scope, name));
    Some(replacement)
}

fn cross_origin_window_internal_method_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let accessing_context = scope.get_current_context();
    let accessed_context = object.get_creation_context(scope)?;
    if accessing_context == accessed_context
        || !object.strict_equals(accessed_context.global(scope).into())
        || window_access_is_allowed(scope, accessing_context, object)
    {
        return None;
    }
    Some(object)
}

fn call_original_intrinsic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(original) = v8::Local::<v8::Function>::try_from(args.data()) else {
        throw_type_error(scope, "Invalid intrinsic delegate.");
        return;
    };
    let values = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    if let Some(value) = original.call(scope, args.this().into(), &values) {
        rv.set(value);
    }
}

fn cross_origin_window_object_set_prototype_of_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(target) = cross_origin_window_internal_method_target(scope, args.get(0)) else {
        call_original_intrinsic(scope, &args, rv);
        return;
    };
    let prototype = args.get(1);
    if prototype.is_null() {
        rv.set(target.into());
    } else if prototype.is_object() {
        throw_type_error(
            scope,
            "Immutable prototype object cannot have its prototype set.",
        );
    } else {
        call_original_intrinsic(scope, &args, rv);
    }
}

fn cross_origin_window_reflect_set_prototype_of_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_window_internal_method_target(scope, args.get(0)).is_none() {
        call_original_intrinsic(scope, &args, rv);
        return;
    }
    let prototype = args.get(1);
    if prototype.is_null() {
        rv.set_bool(true);
    } else if prototype.is_object() {
        rv.set_bool(false);
    } else {
        call_original_intrinsic(scope, &args, rv);
    }
}

fn cross_origin_window_legacy_proto_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_window_internal_method_target(scope, args.this().into()).is_none() {
        call_original_intrinsic(scope, &args, rv);
        return;
    }
    let prototype = args.get(0);
    if prototype.is_null() {
        rv.set_undefined();
    } else if prototype.is_object() {
        throw_type_error(
            scope,
            "Immutable prototype object cannot have its prototype set.",
        );
    } else {
        call_original_intrinsic(scope, &args, rv);
    }
}

fn cross_origin_window_object_prevent_extensions_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_window_internal_method_target(scope, args.get(0)).is_some() {
        throw_type_error(scope, "Cannot prevent extensions on a cross-origin Window.");
        return;
    }
    call_original_intrinsic(scope, &args, rv);
}

fn cross_origin_window_reflect_prevent_extensions_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_window_internal_method_target(scope, args.get(0)).is_some() {
        rv.set_bool(false);
        return;
    }
    call_original_intrinsic(scope, &args, rv);
}
