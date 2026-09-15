use super::helpers::{
    create_element_ns_for_document, create_element_options, create_element_with_parts_for_document,
    create_element_wrapper_for_document, registry_association_for_create_element,
    registry_association_has_autonomous_definition, validate_create_element_name,
    validate_create_element_ns_name,
};
use super::*;
use crate::native_bridge::document::validate_registry_association_for_document;
use crate::native_bridge::set_wrapped_handle_or_null_for_receiver;

pub(in crate::native_bridge) fn node_create_element_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(kind) = detached_document_receiver_kind(scope, &args) {
        match kind {
            DetachedDocumentReceiverKind::Html => {
                detached_create_html_element_method_callback(scope, args, rv)
            }
            DetachedDocumentReceiverKind::Xml => {
                detached_create_xml_element_method_callback(scope, args, rv)
            }
        }
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DocumentCreateElementArgs>(scope, &args) else {
        return;
    };
    if !validate_create_element_name(scope, &parsed.local_name) {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_document(runtime, handle) {
        rv.set_null();
        return;
    }
    let mut options = create_element_options(scope, &args, 1);
    if options.registry_association.is_none() {
        options.registry_association =
            unsafe { &*runtime_ptr }.custom_element_registry_association(handle);
    }
    if !validate_registry_association_for_document(
        scope,
        runtime_ptr,
        handle,
        options.registry_association,
    ) {
        return;
    }
    if !is_html_document(runtime, handle) {
        let namespace = runtime
            .dom_host()
            .document_content_type_for_handle(handle)
            .is_some_and(|content_type| content_type.eq_ignore_ascii_case("application/xhtml+xml"))
            .then_some(XHTML_NS);
        let Some(created_handle) = create_element_with_parts_for_document(
            runtime_ptr,
            handle,
            namespace,
            None,
            &parsed.local_name,
        ) else {
            rv.set_null();
            return;
        };
        unsafe { &mut *runtime_ptr }.capture_node_creation_stack_trace(scope, created_handle);
        set_wrapped_handle_or_null_for_receiver(
            scope,
            &mut rv,
            runtime_ptr,
            args.this(),
            Some(created_handle),
        );
        return;
    }
    match create_element_wrapper_for_document(
        scope,
        args.this(),
        runtime_ptr,
        handle,
        &parsed.local_name,
        options.is_name.as_deref(),
        options.registry_association,
        None,
    ) {
        Some(element) => rv.set(element.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_create_element_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(kind) = detached_document_receiver_kind(scope, &args) {
        match kind {
            DetachedDocumentReceiverKind::Html => {
                detached_create_html_element_ns_method_callback(scope, args, rv)
            }
            DetachedDocumentReceiverKind::Xml => {
                detached_create_xml_element_ns_method_callback(scope, args, rv)
            }
        }
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DocumentCreateElementNsArgs>(scope, &args) else {
        return;
    };
    let namespace = normalize_namespace(parsed.namespace);
    if !validate_create_element_ns_name(scope, namespace.as_deref(), &parsed.qualified_name) {
        return;
    }
    let mut options = create_element_options(scope, &args, 2);
    if options.registry_association.is_none() {
        options.registry_association =
            unsafe { &*runtime_ptr }.custom_element_registry_association(handle);
    }
    if !validate_registry_association_for_document(
        scope,
        runtime_ptr,
        handle,
        options.registry_association,
    ) {
        return;
    }
    if namespace.as_deref() == Some(XHTML_NS) {
        let (prefix, local_name) = parsed
            .qualified_name
            .rsplit_once(':')
            .map(|(prefix, local_name)| (Some(prefix), local_name))
            .unwrap_or((None, parsed.qualified_name.as_str()));
        let registry_association = registry_association_for_create_element(
            runtime_ptr,
            handle,
            options.registry_association,
        );
        let should_use_registry_path = options.registry_association.is_some()
            || options.is_name.is_some()
            || registry_association_has_autonomous_definition(
                runtime_ptr,
                registry_association,
                local_name,
            );
        if !should_use_registry_path {
            let Some(created_handle) = create_element_ns_for_document(
                runtime_ptr,
                handle,
                namespace.as_deref(),
                &parsed.qualified_name,
            ) else {
                rv.set_null();
                return;
            };
            unsafe { &mut *runtime_ptr }.capture_node_creation_stack_trace(scope, created_handle);
            set_wrapped_handle_or_null_for_receiver(
                scope,
                &mut rv,
                runtime_ptr,
                args.this(),
                Some(created_handle),
            );
            return;
        }
        match create_element_wrapper_for_document(
            scope,
            args.this(),
            runtime_ptr,
            handle,
            local_name,
            options.is_name.as_deref(),
            options.registry_association,
            prefix,
        ) {
            Some(element) => rv.set(element.into()),
            None => rv.set_null(),
        }
        return;
    }
    let Some(created_handle) = create_element_ns_for_document(
        runtime_ptr,
        handle,
        namespace.as_deref(),
        &parsed.qualified_name,
    ) else {
        rv.set_null();
        return;
    };
    unsafe { &mut *runtime_ptr }.capture_node_creation_stack_trace(scope, created_handle);
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        Some(created_handle),
    );
}
