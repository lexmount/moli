use super::history_runtime::history_traversal_target_window;
use super::navigation_entry::{
    history_entries, history_index, navigation_entry_initial_index, navigation_entry_key_value,
    set_history_entries, set_history_length,
};
use super::navigation_events::dispatch_navigation_entry_dispose;
use super::navigation_serialize::sync_navigation_entry_seed_from_owner;
use super::navigation_window::{
    runtime_top_window_owner, runtime_window_dispatch_scope, runtime_window_owner,
    window_history_for_holder,
};
use crate::native_bridge::joint_history::{
    JointHistoryEntry, JointHistoryPosition, JointHistorySnapshot, JointHistoryTarget,
    JointHistoryTraversal,
};
use crate::native_bridge::{
    JsContextHost, OwnerDispatchScope, PendingHistoryTraversal, WindowTaskTarget,
};
use crate::util::context_host_ptr_from_global_bridge;
use moli_page_types::SameDocumentHistoryUpdate;

fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<JointHistorySnapshot> {
    let entries = history_entries(scope, history)?;
    let mut snapshot = JointHistorySnapshot {
        entries: Vec::new(),
        current_index: history_index(scope, history),
    };
    for index in 0..entries.length() {
        let Some(entry) = entries
            .get_index(scope, index)
            .and_then(|entry| v8::Local::<v8::Object>::try_from(entry).ok())
        else {
            continue;
        };
        let Some(key) = navigation_entry_key_value(scope, entry) else {
            continue;
        };
        let navigation_index = navigation_entry_initial_index(scope, entry).unwrap_or(index);
        snapshot.entries.push(JointHistoryEntry {
            index,
            navigation_index,
            key,
        });
    }
    Some(snapshot)
}

fn before_push(mut snapshot: JointHistorySnapshot) -> JointHistorySnapshot {
    snapshot
        .entries
        .retain(|entry| entry.index != snapshot.current_index);
    snapshot.current_index = snapshot.current_index.saturating_sub(1);
    snapshot
}

pub(super) fn window_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    owner: OwnerDispatchScope,
) -> Option<v8::Local<'s, v8::Object>> {
    if owner == OwnerDispatchScope::Top {
        return host
            .page_default_context(scope)
            .map(|context| context.global(scope));
    }
    let execution_owner = host.current_window_execution_context_owner(owner)?;
    history_traversal_target_window(scope, host, WindowTaskTarget::new(owner, execution_owner))
}

pub(super) fn refresh<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    owner: v8::Local<'s, v8::Object>,
    pushing: Option<OwnerDispatchScope>,
) -> Option<OwnerDispatchScope> {
    let root_window = runtime_top_window_owner(scope, owner);
    let root = runtime_window_dispatch_scope(scope, root_window)?;
    let root_window = window_for_owner(scope, host, root).unwrap_or(root_window);
    let history = window_history_for_holder(scope, root_window)?;
    let mut root_snapshot = snapshot(scope, history)?;
    if pushing == Some(root) {
        root_snapshot = before_push(root_snapshot);
    }
    host.joint_histories.ensure_root(root, root_snapshot)?;
    let mut live = vec![root];
    for handle in host.child_browsing_context_handles_in_document_order() {
        let Some(window) = host.existing_child_browsing_context_window_wrapper(scope, handle)
        else {
            continue;
        };
        let top = runtime_top_window_owner(scope, window);
        if runtime_window_dispatch_scope(scope, top) != Some(root) {
            continue;
        }
        let child = OwnerDispatchScope::Child(handle);
        live.push(child);
        let Some(history) = window_history_for_holder(scope, window) else {
            continue;
        };
        let Some(mut child_snapshot) = snapshot(scope, history) else {
            continue;
        };
        if pushing == Some(child) {
            child_snapshot = before_push(child_snapshot);
        }
        host.joint_histories
            .get_mut(root)?
            .ensure_child(child, child_snapshot);
    }
    host.joint_histories
        .get_mut(root)?
        .retain_navigables(|owner| live.contains(&owner));
    Some(root)
}

pub(super) fn length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<usize> {
    let owner = runtime_window_owner(scope, history);
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &mut *host_ptr };
    let root = refresh(scope, host, owner, None)?;
    Some(host.joint_histories.get(root)?.length())
}

