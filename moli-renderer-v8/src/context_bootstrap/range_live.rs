use super::selection::selection_update_composed_boundaries_for_child_removal;
use super::*;
use crate::document_runtime::DomHandle;

fn range_container_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    container: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    if let Some(handle) = native_bridge::callback_value_dom_handle(scope, container.into()) {
        return Some(handle);
    }
    native_bridge::document::detached_native_handle_for_runtime(scope, host_ptr, container)
}

pub(super) fn update_live_ranges_for_character_data_edit(
    scope: &mut v8::PinScope<'_, '_>,
    target: DomHandle,
    edit_offset: u32,
    removed_count: u32,
    inserted_count: u32,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }.update_live_range_records_for_character_data_edit(
        scope,
        target,
        edit_offset,
        removed_count,
        inserted_count,
    );
}

pub(super) fn update_live_ranges_for_character_data_reset(
    scope: &mut v8::PinScope<'_, '_>,
    target: DomHandle,
    removed_count: u32,
    inserted_count: u32,
) {
    update_live_ranges_for_character_data_edit(scope, target, 0, removed_count, inserted_count);
}

pub(super) fn update_live_ranges_for_detached_character_data_edit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    edit_offset: u32,
    removed_count: u32,
    inserted_count: u32,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(target_handle) = range_container_handle(scope, host_ptr, target)
    {
        unsafe { &mut *host_ptr }.update_live_range_records_for_character_data_edit(
            scope,
            target_handle,
            edit_offset,
            removed_count,
            inserted_count,
        );
    }
}

pub(super) fn update_live_ranges_for_detached_character_data_reset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    removed_count: u32,
    inserted_count: u32,
) {
    update_live_ranges_for_detached_character_data_edit(
        scope,
        target,
        0,
        removed_count,
        inserted_count,
    );
}

pub(super) fn update_live_ranges_for_child_insertion(
    _scope: &mut v8::PinScope<'_, '_>,
    _parent: DomHandle,
    _index: u32,
    _inserted_child: DomHandle,
) {
    // Native RangeBoundaryPoint lazily repairs child-list insertion offsets
    // from its child-before anchor when the offset is read.
}

pub(super) fn update_live_ranges_for_child_removal(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    _dom_host: &crate::dom::native::DomHost,
    parent: DomHandle,
    removed_child: DomHandle,
    index: u32,
    previous_sibling: Option<DomHandle>,
) {
    unsafe { &mut *host_ptr }.update_live_range_records_for_child_removal(
        scope,
        parent,
        removed_child,
        index,
        previous_sibling,
    );
    selection_update_composed_boundaries_for_child_removal(
        scope,
        host_ptr,
        _dom_host,
        parent,
        removed_child,
        index,
        previous_sibling,
    );
}

pub(super) fn update_live_ranges_for_detached_child_insertion<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _parent: v8::Local<'s, v8::Object>,
    _index: u32,
) {
    // See update_live_ranges_for_child_insertion.
}

pub(super) fn update_live_ranges_for_text_split(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    original: DomHandle,
    new_text: DomHandle,
    offset: u32,
) {
    unsafe { &mut *host_ptr }
        .update_live_range_records_for_text_split(scope, original, new_text, offset);
}

pub(super) fn update_live_ranges_for_detached_text_split<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Object>,
    new_text: v8::Local<'s, v8::Object>,
    offset: u32,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(original_handle) = range_container_handle(scope, host_ptr, original)
        && let Some(new_text_handle) = range_container_handle(scope, host_ptr, new_text)
    {
        unsafe { &mut *host_ptr }.update_live_range_records_for_text_split(
            scope,
            original_handle,
            new_text_handle,
            offset,
        );
    }
}
