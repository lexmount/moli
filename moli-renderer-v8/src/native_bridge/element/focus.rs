use crate::dom::native::{Element, FocusSupport, Node};

use super::super::{
    PageScriptHost, PendingWindowMessageEndpoint, SequentialFocusStartingPoint, document,
    node::node_runtime_and_handle_from_args,
};
use super::forms::{
    dispatch_text_control_event, form_control_is_effectively_disabled, is_text_control,
    text_control_value_dom_string,
};
use super::geometry::{read_element_metrics, scroll_node_into_view_if_needed};
use super::styles::{StyleMode, style_property_value};
use super::{
    ClickEventSource, InputEventCoordinates, construct_focus_event, construct_interest_event,
    dispatch_popover_show_events, dispatch_public_event,
    resolved_reflected_element_attribute_handle,
};
use super::{
    has_reflected_attribute, label_control_handle, label_receives_programmatic_focus,
    reflected_attribute,
};
use crate::document_runtime::{DomHandle, EventTargetHandle};
#[cfg(feature = "devtools")]
use crate::runtime::RendererDomFocusOutcome;
use crate::util::{node_wrapper_from_handle, v8_string, v8str};
use crate::v8_execution_watchdog::{
    SCRIPT_TURN_WATCHDOG_TIMEOUT, V8ExecutionWatchdog, V8ExecutionWatchdogKind,
    V8ExecutionWatchdogOutcome,
};

mod navigation;

struct SequentialFocusEntry {
    tab_index: i32,
    order: usize,
    handles: Vec<DomHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadingFlowMode {
    SourceOrder,
    GridOrder,
    GridRows,
    GridColumns,
    FlexFlow,
    FlexVisual,
}

pub(crate) fn is_disabled_form_control(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    form_control_is_effectively_disabled(runtime, handle)
}

pub(crate) fn contenteditable_editing_host(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    runtime
        .dom_host()
        .dom()
        .contenteditable_editing_host(handle)
}

fn html_area_has_associated_image_map(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    let dom = runtime.dom_host();
    let Some(area) = dom.node(handle).and_then(Node::as_element) else {
        return false;
    };
    if !area.is_html_element("area") {
        return false;
    }

    let mut ancestor = dom.parent_node(handle);
    let map_name = loop {
        let Some(candidate) = ancestor else {
            return false;
        };
        let Some(element) = dom.node(candidate).and_then(Node::as_element) else {
            ancestor = dom.parent_node(candidate);
            continue;
        };
        if element.namespace() == document::XHTML_NS && element.local_name() == "map" {
            let Some(name) = element.attribute("name").filter(|name| !name.is_empty()) else {
                return false;
            };
            break name.to_owned();
        }
        ancestor = dom.parent_node(candidate);
    };

    let Some(document) = dom.owner_document_handle(handle) else {
        return false;
    };
    let mut stack = dom.child_handles(document).collect::<Vec<_>>();
    while let Some(candidate) = stack.pop() {
        let Some(element) = dom.node(candidate).and_then(Node::as_element) else {
            stack.extend(dom.child_handles(candidate));
            continue;
        };
        if element.namespace() == document::XHTML_NS
            && matches!(element.local_name(), "img" | "object")
            && element
                .attribute("usemap")
                .and_then(|value| value.strip_prefix('#'))
                .is_some_and(|name| name == map_name)
        {
            return true;
        }
        stack.extend(dom.child_handles(candidate));
    }
    false
}

pub(super) fn is_focusable(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    let dom = runtime.dom_host().dom();
    if !dom.node(handle).is_some_and(Node::is_connected)
        || is_disabled_form_control(runtime, handle)
        || element_focus_is_suppressed(runtime, handle)
    {
        return false;
    }
    if dom
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_html_element("area"))
        && !html_area_has_associated_image_map(runtime, handle)
    {
        return false;
    }
    match dom.focus_support(handle) {
        FocusSupport::Blocked => false,
        FocusSupport::Supported => true,
        FocusSupport::LayoutDependent => element_is_scrollable(runtime, handle),
    }
}

fn svg_element_is_non_renderable(element: &Element) -> bool {
    // Only directly renderable SVG elements can receive focus. Definitions,
    // effects, animation and unknown elements cannot opt in with display or
    // tabindex. Source <symbol> elements are not <use> shadow-tree instances.
    element.namespace() == document::SVG_NS
        && !matches!(
            element.local_name(),
            "a" | "circle"
                | "ellipse"
                | "foreignObject"
                | "g"
                | "image"
                | "line"
                | "path"
                | "polygon"
                | "polyline"
                | "rect"
                | "svg"
                | "switch"
                | "text"
                | "textPath"
                | "tspan"
                | "use"
        )
}

fn element_focus_is_suppressed(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    let visibility = style_property_value(runtime, handle, StyleMode::Computed, "visibility");
    if matches!(
        visibility.trim().to_ascii_lowercase().as_str(),
        "hidden" | "collapse"
    ) {
        return true;
    }

    let mut current = Some(handle);
    while let Some(candidate) = current {
        if let Some(element) = runtime
            .dom_host()
            .node(candidate)
            .and_then(Node::as_element)
            && (element.has_attribute("inert")
                || element.has_attribute("hidden")
                || svg_element_is_non_renderable(element)
                || computed_display(runtime, candidate) == "none")
        {
            return true;
        }
        current = flat_tree_parent(runtime, candidate);
    }
    false
}

fn overflow_makes_scroll_container(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "auto" | "scroll"
    )
}

fn element_is_scrollable(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    let scrolls_x = overflow_makes_scroll_container(&style_property_value(
        runtime,
        handle,
        StyleMode::Computed,
        "overflow-x",
    ));
    let scrolls_y = overflow_makes_scroll_container(&style_property_value(
        runtime,
        handle,
        StyleMode::Computed,
        "overflow-y",
    ));
    if !scrolls_x && !scrolls_y {
        return false;
    }
    match read_element_metrics(runtime, handle) {
        Ok(Some(metrics)) => {
            metrics.is_scroll_container
                && (scrolls_x && metrics.maximum_scroll_offset.x > metrics.minimum_scroll_offset.x
                    || scrolls_y
                        && metrics.maximum_scroll_offset.y > metrics.minimum_scroll_offset.y)
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                ?handle,
                %error,
                "failed to evaluate scroll-container focusability from layout"
            );
            false
        }
    }
}

fn shadow_including_contains(
    runtime: &PageScriptHost,
    ancestor: DomHandle,
    node: DomHandle,
) -> bool {
    let mut current = Some(node);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = runtime.dom_host().parent_node(handle).or_else(|| {
            runtime
                .dom_host()
                .is_shadow_root(handle)
                .then(|| runtime.dom_host().shadow_root_host(handle))
                .flatten()
        });
    }
    false
}

fn focusable_area_for_element(runtime: &PageScriptHost, handle: DomHandle) -> Option<DomHandle> {
    if is_focusable(runtime, handle) {
        return Some(handle);
    }
    let shadow_root = runtime.dom_host().shadow_root_handle(handle)?;
    (runtime.dom_host().shadow_root_delegates_focus(shadow_root) == Some(true))
        .then(|| first_delegates_focus_target(runtime, shadow_root))
        .flatten()
}

