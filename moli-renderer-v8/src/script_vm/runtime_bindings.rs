use crate::{
    context_bootstrap::dispatch_window_promise_rejection_event,
    exception_reporting::log_unhandled_promise_rejection,
    native_bridge::{JsContextHost, WindowDocumentTaskTarget, WindowExecutionContextBinding},
    page_task_queue::RendererPagePromiseRejectionTaskKind,
    util::context_host_ptr_from_global_bridge,
};
use std::{
    cell::RefCell,
    pin::pin,
    rc::{Rc, Weak},
    time::Instant,
};

const MAX_REPORTED_WINDOW_PROMISE_REJECTIONS: usize = 1024;

#[derive(Clone)]
struct PendingPromiseRejection {
    promise: v8::Global<v8::Promise>,
    reason: Option<v8::Global<v8::Value>>,
    // Promise events follow the exact realm where V8 first reported the
    // rejection. The strict binding restores both the V8 context and the
    // registry-backed LocalWindow/access-policy identity.
    realm: WindowExecutionContextBinding,
}

/// V8 handles stay on the creating Host. The Page queue carries only the
/// exact Document owner and an id for this detached notification batch.
pub(crate) struct PromiseRejectionTaskPayload {
    rejections: Vec<PendingPromiseRejection>,
}

#[derive(Clone)]
pub(crate) struct PromiseRejectDispatchSlot {
    pub(super) host_weak: Weak<RefCell<JsContextHost>>,
    pending_unhandled_rejections: Rc<RefCell<Vec<PendingPromiseRejection>>>,
    reported_unhandled_rejections: Rc<RefCell<Vec<PendingPromiseRejection>>>,
}

struct PromiseRejectDispatchState {
    host: Rc<RefCell<JsContextHost>>,
    pending: Rc<RefCell<Vec<PendingPromiseRejection>>>,
    reported: Rc<RefCell<Vec<PendingPromiseRejection>>>,
}

pub(super) fn promise_reject_dispatch_slot(
    host_rc: Rc<RefCell<JsContextHost>>,
) -> PromiseRejectDispatchSlot {
    PromiseRejectDispatchSlot {
        host_weak: Rc::downgrade(&host_rc),
        pending_unhandled_rejections: Rc::new(RefCell::new(Vec::new())),
        reported_unhandled_rejections: Rc::new(RefCell::new(Vec::new())),
    }
}

pub(super) fn install_promise_reject_dispatch_for_context(
    context: v8::Local<'_, v8::Context>,
    slot: &PromiseRejectDispatchSlot,
) {
    let _previous = context.set_slot(Rc::new(slot.clone()));
}

fn promise_reject_dispatch_state(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<PromiseRejectDispatchState> {
    scope
        .get_current_context()
        .get_slot::<PromiseRejectDispatchSlot>()
        .as_deref()
        .and_then(|slot| {
            Some(PromiseRejectDispatchState {
                host: slot.host_weak.upgrade()?,
                pending: slot.pending_unhandled_rejections.clone(),
                reported: slot.reported_unhandled_rejections.clone(),
            })
        })
}

#[cfg(test)]
pub(super) fn promise_reject_dispatch_is_available_for_test(
    scope: &mut v8::PinScope<'_, '_>,
) -> bool {
    promise_reject_dispatch_state(scope).is_some()
}

pub(super) fn clear_promise_rejection_dispatch_state(slot: &PromiseRejectDispatchSlot) {
    slot.pending_unhandled_rejections.borrow_mut().clear();
    slot.reported_unhandled_rejections.borrow_mut().clear();
}

fn pending_promise_rejection_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rejection: &PendingPromiseRejection,
    promise: v8::Local<'s, v8::Promise>,
) -> bool {
    v8::Local::new(scope, &rejection.promise).strict_equals(promise.into())
}

fn remember_reported_promise_rejection(
    reported: &Rc<RefCell<Vec<PendingPromiseRejection>>>,
    rejection: PendingPromiseRejection,
) {
    let mut reported = reported.borrow_mut();
    if reported.len() >= MAX_REPORTED_WINDOW_PROMISE_REJECTIONS {
        reported.remove(0);
    }
    reported.push(rejection);
}

fn queue_promise_rejection_notification(
    state: &PromiseRejectDispatchState,
    kind: RendererPagePromiseRejectionTaskKind,
    rejections: Vec<PendingPromiseRejection>,
) {
    let Some(first) = rejections.first() else {
        return;
    };
    let host_ptr: *mut JsContextHost = (*state.host).as_ptr();
    let host = unsafe { &mut *host_ptr };
    if !first.realm.is_current(host) {
        return;
    }
    let Some(target) =
        host.current_window_document_task_target_for_dispatch_scope(first.realm.dispatch_scope())
    else {
        return;
    };
    if !host.queue_promise_rejection_task(target, kind, PromiseRejectionTaskPayload { rejections })
    {
        tracing::debug!("promise rejection task route closed");
    }
}

