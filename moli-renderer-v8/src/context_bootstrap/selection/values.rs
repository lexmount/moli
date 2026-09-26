use super::super::range::{
    RangeBoundarySide, current_document_object, new_range_for_document,
    range_boundary_container_object, range_boundary_offset, range_native_record_handle,
    set_range_boundary,
};
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::document::document_associated_window_for_handle;
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

pub(in crate::context_bootstrap) fn selection_spans_dom_roots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
) -> bool {
    selection_record_handle(scope, selection)
        .and_then(|handle| {
            context_host_ptr_from_global_bridge(scope)
                .map(|host_ptr| unsafe { &*host_ptr }.selection_record_spans_dom_roots(handle))
        })
        .unwrap_or(false)
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
    // A Range may be created in one realm and selected in another. Resolve
    // the association from native records, independently of the callee realm.
    let range_handle = range_native_record_handle(scope, range)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let document = unsafe { &*host_ptr }.selection_document_for_range(range_handle)?;
    let window = document_associated_window_for_handle(scope, host_ptr, document)?;
    let selection = get_private_value(scope, window, WINDOW_SELECTION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
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
    if !selection_range_belongs_to_document(
        scope,
        state.selection,
        range_start_node,
        range_end_node,
    ) {
        selection_clear(scope, state.selection);
        return;
    }

    let range_start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    let range_end_offset = range_boundary_offset(scope, range, RangeBoundarySide::End) as u32;
    // A collapsed DOM Range can still project a composed span across roots.
    let direction = if composed_start_offset == composed_end_offset
        && callback_value_dom_handle(scope, composed_start_node.into())
            == callback_value_dom_handle(scope, composed_end_node.into())
    {
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

pub(in crate::context_bootstrap) fn selection_range_belongs_to_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    start_node: v8::Local<'s, v8::Object>,
    end_node: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(document) =
        selection_owner_document(scope, selection).or_else(|| current_document_object(scope))
    else {
        return true;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(document) = callback_value_dom_handle(scope, document.into()) else {
        return false;
    };
    // Connectivity and ownership are native relationships. Author properties
    // named parentNode, nodeType or host must not change Selection membership
    // (or run script while checking an associated Range).
    let dom = unsafe { &*host_ptr }.dom_host();
    [start_node, end_node].into_iter().all(|node| {
        callback_value_dom_handle(scope, node.into()).is_some_and(|handle| {
            dom.is_connected_to_document(handle)
                && (handle == document || dom.owner_document_handle(handle) == Some(document))
        })
    })
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