fn first_autofocus_delegate(runtime: &PageScriptHost, root: DomHandle) -> Option<DomHandle> {
    let mut stack = runtime.dom_host().child_handles(root).collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        let autofocus = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute("autofocus"));
        if autofocus && let Some(target) = focusable_area_for_element(runtime, handle) {
            return Some(target);
        }

        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
    }
    None
}

fn first_focus_delegate(
    runtime: &PageScriptHost,
    root: DomHandle,
    require_sequential_focus: bool,
) -> Option<DomHandle> {
    if let Some(target) = first_autofocus_delegate(runtime, root) {
        return Some(target);
    }

    let mut stack = runtime.dom_host().child_handles(root).collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        let is_direct_delegate = if require_sequential_focus {
            sequential_tab_index(runtime, handle, None).is_some()
        } else {
            is_focusable(runtime, handle)
        };
        if is_direct_delegate {
            return Some(handle);
        }

        if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
            && runtime.dom_host().shadow_root_delegates_focus(shadow_root) == Some(true)
            && let Some(target) = first_delegates_focus_target(runtime, shadow_root)
        {
            return Some(target);
        }

        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
    }
    None
}

fn first_delegates_focus_target(runtime: &PageScriptHost, root: DomHandle) -> Option<DomHandle> {
    first_focus_delegate(runtime, root, false)
}

fn first_dialog_focus_target(runtime: &PageScriptHost, dialog: DomHandle) -> Option<DomHandle> {
    first_focus_delegate(runtime, dialog, true)
}

fn delegated_focus_target(runtime: &PageScriptHost, handle: DomHandle) -> Option<DomHandle> {
    let root = runtime.dom_host().shadow_root_handle(handle)?;
    if runtime.dom_host().shadow_root_delegates_focus(root) != Some(true) {
        return None;
    }
    if let Some(active) = runtime.active_element_handle()
        && shadow_including_contains(runtime, root, active)
    {
        return Some(active);
    }
    first_delegates_focus_target(runtime, root)
}

/// Whether the Document has post-parse autofocus work worth
/// admitting to the rendering-update task source.
///
/// Current focus and candidate focusability are resolved during rendering:
/// script can blur, remove or enable an element before the selected task runs.
/// A Document that still has author focus must consume its candidates and
/// complete its one-time autofocus decision at that rendering opportunity.
pub(crate) fn post_parse_autofocus_is_pending(
    runtime: &PageScriptHost,
    document: DomHandle,
) -> bool {
    !runtime.autofocus_processed(document) && runtime.next_autofocus_candidate(document).is_some()
}

pub(crate) fn process_post_parse_autofocus(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.autofocus_processed(document) || runtime.next_autofocus_candidate(document).is_none()
    {
        return false;
    }
    if runtime.document_focused_area(document).is_some()
        || runtime
            .dom_host()
            .document_target_element(document)
            .is_some()
    {
        runtime.mark_autofocus_processed(document);
        return false;
    }
    while let Some(candidate) = runtime.next_autofocus_candidate(document) {
        let candidate_document = runtime.dom_host().owner_document_handle(candidate);
        if !runtime.dom_host().is_connected(candidate)
            || candidate_document.and_then(|doc| runtime.top_level_document_for_document(doc))
                != Some(document)
        {
            runtime.pop_autofocus_candidate(document);
            continue;
        }
        let candidate_document = candidate_document.expect("active candidate Document");
        // The first remaining candidate keeps its place while its own Document
        // waits for styles. An unrelated frame must not delay or overtake it.
        if runtime.autofocus_document_has_blocking_stylesheets(candidate_document) {
            return false;
        }
        runtime.pop_autofocus_candidate(document);
        if runtime.autofocus_ancestor_has_target(candidate_document) {
            continue;
        }
        let target = focusable_area_for_element(runtime, candidate)
            .map(|target| delegated_focus_target(runtime, target).unwrap_or(target));
        let Some(target) = target else {
            continue;
        };
        // Focus listeners can replace the Document. Complete this Document's
        // one-time autofocus state before running any author code.
        runtime.mark_autofocus_processed(document);
        update_focus(scope, runtime_ptr, Some(target));
        return true;
    }
    false
}

pub(crate) fn reset_document_navigation_focus(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    user_initiated: bool,
) {
    let runtime = unsafe { &*runtime_ptr };
    let Some(endpoint) = runtime.window_endpoint_for_document(document) else {
        return;
    };
    let allowed = user_initiated
        || runtime.window_has_transient_user_activation(endpoint.dispatch_scope())
        || runtime
            .document_permissions_policy_for_document_handle(document)
            .is_some_and(|policy| policy.focus_without_user_activation_enabled());
    if !allowed {
        return;
    }
    // Navigation autofocus is a fresh search of this Document. The one-time
    // post-parse autofocus processed flag does not apply to an intercepted navigation.
    let focus_target = first_autofocus_delegate(runtime, document).or_else(|| {
        runtime
            .dom_host()
            .document_body_handle_for_document(document)
            .or_else(|| {
                runtime
                    .dom_host()
                    .document_element_handle_for_document(document)
            })
    });
    let target = focus_target.and_then(|element| focusable_area_for_element(runtime, element));
    if let Some(target) = target {
        update_focus(scope, runtime_ptr, Some(target));
    } else {
        let previous = runtime.active_element_handle();
        let viewport = match endpoint {
            PendingWindowMessageEndpoint::ChildWindow(container) => Some(container),
            _ => None,
        };
        update_focus_with_selection_behavior(
            scope,
            runtime_ptr,
            previous,
            viewport,
            Some(document),
            SelectionBehaviorOnFocus::Preserve,
        );
    }
    unsafe { &mut *runtime_ptr }.set_sequential_focus_starting_point(document, focus_target);
}

pub(super) fn run_dialog_focusing_steps(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    dialog: DomHandle,
) {
    let target = {
        let runtime = unsafe { &*runtime_ptr };
        if has_reflected_attribute(runtime, dialog, "autofocus") {
            dialog
        } else {
            first_dialog_focus_target(runtime, dialog).unwrap_or(dialog)
        }
    };
    focus_element(scope, runtime_ptr, target);

    let top_document = {
        let runtime = unsafe { &*runtime_ptr };
        let owner_document = runtime.dom_host().owner_document_handle(target);
        let same_origin_with_top = owner_document == Some(runtime.document_handle())
            || owner_document.is_some_and(|document| {
                runtime
                    .lightweight_popup_id_for_document_handle(document)
                    .is_some()
            })
            || owner_document
                .and_then(|document| {
                    runtime.child_browsing_context_host_for_document_handle(document)
                })
                .is_some_and(|child| {
                    runtime.child_window_has_same_origin_with_its_top_level_origin(child)
                });
        owner_document
            .filter(|_| same_origin_with_top)
            .and_then(|document| runtime.top_level_document_for_document(document))
    };
    if let Some(document) = top_document {
        unsafe { &mut *runtime_ptr }.mark_autofocus_processed(document);
    }
}

pub(super) fn apply_modal_dialog_focus_fixup(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    dialog: DomHandle,
) {
    let should_clear_focus = {
        let runtime = unsafe { &*runtime_ptr };
        runtime
            .active_element_handle()
            .is_some_and(|active| !shadow_including_contains(runtime, dialog, active))
    };
    if should_clear_focus {
        update_focus(scope, runtime_ptr, None);
    }
}

