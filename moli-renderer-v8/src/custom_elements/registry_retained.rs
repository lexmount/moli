use super::{ChildCustomElementRegistryKey, CustomElementRegistryKey};
use crate::native_bridge::JsContextHost;
use crate::util::{get_private_value, set_private_value, v8_string};

const DEFINITIONS_SLOT: &str = "__moliCustomElementsRetainedDefinitions";
const PROMISES_SLOT: &str = "__moliCustomElementsRetainedPromises";

fn registry_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
    slot: &str,
    create: bool,
) -> Option<v8::Local<'s, v8::Map>> {
    if let Some(value) = get_private_value(scope, registry, slot) {
        return v8::Local::try_from(value).ok();
    }
    if !create {
        return None;
    }
    let map = v8::Map::new(scope);
    set_private_value(scope, registry, slot, map.into());
    Some(map)
}

pub(crate) fn retain_child_registry_definition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
    key: CustomElementRegistryKey,
    name: &str,
    constructor: v8::Local<'s, v8::Function>,
) {
    if !matches!(key, CustomElementRegistryKey::Child(_)) {
        return;
    }
    // The native store is retired with its Document. Retained JS objects keep
    // queryable definitions and pending promises alive through ordinary V8
    // references, without rooting the retired Realm from the native host.
    let Some(name) = v8_string(scope, name) else {
        return;
    };
    if let Some(map) = registry_map(scope, registry, DEFINITIONS_SLOT, true) {
        let _ = map.set(scope, name.into(), constructor.into());
    }
    if let Some(map) = registry_map(scope, registry, PROMISES_SLOT, false) {
        let _ = map.delete(scope, name.into());
    }
}

pub(crate) fn retained_child_registry_definition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let map = registry_map(scope, registry, DEFINITIONS_SLOT, false)?;
    let name = v8_string(scope, name)?;
    v8::Local::try_from(map.get(scope, name.into())?).ok()
}

pub(crate) fn retained_child_registry_definition_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
    constructor: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::String>> {
    let map = registry_map(scope, registry, DEFINITIONS_SLOT, false)?;
    let entries = map.as_array(scope);
    for index in (0..entries.length()).step_by(2) {
        if entries
            .get_index(scope, index + 1)?
            .strict_equals(constructor.into())
        {
            return v8::Local::try_from(entries.get_index(scope, index)?).ok();
        }
    }
    None
}

pub(crate) fn child_registry_when_defined<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    registry: v8::Local<'s, v8::Object>,
    key: ChildCustomElementRegistryKey,
    name: &str,
) -> Option<v8::Local<'s, v8::Promise>> {
    let registry_key = CustomElementRegistryKey::Child(key);
    let constructor = unsafe { &*host_ptr }
        .custom_elements_for_registry_key(registry_key)
        .and_then(|store| store.get(scope, name))
        .or_else(|| retained_child_registry_definition(scope, registry, name));
    if let Some(constructor) = constructor {
        let resolver = v8::PromiseResolver::new(scope)?;
        let _ = resolver.resolve(scope, constructor.into());
        return Some(resolver.get_promise(scope));
    }
    let map = registry_map(scope, registry, PROMISES_SLOT, true)?;
    let name_value = v8_string(scope, name)?;
    if let Some(promise) = map
        .get(scope, name_value.into())
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
    {
        return Some(promise);
    }
    let promise = if unsafe { &*host_ptr }.custom_element_registry_key_is_current(key) {
        unsafe { &mut *host_ptr }
            .custom_elements_mut_for_registry_key(registry_key)
            .when_defined(scope, name)?
    } else {
        // No later Document can define a name in this retired registry.
        v8::PromiseResolver::new(scope)?.get_promise(scope)
    };
    let _ = map.set(scope, name_value.into(), promise.into());
    Some(promise)
}
