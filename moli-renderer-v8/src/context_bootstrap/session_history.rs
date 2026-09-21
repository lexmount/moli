use super::navigation_entry::{
    history_entries, history_index, set_history_entries, set_history_index,
};
use super::navigation_window::{
    runtime_top_window_owner, runtime_window_dispatch_scope, window_history_for_holder,
};
use crate::native_bridge::{JsContextHost, NavigationHistoryEntrySeed, OwnerDispatchScope};
use crate::util::context_host_ptr_from_global_bridge;
use moli_history::{HistoryEntry, HistoryEntryRef};
use moli_page_types::SessionHistoryCommit;
use moli_session_history::{
    JointSessionHistory, NavigationHistoryEntryKey, SessionHistoryContextId, SessionHistoryEntry,
    SessionHistoryPosition, SessionHistoryStepId, SessionHistoryTraversalPlan,
};

#[derive(Clone, Copy)]
pub(super) struct HistoryBinding {
    pub(super) popup: Option<u64>,
    pub(super) context: SessionHistoryContextId,
}

pub(super) fn binding<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    owner: v8::Local<'s, v8::Object>,
) -> HistoryBinding {
    let top = runtime_top_window_owner(scope, owner);
    let popup = crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
        .or_else(|| crate::native_bridge::lightweight_popup_id_from_window(scope, top));
    let dispatch = runtime_window_dispatch_scope(scope, owner).unwrap_or(OwnerDispatchScope::Top);
    HistoryBinding {
        popup,
        context: host.session_histories.context(dispatch),
    }
}

pub(super) fn entry_reference<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<SessionHistoryEntry> {
    let entry = super::history_runtime::native::entry(scope, entry)?;
    Some(native_entry_reference(&entry.borrow()))
}

pub(super) fn native_entry_reference(entry: &HistoryEntry) -> SessionHistoryEntry {
    SessionHistoryEntry {
        key: entry.key.clone(),
        document: entry.document.clone(),
    }
}

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    seed: &NavigationHistoryEntrySeed,
) {
    let Some(snapshot) = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = binding(scope, host, owner);
    let parent = match runtime_window_dispatch_scope(scope, owner) {
        Some(OwnerDispatchScope::Child(handle)) => {
            let parent = host
                .child_browsing_context_parent_handle(handle)
                .map_or(OwnerDispatchScope::Top, OwnerDispatchScope::Child);
            Some(host.session_histories.context(parent))
        }
        _ => None,
    };
    host.session_histories.get_mut(binding.popup).attach(
        binding.context,
        parent,
        SessionHistoryEntry {
            key: NavigationHistoryEntryKey::from_serialized(
                super::navigation_entry::navigation_entry_public_token(snapshot.key.as_str()),
            ),
            document: snapshot.document_id.clone(),
        },
    );
}

pub(super) fn restore<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    seed: &NavigationHistoryEntrySeed,
) {
    let Some(snapshot) = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = binding(scope, host, owner);
    let history = host.session_histories.get_mut(binding.popup);
    if binding.context == SessionHistoryContextId::ROOT
        && let Some(snapshot) = &seed.session_history.traversable
    {
        *history = (**snapshot).clone();
        return;
    }
    let entry = SessionHistoryEntry {
        key: NavigationHistoryEntryKey::from_serialized(
            super::navigation_entry::navigation_entry_public_token(snapshot.key.as_str()),
        ),
        document: snapshot.document_id.clone(),
    };
    // The seed may be installed repeatedly while a child navigation loads.
    // Identity, rather than URL or list shape, makes the commit idempotent.
    if history.entry(binding.context) == Some(&entry)
        || seed.session_history.admitted_entry.is_some()
    {
        return;
    }
    let update = match seed.session_history.commit {
        SessionHistoryCommit::Push => {
            history.push(binding.context, entry);
            Some(moli_page_types::SessionHistoryUpdateKind::Push)
        }
        SessionHistoryCommit::Attach | SessionHistoryCommit::Replace => {
            history.replace(binding.context, entry);
            (seed.session_history.commit == SessionHistoryCommit::Replace)
                .then_some(moli_page_types::SessionHistoryUpdateKind::Replace)
        }
        SessionHistoryCommit::Traverse => seed
            .session_history
            .target_step
            .or_else(|| history.step_for_entry(binding.context, &entry.key))
            .and_then(|step| history.plan_traversal(step))
            .and_then(|plan| history.commit_traversal(&plan))
            .filter(|delta| *delta != 0)
            .map(|delta| moli_page_types::SessionHistoryUpdateKind::Traverse { delta }),
    };
    if let Some(update) = update {
        publish(scope, owner, update);
    }
}