pub(super) fn remember_dialog_previously_focused_element(
    runtime_ptr: *mut PageScriptHost,
    dialog: DomHandle,
) {
    let focused = unsafe { &*runtime_ptr }.active_element_handle();
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_dialog_previously_focused_element(dialog, focused);
}

pub(super) fn restore_dialog_focus_after_close(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    dialog: DomHandle,
    was_modal: bool,
) {
    let (previously_focused, should_restore) = {
        let runtime = unsafe { &*runtime_ptr };
        let previously_focused = runtime
            .dom_host()
            .node(dialog)
            .and_then(Node::as_element)
            .and_then(|element| element.dialog_previously_focused_element());
        let focus_is_inside_dialog = runtime
            .active_element_handle()
            .is_some_and(|active| flat_tree_contains(runtime, dialog, active));
        (previously_focused, was_modal || focus_is_inside_dialog)
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_dialog_previously_focused_element(dialog, None);
    let Some(previously_focused) = previously_focused.filter(|_| should_restore) else {
        return;
    };
    if let Err(error) = focus_element_with_options(scope, runtime_ptr, previously_focused, true) {
        tracing::warn!(?previously_focused, %error, "failed to restore focus after closing dialog");
    }
}

fn flat_tree_contains(runtime: &PageScriptHost, ancestor: DomHandle, node: DomHandle) -> bool {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if candidate == ancestor {
            return true;
        }
        current = flat_tree_parent(runtime, candidate);
    }
    false
}

pub(crate) fn apply_document_focus_fixup(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let Some(active) = runtime.active_element_handle() else {
        return false;
    };
    if runtime.dom_host().owner_document_handle(active) != Some(document)
        || is_focusable(runtime, active)
    {
        return false;
    }
    // The ordinary focus transition owns events, text-control change commits,
    // viewport fallback, and retention of the sequential navigation origin.
    // This runs outside an author animation callback or timer. Its change/blur
    // handlers and cleanup microtasks still need their own execution budget.
    let watchdog = V8ExecutionWatchdog::arm(
        V8ExecutionWatchdogKind::FocusFixup,
        scope.thread_safe_handle(),
        SCRIPT_TURN_WATCHDOG_TIMEOUT,
    );
    // Clear the automatic change before dispatching events so a blur handler's
    // author focus change can still suppress navigation focus reset.
    unsafe { &mut *runtime_ptr }.clear_document_focused_area_for_fixup(document, active);
    update_focus(scope, runtime_ptr, None);
    crate::script_cleanup::perform_callback_cleanup_checkpoint(scope);
    if watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut {
        tracing::warn!("focus fixup exceeded its execution deadline");
    }
    true
}

fn wrap_handle_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Value>> {
    let handle = handle?;
    let runtime = unsafe { &mut *runtime_ptr };
    let wrapped = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)?;
    Some(wrapped.into())
}

fn dispatch_pending_text_control_change_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let committed_value = unsafe { &mut *runtime_ptr }.take_text_control_change_commit(handle);
    let Some(committed_value) = committed_value else {
        return;
    };
    let current_value = text_control_value_dom_string(unsafe { &*runtime_ptr }, handle);
    if current_value != committed_value {
        dispatch_text_control_event(scope, runtime_ptr, handle, "change");
    }
}

pub(crate) fn update_focus(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    next: Option<DomHandle>,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    let previous = runtime.active_element_handle();
    update_focus_from_previous(scope, runtime_ptr, previous, next);
}

/// Focus the Window's active Document, preserving its local focused control.
/// A viewport focus target still identifies its own Document.
pub(crate) fn focus_window_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    let local_area = runtime.document_focused_area(document).filter(|element| {
        runtime.dom_host().owner_document_handle(*element) == Some(document)
            && !runtime.dom_host().is_html_element_named(*element, "iframe")
            && !runtime.dom_host().is_html_element_named(*element, "frame")
            && is_focusable(runtime, *element)
    });
    let viewport =
        local_area.or_else(|| runtime.child_browsing_context_host_for_document_handle(document));
    let previous = runtime.active_element_handle();
    update_focus_with_selection_behavior(
        scope,
        runtime_ptr,
        previous,
        viewport,
        local_area.is_none().then_some(document),
        SelectionBehaviorOnFocus::Preserve,
    );
    unsafe { &mut *runtime_ptr }.note_window_focus_changes();
}

pub(crate) fn dispatch_system_focus_change(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    window: EventTargetHandle,
    focused: bool,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let (current_document, element) = runtime.focused_document_and_element();
    let element = element.filter(|_| current_document == document);
    let (event_type, bubbling_type) = if focused {
        ("focus", "focusin")
    } else {
        ("blur", "focusout")
    };
    let dispatch_window = |scope: &mut v8::PinScope<'_, '_>| {
        let runtime = unsafe { &mut *runtime_ptr };
        if !runtime.document_is_fully_active(document) {
            return false;
        }
        let Some(event) = construct_focus_event(scope, event_type, None, false) else {
            return false;
        };
        runtime
            .dispatch_public_event_best_effort(
                scope,
                runtime_ptr,
                window,
                event,
                "system focus event",
            )
            .is_ok()
    };

    // The selected area's DOM focus and selection survive system focus loss.
    // Notify its Window before the element on gain and after it on loss.
    let watchdog = V8ExecutionWatchdog::arm(
        V8ExecutionWatchdogKind::LifecycleEvent,
        scope.thread_safe_handle(),
        SCRIPT_TURN_WATCHDOG_TIMEOUT,
    );
    let mut dispatched = focused && dispatch_window(scope);
    if let Some(element) = element {
        if !focused {
            dispatch_pending_text_control_change_if_needed(scope, runtime_ptr, element);
        }
        for (event_type, bubbles) in [(event_type, false), (bubbling_type, true)] {
            let runtime = unsafe { &*runtime_ptr };
            if runtime.focused_document_and_element() != (document, Some(element))
                || runtime.document_focused_area(document) != Some(element)
                || !runtime.document_is_fully_active(document)
            {
                break;
            }
            if let Some(event) = construct_focus_event(scope, event_type, None, bubbles) {
                let _ = dispatch_public_event(scope, runtime_ptr, element, event);
                dispatched = true;
            }
        }
    }
    if !focused {
        dispatched |= dispatch_window(scope);
    }
    if watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut {
        tracing::warn!("system focus event exceeded its execution deadline");
    }
    dispatched
}

#[derive(Clone, Copy)]
enum SelectionBehaviorOnFocus {
    Restore,
    Preserve,
}

pub(crate) fn focus_element_preserving_selection(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if !is_focusable(runtime, target) {
        return;
    }
    let previous = runtime.active_element_handle();
    update_focus_with_selection_behavior(
        scope,
        runtime_ptr,
        previous,
        Some(target),
        None,
        SelectionBehaviorOnFocus::Preserve,
    );
}

fn update_focus_from_previous(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
) {
    update_focus_with_selection_behavior(
        scope,
        runtime_ptr,
        previous,
        next,
        None,
        SelectionBehaviorOnFocus::Restore,
    );
}

