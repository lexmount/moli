use std::ffi::c_void;

use moli_webapi_declare::{BindError, WebApiObject};

use crate::{
    frame_owner_model::LocalWindowId,
    native_bridge::{
        JsContextHost, RuntimeBindingExecutionContext, RuntimeObservableContextToken,
        current_runtime_observable_context_token,
    },
    protocol_types::PendingRuntimeBindingCall,
    util::v8str,
};

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct RuntimeBindingDataDeclaration<'scope> {
    host: v8::Local<'scope, v8::External>,
    name: v8::Local<'scope, v8::String>,
    execution_context_id: f64,
    local_window_id: v8::Local<'scope, v8::BigInt>,
    runtime_context_token: v8::Local<'scope, v8::BigInt>,
}

pub(crate) fn build_runtime_binding_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut c_void,
    name: v8::Local<'s, v8::String>,
    execution_context_id: i64,
    execution_context: RuntimeBindingExecutionContext,
) -> Result<v8::Local<'s, v8::Object>, BindError> {
    RuntimeBindingDataDeclaration::new(
        v8::External::new(scope, host_ptr),
        name,
        execution_context_id as f64,
        v8::BigInt::new_from_u64(scope, execution_context.local_window_id().0),
        v8::BigInt::new_from_u64(scope, execution_context.context_token().as_u64()),
    )
    .bind(scope)
}

pub(crate) fn runtime_binding_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if crate::util::page_context_is_disconnected(scope.get_current_context())
        || data
            .get_creation_context(scope)
            .is_some_and(crate::util::page_context_is_disconnected)
    {
        return;
    }
    let Some(host_value) = data.get(scope, v8str(scope, "host").into()) else {
        return;
    };
    let Ok(host_external) = v8::Local::<v8::External>::try_from(host_value) else {
        return;
    };
    let host_ptr = host_external.value() as *mut JsContextHost;
    if host_ptr.is_null() {
        return;
    }

    let Some(local_window_id) = runtime_binding_data_u64(scope, data, "localWindowId") else {
        return;
    };
    let Some(context_token) = runtime_binding_data_u64(scope, data, "runtimeContextToken") else {
        return;
    };
    let execution_context = RuntimeBindingExecutionContext::new(
        LocalWindowId(local_window_id),
        RuntimeObservableContextToken::from_raw(context_token),
    );
    if current_runtime_observable_context_token(scope) != Some(execution_context.context_token()) {
        tracing::debug!(
            ?execution_context,
            actual_context_token = ?current_runtime_observable_context_token(scope),
            "ignored Runtime binding call from mismatched V8 context"
        );
        return;
    }

    let name = data
        .get(scope, v8str(scope, "name").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let execution_context_id = data
        .get(scope, v8str(scope, "executionContextId").into())
        .and_then(|value| value.integer_value(scope))
        .unwrap_or_default();
    let payload = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();

    let call = PendingRuntimeBindingCall {
        source: execution_context.binding_call_source_identity(),
        name,
        payload,
        execution_context_id,
    };
    // SAFETY: every binding function is owned by the V8 context whose bridge
    // keeps this host alive; the callback does not retain the pointer.
    if !unsafe { &mut *host_ptr }.record_runtime_binding_call(execution_context, call) {
        tracing::debug!(
            ?execution_context,
            execution_context_id,
            "ignored Runtime binding call for retired execution context"
        );
    }
}

fn runtime_binding_data_u64(
    scope: &mut v8::PinScope<'_, '_>,
    data: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> Option<u64> {
    let value = data.get(scope, v8str(scope, name).into())?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = value.u64_value();
    lossless.then_some(value)
}
