use super::super::super::{
    document,
    node::{
        node_runtime_and_handle_from_args, node_runtime_and_handle_from_args_or_detached,
        throw_incompatible_method_receiver,
    },
    throw_dom_exception,
};
use super::super::is_disabled_form_control;
use super::super::rendered_state::{ElementBoxState, ElementRenderedState};
use super::default_action::{
    activate_handle_via_synthetic_click, perform_file_chooser_default_action,
};
use crate::dom::{forms::InputType, native::Node};

pub(in crate::native_bridge) fn node_click_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        document::detached_click_method_callback(scope, args, rv);
        return;
    };
    let outcome = activate_handle_via_synthetic_click(scope, runtime_ptr, handle, 0.0, 0.0, 0, 0);
    if let Some(download) = outcome.pending_download {
        unsafe { &mut *runtime_ptr }.record_pending_download_activation(download);
    }
    if let Some(file_chooser) = outcome.pending_file_chooser {
        unsafe { &mut *runtime_ptr }.record_pending_file_chooser_activation(file_chooser);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_show_picker_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "HTMLInputElement", "showPicker");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(input_type) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.is_html_input())
        .map(|element| element.input_type())
    else {
        throw_incompatible_method_receiver(scope, "HTMLInputElement", "showPicker");
        return;
    };
    if is_disabled_form_control(runtime, handle) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "showPicker() cannot be used on disabled controls.",
        );
        return;
    }
    if !matches!(input_type, InputType::File | InputType::Color)
        && picker_is_cross_origin_with_top(runtime, handle)
    {
        throw_dom_exception(
            scope,
            "SecurityError",
            18,
            "showPicker() cannot be used in a cross-origin iframe.",
        );
        return;
    }
    if let Some(file_chooser) = perform_file_chooser_default_action(scope, runtime_ptr, handle) {
        unsafe { &mut *runtime_ptr }.record_pending_file_chooser_activation(file_chooser);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn select_show_picker_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "HTMLSelectElement", "showPicker");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "select") {
        throw_incompatible_method_receiver(scope, "HTMLSelectElement", "showPicker");
        return;
    }
    if is_disabled_form_control(runtime, handle) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "showPicker() cannot be used on disabled controls.",
        );
        return;
    }
    if picker_is_cross_origin_with_top(runtime, handle) {
        throw_dom_exception(
            scope,
            "SecurityError",
            18,
            "showPicker() cannot be used in a cross-origin iframe.",
        );
        return;
    }
    let Some(owner) = runtime
        .owner_dispatch_scope_for_node(handle)
        .filter(|owner| runtime.window_has_transient_user_activation(*owner))
    else {
        throw_dom_exception(
            scope,
            "NotAllowedError",
            0,
            "showPicker() requires transient user activation.",
        );
        return;
    };
    if ElementRenderedState::read(runtime, handle).box_state() != ElementBoxState::Box {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "showPicker() requires a rendered select element.",
        );
        return;
    }
    unsafe { &mut *runtime_ptr }.consume_window_user_activation(owner);
    rv.set_undefined();
}

fn picker_is_cross_origin_with_top(
    runtime: &super::super::super::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    let Some(owner_document) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::owner_document)
    else {
        return false;
    };
    let Some(child_handle) =
        runtime.child_browsing_context_host_for_document_handle(owner_document)
    else {
        return false;
    };
    !runtime.child_window_has_same_origin_with_its_top_level_origin(child_handle)
}