fn update_focus_with_selection_behavior(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    viewport_document: Option<DomHandle>,
    selection_behavior: SelectionBehaviorOnFocus,
) {
    // A viewport has no element handle. Its Document must stay explicit:
    // clearing a parent's focus does not clear its inactive child's focused area.
    if let Some(document) = viewport_document {
        unsafe { &mut *runtime_ptr }.note_document_focused_area(document, None);
    }
    if previous == next
        && viewport_document.is_none_or(|document| {
            unsafe { &*runtime_ptr }.focused_viewport_document() == Some(document)
        })
    {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let next_document = next.and_then(|handle| runtime.dom_host().owner_document_handle(handle));
    let previous_document =
        previous.and_then(|handle| runtime.dom_host().owner_document_handle(handle));
    let previous_viewport = previous_document.or(runtime.focused_viewport_document());
    // Blurring a focused frame returns focus to the viewport of its owner Document.
    let destination_viewport = if next.is_none() {
        viewport_document.or(previous_viewport)
    } else {
        viewport_document
    };
    let window_focus_transition =
        window_focus_transition_for_handles(runtime, previous, next, destination_viewport);
    // Related targets do not cross Document boundaries in the focus chains.
    let same_document = previous_document == next_document;
    let next_value = same_document
        .then(|| wrap_handle_value(scope, runtime_ptr, next))
        .flatten();
    let runtime = unsafe { &mut *runtime_ptr };
    // Blur listeners observe the viewport, before the new target gains focus.
    // Keep inactive Documents' focused areas when moving between Documents.
    let common_ancestor = previous.zip(next).and_then(|(previous, next)| {
        let mut candidate = Some(previous);
        while let Some(handle) = candidate {
            if shadow_including_contains(runtime, handle, next) {
                return Some(handle);
            }
            candidate = flat_tree_parent(runtime, handle);
        }
        None
    });
    runtime.set_active_element_handle(None);
    runtime
        .dom_host()
        .set_focus_transition_common_ancestor(common_ancestor);
    runtime.set_focused_viewport_document(previous_viewport);
    if viewport_document.is_none()
        && (next.is_none()
            || previous.and_then(|handle| runtime.dom_host().owner_document_handle(handle))
                == next_document)
    {
        runtime.mark_focus_changed(previous, None);
    }
    runtime.note_focus_style_activity(previous, None);
    let mut redirected = false;
    if let Some(previous_handle) = previous {
        dispatch_pending_text_control_change_if_needed(scope, runtime_ptr, previous_handle);
        if let Some(event) = construct_node_focus_event(
            scope,
            runtime_ptr,
            previous_handle,
            "blur",
            next_value,
            false,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
        }
        // A blur listener can complete a different focus transition. Finish the
        // old target's focusout, but do not overwrite the listener's decision.
        redirected = unsafe { &*runtime_ptr }.active_element_handle().is_some();
        if let Some(event) = construct_node_focus_event(
            scope,
            runtime_ptr,
            previous_handle,
            "focusout",
            if redirected { None } else { next_value },
            true,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, previous_handle, event);
        }
        dispatch_interest_event_if_needed(scope, runtime_ptr, previous_handle, "loseinterest");
    }
    if redirected || unsafe { &*runtime_ptr }.active_element_handle().is_some() {
        clear_focus_transition_style(unsafe { &mut *runtime_ptr });
        return;
    }
    let target_is_current = |runtime: &PageScriptHost| {
        viewport_document
            .is_none_or(|document| runtime.top_level_document_for_document(document).is_some())
            && next.is_none_or(|handle| {
                runtime.dom_host().owner_document_handle(handle) == next_document
                    && next_document.is_some_and(|document| {
                        runtime.top_level_document_for_document(document).is_some()
                    })
                    && is_focusable(runtime, handle)
            })
    };
    if !target_is_current(unsafe { &*runtime_ptr }) {
        clear_focus_transition_style(unsafe { &mut *runtime_ptr });
        return;
    }
    if let Some((previous_window, _)) = window_focus_transition {
        dispatch_window_focus_event(scope, runtime_ptr, previous_window, "blur");
    }
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.active_element_handle().is_some() || !target_is_current(runtime) {
        clear_focus_transition_style(runtime);
        return;
    }
    runtime.set_active_element_handle(next);
    runtime.set_focused_viewport_document(destination_viewport.filter(|_| next.is_none()));
    if viewport_document.is_none() || next.is_some() {
        runtime.mark_focus_changed(previous, next);
    }
    runtime.note_focus_style_activity(None, next);
    if let Some(handle) = next
        && matches!(selection_behavior, SelectionBehaviorOnFocus::Restore)
    {
        let _ = crate::context_bootstrap::focus_element_selection(scope, runtime_ptr, handle);
        let runtime = unsafe { &mut *runtime_ptr };
        if is_text_control(runtime, handle) {
            runtime.note_text_control_selection(handle);
        }
    }
    let previous_value = same_document
        .then(|| wrap_handle_value(scope, runtime_ptr, previous))
        .flatten();
    if let Some(next_handle) = next {
        if let Some(event) = construct_node_focus_event(
            scope,
            runtime_ptr,
            next_handle,
            "focus",
            previous_value,
            false,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, next_handle, event);
        }
        if unsafe { &*runtime_ptr }.active_element_handle() != Some(next_handle) {
            return;
        }
        if let Some(event) = construct_node_focus_event(
            scope,
            runtime_ptr,
            next_handle,
            "focusin",
            previous_value,
            true,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, next_handle, event);
        }
        if unsafe { &*runtime_ptr }.active_element_handle() != Some(next_handle) {
            return;
        }
        if let Some(target) =
            dispatch_interest_event_if_needed(scope, runtime_ptr, next_handle, "interest")
        {
            show_interest_popover_if_needed(scope, runtime_ptr, next_handle, target);
        }
        if unsafe { &*runtime_ptr }.active_element_handle() != Some(next_handle) {
            return;
        }
        unsafe { &mut *runtime_ptr }.queue_focused_document_fixup(scope);
    }
    if let Some((_, next_window)) = window_focus_transition {
        dispatch_window_focus_event(scope, runtime_ptr, next_window, "focus");
    }
}

fn clear_focus_transition_style(runtime: &mut PageScriptHost) {
    if let Some(ancestor) = runtime
        .dom_host()
        .set_focus_transition_common_ancestor(None)
    {
        runtime.note_focus_style_activity(Some(ancestor), runtime.active_element_handle());
    }
}

fn window_focus_transition_for_handles(
    runtime: &PageScriptHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    viewport_document: Option<DomHandle>,
) -> Option<(PendingWindowMessageEndpoint, PendingWindowMessageEndpoint)> {
    let next_window = next
        .and_then(|handle| focused_window_endpoint(runtime, handle))
        .or_else(|| {
            viewport_document.and_then(|document| runtime.window_endpoint_for_document(document))
        })?;
    let previous_window = previous
        .and_then(|handle| focused_window_endpoint(runtime, handle))
        .or_else(|| {
            runtime
                .focused_viewport_document()
                .and_then(|document| runtime.window_endpoint_for_document(document))
        })
        .or_else(|| {
            // Before the first focus change, the traversable's viewport is
            // focused. The executing realm may already be a child Window.
            let document = viewport_document.or_else(|| {
                next.and_then(|handle| runtime.dom_host().owner_document_handle(handle))
            })?;
            let top = runtime.top_level_document_for_document(document)?;
            runtime.window_endpoint_for_document(top)
        })?;
    (previous_window != next_window).then_some((previous_window, next_window))
}

