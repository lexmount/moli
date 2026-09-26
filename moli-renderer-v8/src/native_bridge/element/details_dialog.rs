use crate::{
    document_runtime::DomHandle,
    page_task_queue::{
        RendererPageElementToggleEventKind, RendererPageElementToggleEventState,
        RendererPageUserInteractionEventKind,
    },
    util::v8_string,
    webidl,
};

use super::super::{
    JsContextHost, node::node_runtime_and_handle_from_object_or_detached, throw_dom_exception,
};
use super::toggle_event::queue_element_toggle_event;
use super::{
    element_has_attribute, html_element_getter_receiver, html_element_setter_receiver,
    property_dom_string_value, set_reflected_boolean_attribute,
};

pub(crate) fn queue_details_toggle_event_for_attribute_change(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    namespace: Option<&str>,
    local_name: &str,
    old_value: Option<&str>,
    new_value: Option<&str>,
) {
    if namespace.is_some()
        || !local_name.eq_ignore_ascii_case("open")
        || old_value.is_some() == new_value.is_some()
        || !unsafe { &*runtime_ptr }
            .dom_host()
            .is_html_element_named(handle, "details")
    {
        return;
    }
    let (old_state, new_state) = if new_value.is_some() {
        (
            RendererPageElementToggleEventState::Closed,
            RendererPageElementToggleEventState::Open,
        )
    } else {
        (
            RendererPageElementToggleEventState::Open,
            RendererPageElementToggleEventState::Closed,
        )
    };
    queue_element_toggle_event(
        scope,
        runtime_ptr,
        RendererPageElementToggleEventKind::Details,
        handle,
        old_state,
        new_state,
        None,
    );
}

pub(crate) fn queue_parser_details_toggle_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if runtime.dom_host().is_html_element_named(handle, "details")
        && element_has_attribute(runtime, handle, "open")
    {
        queue_element_toggle_event(
            scope,
            runtime_ptr,
            RendererPageElementToggleEventKind::Details,
            handle,
            RendererPageElementToggleEventState::Closed,
            RendererPageElementToggleEventState::Open,
            None,
        );
    }
}

pub(crate) fn queue_parser_details_toggle_events_in_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    root: DomHandle,
) {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        queue_parser_details_toggle_event(scope, runtime_ptr, handle);
        let runtime = unsafe { &*runtime_ptr };
        stack.extend(runtime.dom_host().child_handles(handle));
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLDialogElement.close")]
struct DialogCloseArgs {
    #[webidl(with = dialog_close_return_value_arg)]
    return_value: Option<String>,
}

pub(super) fn main_summary_child(runtime: &JsContextHost, details: DomHandle) -> Option<DomHandle> {
    let details_element = runtime
        .dom_host()
        .node(details)
        .and_then(|node| node.as_element())?;
    if !details_element.is_html_element("details") {
        return None;
    }
    runtime.dom_host().child_handles(details).find(|handle| {
        runtime
            .dom_host()
            .node(*handle)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.is_html_element("summary"))
    })
}

pub(super) fn closed_details_child_participates(
    runtime: &JsContextHost,
    details: DomHandle,
    child: DomHandle,
) -> bool {
    if !runtime.dom_host().is_html_element_named(details, "details")
        || element_has_attribute(runtime, details, "open")
    {
        return true;
    }
    main_summary_child(runtime, details) == Some(child)
}

pub(super) fn node_is_hidden_by_closed_details(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let mut branch = handle;
    while let Some(parent) = runtime.dom_host().parent_node(branch) {
        if !closed_details_child_participates(runtime, parent, branch) {
            return true;
        }
        branch = parent;
    }
    false
}

pub(super) fn closed_details_ancestors_to_reveal(
    runtime: &JsContextHost,
    target: DomHandle,
) -> Vec<DomHandle> {
    let mut ancestors = Vec::new();
    let mut branch = target;
    while let Some(parent) = details_reveal_flat_tree_parent(runtime, branch) {
        if runtime.dom_host().is_html_element_named(parent, "details")
            && !element_has_attribute(runtime, parent, "open")
            && main_summary_child(runtime, parent) != Some(branch)
        {
            ancestors.push(parent);
        }
        branch = parent;
    }
    ancestors
}

fn details_reveal_flat_tree_parent(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = runtime.dom_host().parent_node(handle)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    if runtime.dom_host().is_html_element_named(parent, "slot")
        && !runtime
            .dom_host()
            .assigned_nodes_for_slot_with_options(parent, false)
            .is_empty()
    {
        return None;
    }
    if runtime.dom_host().shadow_root_handle(parent).is_some()
        && runtime
            .dom_host()
            .node(handle)
            .is_some_and(|node| node.is_element() || node.is_text())
    {
        return None;
    }
    Some(parent)
}

