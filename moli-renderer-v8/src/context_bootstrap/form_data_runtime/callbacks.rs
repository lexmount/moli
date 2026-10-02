use super::serialize::serialize_form_data_controls;
use super::storage::{form_data_entries, set_form_data_entries};
use crate::{
    native_bridge::{
        PageScriptHost,
        element::{
            construct_form_data_event, dispatch_public_event, form_associated_form_owner,
            is_valid_submit_button,
        },
        node_owner_document_relevant_context, node_runtime_and_handle_from_object_or_detached,
        throw_dom_exception,
    },
    util::throw_type_error,
};
use moli_webapi::form_data::new_world_wrapper;

pub(super) fn entries_for_form<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    form: v8::Local<'s, v8::Object>,
    submitter: Option<v8::Local<'s, v8::Object>>,
) -> Option<Vec<(String, v8::Global<v8::Value>)>> {
    let Ok((form_runtime_ptr, form_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, form)
    else {
        throw_type_error(scope, "FormData constructor requires an HTMLFormElement");
        return None;
    };
    if let Some(submitter) = submitter {
        let Ok((submitter_runtime_ptr, submitter_handle)) =
            node_runtime_and_handle_from_object_or_detached(scope, submitter)
        else {
            throw_type_error(
                scope,
                "FormData constructor submitter must be a submit button",
            );
            return None;
        };
        let runtime = unsafe { &*submitter_runtime_ptr };
        if !is_valid_submit_button(runtime, submitter_handle) {
            throw_type_error(
                scope,
                "FormData constructor submitter must be a submit button",
            );
            return None;
        }
        if submitter_runtime_ptr != form_runtime_ptr
            || form_associated_form_owner(runtime, submitter_handle) != Some(form_handle)
        {
            throw_dom_exception(
                scope,
                "NotFoundError",
                8,
                "The specified element is not owned by this form element.",
            );
            return None;
        }
    }
    construct_form_data_entries_for_form(scope, form_runtime_ptr, form_handle, submitter)
}

pub(crate) fn construct_form_data_entries_for_form<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    form_handle: crate::document_runtime::DomHandle,
    submitter: Option<v8::Local<'s, v8::Object>>,
) -> Option<Vec<(String, v8::Global<v8::Value>)>> {
    if !unsafe { &mut *runtime_ptr }.begin_form_data_construction(form_handle) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Cannot construct FormData while the form data set is already being constructed.",
        );
        return None;
    }

    let Some(context) = node_owner_document_relevant_context(scope, runtime_ptr, form_handle)
    else {
        unsafe { &mut *runtime_ptr }.end_form_data_construction(form_handle);
        return None;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let entries = serialize_form_data_controls(scope, runtime_ptr, form_handle, submitter);
    let entries = dispatch_form_data_event_with_entries(scope, runtime_ptr, form_handle, &entries)
        .unwrap_or(entries);
    unsafe { &mut *runtime_ptr }.end_form_data_construction(form_handle);
    Some(entries)
}

fn dispatch_form_data_event_with_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    form_handle: crate::document_runtime::DomHandle,
    entries: &[(String, v8::Global<v8::Value>)],
) -> Option<Vec<(String, v8::Global<v8::Value>)>> {
    let context = node_owner_document_relevant_context(scope, runtime_ptr, form_handle)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let form_data = new_world_wrapper(scope)?;
    set_form_data_entries(scope, form_data, entries);
    let event = construct_form_data_event(scope, form_data)?;
    let _ = dispatch_public_event(scope, runtime_ptr, form_handle, event);
    Some(form_data_entries(scope, form_data))
}