pub(super) fn position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    owner: v8::Local<'s, v8::Object>,
) -> Option<JointHistoryPosition> {
    let root = refresh(scope, host, owner, None)?;
    let history = host.joint_histories.get(root)?;
    Some(JointHistoryPosition {
        root,
        step: history.source_step(),
        revision: history.revision(),
    })
}

pub(crate) fn reset(scope: &mut v8::PinScope<'_, '_>, host: &mut JsContextHost) {
    let root = OwnerDispatchScope::Top;
    let Some(root_window) = window_for_owner(scope, host, root) else {
        return;
    };
    let Some(root_snapshot) =
        window_history_for_holder(scope, root_window).and_then(|history| snapshot(scope, history))
    else {
        return;
    };
    if host
        .joint_histories
        .ensure_root(root, root_snapshot.clone())
        .is_none()
    {
        return;
    }
    let mut snapshots = vec![(root, root_snapshot)];
    for handle in host.child_browsing_context_handles_in_document_order() {
        let Some(window) = host.existing_child_browsing_context_window_wrapper(scope, handle)
        else {
            continue;
        };
        let top = runtime_top_window_owner(scope, window);
        if runtime_window_dispatch_scope(scope, top) != Some(root) {
            continue;
        }
        if let Some(snapshot) =
            window_history_for_holder(scope, window).and_then(|history| snapshot(scope, history))
        {
            snapshots.push((OwnerDispatchScope::Child(handle), snapshot));
        }
    }
    host.joint_histories.get_mut(root).unwrap().reset(snapshots);
    finish_traversal(scope, host, root);
}

pub(super) fn apply_delta(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    source_target: WindowTaskTarget,
    source: JointHistoryPosition,
    delta: i64,
) {
    let Some(owner) = history_traversal_target_window(scope, host, source_target) else {
        return;
    };
    let Some(root) = refresh(scope, host, owner, None) else {
        return;
    };
    if root != source.root {
        return;
    }
    let Some(joint) = host.joint_histories.get(root) else {
        return;
    };
    let Some(plan) = joint.plan_delta(source.step, delta) else {
        if source_target.dispatch_scope() == OwnerDispatchScope::Top {
            host.record_pending_top_level_history_traversal(delta);
        }
        return;
    };
    apply_plan(scope, host, source_target, root, plan, None);
}

pub(super) fn apply_entry(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    traversal: PendingHistoryTraversal,
) {
    let Some(owner) = history_traversal_target_window(scope, host, traversal.target) else {
        super::navigation_traversal_execution::reject_canceled_history_traversal_results(
            scope,
            &traversal.results,
        );
        return;
    };
    let Some(root) = refresh(scope, host, owner, None) else {
        super::history_runtime::apply_pending_history_traversal(scope, host, traversal);
        return;
    };
    let key = traversal.target_key.clone().or_else(|| {
        let history = window_history_for_holder(scope, owner)?;
        let entries = history_entries(scope, history)?;
        let entry =
            v8::Local::<v8::Object>::try_from(entries.get_index(scope, traversal.target_index)?)
                .ok()?;
        navigation_entry_key_value(scope, entry)
    });
    let plan = key.and_then(|key| {
        host.joint_histories
            .get(root)?
            .plan_entry(traversal.target.dispatch_scope(), &key)
    });
    let Some(plan) = plan else {
        super::navigation_traversal_execution::reject_canceled_history_traversal_results(
            scope,
            &traversal.results,
        );
        return;
    };
    if plan.targets.is_empty() {
        super::history_runtime::apply_pending_history_traversal(scope, host, traversal);
        return;
    }
    apply_plan(scope, host, traversal.target, root, plan, Some(traversal));
}

