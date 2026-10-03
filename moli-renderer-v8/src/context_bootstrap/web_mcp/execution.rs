use super::bindings::{document_frame_tree, document_owner, dom_error, js_string};
use super::devtools::document_frame_id;
use super::events::dispatch_event;
use super::state::{
    AbortRegistration, CallerResult, FormInvocationState, PendingDelivery, PendingInvocation,
    ToolExecutor,
};
use super::tasks::{invocation_context, queue_task, remove_abort_registration, signal_reason};
use super::{declarative, devtools};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use moli_page_types::{
    RendererWebMcpEvent, RendererWebMcpObservation, RendererWebMcpResult, RendererWebMcpToolId,
};

use crate::{
    abort_signal_route::ResolvedAbortSignal,
    window_webidl_callback::PreparedWindowWebIdlCallbackFunction,
    window_webidl_callback::PreparedWindowWebIdlCallbackFunctionOutcome,
};

fn next_invocation_id() -> u64 {
    static NEXT_INVOCATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_INVOCATION
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |id| id.checked_add(1),
        )
        .expect("WebMCP invocation space exhausted")
}

fn register_caller_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: u64,
    signal: Option<v8::Local<'s, v8::Object>>,
) -> Option<AbortRegistration> {
    signal.map(|signal| {
        let data = v8::BigInt::new_from_u64(scope, id);
        let algorithm = v8::Function::builder(caller_abort_callback)
            .data(data.into())
            .build(scope)
            .expect("WebMCP invocation abort algorithm");
        ResolvedAbortSignal::resolve(scope, signal)
            .expect("validated AbortSignal")
            .register_algorithm(scope, algorithm);
        AbortRegistration {
            signal: v8::Global::new(scope, signal),
            algorithm: v8::Global::new(scope, algorithm),
        }
    })
}

fn queue_delivery(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
    delivery: PendingDelivery,
) {
    let resolver = v8::Local::new(scope, &delivery.resolver);
    let Some(context) = resolver.get_promise(scope).get_creation_context(scope) else {
        remove_abort_registration(scope, delivery.caller_abort);
        return;
    };
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .deliveries
        .insert(id, delivery);
    let scope = &mut v8::ContextScope::new(scope, context);
    let data = v8::BigInt::new_from_u64(scope, id);
    let callback = v8::Function::builder(delivery_callback)
        .data(data.into())
        .build(scope)
        .expect("WebMCP caller result task");
    queue_task(scope, host_ptr, callback);
}

fn complete_for_caller(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
    pending: &mut PendingInvocation,
    result: CallerResult,
) {
    declarative::invocation::finish_activity(unsafe { &mut *host_ptr }, pending);
    if let Some(resolver) = pending.resolver.take() {
        queue_delivery(
            scope,
            host_ptr,
            id,
            PendingDelivery {
                caller_owner: pending.caller_owner,
                caller_document: pending.caller_document,
                resolver,
                caller_abort: pending.caller_abort.take(),
                result,
            },
        );
    } else {
        remove_abort_registration(scope, pending.caller_abort.take());
    }
}

pub(super) fn reject_unavailable_tool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    caller_document: DomHandle,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    signal: Option<v8::Local<'s, v8::Object>>,
) {
    let id = next_invocation_id();
    let caller_abort = register_caller_abort(scope, id, signal);
    queue_delivery(
        scope,
        host_ptr,
        id,
        PendingDelivery {
            caller_owner: document_owner(unsafe { &*host_ptr }, caller_document)
                .expect("active caller"),
            caller_document,
            resolver: v8::Global::new(scope, resolver),
            caller_abort,
            result: CallerResult::Error(
                "Tool invocation failed: tool or target document is no longer available.".into(),
            ),
        },
    );
}