pub(crate) fn install_session_history_position(
    scope: &mut v8::PinScope<'_, '_>,
    position: SessionHistoryPosition,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let owner = scope.get_current_context().global(scope);
    let entry = window_history_for_holder(scope, owner).and_then(|history| {
        let index = history_index(scope, history);
        history_entries(scope, history)?
            .get(index as usize)
            .map(|entry| native_entry_reference(&entry.borrow()))
    });
    let history = unsafe { &mut *host_ptr }.session_histories.get_mut(None);
    *history = JointSessionHistory::new(position);
    if let Some(entry) = entry {
        history.attach(SessionHistoryContextId::ROOT, None, entry);
    }
}

/// Called after the main realm is admitted. Child realms may be prebootstrapped
/// before it; creating their JS surfaces cannot register a main-frame entry.
pub(crate) fn initialize_main_session_history(scope: &mut v8::PinScope<'_, '_>) {
    let owner = scope.get_current_context().global(scope);
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let index = history_index(scope, history);
    let entry = history_entries(scope, history).and_then(|entries| {
        entries
            .get(index as usize)
            .map(|entry| native_entry_reference(&entry.borrow()))
    });
    if let Some(entry) = entry
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }
            .session_histories
            .get_mut(None)
            .attach(SessionHistoryContextId::ROOT, None, entry);
    }
}

pub(super) fn length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> usize {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return 1;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = binding(scope, host, owner);
    host.session_histories
        .get_mut(binding.popup)
        .position()
        .length()
}

/// Commit the shared state before exposing currententrychange/dispose to JS.
pub(super) fn commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
    kind: SessionHistoryCommit,
) {
    let Some(entry) = entry_reference(scope, entry) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = binding(scope, host, owner);
    let history = host.session_histories.get_mut(binding.popup);
    match kind {
        SessionHistoryCommit::Push => history.push(binding.context, entry),
        SessionHistoryCommit::Attach | SessionHistoryCommit::Replace => {
            history.replace(binding.context, entry)
        }
        SessionHistoryCommit::Traverse => {
            if let Some(target) = history.step_for_entry(binding.context, &entry.key)
                && let Some(plan) = history.plan_traversal(target)
            {
                history.commit_traversal(&plan);
            }
        }
    }
    let update = match kind {
        SessionHistoryCommit::Push => Some(moli_page_types::SessionHistoryUpdateKind::Push),
        SessionHistoryCommit::Replace => Some(moli_page_types::SessionHistoryUpdateKind::Replace),
        SessionHistoryCommit::Attach | SessionHistoryCommit::Traverse => None,
    };
    if let Some(update) = update {
        publish(scope, owner, update);
    }
}

