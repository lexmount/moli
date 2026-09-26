//! Execution and commit barriers for one traversal of the shared history.
//! The barrier retains task identities, never a second history or cached length.
use super::history_runtime::{
    apply_pending_history_traversal, history_traversal_target_window, route_history_traversal_task,
};
use super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_entry_key_value,
};
use super::navigation_events::dispatch_navigation_entry_dispose;
use super::navigation_seed::history_entry_seed_for_traversal;
use super::navigation_traversal_execution::{
    apply_pending_cross_document_traversal, reject_canceled_history_traversal_results,
};
use super::navigation_window::{
    runtime_window_is_global, window_history_for_holder, window_task_target_for_runtime_owner,
};
use super::session_history;
use crate::native_bridge::{
    JsContextHost, OwnerDispatchScope, PendingCrossDocumentTraversal, PendingHistoryTraversal,
    WindowTaskTarget,
};
use crate::util::context_host_ptr_from_global_bridge;
use moli_session_history::{NavigationHistoryEntryKey, SessionHistoryStepId};

pub(super) fn apply_entry(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    traversal: PendingHistoryTraversal,
) {
    let Some(owner) = history_traversal_target_window(scope, host, traversal.target) else {
        reject_canceled_history_traversal_results(scope, &traversal.results);
        return;
    };
    // Trackers registered before an earlier traversal's navigate event are
    // adopted by that traversal. A reentrant call registered during the event
    // remains upcoming, and rejects if its queued task finds the entry active.
    if !traversal.results.is_empty()
        && let Some(key) = traversal.target_key.as_deref()
        && navigation_current_entry(scope, owner)
            .and_then(|entry| navigation_entry_key_value(scope, entry))
            .is_some_and(|current| current == key)
    {
        let error = super::navigation_result::navigation_dom_exception(
            scope,
            "The traversal target is already the active history entry",
            "InvalidStateError",
        );
        super::history_runtime::results::reject_pending_navigation_results(
            scope,
            &traversal.results,
            error,
        );
        return;
    }
    let step = traversal.joint_step.or_else(|| {
        let history = window_history_for_holder(scope, owner)?;
        let entries = history_entries(scope, history)?;
        let entry = entries.get(traversal.target_index as usize)?;
        session_history::step_for_entry(scope, owner, entry)
    });
    if let Some(step) = step {
        apply_step(scope, host, traversal.target, step, Some(traversal));
    } else {
        apply_pending_history_traversal(scope, host, traversal);
    }
}

