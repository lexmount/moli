use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::native_bridge::callback_value_dom_handle;
use crate::page_task_queue::{
    RendererPageElementToggleEventKind, RendererPageElementToggleEventState,
};
use crate::util::{throw_type_error, v8_string, v8str};
use dom::ElementState as StyloElementState;

use super::super::node::{
    node_is_element, node_runtime_and_handle_from_args_or_detached,
    node_runtime_and_handle_from_object_or_detached,
};
use super::super::throw_dom_exception;
use super::focus::{focus_element, is_focusable, update_focus};
use super::toggle_event::{construct_toggle_event_with_source, queue_element_toggle_event};
use super::{
    PageScriptHost, dispatch_public_event, has_reflected_attribute, reflected_attribute,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum PopoverStack {
    Auto,
    Hint,
}

#[derive(Default)]
pub(in crate::native_bridge) struct DocumentPopovers {
    auto: Vec<DomHandle>,
    hint: Vec<DomHandle>,
    hint_parent: Option<DomHandle>,
    showing: bool,
    hiding: Vec<DomHandle>,
}

impl DocumentPopovers {
    fn stack(&self, kind: PopoverStack) -> &[DomHandle] {
        match kind {
            PopoverStack::Auto => &self.auto,
            PopoverStack::Hint => &self.hint,
        }
    }

    fn stack_mut(&mut self, kind: PopoverStack) -> &mut Vec<DomHandle> {
        match kind {
            PopoverStack::Auto => &mut self.auto,
            PopoverStack::Hint => &mut self.hint,
        }
    }

    fn kind(&self, handle: DomHandle) -> Option<PopoverStack> {
        [PopoverStack::Auto, PopoverStack::Hint]
            .into_iter()
            .find(|&kind| self.stack(kind).contains(&handle))
    }

    fn remove(&mut self, handle: DomHandle) {
        if let Some(kind) = self.kind(handle) {
            self.stack_mut(kind).retain(|&entry| entry != handle);
        }
        if self.hint_parent == Some(handle) || self.hint.is_empty() {
            self.hint_parent = None;
        }
    }
}

impl PageScriptHost {
    pub(in crate::native_bridge) fn retire_document_popovers(&mut self, document: DomHandle) {
        let Some(state) = self.document_popovers.remove(&document) else {
            return;
        };
        for handle in state.auto.into_iter().chain(state.hint) {
            self.take_popover_focus_restore_target(handle);
            if self.dom_host().owner_document_handle(handle) == Some(document) {
                self.dom_host_mut().set_popover_open(handle, false);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct HidePopover {
    restore_focus: bool,
    fire_events: bool,
}

impl HidePopover {
    const DIRECT: Self = Self {
        restore_focus: true,
        fire_events: true,
    };
    const DISMISS: Self = Self {
        restore_focus: false,
        fire_events: true,
    };
    const SILENT: Self = Self {
        restore_focus: false,
        fire_events: false,
    };
}

fn popover_stack_kind(runtime: &PageScriptHost, handle: DomHandle) -> Option<PopoverStack> {
    match canonical_popover_state(&reflected_attribute(runtime, handle, "popover")?) {
        "auto" => Some(PopoverStack::Auto),
        "hint" => Some(PopoverStack::Hint),
        _ => None,
    }
}

pub(super) fn popover_is_open(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.popover_open() && runtime.dom_host().is_connected(handle))
}

fn ensure_popover_supported(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> bool {
    if has_reflected_attribute(runtime, handle, "popover") {
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
    runtime: &PageScriptHost,
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

fn can_show_popover(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &PageScriptHost,
    handle: DomHandle,
    expected_document: Option<DomHandle>,
    throw_exceptions: bool,
) -> bool {
    if !has_reflected_attribute(runtime, handle, "popover") {
        if throw_exceptions {
            ensure_popover_supported(scope, runtime, handle);
        }
        return false;
    }
    if popover_is_open(runtime, handle) {
        return false;
    }
    if expected_document.is_some_and(|document| {
        runtime.dom_host().is_connected(handle)
            && runtime.dom_host().owner_document_handle(handle) == Some(document)
            && runtime.document_is_fully_active(document)
            && !runtime
                .dom_host()
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.dialog_modal())
    }) {
        return true;
    }
    if throw_exceptions {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "A popover must be connected to its original active document and cannot be modal.",
        );
    }
    false
}

pub(crate) fn dispatch_popover_show_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    source_handle: DomHandle,
) {
    let _ = set_popover_open_state(scope, runtime_ptr, target, true, Some(source_handle), false);
}

pub(crate) fn dispatch_popover_hide_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    source_handle: DomHandle,
) {
    let _ = set_popover_open_state(
        scope,
        runtime_ptr,
        target,
        false,
        Some(source_handle),
        false,
    );
}

pub(crate) fn dispatch_popover_toggle_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    source_handle: DomHandle,
) {
    let open = !popover_is_open(unsafe { &*runtime_ptr }, target);
    let _ = set_popover_open_state(scope, runtime_ptr, target, open, Some(source_handle), false);
}

fn set_popover_open_state(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    open: bool,
    source: Option<DomHandle>,
    throw_exceptions: bool,
) -> bool {
    if open {
        show_popover(scope, runtime_ptr, handle, source, throw_exceptions);
    } else {
        hide_popover(scope, runtime_ptr, handle, source, HidePopover::DIRECT);
    }
    popover_is_open(unsafe { &*runtime_ptr }, handle)
}

fn show_popover(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    source: Option<DomHandle>,
    throw_exceptions: bool,
) {
    let runtime = unsafe { &*runtime_ptr };
    let owner_document = runtime.dom_host().owner_document_handle(handle);
    if owner_document
        .and_then(|document| runtime.document_popovers.get(&document))
        .is_some_and(|state| state.showing || !state.hiding.is_empty())
    {
        if throw_exceptions {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "A popover transition is already in progress in this document.",
            );
        }
        return;
    }
    if !can_show_popover(scope, runtime, handle, owner_document, throw_exceptions) {
        return;
    }
    let Some(document) = owner_document else {
        return;
    };
    unsafe { &mut *runtime_ptr }
        .document_popovers
        .entry(document)
        .or_default()
        .showing = true;
    show_popover_steps(
        scope,
        runtime_ptr,
        handle,
        document,
        source,
        throw_exceptions,
    );
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(state) = runtime.document_popovers.get_mut(&document) {
        state.showing = false;
    }
    discard_idle_popover_state(runtime, document);
}

fn show_popover_steps(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    document: DomHandle,
    source: Option<DomHandle>,
    throw_exceptions: bool,
) {
    if !dispatch_popover_beforetoggle(scope, runtime_ptr, handle, true, source) {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if !can_show_popover(scope, runtime, handle, Some(document), throw_exceptions) {
        return;
    }
    let original_kind = popover_stack_kind(runtime, handle);
    let mut stack_kind = original_kind;
    let mut ancestor = None;
    if let Some(kind) = original_kind {
        ancestor = topmost_popover_ancestor(runtime, document, handle, source);
        if kind == PopoverStack::Auto
            && ancestor.is_some_and(|(_, kind)| kind == PopoverStack::Hint)
        {
            stack_kind = Some(PopoverStack::Hint);
        }
        let endpoint = ancestor.map(|(ancestor, _)| ancestor);
        hide_popover_stack_until(
            scope,
            runtime_ptr,
            document,
            endpoint,
            PopoverStack::Hint,
            HidePopover::DISMISS,
        );
        if stack_kind == Some(PopoverStack::Auto) {
            hide_popover_stack_until(
                scope,
                runtime_ptr,
                document,
                endpoint,
                PopoverStack::Auto,
                HidePopover::DISMISS,
            );
        }
        let runtime = unsafe { &*runtime_ptr };
        if popover_stack_kind(runtime, handle) != original_kind {
            if throw_exceptions {
                throw_dom_exception(
                    scope,
                    "InvalidStateError",
                    11,
                    "The popover mode changed while it was showing.",
                );
            }
            return;
        }
        if !can_show_popover(scope, runtime, handle, Some(document), throw_exceptions) {
            return;
        }
    }

    let runtime = unsafe { &mut *runtime_ptr };
    if !update_popover_visibility(runtime, handle, true) {
        return;
    }
    if let Some(kind) = stack_kind {
        let state = runtime.document_popovers.entry(document).or_default();
        let first = state.auto.is_empty() && state.hint.is_empty();
        if let (PopoverStack::Hint, Some((parent, PopoverStack::Auto))) = (kind, ancestor)
            && state.auto.contains(&parent)
        {
            state.hint_parent = Some(parent);
        }
        state.stack_mut(kind).push(handle);
        if first {
            let previous = runtime
                .active_element_handle()
                .or_else(|| runtime.document_focus_fallback_handle());
            runtime.set_popover_focus_restore_target(handle, previous);
        }
    }
    autofocus_popover_descendant(scope, runtime_ptr, handle);
    queue_popover_toggle(scope, runtime_ptr, handle, true, source);
}

fn hide_popover(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    source: Option<DomHandle>,
    mut options: HidePopover,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if !popover_is_open(runtime, handle) {
        return;
    }
    let Some(document) = runtime.dom_host().owner_document_handle(handle) else {
        return;
    };
    let state = runtime.document_popovers.entry(document).or_default();
    options.fire_events &= !state.hiding.contains(&handle);
    state.hiding.push(handle);
    hide_dependent_popovers(scope, runtime_ptr, document, handle, options);
    if popover_is_open_in_document(unsafe { &*runtime_ptr }, document, handle) {
        if options.fire_events {
            dispatch_popover_beforetoggle(scope, runtime_ptr, handle, false, source);
        }
        let runtime = unsafe { &mut *runtime_ptr };
        if popover_is_open_in_document(runtime, document, handle)
            && update_popover_visibility(runtime, handle, false)
        {
            if let Some(state) = runtime.document_popovers.get_mut(&document) {
                state.remove(handle);
            }
            if options.fire_events {
                queue_popover_toggle(scope, runtime_ptr, handle, false, source);
            }
            if options.restore_focus {
                restore_popover_focus_if_needed(scope, runtime_ptr, handle);
            } else {
                unsafe { &mut *runtime_ptr }.take_popover_focus_restore_target(handle);
            }
        }
    }
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(state) = runtime.document_popovers.get_mut(&document) {
        let hidden = state.hiding.pop();
        debug_assert_eq!(hidden, Some(handle));
    }
    discard_idle_popover_state(runtime, document);
}

fn popover_is_open_in_document(
    runtime: &PageScriptHost,
    document: DomHandle,
    handle: DomHandle,
) -> bool {
    popover_is_open(runtime, handle)
        && runtime.dom_host().owner_document_handle(handle) == Some(document)
}

fn hide_dependent_popovers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    handle: DomHandle,
    options: HidePopover,
) {
    let Some(state) = (unsafe { &*runtime_ptr }).document_popovers.get(&document) else {
        return;
    };
    let hide_hint_stack = state.hint_parent == Some(handle);
    let kind = state.kind(handle);
    if hide_hint_stack || kind == Some(PopoverStack::Hint) {
        let endpoint = if hide_hint_stack { None } else { Some(handle) };
        hide_popover_stack_until(
            scope,
            runtime_ptr,
            document,
            endpoint,
            PopoverStack::Hint,
            options,
        );
    }
    if kind == Some(PopoverStack::Auto) {
        hide_popover_stack_until(
            scope,
            runtime_ptr,
            document,
            Some(handle),
            PopoverStack::Auto,
            options,
        );
    }
}

fn update_popover_visibility(runtime: &mut PageScriptHost, handle: DomHandle, open: bool) -> bool {
    let old_style_state = runtime.retained_current_element_state(handle);
    if !runtime.dom_host_mut().set_popover_open(handle, open) {
        return false;
    }
    runtime.note_element_state_style_activity_with_old_state(
        handle,
        StyloElementState::POPOVER_OPEN,
        old_style_state,
    );
    runtime.invalidate_layout_after_interaction_state_change();
    true
}

fn dispatch_popover_beforetoggle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    open: bool,
    source: Option<DomHandle>,
) -> bool {
    let (old_state, new_state) = toggle_states(open);
    construct_toggle_event_with_source(
        scope,
        "beforetoggle",
        old_state.as_str(),
        new_state.as_str(),
        open,
        source,
    )
    .is_none_or(|event| dispatch_public_event(scope, runtime_ptr, handle, event).allows_default())
}