/// Capture the next top-level Document's history without committing a pending
/// navigation to the old Document. Failed/canceled loads leave its cursor alone.
pub(super) fn capture_for_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    seed: &mut NavigationHistoryEntrySeed,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = binding(scope, host, owner);
    if binding.context != SessionHistoryContextId::ROOT {
        return;
    }
    let Some(entry) = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
    else {
        return;
    };
    let entry = SessionHistoryEntry {
        key: NavigationHistoryEntryKey::from_serialized(
            super::navigation_entry::navigation_entry_public_token(entry.key.as_str()),
        ),
        document: entry.document_id.clone(),
    };
    let mut history = host.session_histories.get_mut(binding.popup).clone();
    match seed.session_history.commit {
        SessionHistoryCommit::Push => history.push(binding.context, entry),
        SessionHistoryCommit::Attach | SessionHistoryCommit::Replace => {
            history.replace(binding.context, entry)
        }
        SessionHistoryCommit::Traverse => {
            if let Some(target) = seed
                .session_history
                .target_step
                .or_else(|| history.step_for_entry(binding.context, &entry.key))
                && let Some(plan) = history.plan_traversal(target)
            {
                history.commit_traversal(&plan);
            }
        }
    }
    seed.session_history.traversable = Some(Box::new(history));
}

pub(super) fn owner_for_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    context: SessionHistoryContextId,
    popup: Option<u64>,
) -> Option<v8::Local<'s, v8::Object>> {
    match host.session_histories.owner(context, popup)? {
        OwnerDispatchScope::Top => Some(
            host.page_default_context(scope)
                .unwrap_or_else(|| scope.get_current_context())
                .global(scope),
        ),
        OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_window_wrapper(scope, handle)
        }
        OwnerDispatchScope::LightweightPopup(id) => host.lightweight_popup_window(scope, id),
    }
}

pub(super) fn step_for_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry: &HistoryEntryRef,
) -> Option<SessionHistoryStepId> {
    let key = entry.borrow().key.clone();
    let host = unsafe { &mut *context_host_ptr_from_global_bridge(scope)? };
    let binding = binding(scope, host, owner);
    host.session_histories
        .get_mut(binding.popup)
        .step_for_entry(binding.context, &key)
}

/// Remove forward entries in every live Document view, returning objects for
/// dispose delivery after the complete state change. The views never determine
/// the number or order of joint steps.
pub(super) fn prune_views<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Vec::new();
    };
    let host = unsafe { &mut *host_ptr };
    let binding = binding(scope, host, owner);
    let model = host.session_histories.get_mut(binding.popup).clone();
    let mut owners = vec![runtime_top_window_owner(scope, owner)];
    if binding.popup.is_none() {
        for handle in host.child_browsing_context_handles_in_document_order() {
            if let Some(window) = host.child_browsing_context_window_wrapper(scope, handle) {
                owners.push(window);
            }
        }
    }
    let mut removed = Vec::new();
    for owner in owners {
        let context = self::binding(scope, host, owner).context;
        let Some(history) = window_history_for_holder(scope, owner) else {
            continue;
        };
        let Some(entries) = history_entries(scope, history) else {
            continue;
        };
        let old_current = history_index(scope, history);
        let mut retained = Vec::new();
        let mut current = 0;
        let old_len = entries.len();
        for (index, entry) in entries.into_iter().enumerate() {
            let keep =
                index <= old_current as usize || model.contains_entry(context, &entry.borrow().key);
            if keep {
                if index == old_current as usize {
                    current = retained.len() as u32;
                }
                retained.push(entry);
            } else {
                // Keep observable wrappers alive until dispose delivery, before pruning the cache.
                removed.push(super::history_runtime::native::entry_wrapper(
                    scope, owner, entry,
                ));
            }
        }
        if retained.len() == old_len {
            continue;
        }
        // Removing a suffix preserves Navigation's filtered indices. Raw
        // history includes hidden entries and cannot supply these indices.
        set_history_entries(scope, history, retained);
        set_history_index(scope, history, current);
        super::navigation_serialize::sync_child_navigation_entry_seed_from_owner(scope, owner);
    }
    removed
}

pub(crate) fn prune_joint_session_history(scope: &mut v8::PinScope<'_, '_>) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }
            .session_histories
            .get_mut(None)
            .prune_all_but_current();
    }
    let owner = scope.get_current_context().global(scope);
    publish(
        scope,
        owner,
        moli_page_types::SessionHistoryUpdateKind::PruneAllButCurrent,
    );
}

