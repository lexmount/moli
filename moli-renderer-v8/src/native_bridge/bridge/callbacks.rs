use super::super::{BridgeHandle, JsContextHost, ReflectorId, bridge_handle_from_object};
use super::NativeDomBridge;
use crate::{
    document_runtime::DomHandle,
    dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT,
    util::{get_private_object, serialize_v8_array},
};

/// Returns the canonical JavaScript identity exposed for a native node.
///
/// `NativeDomBridge::wrap_handle` owns the current-realm implementation object.
/// Web API results must additionally honor an existing foreign/detached pairing,
/// so callers use this function instead of choosing an identity policy.
pub(crate) fn set_wrapped_handle_or_null(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    handle: Option<DomHandle>,
) {
    let Some(handle) = handle else {
        rv.set_null();
        return;
    };
    match wrapped_handle_value(scope, runtime_ptr, handle) {
        Some(node) => rv.set(node),
        None => rv.set_null(),
    }
}

pub(crate) fn set_wrapped_handle_or_null_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    receiver: v8::Local<'s, v8::Object>,
    handle: Option<DomHandle>,
) {
    let Some(handle) = handle else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime
        .native_bridge_mut()
        .wrap_handle_for_receiver(scope, runtime_ptr, receiver, handle)
    {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(crate) fn wrapped_handle_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Value>> {
    let live = {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, handle)?
    };
    if let Some(foreign) = get_private_object(scope, live, DOM_PARSER_FOREIGN_NODE_SLOT) {
        return Some(foreign.into());
    }
    let detached_owner = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(handle)
        .and_then(|owner_document| {
            crate::native_bridge::document::paired_detached_native_object_for_handle(
                scope,
                runtime_ptr,
                owner_document,
            )
        });
    if detached_owner.is_some() {
        return crate::native_bridge::document::detached_native_object_for_handle(
            scope,
            runtime_ptr,
            handle,
        )
        .map(Into::into);
    }
    Some(live.into())
}

pub(crate) fn set_wrapped_handle_array(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    handles: &[DomHandle],
) {
    let mut values = Vec::with_capacity(handles.len());
    for handle in handles.iter().copied() {
        let Some(node) = wrapped_handle_value(scope, runtime_ptr, handle) else {
            rv.set_null();
            return;
        };
        values.push(node);
    }
    let array =
        serialize_v8_array(scope, values.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(array.into());
}

pub(crate) fn runtime_ptr_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> std::result::Result<*mut JsContextHost, String> {
    let value = object
        .get_internal_field(scope, 0)
        .ok_or_else(|| "native bridge object missing runtime field".to_owned())?;
    let external = v8::Local::<v8::External>::try_from(value)
        .map_err(|_| "native bridge runtime field was not an External".to_owned())?;
    let runtime_ptr = external.value() as *mut JsContextHost;
    if runtime_ptr.is_null() {
        return Err("native bridge runtime pointer was null".to_owned());
    }
    Ok(runtime_ptr)
}

pub(crate) fn callback_arg_dom_handle(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> Option<DomHandle> {
    callback_value_dom_handle(scope, args.get(index))
}

pub(crate) fn callback_value_dom_handle(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if value.is_null_or_undefined() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let (_, handle) = bridge_handle_from_object(scope, object).ok()?;
    match handle {
        BridgeHandle::Node(handle) => Some(handle),
        BridgeHandle::Window
        | BridgeHandle::ClassList(_, _)
        | BridgeHandle::Dataset(_)
        | BridgeHandle::Style(_)
        | BridgeHandle::ComputedStyle(_, _) => None,
    }
}

impl NativeDomBridge {
    pub(in crate::native_bridge) fn bridge_handle(
        &self,
        reflector_id: ReflectorId,
    ) -> Option<BridgeHandle> {
        self.identity.bridge_handle(reflector_id)
    }
}