fn delivery_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((host_ptr, id)) = invocation_context(scope, &args) else {
        return;
    };
    let Some(delivery) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .deliveries
        .remove(&id)
    else {
        return;
    };
    remove_abort_registration(scope, delivery.caller_abort);
    if document_owner(unsafe { &*host_ptr }, delivery.caller_document)
        != Some(delivery.caller_owner)
    {
        return;
    }
    let resolver = v8::Local::new(scope, &delivery.resolver);
    match delivery.result {
        CallerResult::Completed(value) => {
            let _ = resolver.resolve(scope, js_string(scope, &value).into());
        }
        CallerResult::Navigated => {
            let _ = resolver.resolve(scope, v8::null(scope).into());
        }
        CallerResult::Error(message) => {
            let error = dom_error(scope, "UnknownError", &message);
            let _ = resolver.reject(scope, error);
        }
    }
}

// Each invocation owns its signal and resolver independently of the tool
// registration. Neither an unregister nor a late callback settlement can
// complete a different invocation of the same tool.
#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_invocation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    caller_document: DomHandle,
    document: DomHandle,
    target: v8::Global<v8::Object>,
    name: String,
    input: String,
    resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    caller_signal: Option<v8::Local<'s, v8::Object>>,
) -> Option<u64> {
    let target_context = v8::Local::new(scope, &target).get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, target_context);
    let signal =
        crate::native_bridge::abort::create_signal(scope, unsafe { &mut *host_ptr }, false, None)?;
    let id = next_invocation_id();
    let data = v8::BigInt::new_from_u64(scope, id);
    let caller_abort = register_caller_abort(scope, id, caller_signal);
    let owner = document_owner(unsafe { &*host_ptr }, document).expect("active tool document");
    let caller_owner =
        document_owner(unsafe { &*host_ptr }, caller_document).expect("active caller document");
    let frame_tree = document_frame_tree(unsafe { &*host_ptr }, document);
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .pending
        .insert(
            id,
            PendingInvocation {
                document,
                owner,
                caller_owner,
                caller_document,
                frame_tree,
                name,
                target,
                resolver: resolver.map(|resolver| v8::Global::new(scope, resolver)),
                signal: v8::Global::new(scope, signal),
                caller_abort,
                input,
                caller_cancel_requested: false,
                form: None,
            },
        );
    let start = v8::Function::builder(start_callback)
        .data(data.into())
        .build(scope)
        .expect("WebMCP invocation task");
    queue_task(scope, host_ptr, start);
    Some(id)
}