pub(super) fn plan_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    step: SessionHistoryStepId,
) -> Option<SessionHistoryTraversalPlan> {
    let host = unsafe { &mut *context_host_ptr_from_global_bridge(scope)? };
    let binding = binding(scope, host, owner);
    host.session_histories
        .get_mut(binding.popup)
        .plan_traversal(step)
}

pub(super) fn project_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    plan: &SessionHistoryTraversalPlan,
) -> Option<Vec<super::navigation_traversal_execution::TraversalTarget<'s>>> {
    let host = unsafe { &mut *context_host_ptr_from_global_bridge(scope)? };
    let binding = binding(scope, host, owner);
    if !host
        .session_histories
        .get_mut(binding.popup)
        .is_traversal_plan_current(plan)
    {
        return None;
    }
    let entries = plan.target_entries();
    // Opaque browser-owned steps return to the browser controller. Project
    // the complete destination because live Documents may still be loading a
    // previously accepted step, even for a context unchanged in this plan.
    if !entries.contains_key(&SessionHistoryContextId::ROOT) {
        return None;
    }
    let source = super::navigation_window::window_task_target_for_runtime_owner(scope, host, owner)?.dispatch_scope();
    let root = binding.popup.map_or(OwnerDispatchScope::Top, OwnerDispatchScope::LightweightPopup);
    let mut targets = Vec::new();
    for (&context, entry) in entries {
        let Some(owner) = owner_for_context(scope, host, context, binding.popup) else {
            continue;
        };
        let Some(history) = window_history_for_holder(scope, owner) else {
            continue;
        };
        let local_entries = history_entries(scope, history)?;
        let current_index = history_index(scope, history);
        let index = local_entries
            .iter()
            .position(|candidate| candidate.borrow().key == entry.key)
            .map(|index| index as u32);
        let Some(target_index) = index else {
            if context == SessionHistoryContextId::ROOT {
                return None;
            }
            continue;
        };
        if current_index != target_index {
            let target = super::navigation_window::window_task_target_for_runtime_owner(scope, host, owner)?.dispatch_scope();
            if !host.sandbox_allows_history_traversal(source, target, root) {
                return Some(Vec::new());
            }
            targets.push(super::navigation_traversal_execution::TraversalTarget {
                owner,
                history,
                current_index,
                target_index,
                joint_step: Some(plan.target_step()),
            });
        }
    }
    Some(targets)
}

pub(super) fn commit_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    plan: &SessionHistoryTraversalPlan,
) -> Option<i64> {
    let host = unsafe { &mut *context_host_ptr_from_global_bridge(scope)? };
    let binding = binding(scope, host, owner);
    host.session_histories
        .get_mut(binding.popup)
        .commit_traversal(plan)
}

pub(super) fn publish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    update: moli_page_types::SessionHistoryUpdateKind,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let binding = binding(scope, host, owner);
    if binding.popup.is_some() {
        return;
    }
    let history = host.session_histories.get_mut(None);
    let position = history.position();
    let root_key = history
        .entry(SessionHistoryContextId::ROOT)
        .map(|entry| entry.key.clone());
    let root_entry_steps = root_key.as_ref().map_or_else(Vec::new, |key| {
        history.steps_for_entry(SessionHistoryContextId::ROOT, key)
    });
    let top = runtime_top_window_owner(scope, owner);
    let root_url = window_history_for_holder(scope, top)
        .and_then(|history| history_entries(scope, history))
        .and_then(|entries| {
            entries.iter().find_map(|entry| {
                let entry = entry.borrow();
                root_key
                    .as_ref()
                    .filter(|root| *root == &entry.key)
                    .map(|_| entry.url.clone())
            })
        })
        .unwrap_or_else(|| host.document_url().as_str().to_owned());
    host.record_session_history_update(moli_page_types::SessionHistoryUpdate {
        position,
        update,
        root_url,
        root_entry_steps,
    });
}
