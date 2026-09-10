use super::type_set::new_view_transition_type_set;
use super::*;
use crate::{
    util::{get_private_value, set_private_value},
    window_webidl_callback::{
        PreparedWindowWebIdlCallbackFunction, PreparedWindowWebIdlCallbackFunctionOutcome,
    },
};
use moli_webapi_declare::WebApiObject;

const DOCUMENT_ACTIVE_VIEW_TRANSITION_SLOT: &str = "__lmDocumentActiveViewTransition";
const VIEW_TRANSITION_DOCUMENT_SLOT: &str = "__lmViewTransitionDocument";
const VIEW_TRANSITION_CALLBACK_STATE_SLOT: &str = "__lmViewTransitionCallbackState";
const VIEW_TRANSITION_READY_SLOT: &str = "__lmViewTransitionReady";
const VIEW_TRANSITION_READY_RESOLVER_SLOT: &str = "__lmViewTransitionReadyResolver";
const VIEW_TRANSITION_FINISHED_SLOT: &str = "__lmViewTransitionFinished";
const VIEW_TRANSITION_FINISHED_RESOLVER_SLOT: &str = "__lmViewTransitionFinishedResolver";
const VIEW_TRANSITION_UPDATE_DONE_SLOT: &str = "__lmViewTransitionUpdateDone";
const VIEW_TRANSITION_UPDATE_DONE_RESOLVER_SLOT: &str = "__lmViewTransitionUpdateDoneResolver";
const VIEW_TRANSITION_TYPES_SLOT: &str = "__lmViewTransitionTypes";
const VIEW_TRANSITION_SKIPPED_SLOT: &str = "__lmViewTransitionSkipped";
const VIEW_TRANSITION_DONE_SLOT: &str = "__lmViewTransitionDone";
const VIEW_TRANSITION_WAIT_COUNT_SLOT: &str = "__lmViewTransitionWaitCount";
const VIEW_TRANSITION_FINISH_REQUESTED_SLOT: &str = "__lmViewTransitionFinishRequested";
const CALLBACK_PENDING: &str = "pending";
const CALLBACK_RUNNING: &str = "running";
const CALLBACK_SUCCEEDED: &str = "succeeded";
const CALLBACK_FAILED: &str = "failed";

#[derive(WebApiObject)]
#[webapi(interface = "ViewTransition")]
struct ViewTransitionObjectDeclaration<'s> {
    #[webapi(slot = VIEW_TRANSITION_DOCUMENT_SLOT)]
    document: v8::Local<'s, v8::Object>,

    #[webapi(slot = VIEW_TRANSITION_CALLBACK_STATE_SLOT)]
    callback_state: &'static str,

    #[webapi(slot = VIEW_TRANSITION_READY_SLOT)]
    ready: v8::Local<'s, v8::Promise>,

    #[webapi(slot = VIEW_TRANSITION_READY_RESOLVER_SLOT)]
    ready_resolver: v8::Local<'s, v8::PromiseResolver>,

    #[webapi(slot = VIEW_TRANSITION_FINISHED_SLOT)]
    finished: v8::Local<'s, v8::Promise>,

    #[webapi(slot = VIEW_TRANSITION_FINISHED_RESOLVER_SLOT)]
    finished_resolver: v8::Local<'s, v8::PromiseResolver>,

    #[webapi(slot = VIEW_TRANSITION_UPDATE_DONE_SLOT)]
    update_callback_done: v8::Local<'s, v8::Promise>,

    #[webapi(slot = VIEW_TRANSITION_UPDATE_DONE_RESOLVER_SLOT)]
    update_callback_done_resolver: v8::Local<'s, v8::PromiseResolver>,

    #[webapi(slot = VIEW_TRANSITION_TYPES_SLOT)]
    types: v8::Local<'s, v8::Object>,

    #[webapi(slot = VIEW_TRANSITION_SKIPPED_SLOT)]
    skipped: bool,

    #[webapi(slot = VIEW_TRANSITION_DONE_SLOT)]
    done: bool,

    #[webapi(slot = VIEW_TRANSITION_WAIT_COUNT_SLOT)]
    wait_count: f64,

    #[webapi(slot = VIEW_TRANSITION_FINISH_REQUESTED_SLOT)]
    finish_requested: bool,
}

