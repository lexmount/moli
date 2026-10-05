//! Declarative execution uses native control state and the normal submit path.
use super::super::bindings::document_owner;
use super::super::state::{FormInvocation, FormInvocationState, PendingInvocation, ToolExecutor};
use super::super::{execution, navigation};
use super::schema;
use crate::{
    document_runtime::DomHandle,
    dom::{forms::InputType, native::Node},
    native_bridge::{
        JsContextHost,
        element::{
            dispatch_text_control_event, focus_element, form_control_elements,
            form_control_is_effectively_disabled, is_valid_submit_button,
            submit_form_with_submit_event, text_control_value,
        },
    },
};
use moli_webmcp::{Fill, prepare_fill};

pub(in crate::context_bootstrap::web_mcp) fn start(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
    form: DomHandle,
    autosubmit: bool,
    input: &str,
) {
    let host = unsafe { &*host_ptr };
    if host.native_bridge().web_mcp.pending_form_id(form).is_some() {
        execution::finish_error(scope, host_ptr, id, "Form tool is already running");
        return;
    }
    let controls = form_control_elements(host, form);
    let Some(plan) = prepare_fill(
        host.dom_host(),
        schema::parameter_controls(host, controls.iter().copied()),
        input,
    ) else {
        execution::finish_error(scope, host_ptr, id, "Invalid form tool input");
        return;
    };
    let submitter = controls.into_iter().find(|control| {
        is_valid_submit_button(host, *control)
            && !form_control_is_effectively_disabled(host, *control)
    });
    if !autosubmit && submitter.is_none() {
        execution::finish_error(scope, host_ptr, id, "Form tool requires a submit button");
        return;
    }
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .pending
        .get_mut(&id)
        .expect("form invocation")
        .form = Some(FormInvocation {
        handle: form,
        submitter,
        state: FormInvocationState::Filling,
    });
    for fill in plan {
        let (control, changed) = match fill {
            Fill::Value(handle, value) => {
                let before = text_control_value(unsafe { &*host_ptr }, handle);
                unsafe { &mut *host_ptr }.set_input_value(handle, &value);
                (
                    handle,
                    before != text_control_value(unsafe { &*host_ptr }, handle),
                )
            }
            Fill::Checked(handle, checked) => {
                (handle, fill_checked(scope, host_ptr, handle, checked))
            }
            Fill::Checkable {
                handle,
                selected,
                radio,
            } => {
                let checked = selected.contains(&text_control_value(unsafe { &*host_ptr }, handle));
                if radio && !checked {
                    continue;
                }
                (handle, fill_checked(scope, host_ptr, handle, checked))
            }
            Fill::Select(handle, selected_values) => {
                let dom = unsafe { &*host_ptr }.dom_host();
                let multiple = dom.get_attribute(handle, "multiple").is_some();
                let mut matched = false;
                let options = dom
                    .select_option_elements(handle)
                    .into_iter()
                    .map(|option| {
                        let selected = dom
                            .option_value(option)
                            .is_some_and(|value| selected_values.contains(&value))
                            && (multiple || !matched);
                        matched |= selected;
                        (option, selected)
                    })
                    .collect::<Vec<_>>();
                let before = unsafe { &*host_ptr }
                    .dom_host()
                    .select_selected_option_elements(handle);
                let any_selected = options.iter().any(|(_, selected)| *selected);
                for (option, selected) in options {
                    unsafe { &mut *host_ptr }.set_selected_state(scope, host_ptr, option, selected);
                }
                unsafe { &mut *host_ptr }.set_select_explicit_none(
                    scope,
                    host_ptr,
                    handle,
                    !any_selected,
                );
                (
                    handle,
                    before
                        != unsafe { &*host_ptr }
                            .dom_host()
                            .select_selected_option_elements(handle),
                )
            }
        };
        let checkable = unsafe { &*host_ptr }
            .dom_host()
            .node(control)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_input()
                    && matches!(element.input_type(), InputType::Checkbox | InputType::Radio)
            });
        if changed {
            dispatch_text_control_event(scope, host_ptr, control, "input");
        }
        if changed || checkable {
            dispatch_text_control_event(scope, host_ptr, control, "change");
        }
        if !form_is_current(unsafe { &*host_ptr }, id, form) {
            execution::finish_error(
                scope,
                host_ptr,
                id,
                "Form tool was removed while filling controls",
            );
            return;
        }
    }
    let Some(pending) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .pending
        .get_mut(&id)
    else {
        return;
    };
    pending.form.as_mut().expect("form invocation").state = FormInvocationState::Ready;
    unsafe { &mut *host_ptr }.set_web_mcp_activity(form, true, false);
    if let Some(submitter) = submitter {
        unsafe { &mut *host_ptr }.set_web_mcp_activity(submitter, false, true);
    }
    if autosubmit {
        let _ = submit_form_with_submit_event(scope, host_ptr, form, submitter, false);
    } else if let Some(submitter) = submitter {
        focus_element(scope, host_ptr, submitter);
    }
}

