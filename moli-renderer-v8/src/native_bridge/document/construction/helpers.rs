use super::*;

pub(super) fn create_element_wrapper_for_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut JsContextHost,
    document_handle: crate::document_runtime::DomHandle,
    local_name: &str,
    is_name: Option<&str>,
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

pub(super) struct CreateElementOptions {
    pub(super) is_name: Option<String>,
    pub(super) registry_association: Option<custom_elements::CustomElementRegistryAssociation>,
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

pub(super) fn create_element_options(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> CreateElementOptions {
    let value = args.get(index);
    if value.is_null_or_undefined() {
        return CreateElementOptions {
            is_name: None,
            registry_association: None,
        };
    }
    if value.is_string() {
        // Legacy string createElement/createElementNS options are ignored by
        // current DOM custom-elements semantics; only dictionary `is` is used.
        return CreateElementOptions {
            is_name: None,
            registry_association: None,
        };
    }
    let Some(options) = value.to_object(scope) else {
        return CreateElementOptions {
            is_name: None,
            registry_association: None,
        };
    };
    let is_name = options
        .get(scope, v8str(scope, "is").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    let registry_association = options
        .get(scope, v8str(scope, "customElementRegistry").into())
        .and_then(|value| custom_elements::registry_association_from_value(scope, value));
    CreateElementOptions {
        is_name,
        registry_association,
    }
}