fn apply_plan(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    source_target: WindowTaskTarget,
    root: OwnerDispatchScope,
    plan: JointHistoryTraversal,
    mut method: Option<PendingHistoryTraversal>,
) {
    if plan.targets.iter().any(|target| {
        !host.sandbox_allows_history_traversal(source_target.dispatch_scope(), target.owner, root)
    }) {
        if let Some(method) = method {
            let error = super::navigation_result::navigation_dom_exception(
                scope,
                "The initiator cannot traverse all affected navigables",
                "SecurityError",
            );
            super::history_runtime::reject_pending_navigation_results(
                scope,
                &method.results,
                error,
            );
        }
        return;
    }
    // A new top-level Document is committed by the browser. Convert the
    // selected joint step to the browser's top-level entry delta.
    for target in &plan.targets {
        if target.owner != OwnerDispatchScope::Top {
            continue;
        }
        let Some(window) = window_for_owner(scope, host, target.owner) else {
            return;
        };
        let Some(history) = window_history_for_holder(scope, window) else {
            return;
        };
        let current = history_index(scope, history);
        if super::navigation_seed::history_entry_seed_for_traversal(
            scope,
            window,
            current,
            target.index,
        )
        .is_some()
        {
            host.record_pending_top_level_history_traversal(
                i64::from(target.index) - i64::from(current),
            );
            return;
        }
    }
    let targets = plan
        .targets
        .iter()
        .map(|target| {
            let execution_owner = host.current_window_execution_context_owner(target.owner)?;
            Some((
                target.clone(),
                WindowTaskTarget::new(target.owner, execution_owner),
            ))
        })
        .collect::<Option<Vec<_>>>();
    let Some(targets) = targets else {
        if let Some(method) = method {
            super::navigation_traversal_execution::reject_canceled_history_traversal_results(
                scope,
                &method.results,
            );
        }
        return;
    };
    host.joint_histories
        .get_mut(root)
        .unwrap()
        .begin_traversal(plan.clone());
    host.retain_active_joint_history_delta_position(root);
    let mut canceled = false;
    for (target, exact_target) in targets {
        if host.current_window_execution_context_owner(target.owner) != Some(exact_target.owner()) {
            canceled = true;
            break;
        }
        let Some(window) = window_for_owner(scope, host, target.owner) else {
            canceled = true;
            break;
        };
        let context = window
            .get_creation_context(scope)
            .unwrap_or_else(|| scope.get_current_context());
        let scope = &mut v8::ContextScope::new(scope, context);
        let Some(history) = window_history_for_holder(scope, window) else {
            canceled = true;
            break;
        };
        let current = history_index(scope, history);
        let (info, results) = if target.owner == source_target.dispatch_scope() {
            method.take().map_or_else(
                || (None, Vec::new()),
                |method| (method.info, method.results),
            )
        } else {
            (None, Vec::new())
        };
        let accepted = if let Some((url, seed)) =
            super::navigation_seed::history_entry_seed_for_traversal(
                scope,
                window,
                current,
                target.index,
            ) {
            super::navigation_traversal_execution::apply_pending_cross_document_traversal(
                scope,
                host,
                crate::native_bridge::PendingCrossDocumentTraversal {
                    target: exact_target,
                    target_index: target.index,
                    target_key: Some(target.key),
                    target_url: url.to_string(),
                    seed,
                    info,
                    results,
                },
            )
        } else {
            super::history_runtime::apply_pending_history_traversal(
                scope,
                host,
                crate::native_bridge::PendingHistoryTraversal {
                    target: exact_target,
                    target_index: target.index,
                    target_key: Some(target.key),
                    info,
                    results,
                },
            )
        };
        if !accepted {
            canceled = true;
            break;
        }
    }
    if canceled && let Some(method) = method {
        super::navigation_traversal_execution::reject_canceled_history_traversal_results(
            scope,
            &method.results,
        );
    }
    if let Some(joint) = host.joint_histories.get_mut(root) {
        let removed = if canceled {
            joint.cancel_traversal()
        } else {
            joint.finish_traversal()
        };
        prune_runtime_entries(scope, host, removed);
        finish_traversal(scope, host, root);
    }
}

pub(crate) fn commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    update: SameDocumentHistoryUpdate,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(dispatch) = runtime_window_dispatch_scope(scope, owner) else {
        return;
    };
    let pushing = (update == SameDocumentHistoryUpdate::Push).then_some(dispatch);
    let Some(root) = refresh(scope, host, owner, pushing) else {
        return;
    };
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let Some(snapshot) = snapshot(scope, history) else {
        return;
    };
    let Some(joint) = host.joint_histories.get_mut(root) else {
        return;
    };
    let removed = match update {
        SameDocumentHistoryUpdate::Push => joint.push(dispatch, snapshot),
        SameDocumentHistoryUpdate::Replace => {
            joint.replace(dispatch, snapshot);
            Vec::new()
        }
        SameDocumentHistoryUpdate::Traverse { .. } => {
            if let Some(entry) = snapshot
                .entries
                .iter()
                .find(|entry| entry.index == snapshot.current_index)
            {
                joint.commit_traversal_entry(dispatch, &entry.key)
            } else {
                Vec::new()
            }
        }
    };
    let length = joint.length() as f64;
    let root_window = runtime_top_window_owner(scope, owner);
    if let Some(top_history) = window_history_for_holder(scope, root_window) {
        set_history_length(scope, top_history, length);
    }
    set_history_length(scope, history, length);
    prune_runtime_entries(scope, host, removed);
    finish_traversal(scope, host, root);
}

