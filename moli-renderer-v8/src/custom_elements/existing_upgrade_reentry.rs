use super::construction_failure::{
    CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_HANDLE_SLOT, CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_OWNER_SLOT,
};

use super::super::{
    document_runtime::DomHandle, native_bridge::JsContextHost, util::get_private_value,
};

pub(super) fn already_constructed_reentry_consumed_pending_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    exception: v8::Local<'s, v8::Value>,
    constructor: v8::Local<'s, v8::Function>,
    definition_name: &str,
) -> bool {
    let host = unsafe { &*host_ptr };
    let current_registry_key = host.custom_element_registry_key_for_owned_handle(handle);
    already_constructed_error_handle(scope, exception, host.custom_element_registry_owner_id())
        == Some(handle)
        && host
            .custom_elements_for_node_handle(handle)
            .is_some_and(|store| store.pending_construction_is_already_constructed(handle))
        && host
            .custom_element_definition_for_constructor(scope, constructor)
            .is_some_and(|(direct_registry_key, direct_definition_name, _)| {
                Some(direct_registry_key) != current_registry_key
                    || direct_definition_name != definition_name
            })
}

fn already_constructed_error_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
    owner: crate::native_bridge::NativeDomHostId,
) -> Option<DomHandle> {
    let object = v8::Local::<v8::Object>::try_from(exception).ok()?;
    let marker = get_private_value(scope, object, CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_OWNER_SLOT)?;
    let marker = v8::Local::<v8::BigInt>::try_from(marker).ok()?.u64_value();
    if marker != (owner.get(), true) {
        return None;
    }
    let value = get_private_value(
        scope,
        object,
        CUSTOM_ELEMENT_ALREADY_CONSTRUCTED_HANDLE_SLOT,
    )?;
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then(|| DomHandle::new(index as usize));
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize))
}
