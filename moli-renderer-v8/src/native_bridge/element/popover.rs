use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::native_bridge::callback_value_dom_handle;
use crate::page_task_queue::{
    RendererPageElementToggleEventKind, RendererPageElementToggleEventState,
};
use crate::util::{node_wrapper_from_handle, throw_type_error, v8_string, v8str};
use dom::ElementState as StyloElementState;

use super::super::node::{
    node_is_element, node_runtime_and_handle_from_args_or_detached,
    node_runtime_and_handle_from_object_or_detached,
};
use super::super::throw_dom_exception;
use super::focus::{focus_element, is_focusable, update_focus};
use super::toggle_event::queue_element_toggle_event;
use super::{
    JsContextHost, dispatch_public_event, element_attribute, element_has_attribute,
    remove_reflected_attribute, set_reflected_attribute,
};

fn canonical_popover_state(raw: &str) -> &'static str {
    if raw.is_empty() || raw.eq_ignore_ascii_case("auto") {
        "auto"
    } else if raw.eq_ignore_ascii_case("hint") {
        "hint"
    } else {
        "manual"
    }
}

fn is_manual_popover(runtime: &JsContextHost, handle: DomHandle) -> bool {
    element_attribute(runtime, handle, "popover")
        .as_deref()
        .is_some_and(|value| canonical_popover_state(value) == "manual")
}

fn popover_is_open(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.popover_open() && runtime.dom_host().is_connected(handle))
}

fn ensure_popover_supported(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    if element_has_attribute(runtime, handle, "popover") {
        return true;
    }
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Popover methods require a popover attribute.",
    );
    false
}

fn ensure_popover_connected(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    if runtime.dom_host().is_connected(handle) {
        return true;
    }
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "Popover methods require a connected element.",
    );
    false
}

fn construct_popover_toggle_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    old_state: &str,
    new_state: &str,
    cancelable: bool,
    source: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = source
        .and_then(|handle| node_wrapper_from_handle(scope, handle).map(Into::into))
        .unwrap_or_else(|| v8::null(scope).into());
    super::events::construct_toggle_event(
        scope, event_type, old_state, new_state, cancelable, source,
    )
}

pub(crate) fn dispatch_popover_removal_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    if let Some(event) =
        construct_popover_toggle_event(scope, "beforetoggle", "open", "closed", false, None)
    {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    queue_element_toggle_event(
        scope,
        runtime_ptr,
        RendererPageElementToggleEventKind::Popover,
        handle,
        RendererPageElementToggleEventState::Open,
        RendererPageElementToggleEventState::Closed,
        None,
    );
}

pub(crate) fn dispatch_popover_show_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    source_handle: DomHandle,
) {
    let _ = set_popover_open_state(scope, runtime_ptr, target, true, Some(source_handle));
}

pub(crate) fn dispatch_popover_toggle_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    source_handle: DomHandle,
) {
    let open = !popover_is_open(unsafe { &*runtime_ptr }, target);
    let _ = set_popover_open_state(scope, runtime_ptr, target, open, Some(source_handle));
}