pub(super) fn new_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    initial_types: &[String],
) -> Option<v8::Local<'s, v8::Object>> {
    let ready_resolver = v8::PromiseResolver::new(scope)?;
    let ready = ready_resolver.get_promise(scope);
    let finished_resolver = v8::PromiseResolver::new(scope)?;
    let finished = finished_resolver.get_promise(scope);
    let update_callback_done_resolver = v8::PromiseResolver::new(scope)?;
    let update_callback_done = update_callback_done_resolver.get_promise(scope);
    let types = new_view_transition_type_set(scope, initial_types)?;

    ViewTransitionObjectDeclaration {
        document,
        callback_state: CALLBACK_PENDING,
        ready,
        ready_resolver,
        finished,
        finished_resolver,
        update_callback_done,
        update_callback_done_resolver,
        types,
        skipped: false,
        done: false,
        wait_count: 0.0,
        finish_requested: false,
    }
    .bind(scope)
    .ok()
}

pub(super) fn view_transition_skip_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_view_transition_receiver(scope, args.this(), "skipTransition") {
        return;
    }
    skip_view_transition(scope, args.this());
    rv.set_undefined();
}

pub(super) fn view_transition_wait_until_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_view_transition_receiver(scope, args.this(), "waitUntil") {
        return;
    }
    let transition = args.this();
    let next_wait_count = wait_count(scope, transition) + 1;
    set_wait_count(scope, transition, next_wait_count);

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        decrement_wait_count(scope, transition);
        return;
    };
    let promise = resolver.get_promise(scope);
    if resolver.resolve(scope, args.get(0)) != Some(true) {
        decrement_wait_count(scope, transition);
        return;
    }
    let Some(on_fulfilled) = v8::Function::builder(view_transition_wait_until_settled_callback)
        .data(transition.into())
        .build(scope)
    else {
        decrement_wait_count(scope, transition);
        return;
    };
    let Some(on_rejected) = v8::Function::builder(view_transition_wait_until_settled_callback)
        .data(transition.into())
        .build(scope)
    else {
        decrement_wait_count(scope, transition);
        return;
    };
    if promise.then2(scope, on_fulfilled, on_rejected).is_none() {
        decrement_wait_count(scope, transition);
    }
    rv.set_undefined();
}

fn view_transition_wait_until_settled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(transition) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    decrement_wait_count(scope, transition);
}

pub(super) fn view_transition_ready_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    return_view_transition_slot(
        scope,
        args.this(),
        "ready",
        VIEW_TRANSITION_READY_SLOT,
        &mut rv,
    );
}

pub(super) fn view_transition_finished_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    return_view_transition_slot(
        scope,
        args.this(),
        "finished",
        VIEW_TRANSITION_FINISHED_SLOT,
        &mut rv,
    );
}

pub(super) fn view_transition_update_callback_done_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    return_view_transition_slot(
        scope,
        args.this(),
        "updateCallbackDone",
        VIEW_TRANSITION_UPDATE_DONE_SLOT,
        &mut rv,
    );
}

pub(super) fn view_transition_types_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    return_view_transition_slot(
        scope,
        args.this(),
        "types",
        VIEW_TRANSITION_TYPES_SLOT,
        &mut rv,
    );
}

fn return_view_transition_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
    slot: &str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    if !require_view_transition_receiver(scope, receiver, member) {
        return;
    }
    if let Some(value) = get_private_value(scope, receiver, slot) {
        rv.set(value);
    }
}

fn require_view_transition_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, receiver, "ViewTransition") {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Failed to execute '{member}' on 'ViewTransition': Illegal invocation."),
    );
    false
}

pub(super) fn skip_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) {
    if transition_done(scope, transition) || transition_skipped(scope, transition) {
        return;
    }
    set_private_bool(scope, transition, VIEW_TRANSITION_SKIPPED_SLOT, true);
    clear_active_view_transition(scope, transition);
    let error = new_dom_exception_value(scope, "Transition was skipped", "AbortError");
    reject_resolver(
        scope,
        transition,
        VIEW_TRANSITION_READY_RESOLVER_SLOT,
        error,
    );
    if callback_state(scope, transition).as_deref() == Some(CALLBACK_SUCCEEDED) {
        finish_view_transition(scope, transition);
    }
}

