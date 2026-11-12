use super::super::{
    document_runtime::DomHandle,
    native_bridge::{PageScriptHost, wrapped_handle_value},
};

pub(super) fn custom_element_wrapper_for_existing_upgrade<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    wrapped_handle_value(scope, host_ptr, handle)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn definition_disables_existing_shadow(
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
) -> bool {
    unsafe { &*host_ptr }
        .dom_host()
        .shadow_root_handle(handle)
        .is_some()
        && unsafe { &*host_ptr }
            .custom_elements_for_node_handle(handle)
            .is_some_and(|store| store.definition_disables_shadow_for_handle(host_ptr, handle))
}
