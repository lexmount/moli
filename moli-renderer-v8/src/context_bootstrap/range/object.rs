use super::static_storage::{
    set_static_range_boundary, static_range_boundary_container_object, static_range_boundary_offset,
};
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{RangeBoundarySide, RangeRecordHandle};

pub(in crate::context_bootstrap) const RANGE_RECORD_LIFETIME_INTERNAL_FIELD_INDEX: usize = 0;
pub(in crate::context_bootstrap) const RANGE_RECORD_ID_INTERNAL_FIELD_INDEX: usize = 1;
pub(in crate::context_bootstrap) const RANGE_WRAPPER_INTERNAL_FIELD_COUNT: usize = 2;

pub(in crate::context_bootstrap) fn current_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    global
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn new_range_for_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = crate::native_bridge::node_relevant_context(scope, document)?;
    if relevant_context != scope.get_current_context() {
        let document = v8::Global::new(scope, document);
        let range = {
            let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
            let document = v8::Local::new(target_scope, &document);
            let range = new_range_for_document_in_current_context(target_scope, document)?;
            v8::Global::new(target_scope, range)
        };
        return Some(v8::Local::new(scope, &range));
    }
    new_range_for_document_in_current_context(scope, document)
}

fn new_range_for_document_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let ctor = global.get(scope, v8str(scope, "Range").into())?;
    let ctor = v8::Local::<v8::Function>::try_from(ctor).ok()?;
    let range = ctor.new_instance(scope, &[])?;
    initialize_range_object(scope, range, document);
    Some(range)
}

pub(in crate::context_bootstrap) fn initialize_range_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    document: v8::Local<'s, v8::Object>,
) {
    if range.internal_field_count() < RANGE_WRAPPER_INTERNAL_FIELD_COUNT {
        return;
    }
    let existing_handle = range_native_record_handle(scope, range);
    initialize_native_range_record(scope, range, document, existing_handle);
}

pub(in crate::context_bootstrap) fn set_range_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
    container: v8::Local<'s, v8::Object>,
    offset: u32,
) {
    if set_native_range_record_boundary(scope, range, side, container, offset)
        || range_native_record_handle(scope, range).is_some()
    {
        return;
    }
    set_static_range_boundary(scope, range, side, container, offset);
}

pub(in crate::context_bootstrap) fn range_boundary_container_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(native) = native_range_boundary_container_object(scope, range, side) {
        return Some(native);
    }
    static_range_boundary_container_object(scope, range, side)
}

pub(in crate::context_bootstrap) fn range_boundary_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> f64 {
    if let Some(native) = native_range_boundary_offset(scope, range, side) {
        return native as f64;
    }
    static_range_boundary_offset(scope, range, side)
}

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap) struct RangeBoundaryHandle {
    pub(in crate::context_bootstrap) container: DomHandle,
    pub(in crate::context_bootstrap) offset: u32,
}

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap) struct RangeBoundaryHandles {
    pub(in crate::context_bootstrap) start: RangeBoundaryHandle,
    pub(in crate::context_bootstrap) end: RangeBoundaryHandle,
}

pub(in crate::context_bootstrap) fn native_range_boundary_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<RangeBoundaryHandles> {
    let handle = range_native_record_handle(scope, range)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let (start, end) = unsafe { &mut *host_ptr }.range_record_boundary_handles(handle)?;
    Some(RangeBoundaryHandles {
        start: RangeBoundaryHandle {
            container: start.0,
            offset: start.1,
        },
        end: RangeBoundaryHandle {
            container: end.0,
            offset: end.1,
        },
    })
}

pub(in crate::context_bootstrap) fn native_range_boundary_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> Option<crate::range_boundary::RangeBoundaryPoint> {
    let handle = range_native_record_handle(scope, range)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.range_record_boundary_point(handle, side)
}

fn initialize_native_range_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    document: v8::Local<'s, v8::Object>,
    existing_handle: Option<RangeRecordHandle>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(document_handle) = dom_handle_for_range_object(scope, host_ptr, document) else {
        return;
    };
    let handle = existing_handle
        .and_then(|handle| {
            unsafe { &mut *host_ptr }
                .reset_range_record(handle, document_handle)
                .then_some(handle)
        })
        .or_else(|| unsafe { &mut *host_ptr }.create_range_record(document_handle));
    if let Some(handle) = handle {
        set_range_native_record_handle_storage(scope, range, handle);
        unsafe { &mut *host_ptr }.register_live_range_record(scope, range, handle);
    }
}

pub(in crate::context_bootstrap) fn range_native_record_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<RangeRecordHandle> {
    let lifetime_value =
        range.get_internal_field(scope, RANGE_RECORD_LIFETIME_INTERNAL_FIELD_INDEX)?;
    let lifetime_big = v8::Local::<v8::BigInt>::try_from(lifetime_value).ok()?;
    let (lifetime_token, lifetime_lossless) = lifetime_big.u64_value();
    if !lifetime_lossless {
        return None;
    }
    let id_value = range.get_internal_field(scope, RANGE_RECORD_ID_INTERNAL_FIELD_INDEX)?;
    let id_big = v8::Local::<v8::BigInt>::try_from(id_value).ok()?;
    let (id, id_lossless) = id_big.u64_value();
    id_lossless
        .then(|| RangeRecordHandle::new(lifetime_token, id))
        .flatten()
}

fn set_range_native_record_handle_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    handle: RangeRecordHandle,
) {
    let lifetime = v8::BigInt::new_from_u64(scope, handle.lifetime_token());
    let _ = range.set_internal_field(RANGE_RECORD_LIFETIME_INTERNAL_FIELD_INDEX, lifetime.into());
    let id = v8::BigInt::new_from_u64(scope, handle.raw_id());
    let _ = range.set_internal_field(RANGE_RECORD_ID_INTERNAL_FIELD_INDEX, id.into());
}

fn native_range_boundary_container_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> Option<v8::Local<'s, v8::Object>> {
    let handle = range_native_record_handle(scope, range)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = unsafe { &*host_ptr }.range_record_boundary_container(handle, side)?;
    native_bridge::wrapped_handle_value(scope, host_ptr, handle)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn native_range_boundary_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> Option<u32> {
    let handle = range_native_record_handle(scope, range)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &mut *host_ptr }.range_record_boundary_offset(handle, side)
}

fn set_native_range_record_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
    container: v8::Local<'s, v8::Object>,
    offset: u32,
) -> bool {
    let Some(handle) = range_native_record_handle(scope, range) else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(container_handle) = dom_handle_for_range_object(scope, host_ptr, container) else {
        return false;
    };
    unsafe { &mut *host_ptr }.set_range_record_boundary(handle, side, container_handle, offset)
}

fn dom_handle_for_range_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    native_bridge::callback_value_dom_handle(scope, object.into()).or_else(|| {
        native_bridge::document::detached_native_handle_for_runtime(scope, host_ptr, object)
    })
}

pub(in crate::context_bootstrap) fn range_is_collapsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(handle) = range_native_record_handle(scope, range)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(collapsed) = unsafe { &mut *host_ptr }.range_record_is_collapsed(handle)
    {
        return collapsed;
    }
    let Some(start_container) =
        range_boundary_container_object(scope, range, RangeBoundarySide::Start)
    else {
        return true;
    };
    let Some(end_container) = range_boundary_container_object(scope, range, RangeBoundarySide::End)
    else {
        return true;
    };
    start_container.strict_equals(end_container.into())
        && (range_boundary_offset(scope, range, RangeBoundarySide::Start)
            == range_boundary_offset(scope, range, RangeBoundarySide::End))
}
