mod paths;
mod request;
mod service_worker;

use self::paths::{
    record_intercepted_fetch, reject_bad_port_fetch, reject_blocked_fetch, reject_csp_fetch,
    reject_offline_fetch, reject_url_policy_fetch, resolve_local_fetch, spawn_network_fetch,
};
use self::request::prepare_window_fetch_request;
use self::service_worker::dispatch_service_worker_fetch;
use super::input::{ParsedWindowFetchInput, parse_window_fetch_input};
use super::promise::{make_rejected_promise, make_rejected_promise_with_value};
use super::*;
use crate::native_bridge::abort::abort_error_value;
use crate::structured_clone::V8StructuredClonePayload;
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct FetchAbortListenerOptionsDeclaration {
    #[webapi(init = true)]
    once: (),
}

fn window_fetch_abort_listener_id(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<u64> {
    if let Ok(value) = v8::Local::<v8::BigInt>::try_from(value) {
        let (internal_id, lossless) = value.u64_value();
        return lossless.then_some(internal_id);
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 1.0 && value.fract() == 0.0)
        .map(|value| value as u64)
}

fn window_fetch_abort_signal_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(internal_id) = window_fetch_abort_listener_id(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let reason = Some(args.this())
        .and_then(|signal| host.abort_signal_reason(scope, signal))
        .unwrap_or_else(|| abort_error_value(scope));
    let reason_payload = structured_serialize_fetch_abort_reason(scope, reason);
    let _ = host.abort_fetch_promise(scope, internal_id, reason, reason_payload);
    rv.set_undefined();
}

fn structured_serialize_fetch_abort_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<V8StructuredClonePayload> {
    let clone_value = error_name_message_snapshot(scope, reason).unwrap_or(reason);
    crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        clone_value,
        None,
        "AbortSignal",
    )
}

fn error_name_message_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let error_constructor = crate::util::registered_intrinsic_constructor(scope, global, "Error")?;
    if !value.instance_of(scope, error_constructor).unwrap_or(false) {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let snapshot = ObjectLiteralDeclaration::bind(scope);
    for key in ["name", "message"] {
        let key_value = v8str(scope, key);
        if let Some(value) = object.get(scope, key_value.into()) {
            snapshot.set_value_property(scope, key_value.into(), value);
        }
    }
    Some(snapshot.into_value())
}