fn focused_window_endpoint(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> Option<PendingWindowMessageEndpoint> {
    if runtime.dom_host().is_html_element_named(handle, "iframe")
        || runtime.dom_host().is_html_element_named(handle, "frame")
    {
        return Some(PendingWindowMessageEndpoint::ChildWindow(handle));
    }
    let document = runtime.dom_host().owner_document_handle(handle)?;
    if let Some(child_handle) = runtime.child_browsing_context_host_for_document_handle(document) {
        return Some(PendingWindowMessageEndpoint::ChildWindow(child_handle));
    }
    if let Some(popup_id) = runtime.lightweight_popup_id_for_document_handle(document) {
        return Some(PendingWindowMessageEndpoint::LightweightPopup(popup_id));
    }
    (document == runtime.document_handle()).then_some(PendingWindowMessageEndpoint::TopWindow)
}

fn construct_node_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    event_type: &str,
    related_target: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let endpoint = focused_window_endpoint(unsafe { &*runtime_ptr }, target)?;
    construct_window_focus_event(
        scope,
        runtime_ptr,
        endpoint,
        event_type,
        related_target,
        bubbles,
    )
}

fn construct_window_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    endpoint: PendingWindowMessageEndpoint,
    event_type: &str,
    related_target: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime = unsafe { &mut *runtime_ptr };
    match endpoint {
        PendingWindowMessageEndpoint::ChildWindow(child) => {
            runtime
                .ensure_prebootstrapped_child_default_context(scope, child)
                .ok()?;
        }
        PendingWindowMessageEndpoint::LightweightPopup(popup) => {
            runtime
                .ensure_lightweight_popup_execution_context(scope, popup)
                .then_some(())?;
        }
        PendingWindowMessageEndpoint::TopWindow => {}
    }
    let dispatch_scope = endpoint.dispatch_scope();
    let owner = runtime.current_window_execution_context_owner(dispatch_scope)?;
    let (_, context) = runtime.window_execution_context(scope, owner, dispatch_scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    construct_focus_event(scope, event_type, related_target, bubbles)
}

fn dispatch_window_focus_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    endpoint: PendingWindowMessageEndpoint,
    event_type: &str,
) {
    let Some(event) =
        construct_window_focus_event(scope, runtime_ptr, endpoint, event_type, None, false)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    match endpoint {
        PendingWindowMessageEndpoint::TopWindow => {
            // The generic Window target resolves in the entered realm.
            let Some(context) = event.get_creation_context(scope) else {
                return;
            };
            let scope = &mut v8::ContextScope::new(scope, context);
            let _ = runtime.dispatch_public_event_best_effort(
                scope,
                runtime_ptr,
                EventTargetHandle::Window,
                event,
                "window focus event",
            );
        }
        PendingWindowMessageEndpoint::ChildWindow(handle) => {
            runtime.dispatch_child_window_event(scope, handle, event_type, event);
        }
        PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
            let _ =
                runtime.dispatch_lightweight_popup_window_event(scope, popup_id, event_type, event);
        }
    }
}

fn dispatch_interest_event_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    source_handle: DomHandle,
    event_type: &str,
) -> Option<DomHandle> {
    let target = interest_for_element_target(runtime_ptr, source_handle)?;
    let source = node_wrapper_from_handle(scope, source_handle)?;
    let event = construct_interest_event(scope, event_type, source.into())?;
    let _ = dispatch_public_event(scope, runtime_ptr, target, event);
    Some(target)
}

pub(crate) fn perform_hover_interest_default_action_for_dispatched_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    source_handle: DomHandle,
) {
    let Some(target) =
        dispatch_interest_event_if_needed(scope, runtime_ptr, source_handle, "interest")
    else {
        return;
    };
    show_interest_popover_if_needed(scope, runtime_ptr, source_handle, target);
}

