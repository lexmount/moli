use super::super::*;
use super::FOREIGN_IDENTITY_LIVE_HANDLE_SLOT;
use super::clone::clone_js_node_like_into_document;
use super::detach::detach_foreign_node_from_parent;
use super::js_values::js_node_type;
use super::pairing::{
    pair_foreign_node_with_live_handle, pair_foreign_node_with_live_handle_for_identity,
};
use crate::{
    native_bridge::document::{
        DETACHED_LIVE_DELEGATE_SLOT, DETACHED_STATE_SLOT, define_detached_native_handle,
        detached_native_handle_for_runtime, is_attr_node_value,
    },
    util::{get_private_object, get_private_value},
};

enum NodeArgumentResolution<'s> {
    Missing,
    SameRuntimeNative(DomHandle),
    LiveDelegate(DomHandle),
    CrossRuntimeNative(v8::Local<'s, v8::Object>),
    DetachedSnapshot(v8::Local<'s, v8::Object>),
    ForeignObject(v8::Local<'s, v8::Object>),
    Invalid,
}

pub(in crate::native_bridge) enum ExistingNodeArgument {
    Handle(DomHandle),
    ForeignNode,
    Invalid,
}

pub(in crate::native_bridge) fn live_delegate_arg_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    object: v8::Local<'_, v8::Object>,
) -> Option<DomHandle> {
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    let delegate = get_private_object(scope, object, DETACHED_LIVE_DELEGATE_SLOT)?;
    let (delegate_runtime_ptr, handle) =
        node_runtime_and_handle_from_object(scope, delegate).ok()?;
    (delegate_runtime_ptr == runtime_ptr).then_some(handle)
}

fn is_detached_node_arg(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    get_private_object(scope, object, DETACHED_STATE_SLOT).is_some()
}

fn foreign_identity_arg_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let value = get_private_value(scope, object, FOREIGN_IDENTITY_LIVE_HANDLE_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (index, lossless) = big.u64_value();
    if !lossless {
        return None;
    }
    let handle = DomHandle::new(index as usize);
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some()
        .then_some(handle)
}

fn materialize_foreign_node_arg_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    object: v8::Local<'_, v8::Object>,
) -> Option<DomHandle> {
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    detach_foreign_node_from_parent(scope, object);
    let handle =
        clone_js_node_like_into_document(scope, runtime_ptr, document_handle, object, true)?;
    pair_foreign_node_with_live_handle(scope, runtime_ptr, object, handle)?;
    Some(handle)
}

fn materialize_foreign_node_arg_handle_for_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let handle =
        clone_js_node_like_into_document(scope, runtime_ptr, document_handle, object, true)?;
    pair_foreign_node_with_live_handle_for_identity(scope, runtime_ptr, object, handle)?;
    Some(handle)
}

fn materialize_detached_node_arg_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    if let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, object) {
        return Some(handle);
    }
    let handle =
        clone_js_node_like_into_document(scope, runtime_ptr, document_handle, object, true)?;
    define_detached_native_handle(scope, object, handle);
    Some(handle)
}

fn resolve_node_argument<'v>(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'v, v8::Value>,
) -> NodeArgumentResolution<'v> {
    if value.is_null_or_undefined() {
        return NodeArgumentResolution::Missing;
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return NodeArgumentResolution::Invalid;
    };
    if let Ok((object_runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) {
        if object_runtime_ptr == runtime_ptr {
            return NodeArgumentResolution::SameRuntimeNative(handle);
        }
        return NodeArgumentResolution::CrossRuntimeNative(object);
    }
    if let Some(handle) = live_delegate_arg_handle(scope, runtime_ptr, object) {
        return NodeArgumentResolution::LiveDelegate(handle);
    }
    if is_detached_node_arg(scope, object) {
        return NodeArgumentResolution::DetachedSnapshot(object);
    }
    NodeArgumentResolution::ForeignObject(object)
}

fn is_node_like_foreign_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    matches!(
        js_node_type(scope, object),
        Some(1 | 3 | 4 | 7 | 8 | 9 | 10 | 11)
    )
}