pub(crate) fn window_fetch_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(accessing_host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(make_rejected_promise(scope, "failed to get native bridge").into());
        return;
    };
    let receiver_host_ptr = crate::util::context_host_ptr_from_window_object(scope, args.this())
        .unwrap_or(accessing_host_ptr);

    let receiver = match crate::native_bridge::WindowOperationReceiver::capture_and_authorize(
        scope,
        args.this(),
        unsafe { &*receiver_host_ptr },
        unsafe { &*accessing_host_ptr },
    ) {
        Ok(receiver) => receiver,
        Err(crate::native_bridge::WindowOperationReceiverCaptureError::IllegalInvocation) => {
            // Promise-returning WebIDL operations convert callback-time
            // TypeErrors into rejected promises in the current realm. Blink
            // installs ExceptionToRejectPromiseScope before the brand check.
            rv.set(
                make_rejected_promise(
                    scope,
                    "Failed to execute 'fetch' on 'Window': Illegal invocation",
                )
                .into(),
            );
            return;
        }
        Err(crate::native_bridge::WindowOperationReceiverCaptureError::CrossOrigin) => {
            // V8's WindowProxy access check rejects a cross-origin receiver
            // before the operation can switch to its relevant realm.
            crate::native_bridge::throw_cross_origin_location_security_error(scope);
            return;
        }
    };

    if args.length() < 1 {
        // This validation precedes ScriptState::ForRelevantRealm in Blink's
        // generated binding. The rejection therefore belongs to the current
        // function realm, even when a different same-origin Window is used as
        // `this`. Argument conversion and the native Fetch implementation run
        // only after this pre-IDL boundary.
        rv.set(
            make_rejected_promise(
                scope,
                &crate::webidl::WebIdlError::missing_required(crate::webidl::Context::argument(
                    "fetch", 1,
                ))
                .to_string(),
            )
            .into(),
        );
        return;
    }

    // Generated Blink bindings convert RequestInfo and RequestInit while the
    // caller's realm is still current. Besides choosing the right realm for
    // conversion failures, this permits getters to run; the frozen receiver
    // is revalidated only after all such author code has completed.
    let mut parsed = match parse_window_fetch_input(scope, &args) {
        Ok(parsed) => parsed,
        Err(message) => {
            rv.set(make_rejected_promise(scope, &message).into());
            return;
        }
    };
    let signal_value = window_fetch_signal_value(scope, &args);
    let signal = match signal_value {
        Some(value) => {
            match validate_window_fetch_signal(scope, unsafe { &mut *accessing_host_ptr }, value) {
                Ok(signal) => signal,
                Err(message) => {
                    rv.set(make_rejected_promise(scope, &message).into());
                    return;
                }
            }
        }
        None => None,
    };

    let Some(binding) = receiver.resolve_live_binding(unsafe { &*receiver_host_ptr }) else {
        // This also catches the subtle generation race where a RequestInit
        // getter navigated the iframe. Never look up the replacement
        // LocalWindow by the stable child handle.
        rv.set(
            make_rejected_promise(
                scope,
                "Failed to execute 'fetch' on 'Window': The global scope is shutting down.",
            )
            .into(),
        );
        return;
    };
    if let Some(request_body_owner) = parsed.request_body_owner.take() {
        let request_body_owner = v8::Local::new(scope, request_body_owner);
        mark_request_input_body_used_for_fetch(scope, request_body_owner);
    }
    let fetch_context = crate::native_bridge::WindowFetchContext::from_realm(binding);
    let signal = signal.map(|signal| v8::Global::new(scope, signal));
    let relevant_context = {
        let context = fetch_context.script_realm().context(scope);
        v8::Global::new(scope, context)
    };
    let relevant_context = v8::Local::new(scope, &relevant_context);
    let scope = &mut v8::ContextScope::new(scope, relevant_context);
    window_fetch_callback_in_relevant_realm(scope, parsed, signal, fetch_context, rv);
}