fn fill_checked(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    checked: bool,
) -> bool {
    // Native changes also track dirty checkedness and radio peers; only this
    // control's checked value determines whether WebMCP dispatches input.
    let before = unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.checked());
    unsafe { &mut *host_ptr }.set_checked_state(scope, host_ptr, handle, checked);
    before != checked
}

fn form_is_current(host: &JsContextHost, id: u64, form: DomHandle) -> bool {
    let store = &host.native_bridge().web_mcp;
    let Some(pending) = store.pending.get(&id) else {
        return false;
    };
    document_owner(host, pending.document) == Some(pending.owner)
        && host.dom_host().is_connected(form)
        && store
            .documents
            .get(&pending.document)
            .and_then(|entry| entry.tools.get(&pending.name))
            .is_some_and(
                |tool| matches!(tool.executor, ToolExecutor::Form { handle, .. } if handle == form),
            )
}

pub(crate) fn begin_form_submit(host: &mut JsContextHost, form: DomHandle) -> Option<u64> {
    let store = &mut host.native_bridge_mut().web_mcp;
    let id = store.pending_form_id(form)?;
    let active = store.pending.get_mut(&id)?.form.as_mut()?;
    if !matches!(active.state, FormInvocationState::Ready) {
        return None;
    }
    active.state = FormInvocationState::Submitting;
    Some(id)
}

pub(crate) fn finish_form_submit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
    event: v8::Local<'s, v8::Object>,
    allows_default: bool,
) {
    let Some(pending) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .pending
        .get_mut(&id)
    else {
        return;
    };
    let form = pending.form.as_mut().expect("form invocation");
    if let Some(response) =
        crate::context_bootstrap::events::take_submit_event_response(scope, event)
    {
        form.state = FormInvocationState::Responding(v8::Global::new(scope, response));
        execution::await_response(scope, host_ptr, id, response);
    } else {
        let form_handle = form.handle;
        form.state = FormInvocationState::Ready;
        if !allows_default && !unsafe { &*host_ptr }.has_planned_form_navigation_for(form_handle) {
            execution::finish_error(
                scope,
                host_ptr,
                id,
                "Submit was prevented without respondWith()",
            );
        }
    }
}

pub(crate) fn form_submission_failed(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
) {
    execution::finish_error(scope, host_ptr, id, "Form validation or submission failed");
}

pub(crate) fn finish_form_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    form: DomHandle,
    accepted: bool,
) {
    if accepted && unsafe { &*host_ptr }.has_planned_form_navigation_for(form) {
        return;
    }
    let id = navigation::form_invocation(unsafe { &*host_ptr }, form);
    if let Some(id) = id {
        if accepted {
            execution::finish_navigation(scope, host_ptr, id);
        } else {
            form_submission_failed(scope, host_ptr, id);
        }
    }
}

pub(in crate::context_bootstrap::web_mcp) fn finish_activity(
    host: &mut JsContextHost,
    pending: &PendingInvocation,
) {
    if let Some(form) = &pending.form {
        host.set_web_mcp_activity(form.handle, false, false);
        if let Some(submitter) = form.submitter {
            host.set_web_mcp_activity(submitter, false, false);
        }
    }
}

pub(crate) fn cancel_form_execution(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    form: DomHandle,
) {
    let id = unsafe { &*host_ptr }
        .native_bridge()
        .web_mcp
        .pending_form_id(form);
    if let Some(id) = id {
        execution::finish_error(scope, host_ptr, id, "Form tool execution was reset");
    }
}
