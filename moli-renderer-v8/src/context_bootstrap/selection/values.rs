use super::super::range::{
    RangeBoundarySide, new_range_for_document, range_boundary_container_object,
    range_boundary_offset, range_is_collapsed, range_native_record_handle, set_range_boundary,
};
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::element::contenteditable_editing_host;
use crate::native_bridge::wrapped_handle_value;
use crate::native_bridge::{
    SelectionBoundaryRole, SelectionBoundarySnapshot, SelectionRecordHandle,
    callback_value_dom_handle,
};
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

const SELECTION_RECORD_INTERNAL_FIELD_INDEX: usize = 0;
const SELECTION_WRAPPER_INTERNAL_FIELD_COUNT: usize = 1;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Selection)]
struct SelectionObjectDeclaration {
    #[webapi(slot = SELECTION_RANGE_SLOT, init = "null")]
    range: (),
}

impl SelectionObjectDeclaration {
    fn empty() -> Self {
        Self { range: () }
    }
}

pub(in crate::context_bootstrap) struct SelectionRangeUpdateState<'s> {
    pub selection: v8::Local<'s, v8::Object>,
    pub old_composed_start_node: v8::Local<'s, v8::Object>,
    pub old_composed_start_offset: u32,
    pub old_composed_end_node: v8::Local<'s, v8::Object>,
    pub old_composed_end_offset: u32,
}

pub(in crate::context_bootstrap) fn window_selection_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, WINDOW_SELECTION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn new_selection_runtime_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let object_template = v8::ObjectTemplate::new(scope);
    let _ = object_template.set_internal_field_count(SELECTION_WRAPPER_INTERNAL_FIELD_COUNT);
    let selection = object_template
        .new_instance(scope)
        .expect("Selection wrapper template should instantiate");
    SelectionObjectDeclaration::empty()
        .bind_into(scope, selection)
        .expect("Selection declaration should bind");
    let _ = ensure_selection_record_handle(scope, selection);
    selection
}

pub(in crate::context_bootstrap) fn selection_clear<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) {
    SelectionObjectDeclaration::empty()
        .initialize(scope, selection)
        .expect("Selection declaration should initialize object");
    if let Some(handle) = ensure_selection_record_handle(scope, selection)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.clear_selection_record(handle);
    }
}

pub(in crate::context_bootstrap) fn selection_anchor_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    selection_record_boundary_object(scope, selection, SelectionBoundaryRole::Anchor)
}

pub(in crate::context_bootstrap) fn selection_focus_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    selection_record_boundary_object(scope, selection, SelectionBoundaryRole::Focus)
}

pub(in crate::context_bootstrap) fn selection_anchor_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> u32 {
    selection_record_boundary_offset(scope, selection, SelectionBoundaryRole::Anchor).unwrap_or(0)
}

pub(in crate::context_bootstrap) fn selection_focus_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> u32 {
    selection_record_boundary_offset(scope, selection, SelectionBoundaryRole::Focus).unwrap_or(0)
}

pub(in crate::context_bootstrap) fn selection_direction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let handle = selection_record_handle(scope, selection)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }
        .selection_record_direction(handle)
        .map(str::to_owned)
}

pub(in crate::context_bootstrap) fn selection_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    selection_slot_value(scope, selection, SELECTION_RANGE_SLOT)
        .and_then(|value| (!value.is_null_or_undefined()).then_some(value))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn selection_composed_start_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    selection_record_boundary_object(scope, selection, SelectionBoundaryRole::ComposedStart)
}

pub(in crate::context_bootstrap) fn selection_composed_end_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    selection_record_boundary_object(scope, selection, SelectionBoundaryRole::ComposedEnd)
}

pub(in crate::context_bootstrap) fn selection_composed_start_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> u32 {
    selection_record_boundary_offset(scope, selection, SelectionBoundaryRole::ComposedStart)
        .unwrap_or(0)
}

pub(in crate::context_bootstrap) fn selection_composed_end_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> u32 {
    selection_record_boundary_offset(scope, selection, SelectionBoundaryRole::ComposedEnd)
        .unwrap_or(0)
}

pub(in crate::context_bootstrap) fn selection_owner_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let handle = selection_record_handle(scope, selection)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let owner = unsafe { &*host_ptr }.selection_record_owner_document(handle)?;
    selection_wrap_boundary_handle(scope, host_ptr, owner)
}

