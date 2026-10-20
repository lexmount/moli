//! Web IDL legacy platform object hooks for the three plugin collections.

use super::{item, named_index, names, values};
use crate::util::{call_script_visible_function, new_null_prototype_object};
use moli_webapi_declare::{DataPropertyDescriptorDeclaration, WebApiObject};

#[derive(WebApiObject)]
#[webapi(plain)]
struct CollectionHandler<'s> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    reflect_set: v8::Local<'s, v8::Function>,
    #[webapi(method, callback = set, length = 4, data = self.reflect_set)]
    set: (),
    #[webapi(method, callback = prevent_extensions, length = 1)]
    prevent_extensions: (),
}

pub(super) fn new_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let template = v8::ObjectTemplate::new(scope);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_getter)
            .query(indexed_query)
            .descriptor(indexed_descriptor)
            .definer(indexed_definer)
            .deleter(indexed_deleter)
            .enumerator(indexed_enumerator),
    );
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(named_getter)
            .query(named_query)
            .descriptor(named_descriptor)
            .definer(named_definer)
            .deleter(named_deleter)
            .enumerator(named_enumerator)
            .flags(v8::PropertyHandlerFlags::ONLY_INTERCEPT_STRINGS),
    );
    template.new_instance(scope)
}

pub(super) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    // As with SourceBufferList, interceptors handle virtual own properties;
    // the registered native Proxy supplies the original [[Set]] receiver.
    let handler = CollectionHandler::new(moli_webapi_declare::web_api_reflect_set(scope)?)
        .bind(scope)
        .ok()?;
    let proxy = v8::Proxy::new(scope, object, handler)?;
    moli_webapi_declare::register_web_api_proxy(scope, proxy).ok()?;
    Some(proxy.into())
}

fn array_index(scope: &mut v8::PinScope<'_, '_>, key: v8::Local<'_, v8::Name>) -> Option<u32> {
    let key = v8::Local::<v8::String>::try_from(key)
        .ok()?
        .to_rust_string_lossy(scope);
    key.parse::<u32>()
        .ok()
        .filter(|index| *index != u32::MAX && index.to_string() == key)
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = v8::Local::<v8::Object>::try_from(args.get(0)).expect("native collection target");
    let key = v8::Local::<v8::Name>::try_from(args.get(1)).expect("Proxy property key");
    if let Some(index) = array_index(scope, key)
        && index < values(scope, object).length()
    {
        rv.set_bool(false);
        return;
    }
    // LegacyPlatformObjectGetOwnProperty(ignoreNamedProps = true): real own
    // properties participate, virtual named entries do not. With no own
    // descriptor, OrdinarySet starts at the actual prototype. Reflect.set
    // preserves author accessors, Proxy prototypes and a derived receiver;
    // eventual own creation still reaches our indexed/named definers.
    let Some(own) = object.has_real_named_property(scope, key) else {
        return;
    };
    let start = if own {
        object
    } else {
        let Some(prototype) = object.get_prototype(scope) else {
            return;
        };
        v8::Local::<v8::Object>::try_from(prototype)
            .unwrap_or_else(|_| new_null_prototype_object(scope))
    };
    let reflect_set =
        v8::Local::<v8::Function>::try_from(args.data()).expect("captured Reflect.set");
    if let Some(result) = call_script_visible_function(
        scope,
        reflect_set,
        v8::undefined(scope).into(),
        &[start.into(), key.into(), args.get(2), args.get(3)],
        "plugin collection [[Set]]",
    ) {
        rv.set(result);
    }
}

fn prevent_extensions<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(false);
}

fn named_visible<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> Option<bool> {
    if object.has_real_named_property(scope, key)? {
        return Some(false);
    }
    let mut prototype = object.get_prototype(scope)?;
    while let Ok(object) = v8::Local::<v8::Object>::try_from(prototype) {
        if !crate::context_bootstrap::window_template::is_window_named_properties_object(
            scope, object,
        ) && object.has_own_property(scope, key)?
        {
            return Some(false);
        }
        prototype = object.get_prototype(scope)?;
    }
    Some(true)
}

fn named_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> Option<v8::Local<'s, v8::Value>> {
    let index = named_index(scope, object, key)?;
    named_visible(scope, object, key)?
        .then(|| item(scope, object, index))
        .flatten()
}

fn indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if index >= values(scope, args.holder()).length() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

fn indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, false, true).bind(scope)
    else {
        return v8::Intercepted::kYes;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(index >= values(scope, args.holder()).length());
    v8::Intercepted::kYes
}

fn indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let length = values(scope, args.holder()).length();
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn named_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = named_value(scope, args.holder(), key) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn named_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if named_value(scope, args.holder(), key).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(
        (v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM).as_u32() as i32,
    );
    v8::Intercepted::kYes
}

fn named_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = named_value(scope, args.holder(), key) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, false, false).bind(scope)
    else {
        return v8::Intercepted::kYes;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn named_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    _descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if named_index(scope, args.holder(), key).is_none() {
        return v8::Intercepted::kNo;
    }
    // Visibility is deliberately irrelevant to [[DefineOwnProperty]]. A
    // prototype shadow does not permit replacing a supported native name.
    match args.holder().has_real_named_property(scope, key) {
        Some(true) => v8::Intercepted::kNo,
        _ => {
            rv.set_bool(false);
            v8::Intercepted::kYes
        }
    }
}

fn named_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if named_value(scope, args.holder(), key).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn named_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let names = names(scope, args.holder());
    let mut keys = Vec::new();
    for index in 0..names.length() {
        let key = names
            .get_index(scope, index)
            .expect("dense native name list");
        let name = v8::Local::<v8::Name>::try_from(key).expect("native collection name");
        match named_visible(scope, args.holder(), name) {
            Some(true) => keys.push(key),
            Some(false) => {}
            None => return,
        }
    }
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