fn queue_pending_promise_rejections(scope: &mut v8::PinScope<'_, '_>) -> usize {
    let Some(state) = promise_reject_dispatch_state(scope) else {
        return 0;
    };
    let pending = std::mem::take(&mut *state.pending.borrow_mut());
    let pending_len = pending.len();
    // HTML snapshots and empties each global's about-to-be-notified list at
    // the checkpoint. Later rejections belong to a later task, even if this
    // batch has not yet been selected by the event loop.
    let mut batches = indexmap::IndexMap::<_, Vec<_>>::new();
    for rejection in pending {
        batches
            .entry((
                rejection.realm.owner(),
                rejection.realm.dispatch_scope(),
                rejection.realm.realm_token(),
            ))
            .or_default()
            .push(rejection);
    }
    for (_, rejections) in batches {
        queue_promise_rejection_notification(
            &state,
            RendererPagePromiseRejectionTaskKind::Unhandled,
            rejections,
        );
    }
    pending_len
}

pub(super) fn dispatch_promise_rejection_task(
    scope: &mut v8::PinScope<'_, '_>,
    target: WindowDocumentTaskTarget,
    kind: RendererPagePromiseRejectionTaskKind,
    payload: PromiseRejectionTaskPayload,
) -> bool {
    let Some(state) = promise_reject_dispatch_state(scope) else {
        return false;
    };
    let host_ptr: *mut JsContextHost = (*state.host).as_ptr();
    state
        .reported
        .borrow_mut()
        .retain(|rejection| rejection.realm.is_current(unsafe { &*host_ptr }));

    let mut dispatched = false;
    for rejection in payload.rejections {
        if unsafe { &*host_ptr }
            .current_window_document_task_target_for_dispatch_scope(target.dispatch_scope())
            != Some(target)
        {
            break;
        }
        let _ = rejection
            .realm
            .with_current_scope(scope, host_ptr, |scope, dispatch_scope| {
                let promise = v8::Local::new(scope, &rejection.promise);
                // HTML rechecks PromiseIsHandled immediately before notifying.
                // The host, or an earlier notification in this detached batch,
                // may have handled it since it was first queued.
                if kind == RendererPagePromiseRejectionTaskKind::Unhandled && promise.has_handler()
                {
                    return;
                }
                let reason = rejection
                    .reason
                    .as_ref()
                    .map(|reason| v8::Local::new(scope, reason));
                let outcome = dispatch_window_promise_rejection_event(
                    scope,
                    host_ptr,
                    dispatch_scope,
                    match kind {
                        RendererPagePromiseRejectionTaskKind::Unhandled => "unhandledrejection",
                        RendererPagePromiseRejectionTaskKind::Handled => "rejectionhandled",
                    },
                    promise,
                    reason,
                );
                // A handler added by the `unhandledrejection` listener must not
                // synchronously turn this notification into `rejectionhandled`.
                // Only promises that remain unhandled after dispatch belong in
                // the outstanding reported-rejection set.
                dispatched |= outcome.is_ok();
                if kind == RendererPagePromiseRejectionTaskKind::Unhandled && !promise.has_handler()
                {
                    remember_reported_promise_rejection(&state.reported, rejection.clone());
                }
                if kind == RendererPagePromiseRejectionTaskKind::Unhandled
                    && matches!(outcome, Ok(true))
                {
                    log_unhandled_promise_rejection(scope, reason);
                }
            });
    }
    dispatched
}

