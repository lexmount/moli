use super::*;

pub(super) fn create_element_wrapper_for_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut JsContextHost,
    document_handle: crate::document_runtime::DomHandle,
    local_name: &str,
    is_name: Option<&[u16]>,
    registry_association: Option<custom_elements::CustomElementRegistryAssociation>,
    post_construction_prefix: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = receiver.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let element = custom_elements::create_element_for_document_local_name_is_and_registry(
        scope,
        runtime_ptr,
        document_handle,
        local_name,
        is_name,
        registry_association,
        post_construction_prefix,
    );
    if let Some(element) = element
        && let Ok((_, handle)) = node_runtime_and_handle_from_object(scope, element)
    {
        unsafe { &mut *runtime_ptr }.capture_node_creation_stack_trace(scope, handle);
    }
    element
}

pub(super) fn create_element_ns_for_document(
    runtime_ptr: *mut JsContextHost,
    document_handle: crate::document_runtime::DomHandle,
    namespace: Option<&str>,
    qualified_name: &str,
) -> Option<crate::document_runtime::DomHandle> {
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_element_ns(namespace, qualified_name)?;
    if runtime.dom_host().owner_document_handle(handle) != Some(document_handle) {
        runtime.initialize_new_native_node_owner_document(document_handle, handle)?;
    }
    Some(handle)
}

pub(super) fn create_element_with_parts_for_document(
    runtime_ptr: *mut JsContextHost,
    document_handle: crate::document_runtime::DomHandle,
    namespace: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
) -> Option<crate::document_runtime::DomHandle> {
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime
        .dom_host_mut()
        .create_element_with_parts(namespace, prefix, local_name);
    if runtime.dom_host().owner_document_handle(handle) != Some(document_handle) {
        runtime.initialize_new_native_node_owner_document(document_handle, handle)?;
    }
    Some(handle)
}

pub(super) fn registry_association_for_create_element(
    runtime_ptr: *mut JsContextHost,
    document_handle: crate::document_runtime::DomHandle,
    explicit_registry_association: Option<custom_elements::CustomElementRegistryAssociation>,
) -> custom_elements::CustomElementRegistryAssociation {
    explicit_registry_association.unwrap_or_else(|| {
        unsafe { &*runtime_ptr }.effective_custom_element_registry_association(document_handle)
    })
}

pub(super) fn registry_association_has_autonomous_definition(
    runtime_ptr: *mut JsContextHost,
    registry_association: custom_elements::CustomElementRegistryAssociation,
    local_name: &str,
) -> bool {
    match registry_association {
        custom_elements::CustomElementRegistryAssociation::Null => false,
        custom_elements::CustomElementRegistryAssociation::Registry(registry_key) => {
            unsafe { &*runtime_ptr }
                .custom_elements_for_registry_key(registry_key)
                .is_some_and(|store| store.has_autonomous_definition(local_name))
        }
    }
}

pub(super) fn validate_create_element_name(
    scope: &mut v8::PinScope<'_, '_>,
    local_name: &str,
) -> bool {
    if validate_element_name(local_name) {
        return true;
    }
    throw_dom_exception(
        scope,
        "InvalidCharacterError",
        5,
        "String contains an invalid character",
    );
    false
}

pub(super) fn validate_create_element_ns_name(
    scope: &mut v8::PinScope<'_, '_>,
    namespace: Option<&str>,
    qualified_name: &str,
) -> bool {
    match validate_qualified_element_name_and_namespace(namespace, qualified_name) {
        Ok(_) => true,
        Err((name, code, message)) => {
            throw_dom_exception(scope, name, code, message);
            false
        }
    }
}