fn start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((host_ptr, id)) = invocation_context(scope, &args) else {
        return;
    };
    let invocation = {
        let host = unsafe { &*host_ptr };
        let Some(pending) = host.native_bridge().web_mcp.pending.get(&id) else {
            return;
        };
        let executor = host
            .native_bridge()
            .web_mcp
            .documents
            .get(&pending.document)
            .filter(|entry| {
                entry.owner == pending.owner
                    && document_owner(host, pending.document) == Some(entry.owner)
            })
            .and_then(|entry| entry.tools.get(&pending.name))
            .map(|tool| match &tool.executor {
                ToolExecutor::Callback(callback) => {
                    InvocationExecutor::Callback(callback.prepare(scope))
                }
                ToolExecutor::Form {
                    handle, autosubmit, ..
                } => InvocationExecutor::Form(*handle, *autosubmit),
            });
        (
            executor,
            pending.document,
            pending.input.clone(),
            pending.name.clone(),
            v8::Local::new(scope, &pending.signal),
            v8::Local::new(scope, &pending.target),
            pending.frame_tree,
        )
    };
    let (Some(executor), document, input, name, signal, target, tree) = invocation else {
        finish_error(scope, host_ptr, id, "Tool is no longer registered");
        return;
    };
    devtools::emit_in_tree(
        unsafe { &*host_ptr },
        tree,
        RendererWebMcpEvent::ToolInvoked {
            tool: RendererWebMcpToolId {
                frame_id: document_frame_id(unsafe { &*host_ptr }, document),
                name: name.clone(),
            },
            invocation_id: id,
            input: input.clone(),
        },
    );
    let callback = match executor {
        InvocationExecutor::Callback(callback) => callback,
        InvocationExecutor::Form(form, autosubmit) => {
            let owner = document_owner(unsafe { &*host_ptr }, document);
            declarative::invocation::start(scope, host_ptr, id, form, autosubmit, &input);
            if owner.is_some() && document_owner(unsafe { &*host_ptr }, document) == owner {
                dispatch_event(scope, target, "toolactivated", Some(&name));
            }
            return;
        }
    };
    let callback_unavailable = "Tool callback is no longer available";
    let result = callback.invoke(
        scope,
        unsafe { &*host_ptr },
        v8::undefined(scope).into(),
        &[],
        |scope, callback, receiver, _arguments| {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let mut scope = try_catch.init();
            // Invoke enters the callback's relevant realm before allocating
            // input (including nested objects/arrays) and its options object.
            let input = v8::json::parse(&scope, js_string(&scope, &input))
                .filter(|input| input.is_object())
                .ok_or(CallbackInvocationError::InvalidInput)?;
            let options = v8::Object::new(&scope);
            let _ = options.create_data_property(
                &scope,
                js_string(&scope, "signal").into(),
                signal.into(),
            );
            let resolver = v8::PromiseResolver::new(&scope)
                .ok_or(CallbackInvocationError::CallbackUnavailable)?;
            match callback.call(&scope, receiver, &[input, options.into()]) {
                Some(value) => {
                    resolver
                        .resolve(&scope, value)
                        .ok_or(CallbackInvocationError::CallbackUnavailable)?;
                }
                None => {
                    let exception = scope
                        .exception()
                        .unwrap_or_else(|| v8::undefined(&scope).into());
                    scope.reset();
                    resolver
                        .reject(&scope, exception)
                        .ok_or(CallbackInvocationError::CallbackUnavailable)?;
                }
            }
            Ok(v8::Global::new(&scope, resolver.get_promise(&scope)))
        },
    );
    // Chromium activates a live tool even when parsing its input fails. Keep
    // that failure distinct from a retired or otherwise unavailable callback,
    // and retain the owner before finish_error removes the pending invocation.
    let activation_owner = match result {
        PreparedWindowWebIdlCallbackFunctionOutcome::Returned(promise) => {
            let promise = v8::Local::new(scope, &promise);
            await_response(scope, host_ptr, id, promise);
            unsafe { &*host_ptr }
                .native_bridge()
                .web_mcp
                .pending
                .get(&id)
                .map(|pending| pending.owner)
        }
        PreparedWindowWebIdlCallbackFunctionOutcome::Failed(
            CallbackInvocationError::InvalidInput,
        ) => {
            let owner = unsafe { &*host_ptr }
                .native_bridge()
                .web_mcp
                .pending
                .get(&id)
                .map(|pending| pending.owner);
            finish_error(scope, host_ptr, id, "Invalid tool input");
            owner
        }
        PreparedWindowWebIdlCallbackFunctionOutcome::Failed(
            CallbackInvocationError::CallbackUnavailable,
        )
        | PreparedWindowWebIdlCallbackFunctionOutcome::Retired => {
            finish_error(scope, host_ptr, id, callback_unavailable);
            None
        }
    };
    if activation_owner.is_some()
        && document_owner(unsafe { &*host_ptr }, document) == activation_owner
    {
        dispatch_event(scope, target, "toolactivated", Some(&name));
    }
}

enum CallbackInvocationError {
    InvalidInput,
    CallbackUnavailable,
}

enum InvocationExecutor {
    Callback(PreparedWindowWebIdlCallbackFunction),
    Form(DomHandle, bool),
}

pub(super) fn await_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
    promise: v8::Local<'s, v8::Promise>,
) {
    if !unsafe { &*host_ptr }
        .native_bridge()
        .web_mcp
        .pending
        .contains_key(&id)
    {
        return;
    }
    let data = v8::BigInt::new_from_u64(scope, id);
    let fulfilled = v8::Function::builder(fulfilled_callback)
        .data(data.into())
        .build(scope)
        .expect("WebMCP fulfillment callback");
    let rejected = v8::Function::builder(rejected_callback)
        .data(data.into())
        .build(scope)
        .expect("WebMCP rejection callback");
    let _ = promise.then2(scope, fulfilled, rejected);
}