fn finish_traversal(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    root: OwnerDispatchScope,
) {
    let Some((step, revision)) = host
        .joint_histories
        .get_mut(root)
        .and_then(|joint| joint.take_completed_traversal())
    else {
        return;
    };
    let position = JointHistoryPosition {
        root,
        step,
        revision,
    };
    // Only a traversal commit advances queued deltas. A popstate listener can
    // synchronously push a new entry without changing those requests' source.
    host.commit_active_joint_history_delta_position(position);
    for producer in host.finish_joint_history_traversal(position) {
        super::history_runtime::route_history_traversal_task(scope, host, producer);
    }
}

pub(super) fn cancel_precommit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    target_index: u32,
) {
    let owner = runtime_window_owner(scope, history);
    let Some(dispatch) = runtime_window_dispatch_scope(scope, owner) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(root) = host.joint_histories.root_for_owner(dispatch) else {
        return;
    };
    let Some(joint) = host.joint_histories.get_mut(root) else {
        return;
    };
    if joint
        .pending_target(dispatch)
        .is_none_or(|target| target.index != target_index)
    {
        return;
    }
    let removed = joint.cancel_traversal();
    prune_runtime_entries(scope, host, removed);
    finish_traversal(scope, host, root);
}

pub(crate) fn finish_without_document_commit(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    owner: OwnerDispatchScope,
) {
    let Some(root) = host.joint_histories.root_for_owner(owner) else {
        return;
    };
    let Some(joint) = host.joint_histories.get_mut(root) else {
        return;
    };
    let Some(target) = joint.pending_target(owner).cloned() else {
        return;
    };
    let removed = joint.finish_traversal_entry(owner, &target.key);
    prune_runtime_entries(scope, host, removed);
    finish_traversal(scope, host, root);
}

pub(crate) fn remove_navigable(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    owner: OwnerDispatchScope,
) {
    let Some(root) = host.joint_histories.root_for_owner(owner) else {
        return;
    };
    let Some(joint) = host.joint_histories.get_mut(root) else {
        return;
    };
    joint.retain_navigables(|candidate| candidate != owner);
    let removed = joint.finish_traversal();
    prune_runtime_entries(scope, host, removed);
    finish_traversal(scope, host, root);
}

fn prune_runtime_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    removed: Vec<JointHistoryTarget>,
) {
    let mut owners = Vec::new();
    for target in &removed {
        if !owners.contains(&target.owner) {
            owners.push(target.owner);
        }
    }
    let mut disposed = Vec::new();
    for owner in owners {
        let Some(window) = window_for_owner(scope, host, owner) else {
            continue;
        };
        let Some(history) = window_history_for_holder(scope, window) else {
            continue;
        };
        let Some(entries) = history_entries(scope, history) else {
            continue;
        };
        let mut retained = Vec::new();
        for index in 0..entries.length() {
            let Some(value) = entries.get_index(scope, index) else {
                continue;
            };
            let Ok(entry) = v8::Local::<v8::Object>::try_from(value) else {
                continue;
            };
            let key = navigation_entry_key_value(scope, entry);
            if removed
                .iter()
                .any(|target| target.owner == owner && Some(target.key.as_str()) == key.as_deref())
            {
                disposed.push(entry);
            } else {
                retained.push((index, entry));
            }
        }
        let length = retained.last().map_or(0, |(index, _)| index + 1);
        let next = v8::Array::new(scope, length as i32);
        for (index, entry) in retained {
            let _ = next.set_index(scope, index, entry.into());
        }
        set_history_entries(scope, history, next);
        sync_navigation_entry_seed_from_owner(scope, window);
    }
    for entry in disposed {
        dispatch_navigation_entry_dispose(scope, entry);
    }
}