fn window_fetch_callback_in_relevant_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: ParsedWindowFetchInput,
    signal: Option<v8::Global<v8::Object>>,
    fetch_context: crate::native_bridge::WindowFetchContext,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(make_rejected_promise(scope, "failed to get native bridge").into());
        return;
    };

    let host = unsafe { &mut *host_ptr };
    let signal = signal.as_ref().map(|signal| v8::Local::new(scope, signal));
    let prepared = match prepare_window_fetch_request(scope, parsed, fetch_context, host) {
        Ok(prepared) => prepared,
        Err(message) => {
            rv.set(make_rejected_promise(scope, &message).into());
            return;
        }
    };
    host.break_on_dom_debugger_xhr_or_fetch_network_request(prepared.resolved_url.as_str());
    if let Some(signal) = signal
        && host.abort_signal_aborted(scope, signal)
    {
        let reason = host
            .abort_signal_reason(scope, signal)
            .unwrap_or_else(|| abort_error_value(scope));
        rv.set(make_rejected_promise_with_value(scope, reason).into());
        return;
    }

    let owner = prepared.request_scope();
    if let Some(violation) = host
        .check_document_connect_csp_for_owner(
            scope,
            owner,
            &prepared.document_url,
            &prepared.resolved_url,
        )
        .into_blocking_violation()
    {
        let message = crate::document_runtime::document_content_security_policy_error_message(
            &violation, "fetch",
        );
        let message = reject_csp_fetch(host, prepared, message);
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    if let Err(error) = moli_url_policy::route_fetch_url(&prepared.resolved_url) {
        let message = reject_url_policy_fetch(host, prepared, error.to_string());
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    if moli_fetch::should_request_be_blocked_due_to_bad_port(&prepared.resolved_url) {
        let message = reject_bad_port_fetch(host, prepared);
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);

    if host.is_url_blocked(&prepared.resolved_url) {
        let message = reject_blocked_fetch(host, prepared);
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    if host.should_intercept_subresource(SubresourceResourceType::Fetch) {
        record_intercepted_fetch(scope, host, resolver, prepared);
        rv.set(promise.into());
        return;
    }

    if host.network_offline() {
        let message = reject_offline_fetch(host, prepared);
        rv.set(make_rejected_promise(scope, &message).into());
        return;
    }

    match dispatch_service_worker_fetch(scope, host, resolver, &prepared) {
        Ok(Some(internal_id)) => {
            if let Some(signal) = signal {
                install_window_fetch_abort_listener(scope, signal, internal_id);
            }
            rv.set(promise.into());
            return;
        }
        Ok(None) => {}
        Err(message) => {
            rv.set(make_rejected_promise(scope, &message).into());
            return;
        }
    }

    match resolve_local_fetch(host, &prepared) {
        Ok(Some((document_url, response))) => {
            let response_obj = build_fetch_response_object_for_request_mode(
                scope,
                &document_url,
                prepared.request_mode,
                response,
            );
            resolver.resolve(scope, response_obj.into());
            rv.set(promise.into());
            return;
        }
        Ok(None) => {}
        Err(message) => {
            let exception = v8_string(scope, &message)
                .map(|message| v8::Exception::type_error(scope, message))
                .unwrap_or_else(|| v8::undefined(scope).into());
            resolver.reject(scope, exception);
            rv.set(promise.into());
            return;
        }
    }

    let internal_id = match spawn_network_fetch(scope, host, resolver, prepared) {
        Ok(internal_id) => internal_id,
        Err(message) => {
            rv.set(make_rejected_promise(scope, &message).into());
            return;
        }
    };
    if let Some(signal) = signal {
        install_window_fetch_abort_listener(scope, signal, internal_id);
    }
    rv.set(promise.into());
}

fn window_fetch_signal_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Value>> {
    let signal_key = v8str(scope, "signal");
    if args.length() > 1 {
        let init_arg = args.get(1);
        if !init_arg.is_null_or_undefined()
            && let Ok(init) = v8::Local::<v8::Object>::try_from(init_arg)
            && init.has(scope, signal_key.into()).unwrap_or(false)
        {
            let value = init
                .get(scope, signal_key.into())
                .unwrap_or_else(|| v8::undefined(scope).into());
            return Some(value);
        }
    }

    let request_like = args.get(0);
    if !request_like.is_null_or_undefined()
        && request_like.is_object()
        && let Ok(request_like) = v8::Local::<v8::Object>::try_from(request_like)
        && let Some(value) = request_like.get(scope, signal_key.into())
    {
        return Some(value);
    }

    None
}

fn validate_window_fetch_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<v8::Local<'s, v8::Object>>, String> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let Ok(signal) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(
            "Failed to execute 'fetch' on 'Window': signal must be an AbortSignal.".to_owned(),
        );
    };
    if !host.is_abort_signal(scope, signal) {
        return Err(
            "Failed to execute 'fetch' on 'Window': signal must be an AbortSignal.".to_owned(),
        );
    }
    Ok(Some(signal))
}

fn install_window_fetch_abort_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    internal_id: u64,
) {
    let Some(listener) = v8::Function::builder(window_fetch_abort_signal_callback)
        .data(v8::BigInt::new_from_u64(scope, internal_id).into())
        .length(1)
        .build(scope)
    else {
        return;
    };
    let Some(add_event_listener) = signal
        .get(scope, v8str(scope, "addEventListener").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let options = FetchAbortListenerOptionsDeclaration::default()
        .bind(scope)
        .expect("fetch abort listener options declaration should bind");
    let _ = add_event_listener.call(
        scope,
        signal.into(),
        &[
            v8str(scope, "abort").into(),
            listener.into(),
            options.into(),
        ],
    );
}