fn set_popover_open_state(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    open: bool,
    source: Option<DomHandle>,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let was_open = popover_is_open(runtime, handle);
    if was_open == open {
        return open;
    }
    let old_style_state = runtime.retained_current_element_state(handle);
    let (old_state, new_state) = if open {
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
    if let Some(event) = construct_popover_toggle_event(
        scope,
        "beforetoggle",
        old_state.as_str(),
        new_state.as_str(),
        open,
        source,
    ) {
        let outcome = dispatch_public_event(scope, runtime_ptr, handle, event);
        if open && !outcome.allows_default() {
            return false;
        }
    }
    if open && !ensure_popover_connected(scope, unsafe { &*runtime_ptr }, handle) {
        return false;
    }
    if open && !is_manual_popover(unsafe { &*runtime_ptr }, handle) {
        close_open_auto_popovers(scope, runtime_ptr, handle, source);
    }
    if open && !ensure_popover_connected(scope, unsafe { &*runtime_ptr }, handle) {
        return false;
    }
    if open && !is_manual_popover(unsafe { &*runtime_ptr }, handle) {
        let runtime = unsafe { &*runtime_ptr };
        let restore_target = runtime
            .active_element_handle()
            .or_else(|| runtime.document_focus_fallback_handle());
        unsafe { &mut *runtime_ptr }.set_popover_focus_restore_target(handle, restore_target);
    }
    let changed = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_popover_open(handle, open);
    if changed {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.note_element_state_style_activity_with_old_state(
            handle,
            StyloElementState::POPOVER_OPEN,
            old_style_state,
        );
        runtime.invalidate_layout_after_interaction_state_change();
        if open {
            autofocus_popover_descendant(scope, runtime_ptr, handle);
        } else if !is_manual_popover(unsafe { &*runtime_ptr }, handle) {
            restore_popover_focus_if_needed(scope, runtime_ptr, handle);
        }
        queue_element_toggle_event(
            scope,
            runtime_ptr,
            RendererPageElementToggleEventKind::Popover,
            handle,
            old_state,
            new_state,
            source,
        );
    }
    open
}

fn autofocus_popover_descendant(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.has_attribute("autofocus"))
    {
        focus_element(scope, runtime_ptr, handle);
        return;
    }
    let target = first_light_tree_autofocus_descendant(runtime, handle);
    if let Some(target) = target {
        focus_element(scope, runtime_ptr, target);
    }
}

fn first_light_tree_autofocus_descendant(
    runtime: &JsContextHost,
    root: DomHandle,
) -> Option<DomHandle> {
    let mut stack = runtime.dom_host().child_handles(root).collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        if runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute("autofocus"))
            && is_focusable(runtime, handle)
        {
            return Some(handle);
        }
        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
    }
    None
}

fn restore_popover_focus_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let restore_target = unsafe { &mut *runtime_ptr }.take_popover_focus_restore_target(handle);
    let runtime = unsafe { &*runtime_ptr };
    let Some(active) = runtime.active_element_handle() else {
        return;
    };
    if !shadow_including_tree_contains(runtime, handle, active) {
        return;
    }
    let next = restore_target.filter(|target| {
        runtime
            .dom_host()
            .node(*target)
            .is_some_and(Node::is_connected)
            && is_focusable(runtime, *target)
            && !shadow_including_tree_contains(runtime, handle, *target)
    });
    update_focus(scope, runtime_ptr, next);
}

fn shadow_including_tree_contains(
    runtime: &JsContextHost,
    root: DomHandle,
    handle: DomHandle,
) -> bool {
    let mut current = handle;
    loop {
        if current == root {
            return true;
        }
        if let Some(parent) = runtime.dom_host().node(current).and_then(Node::parent_node) {
            current = parent;
            continue;
        }
        if runtime.dom_host().is_shadow_root(current)
            && let Some(host) = runtime.dom_host().shadow_root_host(current)
        {
            current = host;
            continue;
        }
        return false;
    }
}

fn close_open_auto_popovers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    opening: DomHandle,
    source: Option<DomHandle>,
) {
    let runtime = unsafe { &*runtime_ptr };
    let Some(root) = runtime.dom_host().root_node_handle(opening) else {
        return;
    };
    let candidates = runtime.dom_host().elements_by_tag_name(root, "*", true);
    for candidate in candidates {
        if candidate == opening || is_manual_popover(runtime, candidate) {
            continue;
        }
        if !popover_is_open(runtime, candidate) {
            continue;
        }
        if source.is_some_and(|source| runtime.dom_host().dom().contains(candidate, source)) {
            continue;
        }
        let _ = set_popover_open_state(scope, runtime_ptr, candidate, false, None);
    }
}

fn throw_redundant_popover_state_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "Popover is already in the requested state.",
    );
}

fn parse_popover_invocation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    allow_force: bool,
) -> Option<(Option<bool>, Option<DomHandle>)> {
    let value = args.get(0);
    if value.is_undefined() {
        return Some((None, None));
    }
    if value.is_boolean() {
        return Some((allow_force.then(|| value.boolean_value(scope)), None));
    }
    if !value.is_object() || value.is_null() {
        return Some((None, None));
    }
    let object = value.to_object(scope)?;
    let force = if allow_force {
        object
            .get(scope, v8str(scope, "force").into())
            .filter(|value| !value.is_undefined())
            .map(|value| value.boolean_value(scope))
    } else {
        None
    };
    let source_value = object.get(scope, v8str(scope, "source").into())?;
    let source = if source_value.is_undefined() {
        None
    } else if source_value.is_null() {
        throw_type_error(scope, "Popover source must be an Element.");
        return None;
    } else {
        let Some(handle) = callback_value_dom_handle(scope, source_value) else {
            throw_type_error(scope, "Popover source must be an Element.");
            return None;
        };
        Some(handle)
    };
    Some((force, source))
}