pub(crate) fn run_view_transition_update_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    transition: v8::Local<'s, v8::Object>,
    callback: Option<&PreparedWindowWebIdlCallbackFunction>,
) {
    if callback_state(scope, transition).as_deref() != Some(CALLBACK_PENDING) {
        return;
    }
    set_private_string(
        scope,
        transition,
        VIEW_TRANSITION_CALLBACK_STATE_SLOT,
        CALLBACK_RUNNING,
    );

    let installed_reactions = if let Some(callback) = callback {
        let receiver = v8::undefined(scope).into();
        let arguments = [];
        match callback.invoke(
            scope,
            unsafe { &*host_ptr },
            receiver,
            &arguments,
            |scope, callback, receiver, arguments| {
                let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();
                let result = callback.call(&scope, receiver, arguments).ok_or_else(|| {
                    ViewTransitionUpdateInvocationFailure::CallbackThrew(
                        scope
                            .exception()
                            .map(|error| v8::Global::new(&scope, error)),
                    )
                })?;
                install_view_transition_update_reactions(&mut scope, transition, result)
                    .then_some(())
                    .ok_or(ViewTransitionUpdateInvocationFailure::PromiseNormalizationFailed)
            },
        ) {
            PreparedWindowWebIdlCallbackFunctionOutcome::Returned(()) => true,
            PreparedWindowWebIdlCallbackFunctionOutcome::Failed(
                ViewTransitionUpdateInvocationFailure::CallbackThrew(error),
            ) => {
                let error = error
                    .as_ref()
                    .map(|error| v8::Local::new(scope, error))
                    .unwrap_or_else(|| view_transition_callback_failure(scope));
                reject_view_transition(scope, transition, error);
                return;
            }
            PreparedWindowWebIdlCallbackFunctionOutcome::Failed(
                ViewTransitionUpdateInvocationFailure::PromiseNormalizationFailed,
            ) => false,
            PreparedWindowWebIdlCallbackFunctionOutcome::Retired => {
                let error = new_dom_exception_value(scope, "Transition was skipped", "AbortError");
                reject_view_transition(scope, transition, error);
                return;
            }
        }
    } else {
        let result = v8::undefined(scope).into();
        install_view_transition_update_reactions(scope, transition, result)
    };
    if !installed_reactions {
        let error = view_transition_callback_failure(scope);
        reject_view_transition(scope, transition, error);
    }
}

enum ViewTransitionUpdateInvocationFailure {
    CallbackThrew(Option<v8::Global<v8::Value>>),
    PromiseNormalizationFailed,
}

fn install_view_transition_update_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    result: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return false;
    };
    let promise = resolver.get_promise(scope);
    if resolver.resolve(scope, result) != Some(true) {
        return false;
    }
    let Some(on_fulfilled) = v8::Function::builder(view_transition_update_fulfilled_callback)
        .data(transition.into())
        .build(scope)
    else {
        return false;
    };
    let Some(on_rejected) = v8::Function::builder(view_transition_update_rejected_callback)
        .data(transition.into())
        .build(scope)
    else {
        return false;
    };
    promise.then2(scope, on_fulfilled, on_rejected).is_some()
}

fn view_transition_callback_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    let message = v8str(scope, "View transition callback failed");
    v8::Exception::error(scope, message)
}

fn view_transition_update_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(transition) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if callback_state(scope, transition).as_deref() != Some(CALLBACK_RUNNING) {
        return;
    }
    set_private_string(
        scope,
        transition,
        VIEW_TRANSITION_CALLBACK_STATE_SLOT,
        CALLBACK_SUCCEEDED,
    );
    resolve_resolver(scope, transition, VIEW_TRANSITION_UPDATE_DONE_RESOLVER_SLOT);
    if !transition_skipped(scope, transition) {
        resolve_resolver(scope, transition, VIEW_TRANSITION_READY_RESOLVER_SLOT);
    }
    set_private_bool(
        scope,
        transition,
        VIEW_TRANSITION_FINISH_REQUESTED_SLOT,
        true,
    );
    maybe_finish_view_transition(scope, transition);
}

