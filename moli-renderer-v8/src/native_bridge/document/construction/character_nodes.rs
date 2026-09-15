use super::*;
use crate::native_bridge::document::{
    bridge_create_cdata_section_not_supported_callback,
    detached_create_cdata_section_html_method_callback,
    detached_create_cdata_section_method_callback,
};
use crate::native_bridge::set_wrapped_handle_or_null_for_receiver;

pub(in crate::native_bridge) fn node_create_text_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_create_text_node_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DocumentCreateTextNodeArgs>(scope, &args) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_text_node_for_document(document_handle, &parsed.data);
    runtime.capture_node_creation_stack_trace(scope, handle);
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, args.this(), Some(handle));
}

pub(in crate::native_bridge) fn node_create_comment_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_create_comment_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DocumentCreateCommentArgs>(scope, &args) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_comment_for_document(document_handle, &parsed.data);
    runtime.capture_node_creation_stack_trace(scope, handle);
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, args.this(), Some(handle));
}

pub(in crate::native_bridge) fn node_create_document_fragment_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_create_document_fragment_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_document_fragment_for_document(document_handle);
    runtime.capture_node_creation_stack_trace(scope, handle);
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, args.this(), Some(handle));
}

pub(in crate::native_bridge) fn node_create_processing_instruction_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_create_processing_instruction_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
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
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = runtime.create_processing_instruction_for_document(
        document_handle,
        &parsed.target,
        &parsed.data,
    );
    runtime.capture_node_creation_stack_trace(scope, handle);
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, args.this(), Some(handle));
}

pub(in crate::native_bridge) fn node_create_cdata_section_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(kind) = detached_document_receiver_kind(scope, &args) {
        match kind {
            DetachedDocumentReceiverKind::Html => {
                detached_create_cdata_section_html_method_callback(scope, args, rv)
            }
            DetachedDocumentReceiverKind::Xml => {
                detached_create_cdata_section_method_callback(scope, args, rv)
            }
        }
        return;
    }
    bridge_create_cdata_section_not_supported_callback(scope, args, rv);
}