pub(in crate::context_bootstrap) fn selection_bind_owner_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    document: v8::Local<'s, v8::Object>,
) {
    let Some(handle) = ensure_selection_record_handle(scope, selection) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(document_handle) = selection_dom_handle_for_object(scope, host_ptr, document) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    if host.selection_record_owner_document(handle) != Some(document_handle) {
        host.clear_selection_record(handle);
        host.set_selection_record_owner_document(handle, document_handle);
    }
}

pub(in crate::context_bootstrap) fn selection_has_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> bool {
    selection_record_handle(scope, selection)
        .and_then(|handle| {
            context_host_ptr_from_global_bridge(scope)
                .map(|host_ptr| unsafe { &*host_ptr }.selection_record_has_range(handle))
        })
        .unwrap_or(false)
}

pub(in crate::context_bootstrap) fn selection_is_collapsed_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> bool {
    selection_record_handle(scope, selection)
        .and_then(|handle| {
            context_host_ptr_from_global_bridge(scope)
                .map(|host_ptr| unsafe { &mut *host_ptr }.selection_record_is_collapsed(handle))
        })
        .unwrap_or(true)
}

pub(in crate::context_bootstrap) fn selection_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    range: v8::Local<'s, v8::Object>,
    anchor_node: v8::Local<'s, v8::Object>,
    anchor_offset: u32,
    focus_node: v8::Local<'s, v8::Object>,
    focus_offset: u32,
    direction: &str,
) {
    let (range_start_node, range_start_offset, range_end_node, range_end_offset) =
        match boundary_order(scope, anchor_node, anchor_offset, focus_node, focus_offset) {
            std::cmp::Ordering::Greater => (focus_node, focus_offset, anchor_node, anchor_offset),
            _ => (anchor_node, anchor_offset, focus_node, focus_offset),
        };
    selection_store_with_range_boundaries(
        scope,
        selection,
        range,
        anchor_node,
        anchor_offset,
        focus_node,
        focus_offset,
        direction,
        range_start_node,
        range_start_offset,
        range_end_node,
        range_end_offset,
    );
}

pub(in crate::context_bootstrap) fn selection_store_with_range_boundaries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    range: v8::Local<'s, v8::Object>,
    anchor_node: v8::Local<'s, v8::Object>,
    anchor_offset: u32,
    focus_node: v8::Local<'s, v8::Object>,
    focus_offset: u32,
    direction: &str,
    range_start_node: v8::Local<'s, v8::Object>,
    range_start_offset: u32,
    range_end_node: v8::Local<'s, v8::Object>,
    range_end_offset: u32,
) {
    selection_store_with_composed_boundaries(
        scope,
        selection,
        range,
        anchor_node,
        anchor_offset,
        focus_node,
        focus_offset,
        direction,
        range_start_node,
        range_start_offset,
        range_end_node,
        range_end_offset,
        range_start_node,
        range_start_offset,
        range_end_node,
        range_end_offset,
    );
}

#[allow(clippy::too_many_arguments)]
pub(in crate::context_bootstrap) fn selection_store_with_composed_boundaries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    range: v8::Local<'s, v8::Object>,
    anchor_node: v8::Local<'s, v8::Object>,
    anchor_offset: u32,
    focus_node: v8::Local<'s, v8::Object>,
    focus_offset: u32,
    direction: &str,
    range_start_node: v8::Local<'s, v8::Object>,
    range_start_offset: u32,
    range_end_node: v8::Local<'s, v8::Object>,
    range_end_offset: u32,
    composed_start_node: v8::Local<'s, v8::Object>,
    composed_start_offset: u32,
    composed_end_node: v8::Local<'s, v8::Object>,
    composed_end_offset: u32,
) {
    selection_store_native_record(
        scope,
        selection,
        range,
        anchor_node,
        anchor_offset,
        focus_node,
        focus_offset,
        direction,
        composed_start_node,
        composed_start_offset,
        composed_end_node,
        composed_end_offset,
    );
    set_range_boundary(
        scope,
        range,
        RangeBoundarySide::Start,
        range_start_node,
        range_start_offset,
    );
    set_range_boundary(
        scope,
        range,
        RangeBoundarySide::End,
        range_end_node,
        range_end_offset,
    );
}

