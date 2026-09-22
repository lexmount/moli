use super::history_runtime::pending_history_traversal_target_index;
use super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_current_entry_index,
};
use super::navigation_projection::{
    visible_navigation_entries_len, visible_navigation_index_for_entry,
};
use super::navigation_traversal_execution::TraversalTarget;
use super::navigation_window::{
    navigation_document_is_active, navigation_has_disabled_entries, runtime_window_owner,
    window_history_for_holder,
};

pub(super) struct JointTraversalPlan<'s> {
    pub(super) core: moli_session_history::SessionHistoryTraversalPlan,
    pub(super) owner: v8::Local<'s, v8::Object>,
    pub(super) targets: Vec<TraversalTarget<'s>>,
}

impl<'s> JointTraversalPlan<'s> {
    pub(super) fn resolve(
        scope: &mut v8::PinScope<'s, '_>,
        owner: v8::Local<'s, v8::Object>,
        step: moli_session_history::SessionHistoryStepId,
    ) -> Option<Self> {
        let core = super::session_history::plan_traversal(scope, owner, step)?;
        let targets = super::session_history::project_traversal(scope, owner, &core)?;
        Some(Self {
            core,
            owner,
            targets,
        })
    }

    pub(super) fn step(&self) -> moli_session_history::SessionHistoryStepId {
        self.core.target_step()
    }

    pub(super) fn has_cross_document_root(&self, scope: &mut v8::PinScope<'s, '_>) -> bool {
        self.cross_document_root_index(scope).is_some()
    }

    pub(super) fn cross_document_root_index(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<usize> {
        self.targets.iter().position(|target| {
            (super::navigation_window::runtime_window_is_global(scope, target.owner)
                || crate::native_bridge::lightweight_popup_id_from_window(scope, target.owner)
                    .is_some())
                && super::navigation_seed::history_entry_seed_for_traversal(
                    scope,
                    target.owner,
                    target.current_index,
                    target.target_index,
                )
                .is_some()
        })
    }
}

pub(super) enum NavigationTraversalPlan<'s> {
    RejectInvalidState(&'static str),
    ResolveCurrentEntry(v8::Local<'s, v8::Object>),
    Traverse(TraversalTarget<'s>),
}

pub(super) fn navigation_delta_traversal_plan<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    delta: i64,
) -> Option<NavigationTraversalPlan<'s>> {
    let owner = runtime_window_owner(scope, navigation);
    if !navigation_document_is_active(scope, owner) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse a non-fully-active document",
        ));
    }
    if navigation_has_disabled_entries(scope, navigation) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse a document with disabled navigation entries",
        ));
    }
    let history = window_history_for_holder(scope, owner)?;
    // Navigation API methods select an entry relative to the committed
    // current entry. Repeated calls before the task runs share that target.
    let current_index = i64::from(history_index(scope, history));
    let entries = history_entries(scope, history)?;
    let current_entry = navigation_current_entry(scope, owner);
    let current_navigation_index = current_entry
        .and_then(|entry| {
            let record = super::history_runtime::native::entry(scope, entry)?;
            visible_navigation_index_for_entry(scope, &entries, Some(entry), &record)
        })
        .or_else(|| navigation_current_entry_index(scope, owner))
        .unwrap_or(0) as i64;
    let visible_len = visible_navigation_entries_len(scope, &entries, current_entry) as i64;
    if delta < 0 && current_navigation_index <= 0 {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot go back",
        ));
    }
    if delta > 0 && current_navigation_index + delta >= visible_len {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot go forward",
        ));
    }
    let next_index = current_index + delta;
    if next_index < 0 || next_index >= entries.len() as i64 {
        let message = if delta < 0 {
            "Cannot go back"
        } else {
            "Cannot go forward"
        };
        return Some(NavigationTraversalPlan::RejectInvalidState(message));
    }
    Some(NavigationTraversalPlan::Traverse(TraversalTarget {
        owner,
        history,
        current_index: current_index as u32,
        target_index: next_index as u32,
        joint_step: None,
    }))
}

pub(super) fn navigation_index_traversal_plan<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    target_index: u32,
) -> Option<NavigationTraversalPlan<'s>> {
    let owner = runtime_window_owner(scope, navigation);
    if !navigation_document_is_active(scope, owner) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse a non-fully-active document",
        ));
    }
    if navigation_has_disabled_entries(scope, navigation) {
        return Some(NavigationTraversalPlan::RejectInvalidState(
            "Cannot traverse a document with disabled navigation entries",
        ));
    }
    let history = window_history_for_holder(scope, owner)?;
    let entries = history_entries(scope, history)?;
    if target_index as usize >= entries.len() {
        return Some(NavigationTraversalPlan::RejectInvalidState("Invalid key"));
    }
    let pending_target_index = pending_history_traversal_target_index(scope, history);
    if pending_target_index == Some(target_index) {
        return Some(NavigationTraversalPlan::Traverse(TraversalTarget {
            owner,
            history,
            current_index: history_index(scope, history),
            target_index,
            joint_step: None,
        }));
    }
    let current_index = pending_target_index.unwrap_or_else(|| history_index(scope, history));
    if current_index == target_index {
        return Some(NavigationTraversalPlan::ResolveCurrentEntry(owner));
    }
    Some(NavigationTraversalPlan::Traverse(TraversalTarget {
        owner,
        history,
        current_index,
        target_index,
        joint_step: None,
    }))
}

pub(super) fn history_delta_traversal_plan<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    delta: i64,
) -> Option<JointTraversalPlan<'s>> {
    let owner = runtime_window_owner(scope, history);
    let host = unsafe { &mut *crate::util::context_host_ptr_from_global_bridge(scope)? };
    let binding = super::session_history::binding(scope, host, owner);
    // Further requests made through the outgoing popup Document's History
    // are discarded when that Document is replaced. Do not rebase them onto
    // the destination's projected history while its resource load waits.
    if let Some(popup_id) = binding.popup
        && host.lightweight_popup_has_pending_history_traversal(popup_id)
    {
        return None;
    }
    let model = host.session_histories.get_mut(binding.popup).clone();
    let source = host
        .pending_joint_history_step(&model)
        .unwrap_or_else(|| model.current_step());
    let step = model.step_by_delta_from(source, delta)?;
    JointTraversalPlan::resolve(scope, owner, step)
}