pub(crate) fn perform_access_key_default_action_for_dispatched_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    event: v8::Local<'_, v8::Object>,
    document: DomHandle,
) {
    if !event_boolean_property(scope, event, "altKey")
        || event_boolean_property(scope, event, "ctrlKey") != cfg!(target_os = "macos")
        || event_boolean_property(scope, event, "metaKey")
    {
        return;
    }
    let Some(key) = event_string_property(scope, event, "key") else {
        return;
    };
    if key.chars().count() != 1 {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let Some(target) = access_key_target(runtime, document, &key) else {
        return;
    };
    let _ = super::activation::dispatch_click_event(
        scope,
        runtime_ptr,
        target,
        InputEventCoordinates::default(),
        0,
        0,
        0,
        0,
        ClickEventSource::UserAgent,
    );
}

pub(crate) fn perform_tab_focus_default_action_for_dispatched_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    event: v8::Local<'_, v8::Object>,
) {
    if event_string_property(scope, event, "key").as_deref() != Some("Tab") {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let reverse = event_boolean_property(scope, event, "shiftKey");
    if let Some(target) = sequential_focus_target(runtime, reverse) {
        update_focus(scope, runtime_ptr, Some(target));
    }
}

fn sequential_focus_target(runtime: &PageScriptHost, reverse: bool) -> Option<DomHandle> {
    let (mut document, mut active) = runtime.focused_document_and_element();
    let mut starting_point = active
        .map(SequentialFocusStartingPoint::Element)
        .or_else(|| runtime.sequential_focus_starting_point(document));
    loop {
        let order = sequential_focus_order(runtime, document, active);
        if let Some(target) =
            sequential_focus_target_in_document(runtime, document, &order, starting_point, reverse)
        {
            return Some(target);
        }
        let Some(container) = runtime.child_browsing_context_host_for_document_handle(document)
        else {
            // Headless Pages cycle through controls at the traversable boundary.
            return resolve_sequential_focus_order(runtime, &order, reverse);
        };
        // Resume the parent's search from the container's position in its own
        // scope. Each Document orders its own tabindex values independently.
        document = runtime.dom_host().owner_document_handle(container)?;
        active = Some(container);
        starting_point = Some(SequentialFocusStartingPoint::Element(container));
    }
}

fn sequential_focus_target_in_document(
    runtime: &PageScriptHost,
    document: DomHandle,
    order: &[DomHandle],
    starting_point: Option<SequentialFocusStartingPoint>,
    reverse: bool,
) -> Option<DomHandle> {
    let Some(point) = starting_point else {
        return resolve_sequential_focus_order(runtime, order, reverse);
    };
    if let Some(active) = point.element() {
        if let Some(index) = order.iter().position(|&candidate| candidate == active) {
            let remaining = if reverse {
                &order[..index]
            } else {
                &order[index + 1..]
            };
            return resolve_sequential_focus_order(runtime, remaining, reverse);
        }
        if let Some(target) = negative_shadow_scope_tab_target(runtime, active, reverse) {
            return Some(target);
        }
    }
    navigation::sequential_target_in_dom_order(runtime, document, point, order, reverse, |handle| {
        resolve_sequential_focus_candidate(runtime, handle, reverse)
    })
}

fn resolve_sequential_focus_order(
    runtime: &PageScriptHost,
    order: &[DomHandle],
    reverse: bool,
) -> Option<DomHandle> {
    let mut candidates = order.iter().copied();
    let target = |handle| resolve_sequential_focus_candidate(runtime, handle, reverse);
    if reverse {
        candidates.rev().find_map(target)
    } else {
        candidates.find_map(target)
    }
}

fn resolve_sequential_focus_candidate(
    runtime: &PageScriptHost,
    candidate: DomHandle,
    reverse: bool,
) -> Option<DomHandle> {
    let Some(document) = runtime.child_browsing_context_document_handle(candidate) else {
        return Some(candidate);
    };
    if !runtime.document_is_fully_active(document) {
        return None;
    }
    let order = sequential_focus_order(runtime, document, None);
    resolve_sequential_focus_order(runtime, &order, reverse)
}

fn sequential_focus_order(
    runtime: &PageScriptHost,
    document: DomHandle,
    active: Option<DomHandle>,
) -> Vec<DomHandle> {
    sequential_focus_order_for_children(
        runtime,
        runtime.dom_host().child_handles(document).collect(),
        active,
    )
}

fn sequential_focus_order_for_children(
    runtime: &PageScriptHost,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<DomHandle> {
    let mut entries = Vec::new();
    let mut order = 0;
    for child in children {
        collect_sequential_focus_entries(runtime, child, active, &mut entries, &mut order);
    }
    entries.sort_by_key(|entry| {
        if entry.tab_index > 0 {
            (0, entry.tab_index, entry.order)
        } else {
            (1, 0, entry.order)
        }
    });
    entries
        .into_iter()
        .flat_map(|entry| entry.handles)
        .collect()
}

fn collect_sequential_focus_entries(
    runtime: &PageScriptHost,
    handle: DomHandle,
    active: Option<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        append_child_entries(runtime, handle, active, entries, order);
        return;
    };

    if element.is_html_element("slot") {
        let Some(tab_index) = slot_focus_scope_tab_index(runtime, handle) else {
            return;
        };
        let mut handles = Vec::new();
        if slot_generates_focusable_box(runtime, handle) {
            handles.push(handle);
        }
        let mut slot_children = runtime
            .dom_host()
            .assigned_nodes_for_slot_with_options(handle, false);
        if slot_children.is_empty() {
            slot_children = runtime.dom_host().child_handles(handle).collect();
        }
        let child_handles = if let Some(mode) = reading_flow_mode(runtime, handle) {
            sequential_focus_order_for_reading_flow_items(
                runtime,
                handle,
                mode,
                slot_children,
                active,
            )
        } else {
            sequential_focus_order_for_children(runtime, slot_children, active)
        };
        handles.extend(child_handles);
        push_focus_entry_with_tab_index(tab_index, handles, entries, order);
        return;
    }

    if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle) {
        if explicit_negative_tab_index(runtime, handle) {
            return;
        }
        let mut handles = Vec::new();
        let tab_index = sequential_tab_index(runtime, handle, active);
        let delegates_focus =
            runtime.dom_host().shadow_root_delegates_focus(shadow_root) == Some(true);
        if tab_index.is_some() && !delegates_focus {
            handles.push(handle);
        }
        let shadow_children = runtime
            .dom_host()
            .child_handles(shadow_root)
            .collect::<Vec<_>>();
        if let Some(mode) = reading_flow_mode(runtime, handle) {
            handles.extend(sequential_focus_order_for_reading_flow_items(
                runtime,
                handle,
                mode,
                shadow_children,
                active,
            ));
        } else {
            handles.extend(sequential_focus_order_for_children(
                runtime,
                shadow_children,
                active,
            ));
        }
        push_focus_entry_with_tab_index(tab_index.unwrap_or(0), handles, entries, order);
        return;
    }

    if sequential_tab_index(runtime, handle, active).is_some() {
        push_focus_entry(runtime, handle, vec![handle], entries, order);
    }
    if reading_flow_mode(runtime, handle).is_some() {
        push_focus_entry_with_tab_index(
            0,
            sequential_focus_order_for_reading_flow_container(
                runtime,
                handle,
                runtime.dom_host().child_handles(handle).collect(),
                active,
            ),
            entries,
            order,
        );
        return;
    }
    append_child_entries(runtime, handle, active, entries, order);
}

fn append_child_entries(
    runtime: &PageScriptHost,
    handle: DomHandle,
    active: Option<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    for child in runtime.dom_host().child_handles(handle) {
        collect_sequential_focus_entries(runtime, child, active, entries, order);
    }
}

fn push_focus_entry(
    runtime: &PageScriptHost,
    handle: DomHandle,
    handles: Vec<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    push_focus_entry_with_tab_index(
        sequential_tab_index(runtime, handle, None).unwrap_or(0),
        handles,
        entries,
        order,
    );
}

fn push_focus_entry_with_tab_index(
    tab_index: i32,
    handles: Vec<DomHandle>,
    entries: &mut Vec<SequentialFocusEntry>,
    order: &mut usize,
) {
    if handles.is_empty() {
        return;
    }
    entries.push(SequentialFocusEntry {
        tab_index,
        order: *order,
        handles,
    });
    *order += 1;
}