pub(in crate::context_bootstrap) fn selection_range_update_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<SelectionRangeUpdateState<'s>> {
    let selection = window_selection_value(scope)?;
    let selected_range = selection_range(scope, selection)?;
    if !selected_range.strict_equals(range.into()) {
        return None;
    }
    let old_composed_start_node = selection_composed_start_node(scope, selection)
        .or_else(|| range_boundary_container_object(scope, range, RangeBoundarySide::Start))?;
    let old_composed_start_offset = selection_composed_start_node(scope, selection)
        .map(|_| selection_composed_start_offset(scope, selection))
        .unwrap_or_else(|| range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32);
    let old_composed_end_node = selection_composed_end_node(scope, selection)
        .or_else(|| range_boundary_container_object(scope, range, RangeBoundarySide::End))?;
    let old_composed_end_offset = selection_composed_end_node(scope, selection)
        .map(|_| selection_composed_end_offset(scope, selection))
        .unwrap_or_else(|| range_boundary_offset(scope, range, RangeBoundarySide::End) as u32);
    Some(SelectionRangeUpdateState {
        selection,
        old_composed_start_node,
        old_composed_start_offset,
        old_composed_end_node,
        old_composed_end_offset,
    })
}

pub(in crate::context_bootstrap) fn selection_sync_associated_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: SelectionRangeUpdateState<'s>,
    range: v8::Local<'s, v8::Object>,
    composed_start_node: v8::Local<'s, v8::Object>,
    composed_start_offset: u32,
    composed_end_node: v8::Local<'s, v8::Object>,
    composed_end_offset: u32,
) {
    let Some(range_start_node) =
        range_boundary_container_object(scope, range, RangeBoundarySide::Start)
    else {
        selection_clear(scope, state.selection);
        return;
    };
    let Some(range_end_node) =
        range_boundary_container_object(scope, range, RangeBoundarySide::End)
    else {
        selection_clear(scope, state.selection);
        return;
    };
    if !selection_boundary_is_connected(scope, range_start_node)
        || !selection_boundary_is_connected(scope, range_end_node)
    {
        selection_clear(scope, state.selection);
        return;
    }

    let range_start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    let range_end_offset = range_boundary_offset(scope, range, RangeBoundarySide::End) as u32;
    let direction = if range_is_collapsed(scope, range) {
        "none"
    } else {
        "forward"
    };
    selection_update_slots_with_composed_boundaries(
        scope,
        state.selection,
        range,
        range_start_node,
        range_start_offset,
        range_end_node,
        range_end_offset,
        direction,
        composed_start_node,
        composed_start_offset,
        composed_end_node,
        composed_end_offset,
    );
}

pub(in crate::context_bootstrap) fn selection_update_composed_boundaries_for_child_removal(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    dom_host: &crate::dom::native::DomHost,
    parent: DomHandle,
    removed_child: DomHandle,
    index: u32,
    previous_sibling: Option<DomHandle>,
) {
    let Some(selection) = window_selection_value(scope) else {
        return;
    };
    let Some(record_handle) = selection_record_handle(scope, selection) else {
        return;
    };
    let Some(range) = selection_range(scope, selection) else {
        return;
    };
    let observable_anchor = selection_anchor_node(scope, selection)
        .and_then(|node| callback_value_dom_handle(scope, node.into()))
        .map(|handle| (handle, selection_anchor_offset(scope, selection)));
    let observable_focus = selection_focus_node(scope, selection)
        .and_then(|node| callback_value_dom_handle(scope, node.into()))
        .map(|handle| (handle, selection_focus_offset(scope, selection)));
    let rescope_observable = selection_should_rescope_observable_for_child_removal(
        unsafe { &*host_ptr },
        dom_host,
        observable_anchor,
        observable_focus,
        removed_child,
    );
    let anchor_boundary = rescope_observable.then(|| {
        let (handle, offset) =
            observable_anchor.expect("observable anchor is present when rescoping");
        selection_rescope_composed_boundary_for_child_removal(
            dom_host,
            handle,
            offset,
            parent,
            removed_child,
            index,
            previous_sibling,
        )
    });
    let focus_boundary = rescope_observable.then(|| {
        let (handle, offset) =
            observable_focus.expect("observable focus is present when rescoping");
        selection_rescope_composed_boundary_for_child_removal(
            dom_host,
            handle,
            offset,
            parent,
            removed_child,
            index,
            previous_sibling,
        )
    });
    let Some(start_node) = selection_composed_start_node(scope, selection)
        .or_else(|| range_boundary_container_object(scope, range, RangeBoundarySide::Start))
    else {
        return;
    };
    let Some(end_node) = selection_composed_end_node(scope, selection)
        .or_else(|| range_boundary_container_object(scope, range, RangeBoundarySide::End))
    else {
        return;
    };
    let start_offset = selection_composed_start_node(scope, selection)
        .map(|_| selection_composed_start_offset(scope, selection))
        .unwrap_or_else(|| range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32);
    let end_offset = selection_composed_end_node(scope, selection)
        .map(|_| selection_composed_end_offset(scope, selection))
        .unwrap_or_else(|| range_boundary_offset(scope, range, RangeBoundarySide::End) as u32);
    let Some(start_handle) = callback_value_dom_handle(scope, start_node.into()) else {
        return;
    };
    let Some(end_handle) = callback_value_dom_handle(scope, end_node.into()) else {
        return;
    };

    let (next_start_handle, next_start_offset) =
        selection_rescope_composed_boundary_for_child_removal(
            dom_host,
            start_handle,
            start_offset,
            parent,
            removed_child,
            index,
            previous_sibling,
        );
    let (next_end_handle, next_end_offset) = selection_rescope_composed_boundary_for_child_removal(
        dom_host,
        end_handle,
        end_offset,
        parent,
        removed_child,
        index,
        previous_sibling,
    );

    let host = unsafe { &mut *host_ptr };
    if let Some((anchor_handle, anchor_offset)) = anchor_boundary {
        let _ = host.set_selection_record_boundary(
            record_handle,
            SelectionBoundaryRole::Anchor,
            anchor_handle,
            anchor_offset,
        );
    }
    if let Some((focus_handle, focus_offset)) = focus_boundary {
        let _ = host.set_selection_record_boundary(
            record_handle,
            SelectionBoundaryRole::Focus,
            focus_handle,
            focus_offset,
        );
    }
    let _ = host.set_selection_record_boundary(
        record_handle,
        SelectionBoundaryRole::ComposedStart,
        next_start_handle,
        next_start_offset,
    );
    let _ = host.set_selection_record_boundary(
        record_handle,
        SelectionBoundaryRole::ComposedEnd,
        next_end_handle,
        next_end_offset,
    );
}

