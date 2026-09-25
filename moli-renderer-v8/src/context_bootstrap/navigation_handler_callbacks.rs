//! Web IDL callback-function residence for Navigation API handlers.
//!
//! `NavigateEvent` already owns the lifetime of intercept and precommit
//! handlers. The private arrays below therefore retain V8-traced callback
//! carriers on that exact event instead of creating a second Rust registry,
//! Page task, or Promise-settlement owner.
//!
//! Invocation follows the Navigation API's Promise-returning callback
//! contract:
//!
//! - enter the callback's relevant and conversion-time incumbent Realms;
//! - turn a synchronous throw into a rejected Promise in the callback Realm;
//! - preserve a sole callback-Realm Promise, or aggregate multiple callbacks
//!   with a target-Realm `Promise.all`;
//! - skip a handler whose exact callback Window has retired.
//!
//! The navigation transaction remains responsible for waiting on the returned
//! Promise and for commit, abort, supersession, and final settlement.

use super::*;
use crate::{
    util::{
        context_host_ptr_from_global_bridge, get_private_value, serialize_v8_iter_array,
        set_private_value,
    },
    window_webidl_callback::{
        PreparedWindowWebIdlCallbackFunctionOutcome, V8TracedWindowWebIdlCallbackFunction,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;

pub(in crate::context_bootstrap) const NAVIGATE_EVENT_PRECOMMIT_HANDLERS_SLOT: &str =
    "__lmNavigateEventPrecommitHandlers";
pub(in crate::context_bootstrap) const NAVIGATE_EVENT_DEFERRED_HANDLERS_SLOT: &str =
    "__lmNavigateEventDeferredHandlers";
pub(in crate::context_bootstrap) const NAVIGATE_EVENT_ADDED_HANDLERS_SLOT: &str =
    "__lmNavigateEventAddedHandlers";
const HANDLER_EVENT_VIEW_SLOT: &str = "__moliNavigationHandlerEventView";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap) enum NavigationHandlerResidenceFailure {
    /// V8 returned an empty `Maybe`, leaving its exception pending for the
    /// JavaScript-reachable caller to propagate.
    V8ExceptionPending,
    /// V8 rejected the write without an exception. `set_index()` currently
    /// documents this state as unreachable, but treating it explicitly keeps
    /// a future contract change from silently dropping a handler.
    RejectedWithoutException,
}

pub(in crate::context_bootstrap) fn push_navigation_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
    callback: WebIdlCallbackFunction,
) -> Result<(), NavigationHandlerResidenceFailure> {
    let callback = V8TracedWindowWebIdlCallbackFunction::new(scope, callback).into_object();
    set_private_value(scope, callback, HANDLER_EVENT_VIEW_SLOT, event.into());
    let event = super::events::event_backing(scope, event);
    let handlers = get_private_value(scope, event, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| {
            let handlers = v8::Array::new(scope, 0);
            set_private_value(scope, event, slot, handlers.into());
            handlers
        });
    let result = navigation_handler_residence_write_result(handlers.set_index(
        scope,
        handlers.length(),
        callback.into(),
    ));
    if matches!(
        result,
        Err(NavigationHandlerResidenceFailure::RejectedWithoutException)
    ) {
        crate::util::throw_type_error(
            scope,
            "NavigateEvent callback residence rejected its handler",
        );
    }
    result
}

fn navigation_handler_residence_write_result(
    result: Option<bool>,
) -> Result<(), NavigationHandlerResidenceFailure> {
    match result {
        Some(true) => Ok(()),
        Some(false) => Err(NavigationHandlerResidenceFailure::RejectedWithoutException),
        None => Err(NavigationHandlerResidenceFailure::V8ExceptionPending),
    }
}

pub(in crate::context_bootstrap) fn navigation_handler_array_is_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, event, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .is_none_or(|handlers| handlers.length() == 0)
}

pub(in crate::context_bootstrap) fn run_navigation_handler_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
    arguments: &[v8::Local<'s, v8::Value>],
) -> (
    Option<v8::Local<'s, v8::Value>>,
    Option<v8::Local<'s, v8::Value>>,
) {
    run_navigation_handler_arrays(scope, event, &[slot], arguments)
}

