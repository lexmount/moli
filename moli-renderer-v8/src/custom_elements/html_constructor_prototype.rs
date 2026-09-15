use super::super::{
    dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT,
    native_bridge::JsContextHost,
    util::{
        constructor_prototype, get_private_object, global_constructor_prototype,
        new_target_realm_constructor_prototype, receiver_uses_new_target_realm_object_fallback,
    },
};
use super::CustomElementRegistryKey;

pub(super) fn receiver_prototype_chain_contains_constructor_prototype(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    registry_key: CustomElementRegistryKey,
    receiver: v8::Local<'_, v8::Object>,
    constructor_name: &str,
) -> bool {
    let Some(interface_prototype) =
        registry_constructor_prototype(scope, host_ptr, registry_key, constructor_name)
    else {
        return false;
    };
    let mut current = receiver.get_prototype(scope);
    while let Some(prototype) = current {
        if prototype.strict_equals(interface_prototype.into()) {
            return true;
        }
        let Ok(prototype_object) = v8::Local::<v8::Object>::try_from(prototype) else {
            return false;
        };
        current = prototype_object.get_prototype(scope);
    }
    false
}

fn registry_constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    registry_key: CustomElementRegistryKey,
    constructor_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    match registry_key {
        CustomElementRegistryKey::Global | CustomElementRegistryKey::Scoped(_) => {
            global_constructor_prototype(scope, constructor_name)
        }
        CustomElementRegistryKey::Child(handle) => {
            let window = unsafe { &*host_ptr }
                .existing_child_browsing_context_window_wrapper(scope, handle)?;
            constructor_prototype(scope, window, constructor_name)
        }
    }
}

pub(super) fn set_wrapper_html_constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    receiver: v8::Local<'s, v8::Object>,
    new_target: v8::Local<'s, v8::Function>,
    active_constructor_name: &str,
) {
    let Some(prototype) = receiver.get_prototype(scope) else {
        return;
    };
    // V8 has already performed Get(NewTarget, "prototype") while preparing
    // the native constructor receiver. Re-reading it here would invoke Proxy
    // getters twice. A non-object value is represented by the Object
    // prototype from NewTarget's relevant Realm; replace only that fallback
    // with the active HTML interface prototype from the same Realm.
    let prototype =
        if receiver_uses_new_target_realm_object_fallback(scope, receiver, new_target.into()) {
            new_target_realm_constructor_prototype(
                scope,
                new_target.into(),
                active_constructor_name,
            )
            .map(Into::into)
            .unwrap_or(prototype)
        } else {
            prototype
        };
    let _ = wrapper.set_prototype(scope, prototype);
    if let Some(foreign) = get_private_object(scope, wrapper, DOM_PARSER_FOREIGN_NODE_SLOT) {
        let _ = foreign.set_prototype(scope, prototype);
    }
}