fn selection_should_rescope_observable_for_child_removal(
    runtime: &JsContextHost,
    dom_host: &crate::dom::native::DomHost,
    anchor: Option<(DomHandle, u32)>,
    focus: Option<(DomHandle, u32)>,
    removed_child: DomHandle,
) -> bool {
    let (Some((anchor_handle, anchor_offset)), Some((focus_handle, focus_offset))) =
        (anchor, focus)
    else {
        return false;
    };
    anchor_handle == focus_handle
        && anchor_offset == focus_offset
        && contenteditable_editing_host(runtime, anchor_handle) == Some(anchor_handle)
        && selection_shadow_including_descendant_or_self(dom_host, anchor_handle, removed_child)
}

fn selection_rescope_composed_boundary_for_child_removal(
    dom_host: &crate::dom::native::DomHost,
    container: DomHandle,
    offset: u32,
    parent: DomHandle,
    removed_child: DomHandle,
    index: u32,
    previous_sibling: Option<DomHandle>,
) -> (DomHandle, u32) {
    if selection_shadow_including_descendant_or_self(dom_host, container, removed_child) {
        return (
            parent,
            selection_removed_child_boundary_offset(dom_host, parent, index, previous_sibling),
        );
    }
    if container == parent && offset > index {
        return (container, offset - 1);
    }
    (container, offset)
}

fn selection_removed_child_boundary_offset(
    dom_host: &crate::dom::native::DomHost,
    parent: DomHandle,
    index: u32,
    previous_sibling: Option<DomHandle>,
) -> u32 {
    previous_sibling
        .and_then(|previous| dom_host.child_index(parent, previous))
        .and_then(|previous_index| u32::try_from(previous_index + 1).ok())
        .unwrap_or(index)
}

fn selection_shadow_including_descendant_or_self(
    dom_host: &crate::dom::native::DomHost,
    node: DomHandle,
    ancestor: DomHandle,
) -> bool {
    let mut current = Some(node);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = dom_host
            .node(handle)
            .and_then(|node| node.parent_node())
            .or_else(|| {
                dom_host
                    .is_shadow_root(handle)
                    .then(|| dom_host.shadow_root_host(handle))
                    .flatten()
            });
    }
    false
}

fn selection_wrap_boundary_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    wrapped_handle_value(scope, host_ptr, handle)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

#[allow(clippy::too_many_arguments)]
fn selection_update_slots_with_composed_boundaries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    range: v8::Local<'s, v8::Object>,
    anchor_node: v8::Local<'s, v8::Object>,
    anchor_offset: u32,
    focus_node: v8::Local<'s, v8::Object>,
    focus_offset: u32,
    direction: &str,
    composed_start_node: v8::Local<'s, v8::Object>,
    composed_start_offset: u32,
    composed_end_node: v8::Local<'s, v8::Object>,
    composed_end_offset: u32,
) {
    selection_store_native_record(
        scope,
        selection,
        range,
        anchor_node,
        anchor_offset,
        focus_node,
        focus_offset,
        direction,
        composed_start_node,
        composed_start_offset,
        composed_end_node,
        composed_end_offset,
    );
}