fn sequential_focus_order_for_reading_flow_container(
    runtime: &PageScriptHost,
    container: DomHandle,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<DomHandle> {
    let Some(mode) = reading_flow_mode(runtime, container) else {
        return sequential_focus_order_for_children(runtime, children, active);
    };
    reading_flow_item_groups(runtime, container, mode, children, active)
        .into_iter()
        .flat_map(|(_, handles)| handles)
        .collect()
}

fn sequential_focus_order_for_reading_flow_items(
    runtime: &PageScriptHost,
    container: DomHandle,
    mode: ReadingFlowMode,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<DomHandle> {
    reading_flow_item_groups(runtime, container, mode, children, active)
        .into_iter()
        .flat_map(|(_, handles)| handles)
        .collect()
}

fn reading_flow_item_groups(
    runtime: &PageScriptHost,
    container: DomHandle,
    mode: ReadingFlowMode,
    children: Vec<DomHandle>,
    active: Option<DomHandle>,
) -> Vec<((i32, i32, i32, usize), Vec<DomHandle>)> {
    let reverse_tree_order = mode == ReadingFlowMode::FlexVisual
        && style_property_value(runtime, container, StyleMode::Computed, "flex-direction")
            .trim()
            .eq_ignore_ascii_case("row-reverse");
    let mut groups = children
        .into_iter()
        .enumerate()
        .flat_map(|(tree_order, child)| {
            reading_flow_item_group(runtime, child, mode, tree_order, reverse_tree_order, active)
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|(key, _)| *key);
    groups
}

fn reading_flow_item_group(
    runtime: &PageScriptHost,
    child: DomHandle,
    mode: ReadingFlowMode,
    tree_order: usize,
    reverse_tree_order: bool,
    active: Option<DomHandle>,
) -> Vec<((i32, i32, i32, usize), Vec<DomHandle>)> {
    if computed_display(runtime, child) != "contents" {
        return vec![(
            reading_flow_child_order_key(runtime, child, mode, tree_order, reverse_tree_order),
            sequential_focus_order_for_children(runtime, vec![child], active),
        )];
    }

    let mut child_groups = runtime
        .dom_host()
        .child_handles(child)
        .enumerate()
        .flat_map(|(index, grandchild)| {
            reading_flow_item_group(runtime, grandchild, mode, index, reverse_tree_order, active)
        })
        .filter(|(_, handles)| !handles.is_empty())
        .collect::<Vec<_>>();
    child_groups.sort_by_key(|(key, _)| *key);
    let Some((key, _)) = child_groups.first().cloned() else {
        return Vec::new();
    };
    let mut handles = Vec::new();
    if sequential_tab_index(runtime, child, active).is_some() {
        handles.push(child);
    }
    handles.extend(child_groups.into_iter().flat_map(|(_, handles)| handles));
    vec![(key, handles)]
}

fn reading_flow_child_order_key(
    runtime: &PageScriptHost,
    child: DomHandle,
    mode: ReadingFlowMode,
    tree_order: usize,
    reverse_tree_order: bool,
) -> (i32, i32, i32, usize) {
    let position_group = match style_property_value(runtime, child, StyleMode::Computed, "position")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "absolute" | "fixed" => 1,
        _ => 0,
    };
    let reading_order = style_integer_property(runtime, child, "reading-order").unwrap_or(0);
    let layout_order = match mode {
        ReadingFlowMode::SourceOrder => 0,
        _ => style_integer_property(runtime, child, "order").unwrap_or(0),
    };
    let order = if reverse_tree_order {
        usize::MAX - tree_order
    } else {
        tree_order
    };
    (position_group, reading_order, layout_order, order)
}

fn reading_flow_mode(runtime: &PageScriptHost, handle: DomHandle) -> Option<ReadingFlowMode> {
    let display = computed_display(runtime, handle);
    let mode = match style_property_value(runtime, handle, StyleMode::Computed, "reading-flow")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "source-order" if display != "inline" => ReadingFlowMode::SourceOrder,
        "grid-order" if display.contains("grid") => ReadingFlowMode::GridOrder,
        "grid-rows" if display.contains("grid") => ReadingFlowMode::GridRows,
        "grid-columns" if display.contains("grid") => ReadingFlowMode::GridColumns,
        "flex-flow" if display.contains("flex") => ReadingFlowMode::FlexFlow,
        "flex-visual" if display.contains("flex") => ReadingFlowMode::FlexVisual,
        _ => return None,
    };
    Some(mode)
}

fn computed_display(runtime: &PageScriptHost, handle: DomHandle) -> String {
    style_property_value(runtime, handle, StyleMode::Computed, "display")
        .trim()
        .to_ascii_lowercase()
}

fn style_integer_property(
    runtime: &PageScriptHost,
    handle: DomHandle,
    property: &str,
) -> Option<i32> {
    style_property_value(runtime, handle, StyleMode::Computed, property)
        .trim()
        .parse::<i32>()
        .ok()
}

fn slot_focus_scope_tab_index(runtime: &PageScriptHost, handle: DomHandle) -> Option<i32> {
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::explicit_tab_index)
        .unwrap_or(0);
    (value >= 0).then_some(value)
}

fn slot_generates_focusable_box(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    !matches!(
        computed_display(runtime, handle).as_str(),
        "none" | "contents"
    )
}

fn sequential_tab_index(
    runtime: &PageScriptHost,
    handle: DomHandle,
    active: Option<DomHandle>,
) -> Option<i32> {
    if !is_sequentially_focusable(runtime, handle, active) {
        return None;
    }
    sequential_tab_index_without_scroll_descendant_check(runtime, handle)
}

fn sequential_tab_index_without_scroll_descendant_check(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> Option<i32> {
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::explicit_tab_index)
        .unwrap_or_else(|| default_sequential_tab_index(runtime, handle));
    (value >= 0).then_some(value)
}

fn is_sequentially_focusable(
    runtime: &PageScriptHost,
    handle: DomHandle,
    active: Option<DomHandle>,
) -> bool {
    if !is_focusable(runtime, handle) {
        return false;
    }
    if active == Some(handle) {
        return true;
    }
    !element_is_scrollable(runtime, handle) || !has_sequential_focusable_descendant(runtime, handle)
}

fn has_sequential_focusable_descendant(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    let mut stack = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
    if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle) {
        stack.extend(runtime.dom_host().child_handles(shadow_root));
    }
    while let Some(candidate) = stack.pop() {
        if is_interactive_sequential_focusable_descendant(runtime, candidate) {
            return true;
        }
        stack.extend(runtime.dom_host().child_handles(candidate));
        if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(candidate) {
            stack.extend(runtime.dom_host().child_handles(shadow_root));
        }
    }
    false
}

fn is_interactive_sequential_focusable_descendant(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> bool {
    if !is_focusable(runtime, handle) {
        return false;
    }
    let dom = runtime.dom_host().dom();
    let is_interactive_element = dom.has_default_focus_behavior(handle);
    let has_non_negative_tab_index = dom
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::explicit_tab_index)
        .is_some_and(|value| value >= 0);
    (is_interactive_element || has_non_negative_tab_index)
        && sequential_tab_index_without_scroll_descendant_check(runtime, handle).is_some()
}

fn explicit_negative_tab_index(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(Element::explicit_tab_index)
        .is_some_and(|value| value < 0)
}

fn default_sequential_tab_index(runtime: &PageScriptHost, handle: DomHandle) -> i32 {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return -1;
    };
    if (runtime.dom_host().dom().has_default_focus_behavior(handle)
        && !element.is_html_element("dialog"))
        || element_is_scrollable(runtime, handle)
    {
        0
    } else {
        -1
    }
}

fn negative_shadow_scope_tab_target(
    runtime: &PageScriptHost,
    active: DomHandle,
    reverse: bool,
) -> Option<DomHandle> {
    let root = runtime.dom_host().containing_shadow_root(active)?;
    let host = runtime.dom_host().shadow_root_host(root)?;
    if !explicit_negative_tab_index(runtime, host) {
        return None;
    }
    let local_order = sequential_focus_order_for_children(
        runtime,
        runtime.dom_host().child_handles(root).collect(),
        Some(active),
    );
    let position = local_order
        .iter()
        .position(|candidate| *candidate == active)?;
    let remaining = if reverse {
        &local_order[..position]
    } else {
        &local_order[position + 1..]
    };
    resolve_sequential_focus_order(runtime, remaining, reverse)
        .or_else(|| focusable_adjacent_to_negative_shadow_host(runtime, host, reverse))
}

fn focusable_adjacent_to_negative_shadow_host(
    runtime: &PageScriptHost,
    host: DomHandle,
    reverse: bool,
) -> Option<DomHandle> {
    let parent = runtime.dom_host().node(host).and_then(Node::parent_node)?;
    let siblings = runtime.dom_host().child_handles(parent).collect::<Vec<_>>();
    let position = siblings.iter().position(|candidate| *candidate == host)?;
    if reverse {
        return siblings[..position].iter().rev().find_map(|sibling| {
            let order = sequential_focus_order_for_children(runtime, vec![*sibling], None);
            resolve_sequential_focus_order(runtime, &order, true)
        });
    }
    siblings[position + 1..].iter().find_map(|sibling| {
        let order = sequential_focus_order_for_children(runtime, vec![*sibling], None);
        resolve_sequential_focus_order(runtime, &order, false)
    })
}