pub(in crate::context_bootstrap) fn run_navigation_handler_arrays<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slots: &[&'static str],
    arguments: &[v8::Local<'s, v8::Value>],
) -> (
    Option<v8::Local<'s, v8::Value>>,
    Option<v8::Local<'s, v8::Value>>,
) {
    let handler_arrays: Vec<_> = slots
        .iter()
        .filter_map(|slot| take_navigation_handler_array(scope, event, slot))
        .collect();
    if handler_arrays.is_empty() {
        return (None, None);
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return (
            Some(navigation_handler_host_error(
                scope,
                "Navigation handler lost its Window owner",
            )),
            None,
        );
    };
    let capacity = handler_arrays
        .iter()
        .map(|handlers| handlers.length() as usize)
        .sum();
    let mut promises = Vec::with_capacity(capacity);
    for handlers in handler_arrays {
        for index in 0..handlers.length() {
            let carrier = handlers
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .expect("a NavigateEvent handler array must contain callback carriers");
            match invoke_navigation_handler(scope, host_ptr, event, carrier, arguments) {
                Ok(promise) => promises.push(promise),
                Err(error) => return (Some(error), None),
            }
        }
    }
    (None, combined_navigation_handler_result(scope, &promises))
}

fn take_navigation_handler_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    let handlers = get_private_value(scope, event, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    set_private_value(scope, event, slot, v8::undefined(scope).into());
    (handlers.length() > 0).then_some(handlers)
}

enum NavigationHandlerInvocationFailure {
    PromiseAllocation,
    PromiseSettlement,
}

fn invoke_navigation_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    event: v8::Local<'s, v8::Object>,
    carrier: v8::Local<'s, v8::Object>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Result<v8::Local<'s, v8::Value>, v8::Local<'s, v8::Value>> {
    let view =
        crate::util::get_private_object(scope, carrier, HANDLER_EVENT_VIEW_SLOT).unwrap_or(event);
    let arguments: Vec<_> = arguments
        .iter()
        .map(|argument| {
            super::events::navigation_precommit_controller_for_event(scope, *argument, view)
        })
        .collect();
    let callback = V8TracedWindowWebIdlCallbackFunction::from_object(carrier)
        .prepare(scope, unsafe { &*host_ptr });
    match callback.invoke(
        scope,
        unsafe { &*host_ptr },
        view.into(),
        &arguments,
        |scope, callback, receiver, arguments| {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let mut scope = try_catch.init();
            match callback.call(&scope, receiver, arguments) {
                Some(result) => {
                    let current_context = scope.get_current_context();
                    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result)
                        && promise.get_creation_context(&scope) == Some(current_context)
                    {
                        return Ok(v8::Global::new(&scope, promise));
                    }
                    let resolver = v8::PromiseResolver::new(&scope)
                        .ok_or(NavigationHandlerInvocationFailure::PromiseAllocation)?;
                    let local_promise = resolver.get_promise(&scope);
                    let promise = v8::Global::new(&scope, local_promise);
                    if resolver.resolve(&scope, result) != Some(true) {
                        return Err(NavigationHandlerInvocationFailure::PromiseSettlement);
                    }
                    Ok(promise)
                }
                None => {
                    let error = scope
                        .exception()
                        .map(|error| v8::Global::new(&scope, error));
                    scope.reset();
                    let resolver = v8::PromiseResolver::new(&scope)
                        .ok_or(NavigationHandlerInvocationFailure::PromiseAllocation)?;
                    let local_promise = resolver.get_promise(&scope);
                    let promise = v8::Global::new(&scope, local_promise);
                    let error = error
                        .as_ref()
                        .map(|error| v8::Local::new(&scope, error))
                        .unwrap_or_else(|| {
                            navigation_handler_host_error(
                                &mut scope,
                                "Navigation handler invocation failed",
                            )
                        });
                    if resolver.reject(&scope, error) != Some(true) {
                        return Err(NavigationHandlerInvocationFailure::PromiseSettlement);
                    }
                    Ok(promise)
                }
            }
        },
    ) {
        PreparedWindowWebIdlCallbackFunctionOutcome::Returned(promise) => {
            Ok(v8::Local::new(scope, &promise).into())
        }
        PreparedWindowWebIdlCallbackFunctionOutcome::Failed(failure) => {
            let message = match failure {
                NavigationHandlerInvocationFailure::PromiseAllocation => {
                    "Navigation handler Promise allocation failed"
                }
                NavigationHandlerInvocationFailure::PromiseSettlement => {
                    "Navigation handler Promise settlement failed"
                }
            };
            Err(navigation_handler_host_error(scope, message))
        }
        PreparedWindowWebIdlCallbackFunctionOutcome::Retired => {
            resolved_navigation_handler_promise(scope).ok_or_else(|| {
                navigation_handler_host_error(
                    scope,
                    "Navigation handler retirement Promise allocation failed",
                )
            })
        }
    }
}

fn resolved_navigation_handler_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    (resolver.resolve(scope, v8::undefined(scope).into()) == Some(true)).then_some(promise.into())
}

fn combined_navigation_handler_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promises: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    if promises.is_empty() {
        return None;
    }
    if let [promise] = promises {
        return Some(*promise);
    }
    let array = serialize_v8_iter_array(scope, promises.iter().copied())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let promise = scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "Promise").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let all = promise
        .get(scope, v8str(scope, "all").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    all.call(scope, promise.into(), &[array.into()])
}

fn navigation_handler_host_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let message = v8::String::new(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    v8::Exception::error(scope, message)
}

#[cfg(test)]
mod tests {
    use super::{NavigationHandlerResidenceFailure, navigation_handler_residence_write_result};

    #[test]
    fn navigation_handler_residence_write_failure_is_not_treated_as_stored() {
        assert_eq!(
            navigation_handler_residence_write_result(Some(true)),
            Ok(())
        );
        assert_eq!(
            navigation_handler_residence_write_result(None),
            Err(NavigationHandlerResidenceFailure::V8ExceptionPending)
        );
        assert_eq!(
            navigation_handler_residence_write_result(Some(false)),
            Err(NavigationHandlerResidenceFailure::RejectedWithoutException)
        );
    }
}