pub(super) fn apply_step(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    source: WindowTaskTarget,
    step: SessionHistoryStepId,
    method: Option<PendingHistoryTraversal>,
) {
    let Some(owner) = history_traversal_target_window(scope, host, source) else {
        if let Some(method) = method {
            reject_canceled_history_traversal_results(scope, &method.results);
        }
        return;
    };
    let plan = session_history::traversal_is_allowed(scope, owner, step)
        .then(|| super::navigation_traversal_plan::JointTraversalPlan::resolve(scope, owner, step))
        .flatten();
    let Some(plan) = plan else {
        if let Some(method) = method {
            reject_canceled_history_traversal_results(scope, &method.results);
        }
        return;
    };
    let binding = session_history::binding(scope, host, owner);
    let root = plan.cross_document_root_index(scope);
    if let Some(index) = root {
        let target = &plan.targets[index];
        if runtime_window_is_global(scope, target.owner) {
            let Some(delta) = host.session_histories.get_mut(binding.popup).delta_to(step) else {
                return;
            };
            if let Some(entry) = history_entries(scope, target.history)
                .and_then(|entries| entries.get(target.target_index as usize).cloned())
            {
                let key = entry.borrow().key.as_str().to_owned();
                host.top_level_navigation_history()
                    .select_joint_traversal(key, step);
            }
            host.record_pending_top_level_history_traversal(delta);
            return;
        }
    }
    let targets = plan
        .targets
        .iter()
        .map(|target| {
            let entries = history_entries(scope, target.history)?;
            let entry = entries.get(target.target_index as usize)?;
            let key = entry.borrow().key.clone();
            Some((
                session_history::binding(scope, host, target.owner).context,
                key,
            ))
        })
        .collect::<Option<Vec<_>>>();
    let Some(targets) = targets else {
        if let Some(method) = method {
            reject_canceled_history_traversal_results(scope, &method.results);
        }
        return;
    };
    host.session_histories
        .begin_traversal(binding.popup, step, targets);
    host.retain_active_history_delta_position(binding.popup);
    let accepted = if let Some(index) = root {
        let target = &plan.targets[index];
        let pending = window_task_target_for_runtime_owner(scope, host, target.owner).zip(
            history_entry_seed_for_traversal(
                scope,
                target.owner,
                target.current_index,
                target.target_index,
            ),
        );
        if let Some((exact, (_url, mut seed))) = pending {
            seed.session_history.target_step = Some(step);
            let key = history_entries(scope, target.history)
                .and_then(|entries| entries.get(target.target_index as usize).cloned())
                .map(|entry| entry.borrow().key.as_str().to_owned());
            let (info, results) = method.map_or_else(
                || (None, Vec::new()),
                |method| (method.info, method.results),
            );
            apply_pending_cross_document_traversal(
                scope,
                host,
                PendingCrossDocumentTraversal {
                    target: exact,
                    target_index: target.target_index,
                    target_key: key,
                    seed,
                    info,
                    results,
                },
            )
        } else {
            false
        }
    } else {
        let history =
            window_history_for_holder(scope, owner).expect("admitted traversal has a history");
        let traversal = method.unwrap_or_else(|| PendingHistoryTraversal {
            joint_step: Some(step),
            target: source,
            target_index: history_index(scope, history),
            target_key: None,
            info: None,
            results: Vec::new(),
        });
        apply_pending_history_traversal(scope, host, traversal)
    };
    if !accepted {
        cancel_step(scope, owner, step);
    }
}

pub(super) fn cancel_step<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    step: SessionHistoryStepId,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = session_history::binding(scope, host, owner);
    if let Some(current) = host
        .session_histories
        .cancel_traversal_step(binding.popup, step)
    {
        finish(scope, host, owner, binding.popup, current);
    }
}

pub(super) fn finish_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    key: Option<&NavigationHistoryEntryKey>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = session_history::binding(scope, host, owner);
    if let Some(step) =
        host.session_histories
            .finish_traversal_entry(binding.popup, binding.context, key)
    {
        finish(scope, host, owner, binding.popup, step);
    }
}

fn finish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    owner: v8::Local<'s, v8::Object>,
    popup: Option<u64>,
    step: SessionHistoryStepId,
) {
    for entry in session_history::prune_views(scope, owner) {
        dispatch_navigation_entry_dispose(scope, entry);
    }
    for producer in unsafe { &mut *host_ptr }.finish_pending_history_traversal(popup, step) {
        route_history_traversal_task(scope, unsafe { &mut *host_ptr }, producer);
    }
}

pub(crate) fn finish_without_document_commit(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    owner: OwnerDispatchScope,
) {
    let host = unsafe { &mut *host_ptr };
    let popup = host.session_histories.popup_for_owner(owner);
    if !host.session_histories.has_pending_traversal(popup) {
        return;
    }
    let context = host.session_histories.context(owner);
    if let Some((step, delta)) = host
        .session_histories
        .finish_traversal_without_document_commit(popup, context)
        && let Some(root) = session_history::owner_for_context(
            scope,
            host,
            moli_session_history::SessionHistoryContextId::ROOT,
            popup,
        )
    {
        if let Some(delta) = delta.filter(|delta| *delta != 0) {
            session_history::publish(
                scope,
                root,
                moli_page_types::SessionHistoryUpdateKind::Traverse { delta },
            );
        }
        // A retiring child must never materialize its Window again.
        finish(scope, host_ptr, root, popup, step);
    }
}
