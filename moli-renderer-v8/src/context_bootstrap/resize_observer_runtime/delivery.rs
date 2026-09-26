//! One depth-limited ResizeObserver broadcast in a Document rendering update.
//! The ScriptVm publishes layout between broadcasts; this code only samples
//! the published geometry and never causes a layout pass while invoking JS.

use super::*;
use crate::native_bridge::{JsContextHost, WindowDocumentTaskTarget, WindowExecutionContextOwner};

#[derive(Default)]
pub(crate) struct ResizeObserverBroadcast {
    pub(crate) shallowest_depth: Option<usize>,
    pub(crate) skipped: bool,
    pub(crate) invoked: bool,
}

pub(crate) fn broadcast_document_resize_observers(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
    owner: WindowExecutionContextOwner,
    depth: usize,
) -> Result<ResizeObserverBroadcast, moli_layout::LayoutError> {
    let mut result = ResizeObserverBroadcast::default();
    let Some((_, context)) =
        (unsafe { &mut *host_ptr }).resolve_current_rendering_update_context(scope, target)
    else {
        return Ok(result);
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let previous_scope = target.dispatch_scope().enter(scope);
    let outcome = (|| {
        let observers =
            crate::observer_runtime::active_resize_observer_callbacks(scope, host_ptr, Some(owner));
        let mut deliveries = Vec::new();
        // Gather all observers before invoking any callbacks. Updating the
        // last-reported size here would lose observations skipped by depth.
        for observer in observers {
            if scope.is_execution_terminating() {
                break;
            }
            let Some(context) = observer.get_creation_context(scope) else {
                continue;
            };
            let scope = &mut v8::ContextScope::new(scope, context);
            let Some(records) = resize_observer_targets(scope, observer) else {
                continue;
            };
            let mut active = Vec::new();
            for sample in sample_resize_observations(scope, records)? {
                if resize_observer_last_reported_size(scope, sample.record)
                    == Some(sample.observed_size)
                {
                    continue;
                }
                if flattened_target_depth(unsafe { &*host_ptr }, sample.handle) > depth {
                    active.push(sample);
                } else {
                    result.skipped = true;
                }
            }
            let delivery_active = v8::Boolean::new(scope, !active.is_empty());
            set_private_value(
                scope,
                observer,
                RESIZE_OBSERVER_DELIVERY_ACTIVE_SLOT,
                delivery_active.into(),
            );
            if !active.is_empty() {
                deliveries.push((observer, active));
            }
        }
        for (observer, samples) in deliveries {
            if scope.is_execution_terminating()
                || !(unsafe { &*host_ptr }).window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
            {
                break;
            }
            // The native samples above own activeTargets. This private marker
            // lets disconnect() in an earlier callback cancel their delivery;
            // unobserve() only changes the next gather, as required by the API.
            let active = get_private_value(scope, observer, RESIZE_OBSERVER_DELIVERY_ACTIVE_SLOT);
            if !active.is_some_and(|value| value.is_true()) {
                continue;
            }
            let Some(residence) = resize_observer_callback_residence(scope, observer) else {
                continue;
            };
            let Some(callback) =
                crate::observer_runtime::prepare_callback(scope, host_ptr, residence)
            else {
                continue;
            };
            let Some(context) = observer.get_creation_context(scope) else {
                continue;
            };
            let scope = &mut v8::ContextScope::new(scope, context);
            let mut entries = Vec::with_capacity(samples.len());
            for sample in samples {
                let target_depth = flattened_target_depth(unsafe { &*host_ptr }, sample.handle);
                result.shallowest_depth = Some(
                    result
                        .shallowest_depth
                        .map_or(target_depth, |current| current.min(target_depth)),
                );
                set_resize_observer_last_reported_size(scope, sample.record, sample.observed_size);
                entries.push(
                    sample
                        .entry
                        .bind(scope)
                        .expect("ResizeObserverEntry declaration should bind"),
                );
            }
            let pending = v8::Array::new(scope, 0);
            set_resize_observer_pending_targets(scope, observer, pending);
            let entries =
                serialize_v8_iter_array(scope, entries).unwrap_or_else(|| v8::Array::new(scope, 0));
            let receiver: v8::Local<'_, v8::Value> = observer.into();
            let execution_scope = crate::script_cleanup::ScriptExecutionScope::enter(scope);
            let outcome = callback.invoke(
                scope,
                host_ptr,
                "ResizeObserver callback",
                receiver,
                &[entries.into(), receiver],
            );
            if !scope.is_execution_terminating()
                && let WindowWebIdlCallbackFunctionOutcome::Threw(report) = outcome
            {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "resizeobserver",
                    callback.relevant_identity(),
                    None,
                    &report,
                );
            }
            drop(execution_scope);
            crate::script_cleanup::perform_callback_cleanup_checkpoint(scope);
            if !scope.is_execution_terminating() {
                let active = v8::Boolean::new(scope, false);
                set_private_value(
                    scope,
                    observer,
                    RESIZE_OBSERVER_DELIVERY_ACTIVE_SLOT,
                    active.into(),
                );
            }
            result.invoked = true;
        }
        Ok(result)
    })();
    target.dispatch_scope().restore(scope, previous_scope);
    outcome
}

fn flattened_target_depth(
    host: &JsContextHost,
    mut handle: Option<crate::document_runtime::DomHandle>,
) -> usize {
    let mut depth = 0;
    let dom = host.dom_host();
    while let Some(node) = handle {
        depth += 1;
        handle = if let Some(slot) = dom.assigned_slot_for_node(node) {
            Some(slot)
        } else if let Some(parent) = dom.parent_node(node) {
            if dom.is_shadow_root(parent) {
                dom.shadow_root_host(parent)
            } else {
                Some(parent)
            }
        } else {
            None
        };
    }
    depth.max(1)
}

pub(crate) fn report_document_resize_observer_loop_error(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
) {
    let Some((_, context)) =
        (unsafe { &mut *host_ptr }).resolve_current_rendering_update_context(scope, target)
    else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let previous_scope = target.dispatch_scope().enter(scope);
    (unsafe { &mut *host_ptr }).queue_document_observer_update(scope, target.dispatch_scope());
    let execution_scope = crate::script_cleanup::ScriptExecutionScope::enter(scope);
    let _ = crate::context_bootstrap::dispatch_window_error_event_with_details(
        scope,
        host_ptr,
        "ResizeObserver loop completed with undelivered notifications.",
        "",
        0,
        0,
        None,
    );
    drop(execution_scope);
    crate::script_cleanup::perform_callback_cleanup_checkpoint(scope);
    target.dispatch_scope().restore(scope, previous_scope);
}