fn caller_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((host_ptr, id)) = invocation_context(scope, &args) else {
        return;
    };
    let delivery = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .deliveries
        .remove(&id);
    if let Some(delivery) = delivery {
        let reason = delivery
            .caller_abort
            .as_ref()
            .map(|abort| signal_reason(scope, v8::Local::new(scope, &abort.signal)))
            .unwrap_or_else(|| crate::native_bridge::abort::abort_error_value(scope));
        remove_abort_registration(scope, delivery.caller_abort);
        let resolver = v8::Local::new(scope, &delivery.resolver);
        let _ = resolver.reject(scope, reason);
        return;
    }
    let rejection = {
        let Some(pending) = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .web_mcp
            .pending
            .get_mut(&id)
        else {
            return;
        };
        if pending.caller_cancel_requested {
            return;
        }
        pending.caller_cancel_requested = true;
        (
            v8::Local::new(
                scope,
                &pending.resolver.take().expect("page caller resolver"),
            ),
            pending
                .caller_abort
                .as_ref()
                .map(|abort| v8::Local::new(scope, &abort.signal)),
        )
    };
    let (resolver, signal) = rejection;
    let reason = signal
        .map(|signal| signal_reason(scope, signal))
        .unwrap_or_else(|| crate::native_bridge::abort::abort_error_value(scope));
    let _ = resolver.reject(scope, reason);
    // The caller's rejection reaction precedes the target signal's abort event.
    // An already posted invocation still starts before this cancellation task.
    let cancel = v8::Function::builder(cancel_callback)
        .data(args.data())
        .build(scope)
        .expect("WebMCP cancellation task");
    queue_task(scope, host_ptr, cancel);
}

fn cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((host_ptr, id)) = invocation_context(scope, &args) else {
        return;
    };
    cancel_invocation(scope, host_ptr, id);
}

pub(super) fn cancel_invocation(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
) -> bool {
    // Take before dispatching author abort callbacks: late settlements and
    // reentrant cancellation must see an already completed invocation.
    let Some(mut pending) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .pending
        .remove(&id)
    else {
        return false;
    };
    cleanup_invocation(scope, host_ptr, &mut pending);
    devtools::emit_in_tree(
        unsafe { &*host_ptr },
        pending.frame_tree,
        RendererWebMcpEvent::ToolResponded {
            invocation_id: id,
            result: RendererWebMcpResult::Canceled,
        },
    );
    if let Some(resolver) = pending.resolver.take() {
        let resolver = v8::Local::new(scope, &resolver);
        let reason = crate::native_bridge::abort::abort_error_value(scope);
        let _ = resolver.reject(scope, reason);
    }
    abort_target(scope, host_ptr, &pending);
    true
}

pub(super) fn abort_target(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    pending: &PendingInvocation,
) {
    let signal = v8::Local::new(scope, &pending.signal);
    let reason = crate::native_bridge::abort::abort_error_value(scope);
    crate::native_bridge::abort::abort_signal(scope, signal, reason);
    if document_owner(unsafe { &*host_ptr }, pending.document) == Some(pending.owner) {
        dispatch_event(
            scope,
            v8::Local::new(scope, &pending.target),
            "toolcancel",
            Some(&pending.name),
        );
    }
}

