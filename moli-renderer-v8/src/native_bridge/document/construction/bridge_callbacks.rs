use super::helpers::{
    create_element_ns_for_document, create_element_options, create_element_wrapper_for_document,
    registry_association_for_create_element, registry_association_has_autonomous_definition,
    validate_create_element_name, validate_create_element_ns_name,
};
use super::*;

pub(in crate::native_bridge) fn bridge_create_element_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateElementArgs>(scope, &args) else {
        return;
    };
    if !validate_create_element_name(scope, &parsed.local_name) {
        return;
    }
    let bridge = args.this();
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_object(scope, bridge)
    else {
        rv.set_null();
        return;
    };
    let mut options = create_element_options(scope, &args, 1);
    if options.registry_association.is_none() {
        options.registry_association =
            unsafe { &*runtime_ptr }.custom_element_registry_association(document_handle);
    }
    match create_element_wrapper_for_document(
        scope,
        args.this(),
        runtime_ptr,
        document_handle,
        &parsed.local_name,
        options.is_name.as_deref(),
        options.registry_association,
        None,
    ) {
        Some(element) => rv.set(element.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_element_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateElementNsArgs>(scope, &args) else {
        return;
    };
    let namespace = normalize_namespace(parsed.namespace);
    if !validate_create_element_ns_name(scope, namespace.as_deref(), &parsed.qualified_name) {
        return;
    }
    let bridge = args.this();
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_object(scope, bridge)
    else {
        rv.set_null();
        return;
    };
    let mut options = create_element_options(scope, &args, 2);
    if options.registry_association.is_none() {
        options.registry_association =
            unsafe { &*runtime_ptr }.custom_element_registry_association(document_handle);
    }
    if namespace.as_deref() == Some(XHTML_NS) {
        let (prefix, local_name) = parsed
            .qualified_name
            .rsplit_once(':')
            .map(|(prefix, local_name)| (Some(prefix), local_name))
            .unwrap_or((None, parsed.qualified_name.as_str()));
        let registry_association = registry_association_for_create_element(
            runtime_ptr,
            document_handle,
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
            let Some(handle) = create_element_ns_for_document(
                runtime_ptr,
                document_handle,
                namespace.as_deref(),
                &parsed.qualified_name,
            ) else {
                rv.set_null();
                return;
            };
            unsafe { &mut *runtime_ptr }.capture_node_creation_stack_trace(scope, handle);
            match unsafe { &mut *runtime_ptr }
                .native_bridge_mut()
                .wrap_handle_for_receiver(scope, runtime_ptr, args.this(), handle)
            {
                Some(element) => rv.set(element.into()),
                None => rv.set_null(),
            }
            return;
        }
        match create_element_wrapper_for_document(
            scope,
            args.this(),
            runtime_ptr,
            document_handle,
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
    let Some(handle) = create_element_ns_for_document(
        runtime_ptr,
        document_handle,
        namespace.as_deref(),
        &parsed.qualified_name,
    ) else {
        rv.set_null();
        return;
    };
    unsafe { &mut *runtime_ptr }.capture_node_creation_stack_trace(scope, handle);
    match unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle_for_receiver(scope, runtime_ptr, args.this(), handle)
    {
        Some(element) => rv.set(element.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_text_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateTextNodeArgs>(scope, &args) else {
        return;
    };
    let bridge = args.this();
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_object(scope, bridge)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_text_node_for_document(document_handle, &parsed.data);
    runtime.capture_node_creation_stack_trace(scope, handle);
    match runtime.native_bridge_mut().wrap_handle_for_receiver(
        scope,
        runtime_ptr,
        args.this(),
        handle,
    ) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_comment_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateCommentArgs>(scope, &args) else {
        return;
    };
    let bridge = args.this();
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_object(scope, bridge)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_comment_for_document(document_handle, &parsed.data);
    runtime.capture_node_creation_stack_trace(scope, handle);
    match runtime.native_bridge_mut().wrap_handle_for_receiver(
        scope,
        runtime_ptr,
        args.this(),
        handle,
    ) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_processing_instruction_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateProcessingInstructionArgs>(scope, &args)
    else {
        return;
    };
    if !is_valid_pi_target(&parsed.target) {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    if parsed.data.contains("?>") {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    let bridge = args.this();
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_object(scope, bridge)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_processing_instruction_for_document(
        document_handle,
        &parsed.target,
        &parsed.data,
    );
    runtime.capture_node_creation_stack_trace(scope, handle);
    match runtime.native_bridge_mut().wrap_handle_for_receiver(
        scope,
        runtime_ptr,
        args.this(),
        handle,
    ) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}