fn main_summary_details_handle(runtime: &JsContextHost, summary: DomHandle) -> Option<DomHandle> {
    let summary_element = runtime
        .dom_host()
        .node(summary)
        .and_then(|node| node.as_element())?;
    if !summary_element.is_html_element("summary") {
        return None;
    }
    let details = runtime.dom_host().parent_node(summary)?;
    (main_summary_child(runtime, details) == Some(summary)).then_some(details)
}

pub(super) fn perform_summary_click_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let is_summary = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.is_html_element("summary"));
    if !is_summary {
        return false;
    }
    let Some(details) = main_summary_details_handle(runtime, handle) else {
        return true;
    };
    let was_open = element_has_attribute(runtime, details, "open");
    set_reflected_boolean_attribute(scope, runtime_ptr, details, "open", !was_open);
    true
}

fn dialog_close_return_value_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<String>, webidl::WebIdlError> {
    if args.length() <= index || args.get(index).is_undefined() {
        return Ok(None);
    }
    webidl::argument::<webidl::DomString>(
        scope,
        args,
        index,
        webidl::Context::argument("HTMLDialogElement.close", 1),
    )
    .map(|value| Some(value.0))
}

fn dialog_runtime_and_handle_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> std::result::Result<(*mut JsContextHost, DomHandle), String> {
    node_runtime_and_handle_from_object_or_detached(scope, object)
}

fn dialog_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = dialog_runtime_and_handle_from_object(scope, object) else {
        rv.set_undefined();
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        name,
    ));
}

fn dialog_set_open_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    open: bool,
    modal: bool,
) {
    let Ok((runtime_ptr, handle)) = dialog_runtime_and_handle_from_object(scope, object) else {
        return;
    };
    dialog_set_open_state_for_handle(scope, runtime_ptr, handle, open, modal);
}

fn dialog_set_open_state_for_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    open: bool,
    modal: bool,
) {
    set_reflected_boolean_attribute(scope, runtime_ptr, handle, "open", open);
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime
        .dom_host_mut()
        .set_dialog_modal(handle, open && modal);
}

fn dialog_is_modal(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.dialog_modal())
}

pub(super) fn details_open_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, args.this(), "HTMLDetailsElement", "open", "details")
    else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "open",
    ));
}

pub(super) fn details_open_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, args.this(), "HTMLDetailsElement", "open", "details")
    else {
        rv.set_undefined();
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        "open",
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

pub(super) fn dialog_open_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    dialog_boolean_attribute_getter(scope, args.this(), "open", rv);
}

pub(super) fn dialog_open_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    dialog_set_open_state(scope, args.this(), args.get(0).boolean_value(scope), false);
    rv.set_undefined();
}

pub(super) fn dialog_return_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = dialog_runtime_and_handle_from_object(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .map(|element| element.dialog_return_value())
        .unwrap_or_default();
    let Some(value) = v8_string(scope, value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(super) fn dialog_return_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = dialog_runtime_and_handle_from_object(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) =
        property_dom_string_value(scope, args.get(0), "HTMLDialogElement", "returnValue")
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_dialog_return_value(handle, &value);
    rv.set_undefined();
}

pub(super) fn dialog_show_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = dialog_runtime_and_handle_from_object(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if element_has_attribute(runtime, handle, "open") {
        if dialog_is_modal(runtime, handle) {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "The dialog is already open as a modal dialog.",
            );
        }
        return;
    }
    dialog_set_open_state_for_handle(scope, runtime_ptr, handle, true, false);
}

pub(super) fn dialog_show_modal_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = dialog_runtime_and_handle_from_object(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if element_has_attribute(runtime, handle, "open") {
        if !dialog_is_modal(runtime, handle) {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "The dialog is already open as a non-modal dialog.",
            );
        }
        return;
    }
    if !runtime.dom_host().is_connected(handle) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The dialog is not connected to a fully active document.",
        );
        return;
    }
    dialog_set_open_state_for_handle(scope, runtime_ptr, handle, true, true);
}

pub(super) fn dialog_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = args.this();
    let Some(parsed) = webidl::parse_args::<DialogCloseArgs>(scope, &args) else {
        return;
    };
    let Ok((runtime_ptr, handle)) = dialog_runtime_and_handle_from_object(scope, object) else {
        return;
    };
    close_dialog_element(scope, runtime_ptr, handle, parsed.return_value.as_deref());
}

pub(in crate::native_bridge::element) fn close_dialog_element(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    return_value: Option<&str>,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "dialog")
        || !element_has_attribute(runtime, handle, "open")
    {
        return false;
    }

    set_reflected_boolean_attribute(scope, runtime_ptr, handle, "open", false);
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.dom_host_mut().set_dialog_modal(handle, false);
    if let Some(return_value) = return_value {
        let _ = runtime
            .dom_host_mut()
            .set_dialog_return_value(handle, return_value);
    }
    queue_dialog_close_event(scope, runtime, handle);
    true
}

fn queue_dialog_close_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &mut JsContextHost,
    handle: DomHandle,
) {
    let _ = runtime.queue_user_interaction_event_task(
        scope,
        RendererPageUserInteractionEventKind::DialogClose,
        handle,
    );
}