fn fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((host_ptr, id)) = invocation_context(scope, &args) else {
        return;
    };
    let invalidated = {
        let Some(pending) = unsafe { &*host_ptr }
            .native_bridge()
            .web_mcp
            .pending
            .get(&id)
        else {
            return;
        };
        pending
            .form
            .as_ref()
            .is_some_and(|form| matches!(form.state, FormInvocationState::Invalidated))
    };
    if invalidated {
        finish_error(
            scope,
            host_ptr,
            id,
            "Form tool was removed before its response completed",
        );
        return;
    }
    let (output, exception) = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let value = args.get(0);
        let output = if value.is_object() {
            v8::json::stringify(&scope, value)
        } else {
            value.to_string(&scope)
        };
        let output = output.map(|value| value.to_rust_string_lossy(&scope));
        let exception = scope.exception();
        scope.reset();
        (output, exception)
    };
    let Some(output) = output else {
        let imperative = unsafe { &*host_ptr }
            .native_bridge()
            .web_mcp
            .pending
            .get(&id)
            .is_some_and(|pending| pending.form.is_none());
        finish_failure(
            scope,
            host_ptr,
            id,
            "Failed to serialize tool result",
            exception.filter(|_| imperative),
        );
        return;
    };
    // Serializing may invoke toJSON; it may have canceled or retired this call.
    let Some(mut pending) = take_active_invocation(unsafe { &mut *host_ptr }, id) else {
        return;
    };
    let output = if output.is_empty() {
        "Operation succeeded"
    } else {
        &output
    };
    devtools::emit_in_tree(
        unsafe { &*host_ptr },
        pending.frame_tree,
        RendererWebMcpEvent::ToolResponded {
            invocation_id: id,
            result: RendererWebMcpResult::Completed(output.into()),
        },
    );
    complete_for_caller(
        scope,
        host_ptr,
        id,
        &mut pending,
        CallerResult::Completed(output.into()),
    );
}

fn rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((host_ptr, id)) = invocation_context(scope, &args) else {
        return;
    };
    let declarative = unsafe { &*host_ptr }
        .native_bridge()
        .web_mcp
        .pending
        .get(&id)
        .is_some_and(|pending| pending.form.is_some());
    // Do not call author toString while reporting an arbitrary rejection value.
    finish_failure(
        scope,
        host_ptr,
        id,
        "Tool callback failed",
        (!declarative).then(|| args.get(0)),
    );
}

pub(super) fn finish_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
) {
    let Some(mut pending) = take_active_invocation(unsafe { &mut *host_ptr }, id) else {
        return;
    };
    if !pending
        .form
        .as_ref()
        .is_some_and(|form| matches!(form.state, FormInvocationState::Navigating))
    {
        devtools::emit_in_tree(
            unsafe { &*host_ptr },
            pending.frame_tree,
            RendererWebMcpEvent::ToolResponded {
                invocation_id: id,
                result: RendererWebMcpResult::Completed("null".into()),
            },
        );
    }
    complete_for_caller(scope, host_ptr, id, &mut pending, CallerResult::Navigated);
}

pub(super) fn finish_error(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
    message: &str,
) {
    finish_failure(scope, host_ptr, id, message, None);
}

fn finish_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    id: u64,
    message: &str,
    exception: Option<v8::Local<'s, v8::Value>>,
) {
    let Some(mut pending) = take_active_invocation(unsafe { &mut *host_ptr }, id) else {
        return;
    };
    let (sessions, wrapper) = {
        let store = &unsafe { &*host_ptr }.native_bridge().web_mcp;
        (
            store
                .enabled_sessions
                .iter()
                .filter(|_| pending.frame_tree.is_none())
                .cloned()
                .collect::<Vec<_>>(),
            store.object_wrapper.clone(),
        )
    };
    for session in sessions {
        let exception = exception.and_then(|value| wrapper.as_ref()?.wrap(scope, &session, value));
        let message = if exception.is_some() {
            String::new()
        } else {
            message.into()
        };
        unsafe { &*host_ptr }.append_live_turn_observation(
            crate::runtime::RendererProtocolObservation::WebMcp(RendererWebMcpObservation {
                session,
                event: RendererWebMcpEvent::ToolResponded {
                    invocation_id: id,
                    result: RendererWebMcpResult::Error { message, exception },
                },
            }),
        );
    }
    complete_for_caller(
        scope,
        host_ptr,
        id,
        &mut pending,
        CallerResult::Error(message.into()),
    );
}

pub(super) fn cleanup_invocation(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    pending: &mut PendingInvocation,
) {
    declarative::invocation::finish_activity(unsafe { &mut *host_ptr }, pending);
    remove_abort_registration(scope, pending.caller_abort.take());
}

// The target owns its pending record until completion or its cancellation task.
// A caller rejection does not prevent an earlier target reaction from finishing.
fn take_active_invocation(host: &mut JsContextHost, id: u64) -> Option<PendingInvocation> {
    host.native_bridge_mut().web_mcp.pending.remove(&id)
}