fn view_transition_update_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(transition) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    reject_view_transition(scope, transition, args.get(0));
}

pub(super) fn reject_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if transition_done(scope, transition) {
        return;
    }
    set_private_string(
        scope,
        transition,
        VIEW_TRANSITION_CALLBACK_STATE_SLOT,
        CALLBACK_FAILED,
    );
    reject_resolver(
        scope,
        transition,
        VIEW_TRANSITION_UPDATE_DONE_RESOLVER_SLOT,
        error,
    );
    reject_resolver(
        scope,
        transition,
        VIEW_TRANSITION_READY_RESOLVER_SLOT,
        error,
    );
    reject_resolver(
        scope,
        transition,
        VIEW_TRANSITION_FINISHED_RESOLVER_SLOT,
        error,
    );
    set_private_bool(scope, transition, VIEW_TRANSITION_DONE_SLOT, true);
    clear_active_view_transition(scope, transition);
}

fn maybe_finish_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) {
    if !private_bool(scope, transition, VIEW_TRANSITION_FINISH_REQUESTED_SLOT)
        || (!transition_skipped(scope, transition) && wait_count(scope, transition) > 0)
    {
        return;
    }
    finish_view_transition(scope, transition);
}

fn finish_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) {
    if transition_done(scope, transition) {
        return;
    }
    set_private_bool(scope, transition, VIEW_TRANSITION_DONE_SLOT, true);
    clear_active_view_transition(scope, transition);
    resolve_resolver(scope, transition, VIEW_TRANSITION_FINISHED_RESOLVER_SLOT);
}

fn resolve_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    slot: &str,
) {
    if let Some(resolver) = transition_resolver(scope, transition, slot) {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
    }
}

fn reject_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    slot: &str,
    error: v8::Local<'s, v8::Value>,
) {
    if let Some(resolver) = transition_resolver(scope, transition, slot) {
        let _ = resolver.reject(scope, error);
    }
}

fn transition_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    get_private_value(scope, transition, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}

pub(super) fn active_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, document, DOCUMENT_ACTIVE_VIEW_TRANSITION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .filter(|transition| !transition_done(scope, *transition))
}

pub(super) fn set_active_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, document, DOCUMENT_ACTIVE_VIEW_TRANSITION_SLOT, value);
}

fn clear_active_view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) {
    let Some(document) = get_private_value(scope, transition, VIEW_TRANSITION_DOCUMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let is_active = get_private_value(scope, document, DOCUMENT_ACTIVE_VIEW_TRANSITION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .is_some_and(|active| active.strict_equals(transition.into()));
    if is_active {
        set_active_view_transition(scope, document, v8::null(scope).into());
    }
}

fn callback_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, transition, VIEW_TRANSITION_CALLBACK_STATE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn transition_skipped<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) -> bool {
    private_bool(scope, transition, VIEW_TRANSITION_SKIPPED_SLOT)
}

fn transition_done<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) -> bool {
    private_bool(scope, transition, VIEW_TRANSITION_DONE_SLOT)
}

fn private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    get_private_value(scope, object, slot).is_some_and(|value| value.boolean_value(scope))
}

fn set_private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_private_value(scope, object, slot, value.into());
}

fn set_private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, object, slot, value.into());
    }
}

fn wait_count<'s>(scope: &mut v8::PinScope<'s, '_>, transition: v8::Local<'s, v8::Object>) -> u32 {
    get_private_value(scope, transition, VIEW_TRANSITION_WAIT_COUNT_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0)
}

fn set_wait_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    count: u32,
) {
    let count = v8::Integer::new_from_unsigned(scope, count);
    set_private_value(
        scope,
        transition,
        VIEW_TRANSITION_WAIT_COUNT_SLOT,
        count.into(),
    );
}

fn decrement_wait_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) {
    let count = wait_count(scope, transition);
    if count == 0 {
        return;
    }
    set_wait_count(scope, transition, count - 1);
    maybe_finish_view_transition(scope, transition);
}
