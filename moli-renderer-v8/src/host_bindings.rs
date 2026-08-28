use std::{convert::TryFrom, ffi::c_void};

use anyhow::anyhow;
use tracing::error;

use crate::dom::NodeId;
use anyhow::Result;
use moli_webapi_declare::WebApiObject;

use super::{
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    reflector::ReflectorId,
    util::{callback_arg_string, serialize_v8_array, v8_string, v8str},
};
use crate::{context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT, util::get_private_value};

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct HostBindingsDeclaration<'scope> {
    external: v8::Local<'scope, v8::External>,

    #[webapi(
        method = "__moliHostPrepareScriptStart",
        callback = host_prepare_script_start_callback,
        data = self.external
    )]
    prepare_script_start: (),
    #[webapi(
        method = "__moliHostDispatchDocumentEvent",
        callback = host_dispatch_document_event_callback,
        data = self.external
    )]
    dispatch_document_event: (),
    #[webapi(
        method = "__moliHostQuerySelector",
        callback = host_query_selector_callback,
        data = self.external
    )]
    query_selector: (),
    #[webapi(
        method = "__moliHostQuerySelectorAll",
        callback = host_query_selector_all_callback,
        data = self.external
    )]
    query_selector_all: (),
    #[webapi(
        method = "__moliHostMatches",
        callback = host_matches_callback,
        data = self.external
    )]
    matches: (),
    #[webapi(
        method = "__moliHostClosest",
        callback = host_closest_callback,
        data = self.external
    )]
    closest: (),
    #[webapi(
        method = "__moliHostSelectorDebugStats",
        callback = host_selector_debug_stats_callback,
        data = self.external
    )]
    selector_debug_stats: (),
    #[webapi(
        method = "__moliHostResolveInternalNodeReference",
        callback = host_resolve_internal_node_reference_callback,
        data = self.external,
        readonly,
        dont_delete
    )]
    resolve_internal_node_reference: (),
    #[webapi(
        method = "__moliHostResolveInternalInspectorValueReference",
        callback = host_resolve_internal_inspector_value_reference_callback,
        data = self.external,
        readonly,
        dont_delete
    )]
    resolve_internal_inspector_value_reference: (),
    #[webapi(
        method = "__moliHostResolveBackendNodeIdForObject",
        callback = host_resolve_backend_node_id_for_object_callback,
        data = self.external
    )]
    resolve_backend_node_id_for_object: (),
    #[webapi(
        method = "__moliHostResolveChildWindowByHandle",
        callback = host_resolve_child_window_by_handle_callback,
        data = self.external
    )]
    resolve_child_window_by_handle: (),
    #[webapi(
        method = "__moliHostChildFrameOwnerBackendNodeIdForWindow",
        callback = host_child_frame_owner_backend_node_id_for_window_callback,
        data = self.external
    )]
    child_frame_owner_backend_node_id_for_window: (),
    #[webapi(
        method = "__moliHostAuxiliaryPopupIdForObject",
        callback = host_auxiliary_popup_id_for_object_callback,
        data = self.external
    )]
    auxiliary_popup_id_for_object: (),
    #[webapi(
        method = "__moliHostBidiWindowRemoteValue",
        callback = host_bidi_window_remote_value_callback,
        data = self.external,
        readonly,
        dont_delete
    )]
    bidi_window_remote_value: (),
}

pub(super) fn install_host_bindings(
    scope: &mut v8::PinScope<'_, '_>,
    context_host: &mut JsContextHost,
) -> Result<()> {
    let external = v8::External::new(scope, context_host as *mut JsContextHost as *mut c_void);
    let global = scope.get_current_context().global(scope);
    HostBindingsDeclaration::new(external)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize host bindings: {error}"))
}

fn context_host_ptr_from_callback_data(
    scope: &mut v8::PinScope<'_, '_>,
    data: v8::Local<'_, v8::Value>,
) -> std::result::Result<*mut JsContextHost, String> {
    if crate::util::page_context_is_disconnected(scope.get_current_context()) {
        return Err("host callback belongs to a closed Page".to_owned());
    }
    let external = v8::Local::<v8::External>::try_from(data)
        .map_err(|_| "host callback missing external context data".to_owned())?;
    let host_ptr = external.value() as *mut JsContextHost;
    if host_ptr.is_null() {
        return Err("host callback context pointer was null".to_owned());
    }

    Ok(host_ptr)
}

