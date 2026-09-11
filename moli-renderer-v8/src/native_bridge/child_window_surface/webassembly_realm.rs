use crate::{
    native_bridge::helpers::set_object_slot,
    util::{callable_relevant_context, v8_string, v8str},
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ChildWindowWebAssemblyConstructorDataDeclaration<'scope> {
    name: v8::Local<'scope, v8::String>,
    native_constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ChildWindowInstancePrototypeDeclaration<'scope> {
    #[webapi(data_property)]
    constructor: v8::Local<'scope, v8::Function>,
}

const CHILD_WEBASSEMBLY_CONSTRUCTOR_NAMES: &[(&str, i32)] = &[
    ("Module", 1),
    ("Instance", 1),
    ("Memory", 1),
    ("Table", 1),
    ("Global", 1),
    ("CompileError", 1),
    ("LinkError", 1),
    ("RuntimeError", 1),
];

/// Adapts V8's isolate-owned WebAssembly constructors to observable
/// per-realm constructor and NewTarget prototype semantics.
///
/// This is not a child-only Web API declaration. It is an embedder workaround
/// kept at the realm-state boundary until V8 supplies the same identities for
/// these contexts directly.
pub(super) fn install_child_webassembly_realm_adapter(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
) {
    let global = scope.get_current_context().global(scope);
    let Some(native_webassembly) = global
        .get(scope, v8str(scope, "WebAssembly").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let child_webassembly = ObjectLiteralDeclaration::bind(scope).into_object();
    let _ = child_webassembly.set_prototype(scope, native_webassembly.into());
    for (name, length) in CHILD_WEBASSEMBLY_CONSTRUCTOR_NAMES {
        install_child_webassembly_constructor(
            scope,
            native_webassembly,
            child_webassembly,
            window,
            name,
            *length,
        );
    }
    set_object_slot(scope, window, "WebAssembly", child_webassembly.into());
}

fn install_child_webassembly_constructor(
    scope: &mut v8::PinScope<'_, '_>,
    native_webassembly: v8::Local<'_, v8::Object>,
    child_webassembly: v8::Local<'_, v8::Object>,
    window: v8::Local<'_, v8::Object>,
    name: &'static str,
    length: i32,
) {
    let Some(native_constructor) = native_webassembly
        .get(scope, v8str(scope, name).into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let Some(name_value) = v8_string(scope, name) else {
        return;
    };
    let Some(data) =
        ChildWindowWebAssemblyConstructorDataDeclaration::new(name_value, native_constructor)
            .bind(scope)
            .ok()
    else {
        return;
    };
    let Some(constructor) = v8::Function::builder(child_window_webassembly_constructor_callback)
        .data(data.into())
        .length(length)
        .build(scope)
    else {
        return;
    };
    constructor.set_name(v8str(scope, name));
    if let Some(function_prototype) = window
        .get(scope, v8str(scope, "Function").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .and_then(|constructor| constructor.get(scope, v8str(scope, "prototype").into()))
        .or_else(|| native_constructor.get_prototype(scope))
    {
        let _ = constructor.set_prototype(scope, function_prototype);
    }
    copy_constructor_static_properties(scope, native_constructor, constructor);
    let Some(native_instance_prototype) = native_constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        set_object_slot(scope, child_webassembly, name, constructor.into());
        return;
    };
    let Some(child_instance_prototype) = ChildWindowInstancePrototypeDeclaration::new(constructor)
        .bind(scope)
        .ok()
    else {
        set_object_slot(scope, child_webassembly, name, constructor.into());
        return;
    };
    let _ = child_instance_prototype.set_prototype(scope, native_instance_prototype.into());
    let _ = constructor.set(
        scope,
        v8str(scope, "prototype").into(),
        child_instance_prototype.into(),
    );
    crate::context_bootstrap::set_current_context_webassembly_default_prototype(
        scope,
        name,
        child_instance_prototype,
    );
    set_object_slot(scope, child_webassembly, name, constructor.into());
}

fn copy_constructor_static_properties(
    scope: &mut v8::PinScope<'_, '_>,
    source: v8::Local<'_, v8::Function>,
    target: v8::Local<'_, v8::Function>,
) {
    let property_names_args = v8::GetPropertyNamesArgsBuilder::new()
        .mode(v8::KeyCollectionMode::OwnOnly)
        .property_filter(v8::PropertyFilter::ALL_PROPERTIES)
        .build();
    let Some(names) = source.get_own_property_names(scope, property_names_args) else {
        return;
    };
    for index in 0..names.length() {
        let Some(key_value) = names.get_index(scope, index) else {
            continue;
        };
        let Ok(key) = v8::Local::<v8::Name>::try_from(key_value) else {
            continue;
        };
        if key.strict_equals(v8str(scope, "prototype").into())
            || key.strict_equals(v8str(scope, "length").into())
            || key.strict_equals(v8str(scope, "name").into())
        {
            continue;
        }
        let Some(value) = source.get(scope, key.into()) else {
            continue;
        };
        let _ = target.define_own_property(scope, key, value, v8::PropertyAttribute::DONT_ENUM);
    }
}

fn constructor_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(String, v8::Local<'s, v8::Function>)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let name = data
        .get(scope, v8str(scope, "name").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))?;
    let native_constructor = data
        .get(scope, v8str(scope, "nativeConstructor").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    Some((name, native_constructor))
}

fn current_context_default_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = scope.get_current_context();
    crate::context_bootstrap::webassembly_default_prototype_for_context(scope, context, name)
}

fn prototype_for_new_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    new_target: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let new_target = v8::Local::<v8::Object>::try_from(new_target).ok()?;
    let prototype = new_target.get(scope, v8str(scope, "prototype").into())?;
    if let Ok(prototype) = v8::Local::<v8::Object>::try_from(prototype) {
        return Some(prototype);
    }
    let relevant_context = callable_relevant_context(scope, new_target.into())?;
    crate::context_bootstrap::webassembly_default_prototype_for_context(
        scope,
        relevant_context,
        name,
    )
}

fn child_window_webassembly_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((name, native_constructor)) = constructor_data(scope, args.data()) else {
        return;
    };
    let forwarded = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    let value = if args.is_construct_call() {
        native_constructor
            .new_instance(scope, &forwarded)
            .map(Into::into)
    } else {
        native_constructor.call(scope, args.this().into(), &forwarded)
    };
    let Some(value) = value else {
        return;
    };
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(prototype) = if args.is_construct_call() {
            prototype_for_new_target(scope, &name, args.new_target())
        } else {
            current_context_default_prototype(scope, &name)
        }
    {
        let _ = object.set_prototype(scope, prototype.into());
    }
    rv.set(value);
}