fn queue_popover_toggle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    open: bool,
    source: Option<DomHandle>,
) {
    let (old_state, new_state) = toggle_states(open);
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

fn toggle_states(
    open: bool,
) -> (
    RendererPageElementToggleEventState,
    RendererPageElementToggleEventState,
) {
    use RendererPageElementToggleEventState::{Closed, Open};
    if open { (Closed, Open) } else { (Open, Closed) }
}

fn discard_idle_popover_state(runtime: &mut PageScriptHost, document: DomHandle) {
    if runtime
        .document_popovers
        .get(&document)
        .is_some_and(|state| {
            state.auto.is_empty()
                && state.hint.is_empty()
                && !state.showing
                && state.hiding.is_empty()
        })
    {
        runtime.document_popovers.remove(&document);
    }
}

pub(crate) fn remove_popover_stack_entry(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.take_popover_focus_restore_target(handle);
    // Adoption can change nodeDocument before the removal effects are delivered.
    let document = runtime
        .document_popovers
        .iter()
        .find_map(|(document, state)| state.kind(handle).map(|_| *document));
    let Some(document) = document else {
        return;
    };
    hide_dependent_popovers(scope, runtime_ptr, document, handle, HidePopover::SILENT);
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(state) = runtime.document_popovers.get_mut(&document) {
        state.remove(handle);
    }
    discard_idle_popover_state(runtime, document);
}

pub(crate) fn handle_popover_attribute_change(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    namespace: Option<&str>,
    local_name: &str,
    old_value: Option<&str>,
    new_value: Option<&str>,
) {
    if namespace.is_some() || !local_name.eq_ignore_ascii_case("popover") {
        return;
    }
    let old_type = old_value.map(canonical_popover_state);
    let new_type = new_value.map(canonical_popover_state);
    if old_type == new_type {
        return;
    }
    if popover_is_open(unsafe { &*runtime_ptr }, handle) {
        let _ = set_popover_open_state(scope, runtime_ptr, handle, false, None, false);
    } else {
        remove_popover_stack_entry(scope, runtime_ptr, handle);
    }
    let runtime = unsafe { &mut *runtime_ptr };
    if new_value.is_none() && !has_reflected_attribute(runtime, handle, "popover") {
        runtime.clear_popover_toggle_task_tracker(scope, handle);
    }
}

fn autofocus_popover_descendant(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
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
    runtime: &PageScriptHost,
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
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let Some(restore_target) =
        (unsafe { &mut *runtime_ptr }).take_popover_focus_restore_target(handle)
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(active) = runtime.active_element_handle() else {
        return;
    };
    if !shadow_including_tree_contains(runtime, handle, active) {
        return;
    }
    let next = Some(restore_target).filter(|target| {
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
    runtime: &PageScriptHost,
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

fn popover_flat_tree_contains(
    runtime: &PageScriptHost,
    ancestor: DomHandle,
    handle: DomHandle,
) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        if candidate == ancestor {
            return true;
        }
        current = popover_flat_tree_parent(runtime, candidate);
    }
    false
}

fn popover_flat_tree_parent(runtime: &PageScriptHost, handle: DomHandle) -> Option<DomHandle> {
    if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = runtime.dom_host().parent_node(handle)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    if runtime.dom_host().shadow_root_handle(parent).is_some() {
        return None;
    }
    if runtime.dom_host().is_html_element_named(parent, "slot")
        && runtime.dom_host().containing_shadow_root(parent).is_some()
        && !runtime
            .dom_host()
            .assigned_nodes_for_slot_with_options(parent, false)
            .is_empty()
    {
        return None;
    }
    Some(parent)
}

fn topmost_popover_ancestor(
    runtime: &PageScriptHost,
    document: DomHandle,
    handle: DomHandle,
    source: Option<DomHandle>,
) -> Option<(DomHandle, PopoverStack)> {
    let state = runtime.document_popovers.get(&document)?;
    for stack in [PopoverStack::Hint, PopoverStack::Auto] {
        if let Some(ancestor) = state.stack(stack).iter().rev().find(|&&candidate| {
            candidate != handle
                && popover_is_open_in_document(runtime, document, candidate)
                && (popover_flat_tree_contains(runtime, candidate, handle)
                    || source.is_some_and(|source| {
                        popover_flat_tree_contains(runtime, candidate, source)
                    }))
        }) {
            return Some((*ancestor, stack));
        }
    }
    None
}

fn hide_popover_stack_until(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    endpoint: Option<DomHandle>,
    stack: PopoverStack,
    options: HidePopover,
) {
    let stack_snapshot = |runtime: &PageScriptHost| {
        runtime
            .document_popovers
            .get(&document)
            .map(|state| state.stack(stack).to_vec())
            .unwrap_or_default()
    };
    let current = stack_snapshot(unsafe { &*runtime_ptr });
    let keep = endpoint
        .and_then(|endpoint| current.iter().position(|&handle| handle == endpoint))
        .map_or(0, |position| position + 1);
    if keep == current.len() {
        return;
    }
    let (remaining, dismissed) = current.split_at(keep);
    for &handle in dismissed.iter().rev() {
        hide_popover(scope, runtime_ptr, handle, None, options);
    }
    // Event handlers can mutate the tree during dismissal. Reconcile against the
    // original prefix without firing a second round of author callbacks.
    for handle in stack_snapshot(unsafe { &*runtime_ptr }).into_iter().rev() {
        if !remaining.contains(&handle) {
            hide_popover(
                scope,
                runtime_ptr,
                handle,
                None,
                HidePopover {
                    fire_events: false,
                    ..options
                },
            );
        }
    }
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
    let Some(value) = reflected_attribute(unsafe { &*runtime_ptr }, handle, "popover") else {
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
    show_popover(scope, runtime_ptr, handle, source, true);
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
    if !ensure_popover_supported(scope, runtime, handle) {
        rv.set_undefined();
        return;
    }
    if !popover_is_open(runtime, handle) {
        rv.set_undefined();
        return;
    }
    if !ensure_popover_connected(scope, runtime, handle) {
        rv.set_undefined();
        return;
    }
    let _ = set_popover_open_state(scope, runtime_ptr, handle, false, None, false);
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
    let opened = set_popover_open_state(scope, runtime_ptr, handle, next, source, true);
    rv.set_bool(opened);
}

pub(crate) fn perform_popover_invoker_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    invoker: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let Some(target_id) = reflected_attribute(runtime, invoker, "popovertarget") else {
        return false;
    };
    let action = reflected_attribute(runtime, invoker, "popovertargetaction")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "toggle".to_owned());
    if target_id.is_empty() {
        return false;
    }
    let Some(target) = popover_invoker_target_for_id(runtime, invoker, &target_id) else {
        return false;
    };
    if !has_reflected_attribute(runtime, target, "popover") {
        return false;
    }
    let open = match action.as_str() {
        "show" => true,
        "hide" => false,
        _ => !popover_is_open(runtime, target),
    };
    let _ = set_popover_open_state(scope, runtime_ptr, target, open, Some(invoker), false);
    true
}

fn popover_invoker_target_for_id(
    runtime: &PageScriptHost,
    invoker: DomHandle,
    id: &str,
) -> Option<DomHandle> {
    let root = runtime.dom_host().root_node_handle(invoker)?;
    let candidate = runtime
        .dom_host()
        .element_handle_by_id_in_subtree(root, id)?;
    runtime.dom_host().resolve_reference_target_chain(candidate)
}