fn callback_value_internal_node_reference_token(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<u64> {
    const JS_MAX_SAFE_INTEGER_TOKEN: f64 = ((1_u64 << 53) - 1) as f64;

    let number = value.number_value(scope)?;
    if !number.is_finite()
        || number <= 0.0
        || number > JS_MAX_SAFE_INTEGER_TOKEN
        || number.fract() != 0.0
    {
        return None;
    }

    Some(number as u64)
}

fn callback_value_dom_handle(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    host: &JsContextHost,
    default_document: bool,
) -> Option<DomHandle> {
    if value.is_null_or_undefined() {
        return default_document.then(|| host.document_handle());
    }

    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(handle_value) = object.get_internal_field(scope, 1)
        && let Ok(handle_value) = v8::Local::<v8::Value>::try_from(handle_value)
        && let Some(handle_number) = handle_value.number_value(scope)
        && handle_number.is_finite()
        && handle_number.fract() == 0.0
        && handle_number > 0.0
        && let Some(handle) =
            host.resolve_node_wrapper_handle(ReflectorId::from_raw(handle_number as u64))
    {
        return Some(handle);
    }
    None
}

fn callback_arg_dom_handle(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
    host: &JsContextHost,
    default_document: bool,
) -> Option<DomHandle> {
    callback_value_dom_handle(scope, args.get(index), host, default_document)
}

fn callback_arg_usize(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> Option<usize> {
    let number = args.get(index).number_value(scope)?;
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 {
        return None;
    }
    Some(number as usize)
}

fn set_wrapped_handle_or_null(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    host_ptr: *mut JsContextHost,
    handle: Option<DomHandle>,
) {
    let Some(handle) = handle else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    match host
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
    {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

fn set_wrapped_handle_array(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    host_ptr: *mut JsContextHost,
    handles: &[DomHandle],
) {
    let mut values = Vec::with_capacity(handles.len());
    for handle in handles.iter().copied() {
        let host = unsafe { &mut *host_ptr };
        let Some(node) = host
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, handle)
        else {
            rv.set_null();
            return;
        };
        values.push(v8::Local::<v8::Value>::from(node));
    }
    let result =
        serialize_v8_array(scope, values.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(result.into());
}

fn host_resolve_child_window_by_handle_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let Some(handle_index) = callback_arg_usize(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let handle = DomHandle::new(handle_index);
    let window =
        if let Some(window) = host.existing_child_browsing_context_window_wrapper(scope, handle) {
            window
        } else if let Some(window) = host.child_browsing_context_window_wrapper(scope, handle) {
            window
        } else {
            rv.set_null();
            return;
        };
    rv.set(window.into());
}

fn host_child_frame_owner_backend_node_id_for_window_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(owner_node_id) = child_frame_owner_node_id_from_window(scope, args.get(0)) else {
        rv.set_null();
        return;
    };
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(backend_node_id) = host.renderer_backend_node_id_for_live_handle(owner_node_id) else {
        rv.set_null();
        return;
    };
    rv.set(v8::Number::new(scope, backend_node_id as f64).into());
}

fn child_frame_owner_node_id_from_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<NodeId> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let handle_index = get_private_value(scope, object, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)?
        .number_value(scope)?;
    if !handle_index.is_finite() || handle_index < 0.0 || handle_index.fract() != 0.0 {
        return None;
    }
    Some(NodeId::new(handle_index as usize))
}

fn host_auxiliary_popup_id_for_object_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if context_host_ptr_from_callback_data(scope, args.data()).is_err() {
        rv.set_null();
        return;
    }
    let Some(object) = args.get(0).to_object(scope) else {
        rv.set_null();
        return;
    };
    let Some(popup_id) = crate::native_bridge::renderer_owned_auxiliary_popup_id(scope, object)
    else {
        rv.set_null();
        return;
    };
    if let Some(value) = v8_string(scope, &popup_id.to_string()) {
        rv.set(value.into());
    } else {
        rv.set_null();
    }
}

fn host_bidi_window_remote_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    if args.get(0).strict_equals(global.into()) {
        let result = v8::Object::new(scope);
        let _ = result.set(
            scope,
            v8str(scope, "__moliBidiRemoteValue").into(),
            v8::Boolean::new(scope, true).into(),
        );
        let Some(window_type) = v8_string(scope, "window") else {
            rv.set_null();
            return;
        };
        let _ = result.set(scope, v8str(scope, "type").into(), window_type.into());
        let _ = result.set(
            scope,
            v8str(scope, "targetWindow").into(),
            v8::Boolean::new(scope, true).into(),
        );
        rv.set(result.into());
        return;
    }

    let Some(owner_node_id) = child_frame_owner_node_id_from_window(scope, args.get(0)) else {
        rv.set_null();
        return;
    };
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(frame_id) = host.child_browsing_context_frame_id_by_owner_node_id(owner_node_id)
    else {
        rv.set_null();
        return;
    };

    let result = v8::Object::new(scope);
    let _ = result.set(
        scope,
        v8str(scope, "__moliBidiRemoteValue").into(),
        v8::Boolean::new(scope, true).into(),
    );
    let Some(window_type) = v8_string(scope, "window") else {
        rv.set_null();
        return;
    };
    let _ = result.set(scope, v8str(scope, "type").into(), window_type.into());
    let Some(context) = v8_string(scope, &frame_id) else {
        rv.set_null();
        return;
    };
    let _ = result.set(scope, v8str(scope, "context").into(), context.into());
    rv.set(result.into());
}

fn host_resolve_internal_node_reference_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let handle = callback_value_internal_node_reference_token(scope, args.get(0))
        .and_then(|token| host.take_internal_node_reference(token));
    set_wrapped_handle_or_null(scope, &mut rv, host_ptr, handle);
}