pub(crate) fn perform_microtask_checkpoint_and_report_pending_promise_rejections(
    scope: &mut v8::PinScope<'_, '_>,
) {
    let Some(_checkpoint_scope) = crate::script_cleanup::MicrotaskCheckpointScope::enter(scope)
    else {
        return;
    };
    let trace_enabled = moli_trace::cdp_runtime_trace_enabled();
    let dom_binding_trace_enabled = trace_enabled && moli_trace::dom_binding_timing_enabled();
    let trace_started = trace_enabled.then(Instant::now);
    if trace_started.is_some() {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "microtask_checkpoint_start",
        );
        let _ = moli_trace::take_promise_hook_stats();
        if dom_binding_trace_enabled {
            let _ = moli_trace::take_dom_binding_operation_stats();
        }
    }
    let checkpoint_started = trace_enabled.then(Instant::now);
    scope.perform_microtask_checkpoint();
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        crate::native_bridge::restore_deferred_active_child_window_scope_if_present(scope, host);
        crate::native_bridge::restore_deferred_active_lightweight_popup_scope_if_present(
            scope, host,
        );
        host.finish_deferred_child_subresource_request_scope_pop();
    }
    let promise_stats = if trace_enabled {
        moli_trace::take_promise_hook_stats()
    } else {
        moli_trace::PromiseHookStats::default()
    };
    let dom_binding_stats = if dom_binding_trace_enabled {
        moli_trace::take_dom_binding_operation_stats()
    } else {
        Vec::new()
    };
    let dom_operation_count: u64 = dom_binding_stats.iter().map(|stat| stat.count).sum();
    let dom_total_us: u128 = dom_binding_stats.iter().map(|stat| stat.total_us).sum();
    let dom_max = dom_binding_stats
        .iter()
        .max_by_key(|stat| stat.max_us)
        .map(|stat| (stat.op, stat.max_us))
        .unwrap_or(("<none>", 0));
    let dom_operations = dom_binding_stats
        .iter()
        .map(|stat| {
            format!(
                "{}:count={},total_us={},max_us={}",
                stat.op, stat.count, stat.total_us, stat.max_us
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    if let Some(started) = checkpoint_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "microtask_checkpoint_v8_done",
            promise_init_count = promise_stats.init_count,
            promise_resolve_count = promise_stats.resolve_count,
            promise_reaction_before_count = promise_stats.reaction_before_count,
            promise_reaction_after_count = promise_stats.reaction_after_count,
            dom_operation_count,
            dom_total_us,
            dom_max_op = dom_max.0,
            dom_max_op_us = dom_max.1,
            dom_operations = %dom_operations,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
    let queue_started = trace_enabled.then(Instant::now);
    let pending_rejections = queue_pending_promise_rejections(scope);
    crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
    if let Some(started) = queue_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "microtask_checkpoint_rejection_queue_done",
            pending_rejections,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
    if let Some(started) = trace_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "microtask_checkpoint_done",
            pending_rejections,
            promise_init_count = promise_stats.init_count,
            promise_resolve_count = promise_stats.resolve_count,
            promise_reaction_before_count = promise_stats.reaction_before_count,
            promise_reaction_after_count = promise_stats.reaction_after_count,
            dom_operation_count,
            dom_total_us,
            dom_max_op = dom_max.0,
            dom_max_op_us = dom_max.1,
            dom_operations = %dom_operations,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
}

pub(super) unsafe extern "C" fn promise_trace_hook(
    hook_type: v8::PromiseHookType,
    _promise: v8::Local<v8::Promise>,
    _parent: v8::Local<v8::Value>,
) {
    match hook_type {
        v8::PromiseHookType::Init => moli_trace::record_promise_hook_init(),
        v8::PromiseHookType::Resolve => moli_trace::record_promise_hook_resolve(),
        // V8 documents Before/After as PromiseReactionJob boundaries. Counting
        // Before gives the number of promise reaction callbacks entered during
        // the currently active trace window.
        v8::PromiseHookType::Before => moli_trace::record_promise_reaction_before(),
        v8::PromiseHookType::After => moli_trace::record_promise_reaction_after(),
    }
}

pub(super) unsafe extern "C" fn promise_reject_callback(message: v8::PromiseRejectMessage<'_>) {
    let scope = pin!(unsafe { v8::CallbackScope::new(&message) });
    let scope = &mut scope.init();
    let context = scope.get_current_context();
    let scope = &mut v8::ContextScope::new(scope, context);

    let Some(state) = promise_reject_dispatch_state(scope) else {
        return;
    };
    let host_ptr: *mut JsContextHost = (*state.host).as_ptr();

    match message.get_event() {
        v8::PromiseRejectEvent::PromiseRejectWithNoHandler => {
            let Some(realm) =
                unsafe { &*host_ptr }.current_runtime_window_execution_context_binding(scope)
            else {
                return;
            };
            let promise = message.get_promise();
            let mut pending = state.pending.borrow_mut();
            if pending
                .iter()
                .any(|rejection| pending_promise_rejection_matches(scope, rejection, promise))
            {
                return;
            }
            pending.push(PendingPromiseRejection {
                promise: v8::Global::new(scope, promise),
                reason: message
                    .get_value()
                    .map(|reason| v8::Global::new(scope, reason)),
                realm,
            });
        }
        v8::PromiseRejectEvent::PromiseHandlerAddedAfterReject => {
            let promise = message.get_promise();
            let mut pending = state.pending.borrow_mut();
            if let Some(index) = pending
                .iter()
                .position(|rejection| pending_promise_rejection_matches(scope, rejection, promise))
            {
                pending.remove(index);
                return;
            }
            drop(pending);

            let reported = {
                let mut reported = state.reported.borrow_mut();
                reported
                    .iter()
                    .position(|rejection| {
                        pending_promise_rejection_matches(scope, rejection, promise)
                    })
                    .map(|index| reported.swap_remove(index))
            };
            let Some(rejection) = reported else {
                return;
            };
            queue_promise_rejection_notification(
                &state,
                RendererPagePromiseRejectionTaskKind::Handled,
                vec![rejection],
            );
        }
        _ => {}
    }
}

pub(super) unsafe extern "C" fn failed_access_check_callback(
    target: v8::Local<'_, v8::Object>,
    _access_type: v8::AccessType,
    _data: v8::Local<'_, v8::Value>,
) {
    let scope = pin!(unsafe { v8::CallbackScope::new(target) });
    let scope = &mut scope.init();
    let accessing_context = scope.get_current_context();
    let target_context = target.get_creation_context(scope);
    if moli_trace::window_message_trace_enabled() {
        tracing::info!(
            target: "moli_window_message_trace",
            accessing_is_target = target_context == Some(accessing_context),
            stage = "window_cross_origin_failed_access",
        );
    }
    crate::native_bridge::throw_cross_origin_location_security_error(scope);
}