pub(in crate::native_bridge) fn existing_node_arg(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'_, v8::Value>,
) -> ExistingNodeArgument {
    match resolve_node_argument(scope, runtime_ptr, value) {
        NodeArgumentResolution::SameRuntimeNative(handle)
        | NodeArgumentResolution::LiveDelegate(handle) => ExistingNodeArgument::Handle(handle),
        NodeArgumentResolution::DetachedSnapshot(object) => {
            let object = v8::Global::new(scope, object);
            let object = v8::Local::new(scope, object);
            detached_native_handle_for_runtime(scope, runtime_ptr, object)
                .map(ExistingNodeArgument::Handle)
                .unwrap_or(ExistingNodeArgument::ForeignNode)
        }
        NodeArgumentResolution::CrossRuntimeNative(_) => ExistingNodeArgument::ForeignNode,
        NodeArgumentResolution::ForeignObject(object) => {
            if is_attr_node_value(scope, object.into())
                || is_node_like_foreign_object(scope, object)
            {
                ExistingNodeArgument::ForeignNode
            } else {
                ExistingNodeArgument::Invalid
            }
        }
        NodeArgumentResolution::Missing | NodeArgumentResolution::Invalid => {
            ExistingNodeArgument::Invalid
        }
    }
}

pub(in crate::native_bridge) fn node_or_foreign_arg_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    match resolve_node_argument(scope, runtime_ptr, value) {
        NodeArgumentResolution::SameRuntimeNative(handle)
        | NodeArgumentResolution::LiveDelegate(handle) => Some(handle),
        NodeArgumentResolution::CrossRuntimeNative(object) => {
            let document = document_handle
                .unwrap_or_else(|| unsafe { &*runtime_ptr }.dom_host().document_handle());
            JsContextHost::adopt_node_from_host(scope, runtime_ptr, document, object)
        }
        NodeArgumentResolution::ForeignObject(object) => {
            let document_handle = document_handle?;
            materialize_foreign_node_arg_handle(scope, runtime_ptr, document_handle, object)
        }
        NodeArgumentResolution::DetachedSnapshot(_)
        | NodeArgumentResolution::Missing
        | NodeArgumentResolution::Invalid => None,
    }
}

pub(in crate::native_bridge) fn node_or_existing_detached_arg_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        let object = v8::Global::new(scope, object);
        let object = v8::Local::new(scope, object);
        if let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, object) {
            return Some(handle);
        }
    }
    match resolve_node_argument(scope, runtime_ptr, value) {
        NodeArgumentResolution::SameRuntimeNative(handle)
        | NodeArgumentResolution::LiveDelegate(handle) => Some(handle),
        NodeArgumentResolution::DetachedSnapshot(object) => {
            let object = v8::Global::new(scope, object);
            let object = v8::Local::new(scope, object);
            detached_native_handle_for_runtime(scope, runtime_ptr, object)
        }
        NodeArgumentResolution::CrossRuntimeNative(_)
        | NodeArgumentResolution::ForeignObject(_) => None,
        NodeArgumentResolution::Missing | NodeArgumentResolution::Invalid => None,
    }
}

pub(crate) fn node_or_foreign_arg_handle_allow_detached(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    match resolve_node_argument(scope, runtime_ptr, value) {
        NodeArgumentResolution::SameRuntimeNative(handle)
        | NodeArgumentResolution::LiveDelegate(handle) => Some(handle),
        NodeArgumentResolution::DetachedSnapshot(object) => {
            let document_handle = document_handle?;
            let object = v8::Global::new(scope, object);
            let object = v8::Local::new(scope, object);
            materialize_detached_node_arg_handle(scope, runtime_ptr, document_handle, object)
        }
        NodeArgumentResolution::CrossRuntimeNative(object) => {
            let document = document_handle
                .unwrap_or_else(|| unsafe { &*runtime_ptr }.dom_host().document_handle());
            JsContextHost::adopt_node_from_host(scope, runtime_ptr, document, object)
        }
        NodeArgumentResolution::ForeignObject(object) => {
            let document_handle = document_handle?;
            materialize_foreign_node_arg_handle(scope, runtime_ptr, document_handle, object)
        }
        NodeArgumentResolution::Missing | NodeArgumentResolution::Invalid => None,
    }
}

pub(in crate::native_bridge) fn node_or_foreign_arg_handle_preserve_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    value: v8::Local<'s, v8::Value>,
) -> Option<DomHandle> {
    if value.is_null_or_undefined() {
        return None;
    }
    if let Some(handle) = node_arg_handle(scope, runtime_ptr, value) {
        return Some(handle);
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if let Some(handle) = foreign_identity_arg_handle(scope, runtime_ptr, object) {
        return Some(handle);
    }
    if let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, object) {
        return Some(handle);
    }
    if let Some(handle) = live_delegate_arg_handle(scope, runtime_ptr, object) {
        return Some(handle);
    }
    let document_handle = document_handle?;
    materialize_foreign_node_arg_handle_for_identity(scope, runtime_ptr, document_handle, object)
}
