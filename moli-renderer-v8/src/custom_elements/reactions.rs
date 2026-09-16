use crate::{
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    util::{context_host_ptr_from_global_bridge, enqueue_host_microtask},
};

use super::reaction_dispatcher::invoke_custom_element_reactions_for_handle;
pub(crate) use super::reaction_guards::with_custom_element_reaction_scope;
pub(super) use super::reaction_guards::{
    enter_custom_element_reaction, enter_upgrade_dynamic_markup_insertion,
};
pub(super) use super::reaction_types::CustomElementReaction;

pub(super) fn enqueue_custom_element_reaction(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    reaction: CustomElementReaction,
) {
    let needs_backup_microtask = unsafe { &mut *host_ptr }
        .custom_element_reactions_mut()
        .enqueue_reaction(handle, reaction);
    if needs_backup_microtask {
        schedule_backup_custom_element_reaction_microtask(scope, host_ptr);
    }
}

fn schedule_backup_custom_element_reaction_microtask(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let Some(callback) =
        v8::Function::new(scope, custom_element_backup_reaction_microtask_callback)
    else {
        return;
    };
    unsafe { &mut *host_ptr }
        .custom_element_reactions_mut()
        .mark_backup_queue_flush_scheduled();
    enqueue_host_microtask(scope, callback);
}

fn custom_element_backup_reaction_microtask_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    flush_backup_custom_element_reaction_queue(scope, host_ptr);
}

pub(crate) fn push_parser_custom_element_reaction_queue(host_ptr: *mut JsContextHost) {
    unsafe { &mut *host_ptr }
        .custom_element_reactions_mut()
        .push_element_queue();
}

pub(super) fn flush_current_custom_element_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    unsafe { &*host_ptr }.debug_assert_not_in_structural_mutation("custom element reaction flush");
    loop {
        let handle = unsafe { &mut *host_ptr }
            .custom_element_reactions_mut()
            .next_current_element();
        let Some(handle) = handle else {
            break;
        };
        invoke_custom_element_reactions_for_handle(scope, host_ptr, handle);
        unsafe { &mut *host_ptr }
            .custom_element_reactions_mut()
            .remove_reaction_queue_if_drained(handle);
    }
    unsafe { &mut *host_ptr }
        .custom_element_reactions_mut()
        .pop_element_queue();
}

pub(crate) fn flush_parser_custom_element_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    flush_current_custom_element_reaction_queue(scope, host_ptr);
}

fn flush_backup_custom_element_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    unsafe { &*host_ptr }
        .debug_assert_not_in_structural_mutation("backup custom element reaction flush");
    loop {
        let handle = unsafe { &mut *host_ptr }
            .custom_element_reactions_mut()
            .next_backup_element();
        let Some(handle) = handle else {
            break;
        };
        invoke_custom_element_reactions_for_handle(scope, host_ptr, handle);
        unsafe { &mut *host_ptr }
            .custom_element_reactions_mut()
            .remove_reaction_queue_if_drained(handle);
    }
    unsafe { &mut *host_ptr }
        .custom_element_reactions_mut()
        .finish_backup_queue_flush();
}