pub(in crate::native_bridge) fn node_popover_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let Some(value) = element_attribute(unsafe { &*runtime_ptr }, handle, "popover") else {
        rv.set_null();
        return;
    };
    let Some(value) = v8_string(scope, canonical_popover_state(&value)) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_popover_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let value = args.get(0);
    if value.is_null_or_undefined() {
        remove_reflected_attribute(scope, runtime_ptr, handle, "popover");
        return;
    }
    let Some(value) = value.to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    set_reflected_attribute(scope, runtime_ptr, handle, "popover", &value);
}

pub(in crate::native_bridge) fn node_show_popover_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let Some((_, source)) = parse_popover_invocation(scope, &args, false) else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !ensure_popover_supported(scope, runtime, handle)
        || !ensure_popover_connected(scope, runtime, handle)
    {
        rv.set_undefined();
        return;
    }
    if popover_is_open(runtime, handle) {
        throw_redundant_popover_state_error(scope);
        rv.set_undefined();
        return;
    }
    let _ = set_popover_open_state(scope, runtime_ptr, handle, true, source);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_hide_popover_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !ensure_popover_supported(scope, runtime, handle)
        || !ensure_popover_connected(scope, runtime, handle)
    {
        rv.set_undefined();
        return;
    }
    if !popover_is_open(runtime, handle) {
        throw_redundant_popover_state_error(scope);
        rv.set_undefined();
        return;
    }
    let _ = set_popover_open_state(scope, runtime_ptr, handle, false, None);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_toggle_popover_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_bool(false);
        return;
    };
    let Some((force, source)) = parse_popover_invocation(scope, &args, true) else {
        rv.set_bool(false);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !ensure_popover_supported(scope, runtime, handle)
        || !ensure_popover_connected(scope, runtime, handle)
    {
        rv.set_bool(false);
        return;
    }
    let next = force.unwrap_or_else(|| !popover_is_open(unsafe { &*runtime_ptr }, handle));
    let opened = set_popover_open_state(scope, runtime_ptr, handle, next, source);
    rv.set_bool(opened);
}

pub(crate) fn perform_popover_invoker_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    invoker: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let (target_id, action) =
        if let Some(target_id) = element_attribute(runtime, invoker, "popovertarget") {
            let action = element_attribute(runtime, invoker, "popovertargetaction")
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_else(|| "toggle".to_owned());
            (target_id, action)
        } else if let Some(target_id) = element_attribute(runtime, invoker, "commandfor") {
            let action = match element_attribute(runtime, invoker, "command")
                .map(|value| value.to_ascii_lowercase())
                .as_deref()
            {
                Some("show-popover") => "show",
                Some("hide-popover") => "hide",
                Some("toggle-popover") => "toggle",
                _ => return false,
            };
            (target_id, action.to_owned())
        } else {
            return false;
        };
    if target_id.is_empty() {
        return false;
    }
    let Some(target) = popover_invoker_target_for_id(runtime, invoker, &target_id) else {
        return false;
    };
    if !element_has_attribute(runtime, target, "popover") {
        return false;
    }
    let open = match action.as_str() {
        "show" => true,
        "hide" => false,
        _ => !popover_is_open(runtime, target),
    };
    let _ = set_popover_open_state(scope, runtime_ptr, target, open, Some(invoker));
    true
}

fn popover_invoker_target_for_id(
    runtime: &JsContextHost,
    invoker: DomHandle,
    id: &str,
) -> Option<DomHandle> {
    let root = runtime.dom_host().root_node_handle(invoker)?;
    let candidate = runtime
        .dom_host()
        .element_handle_by_id_in_subtree(root, id)?;
    runtime.dom_host().resolve_reference_target_chain(candidate)
}