fn access_key_target(
    runtime: &PageScriptHost,
    document: DomHandle,
    key: &str,
) -> Option<DomHandle> {
    let needle = key.to_ascii_lowercase();
    let mut stack = runtime
        .dom_host()
        .child_handles(document)
        .collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        if runtime.dom_host().is_connected(handle)
            && reflected_attribute(runtime, handle, "accesskey")
                .is_some_and(|value| access_key_attribute_matches(&value, &needle))
        {
            return Some(handle);
        }

        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
        if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle) {
            let mut shadow_children = runtime
                .dom_host()
                .child_handles(shadow_root)
                .collect::<Vec<_>>();
            shadow_children.reverse();
            stack.extend(shadow_children);
        }
    }
    None
}

fn access_key_attribute_matches(value: &str, needle: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case(needle))
}

fn event_boolean_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    name: &str,
) -> bool {
    let Some(key) = v8_string(scope, name) else {
        return false;
    };
    crate::context_bootstrap::event_backing(scope, event)
        .get(scope, key.into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn event_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    name: &str,
) -> Option<String> {
    let key = v8_string(scope, name)?;
    crate::context_bootstrap::event_backing(scope, event)
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
}

fn show_interest_popover_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    source_handle: DomHandle,
    target: DomHandle,
) {
    let runtime = unsafe { &*runtime_ptr };
    if runtime.dom_host().node(target).is_some() {
        dispatch_popover_show_events(scope, runtime_ptr, target, source_handle);
    }
}

fn interest_for_element_target(
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    resolved_reflected_element_attribute_handle(unsafe { &*runtime_ptr }, handle, "interestfor")
}

pub(crate) fn focus_element(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    if let Err(error) = focus_element_with_options(scope, runtime_ptr, handle, false) {
        tracing::warn!(?handle, %error, "failed to scroll focused element into view");
    }
}

#[cfg(feature = "devtools")]
/// Performs the focus default action associated with an uncanceled mouse down.
///
/// Blink dispatches `pointerdown`/`mousedown` before it changes focus. It then
/// walks from the hit element through the flat tree to find a mouse-focusable
/// element; if none exists, the previous element is blurred. Keeping this
/// responsibility next to the focus model avoids teaching the CDP input layer
/// about labels, delegated shadow focus, or focus event ordering.
pub(crate) fn perform_mouse_focus_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    hit: DomHandle,
) {
    let mut candidate = Some(hit);
    while let Some(handle) = candidate {
        let (should_focus, parent) = {
            let runtime = unsafe { &*runtime_ptr };
            let delegates_focus =
                runtime
                    .dom_host()
                    .shadow_root_handle(handle)
                    .is_some_and(|root| {
                        runtime.dom_host().shadow_root_delegates_focus(root) == Some(true)
                            && delegated_focus_target(runtime, handle).is_some()
                    });
            let should_focus = (is_focusable(runtime, handle) || delegates_focus)
                && !focused_chain_requires_async_blur(runtime, handle);
            let parent = if should_focus {
                None
            } else {
                flat_tree_parent(runtime, handle)
            };
            (should_focus, parent)
        };
        if should_focus {
            focus_element(scope, runtime_ptr, handle);
            return;
        }
        candidate = parent;
    }
    update_focus(scope, runtime_ptr, None);
}

fn flat_tree_parent(runtime: &PageScriptHost, handle: DomHandle) -> Option<DomHandle> {
    if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = runtime.dom_host().parent_node(handle)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    Some(parent)
}

fn focus_element_with_options(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    prevent_scroll: bool,
) -> Result<(), moli_layout::LayoutError> {
    let runtime = unsafe { &*runtime_ptr };
    let target = if runtime.dom_host().is_html_element_named(handle, "label")
        && !label_receives_programmatic_focus(runtime, handle)
    {
        label_control_handle(runtime, handle).filter(|control| is_focusable(runtime, *control))
    } else if let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
        && runtime.dom_host().shadow_root_delegates_focus(shadow_root) == Some(true)
    {
        delegated_focus_target(runtime, handle)
    } else {
        is_focusable(runtime, handle).then_some(handle)
    };
    if let Some(target) = target {
        update_focus(scope, runtime_ptr, Some(target));
        if !prevent_scroll {
            scroll_node_into_view_if_needed(scope, runtime_ptr, target, None)?;
        }
    }
    Ok(())
}

#[cfg(feature = "devtools")]
pub(crate) fn focus_live_element_for_inspector(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) -> RendererDomFocusOutcome {
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        return RendererDomFocusOutcome::NodeNotFound;
    };
    if !node.is_element() {
        return RendererDomFocusOutcome::NodeNotElement;
    }
    let delegates_focus = runtime
        .dom_host()
        .shadow_root_handle(handle)
        .is_some_and(|root| {
            runtime.dom_host().shadow_root_delegates_focus(root) == Some(true)
                && delegated_focus_target(runtime, handle).is_some()
        });
    if !is_focusable(runtime, handle) && !delegates_focus {
        return RendererDomFocusOutcome::ElementNotFocusable;
    }

    focus_element(scope, runtime_ptr, handle);
    RendererDomFocusOutcome::Focused
}

fn focus_options_prevent_scroll(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<bool> {
    if args.length() == 0 || args.get(0).is_null_or_undefined() {
        return Some(false);
    }
    let options = args.get(0).to_object(scope)?;
    options
        .get(scope, v8str(scope, "preventScroll").into())
        .map(|value| value.boolean_value(scope))
}

fn throw_focus_layout_error(scope: &mut v8::PinScope<'_, '_>, error: moli_layout::LayoutError) {
    let message = format!("Layout failed while focusing element: {error}");
    if let Some(message) = v8_string(scope, &message) {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

pub(in crate::native_bridge) fn node_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        document::detached_focus_method_callback(scope, args, rv);
        return;
    };
    let Some(prevent_scroll) = focus_options_prevent_scroll(scope, &args) else {
        return;
    };
    if let Err(error) = focus_element_with_options(scope, runtime_ptr, handle, prevent_scroll) {
        throw_focus_layout_error(scope, error);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_blur_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        document::detached_blur_method_callback(scope, args, rv);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let delegated_active = runtime
        .active_element_handle()
        .zip(runtime.dom_host().shadow_root_handle(handle))
        .is_some_and(|(active, root)| {
            runtime.dom_host().shadow_root_delegates_focus(root) == Some(true)
                && shadow_including_contains(runtime, root, active)
        });
    if runtime.active_element_handle() == Some(handle) || delegated_active {
        update_focus(scope, runtime_ptr, None);
    }
    rv.set_undefined();
}

#[cfg(feature = "devtools")]
fn focused_chain_requires_async_blur(runtime: &PageScriptHost, active: DomHandle) -> bool {
    let mut current = Some(active);
    while let Some(handle) = current {
        if super::element_has_attribute(runtime, handle, "inert")
            || super::element_has_attribute(runtime, handle, "hidden")
        {
            return true;
        }
        if style_property_value(runtime, handle, StyleMode::Computed, "display")
            .eq_ignore_ascii_case("none")
        {
            return true;
        }
        current = runtime
            .dom_host()
            .node(handle)
            .and_then(Node::parent_node)
            .or_else(|| {
                runtime
                    .dom_host()
                    .containing_shadow_root(handle)
                    .and_then(|root| runtime.dom_host().shadow_root_host(root))
            });
    }
    false
}