fn selection_boundary_is_connected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    let Some(handle) = callback_value_dom_handle(scope, node.into()) else {
        return true;
    };
    unsafe { &*host_ptr }.dom_host().is_connected(handle)
}

fn selection_record_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<SelectionRecordHandle> {
    if selection.internal_field_count() < SELECTION_WRAPPER_INTERNAL_FIELD_COUNT {
        return None;
    }
    let value = selection.get_internal_field(scope, SELECTION_RECORD_INTERNAL_FIELD_INDEX)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (raw, lossless) = value.u64_value();
    lossless.then(|| SelectionRecordHandle::new(raw)).flatten()
}

fn ensure_selection_record_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> Option<SelectionRecordHandle> {
    if selection.internal_field_count() < SELECTION_WRAPPER_INTERNAL_FIELD_COUNT {
        return None;
    }
    if let Some(handle) = selection_record_handle(scope, selection) {
        return Some(handle);
    }
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = unsafe { &mut *host_ptr }.create_selection_record()?;
    let value = v8::BigInt::new_from_u64(scope, handle.raw());
    let _ = selection.set_internal_field(SELECTION_RECORD_INTERNAL_FIELD_INDEX, value.into());
    Some(handle)
}

fn selection_record_boundary_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    role: SelectionBoundaryRole,
) -> Option<v8::Local<'s, v8::Object>> {
    let boundary = selection_record_boundary(scope, selection, role)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    selection_wrap_boundary_handle(scope, host_ptr, boundary.container)
}

fn selection_record_boundary_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    role: SelectionBoundaryRole,
) -> Option<u32> {
    selection_record_boundary(scope, selection, role).map(|boundary| boundary.offset)
}

fn selection_record_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    role: SelectionBoundaryRole,
) -> Option<SelectionBoundarySnapshot> {
    let handle = selection_record_handle(scope, selection)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &mut *host_ptr }.selection_record_boundary(handle, role)
}

#[allow(clippy::too_many_arguments)]
fn selection_store_native_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    range: v8::Local<'s, v8::Object>,
    anchor_node: v8::Local<'s, v8::Object>,
    anchor_offset: u32,
    focus_node: v8::Local<'s, v8::Object>,
    focus_offset: u32,
    direction: &str,
    composed_start_node: v8::Local<'s, v8::Object>,
    composed_start_offset: u32,
    composed_end_node: v8::Local<'s, v8::Object>,
    composed_end_offset: u32,
) {
    let Some(record_handle) = ensure_selection_record_handle(scope, selection) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(anchor_handle) = selection_dom_handle_for_object(scope, host_ptr, anchor_node) else {
        return;
    };
    let Some(focus_handle) = selection_dom_handle_for_object(scope, host_ptr, focus_node) else {
        return;
    };
    let Some(composed_start_handle) =
        selection_dom_handle_for_object(scope, host_ptr, composed_start_node)
    else {
        return;
    };
    let Some(composed_end_handle) =
        selection_dom_handle_for_object(scope, host_ptr, composed_end_node)
    else {
        return;
    };
    let associated_range = range_native_record_handle(scope, range);
    if unsafe { &mut *host_ptr }.store_selection_record(
        record_handle,
        associated_range,
        (anchor_handle, anchor_offset),
        (focus_handle, focus_offset),
        direction,
        (composed_start_handle, composed_start_offset),
        (composed_end_handle, composed_end_offset),
    ) {
        set_selection_slot_value(scope, selection, SELECTION_RANGE_SLOT, range.into());
    }
}

fn selection_dom_handle_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    callback_value_dom_handle(scope, object.into()).or_else(|| {
        native_bridge::document::detached_native_handle_for_runtime(scope, host_ptr, object)
    })
}

fn selection_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, selection, key)
}

fn set_selection_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    selection: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, selection, key, value);
}

pub(in crate::context_bootstrap) fn selection_set_collapsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
) -> bool {
    let Some(document) = node_owner_document_or_self(scope, node) else {
        return false;
    };
    let Some(range) = new_range_for_document(scope, document) else {
        return false;
    };
    selection_store(scope, selection, range, node, offset, node, offset, "none");
    true
}