fn host_resolve_internal_inspector_value_reference_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let value = callback_value_internal_node_reference_token(scope, args.get(0))
        .and_then(|token| host.take_internal_inspector_value_reference(scope, token));
    rv.set(value.unwrap_or_else(|| v8::null(scope).into()));
}

fn host_resolve_backend_node_id_for_object_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(handle) = callback_arg_dom_handle(scope, &args, 0, host, false) else {
        rv.set_null();
        return;
    };
    let Some(backend_node_id) = host.renderer_backend_node_id_for_live_handle(handle) else {
        rv.set_null();
        return;
    };
    rv.set(v8::Number::new(scope, backend_node_id as f64).into());
}

fn host_prepare_script_start_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(node) = callback_arg_dom_handle(scope, &args, 0, host, false) else {
        rv.set_undefined();
        return;
    };
    let Some(host_script_handle) = callback_arg_string(scope, &args, 1) else {
        rv.set_undefined();
        return;
    };
    match host.plan_and_commit_current_main_runtime_script_start(node, &host_script_handle) {
        Ok(Some(committed)) => {
            let (_, _, source) = committed.into_parts();
            if let Some(source) = v8_string(scope, &source) {
                rv.set(source.into());
            } else {
                rv.set_undefined();
            }
        }
        Ok(None) => rv.set_undefined(),
        Err(message) => {
            if let Some(message) = v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
        }
    }
}

fn host_dispatch_document_event_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(event_type) = callback_arg_string(scope, &args, 0) else {
        return;
    };
    if let Err(message) = host.dispatch_document_event(scope, host_ptr, &event_type) {
        // Host event dispatch paths already do local exception reporting around
        // JS handlers/listeners. Re-throwing here would turn the same failure into
        // a second uncaught V8 exception and duplicate stderr reporting.
        if !message.starts_with("event handler `") {
            error!(
                event_type,
                message = message.as_str(),
                "host dispatch document event failed"
            );
        }
    }
}

fn host_query_selector_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(selector) = callback_arg_string(scope, &args, 1) else {
        rv.set_null();
        return;
    };
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(root) = callback_arg_dom_handle(scope, &args, 0, host, true) else {
        rv.set_null();
        return;
    };

    match host.query_selector(Some(root), &selector) {
        Ok(handle) => set_wrapped_handle_or_null(scope, &mut rv, host_ptr, handle),
        Err(error) => throw_selector_error(scope, &error),
    }
}

fn host_query_selector_all_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(selector) = callback_arg_string(scope, &args, 1) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(root) = callback_arg_dom_handle(scope, &args, 0, host, true) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };

    match host.query_selector_all(Some(root), &selector) {
        Ok(handles) => set_wrapped_handle_array(scope, &mut rv, host_ptr, &handles),
        Err(error) => throw_selector_error(scope, &error),
    }
}

fn host_matches_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_bool(false);
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(node) = callback_arg_dom_handle(scope, &args, 0, host, false) else {
        rv.set_bool(false);
        return;
    };
    let Some(selector) = callback_arg_string(scope, &args, 1) else {
        rv.set_bool(false);
        return;
    };

    match host.matches(node, &selector) {
        Ok(is_match) => rv.set_bool(is_match),
        Err(error) => throw_selector_error(scope, &error),
    }
}

fn host_closest_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let host = &mut *unsafe { &mut *host_ptr };
    let Some(node) = callback_arg_dom_handle(scope, &args, 0, host, false) else {
        rv.set_null();
        return;
    };
    let Some(selector) = callback_arg_string(scope, &args, 1) else {
        rv.set_null();
        return;
    };

    match host.closest(node, &selector) {
        Ok(handle) => set_wrapped_handle_or_null(scope, &mut rv, host_ptr, handle),
        Err(error) => throw_selector_error(scope, &error),
    }
}

fn host_selector_debug_stats_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(host_ptr) = context_host_ptr_from_callback_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let snapshot = unsafe { &*host_ptr }.selector_debug_snapshot();
    let result = SelectorDebugStatsDeclaration {
        query_selector: snapshot.query_selector,
        query_selector_all: snapshot.query_selector_all,
        matches: snapshot.matches,
        closest: snapshot.closest,
    }
    .bind(scope)
    .map(v8::Local::<v8::Value>::from)
    .unwrap_or_else(|_| v8::null(scope).into());
    rv.set(result);
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct SelectorDebugStatsDeclaration {
    query_selector: u32,
    query_selector_all: u32,
    matches: u32,
    closest: u32,
}

fn throw_selector_error(scope: &mut v8::PinScope<'_, '_>, error: &crate::selector::SelectorError) {
    let Some(message) = v8_string(scope, error.message()) else {
        return;
    };
    let exception = v8::Exception::error(scope, message);
    if let Some(object) = exception.to_object(scope) {
        let name_key = v8str(scope, "name");
        let name_value = v8str(scope, "SyntaxError");
        let code_key = v8str(scope, "code");
        let _ = object.set(scope, name_key.into(), name_value.into());
        let _ = object.set(
            scope,
            code_key.into(),
            v8::Number::new(scope, f64::from(error.code())).into(),
        );
    }
    scope.throw_exception(exception);
}
