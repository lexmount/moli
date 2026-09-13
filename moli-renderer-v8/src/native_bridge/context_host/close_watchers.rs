use super::{
    DomHandle, JsContextHost, OwnerDispatchScope, WindowExecutionContextIdentity,
    WindowExecutionContextOwner,
};
use crate::native_bridge::WindowUserActivationState;
use std::rc::Rc;

struct RegisteredCloseWatcher {
    object: v8::Global<v8::Object>,
    realm: crate::native_bridge::RuntimeObservableContextToken,
}

/// Window-owned state shared by its realms. An active watcher stays reachable
/// until it is destroyed or its exact LocalWindow execution owner is retired.
pub(super) struct CloseWatcherManager {
    groups: Vec<Vec<RegisteredCloseWatcher>>,
    allowed_groups: usize,
    next_interaction_allows_group: bool,
    history_action_activation: bool,
    user_activation: Rc<WindowUserActivationState>,
}

impl Default for CloseWatcherManager {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            allowed_groups: 1,
            next_interaction_allows_group: true,
            history_action_activation: false,
            user_activation: Rc::default(),
        }
    }
}

impl JsContextHost {
    pub(crate) fn notify_close_watcher_input_activation(&mut self, handle: DomHandle) {
        if let Some(source) = self.owner_dispatch_scope_for_node(handle) {
            self.notify_close_watcher_user_activation(source);
        }
    }

    pub(crate) fn register_close_watcher<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        identity: WindowExecutionContextIdentity,
        watcher: v8::Local<'s, v8::Object>,
    ) {
        let manager = self
            .close_watcher_managers
            .entry(identity.owner())
            .or_default();
        let watcher = RegisteredCloseWatcher {
            object: v8::Global::new(scope, watcher),
            realm: identity.realm_token(),
        };
        if manager.groups.len() < manager.allowed_groups {
            manager.groups.push(vec![watcher]);
        } else {
            manager
                .groups
                .last_mut()
                .expect("at least one allowed group")
                .push(watcher);
        }
        manager.next_interaction_allows_group = true;
    }

    pub(crate) fn remove_close_watcher<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: WindowExecutionContextOwner,
        watcher: v8::Local<'s, v8::Object>,
    ) {
        let Some(manager) = self.close_watcher_managers.get_mut(&owner) else {
            return;
        };
        for group in &mut manager.groups {
            group.retain(|candidate| {
                !v8::Local::new(scope, &candidate.object).strict_equals(watcher.into())
            });
        }
        manager.groups.retain(|group| !group.is_empty());
    }

    pub(crate) fn close_watchers_to_process<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: WindowExecutionContextOwner,
    ) -> Vec<v8::Local<'s, v8::Object>> {
        self.close_watcher_managers
            .get(&owner)
            .and_then(|manager| manager.groups.last())
            .map(|group| {
                group
                    .iter()
                    .rev()
                    .map(|watcher| v8::Local::new(scope, &watcher.object))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn close_watcher_can_prevent_close(
        &self,
        owner: WindowExecutionContextOwner,
    ) -> bool {
        self.close_watcher_managers
            .get(&owner)
            .is_some_and(|manager| {
                manager.groups.len() < manager.allowed_groups && manager.history_action_activation
            })
    }

    pub(super) fn retire_close_watcher_realm(
        &mut self,
        realm: crate::native_bridge::RuntimeObservableContextToken,
    ) {
        for manager in self.close_watcher_managers.values_mut() {
            for group in &mut manager.groups {
                group.retain(|watcher| watcher.realm != realm);
            }
            manager.groups.retain(|group| !group.is_empty());
        }
    }

    pub(crate) fn finish_close_watcher_processing(&mut self, owner: WindowExecutionContextOwner) {
        if let Some(manager) = self.close_watcher_managers.get_mut(&owner) {
            manager.allowed_groups = manager.allowed_groups.saturating_sub(1).max(1);
        }
    }

    pub(crate) fn notify_close_watcher_user_activation(&mut self, source: OwnerDispatchScope) {
        let live = match source {
            OwnerDispatchScope::Top => self.current_main_document_task_owner().is_some(),
            OwnerDispatchScope::Child(handle) => self.child_browsing_context_is_live(handle),
            OwnerDispatchScope::LightweightPopup(id) => self.lightweight_popup_is_open(id),
        };
        if !live {
            return;
        }
        let origin = self.window_access_origin_for_dispatch_scope(source);
        for target in self.close_watcher_scopes_in_tree(source) {
            let notify = self.close_watcher_scope_descends_from(source, target)
                || (self.close_watcher_scope_descends_from(target, source)
                    && origin
                        .as_ref()
                        .zip(
                            self.window_access_origin_for_dispatch_scope(target)
                                .as_ref(),
                        )
                        .is_some_and(|(source, target)| source.has_same_origin(target)));
            if notify && let Some(owner) = self.current_window_execution_context_owner(target) {
                let manager = self.close_watcher_managers.entry(owner).or_default();
                if manager.next_interaction_allows_group {
                    manager.allowed_groups = manager.allowed_groups.saturating_add(1);
                }
                manager.next_interaction_allows_group = false;
                manager.history_action_activation = true;
                manager.user_activation.notify();
            }
        }
    }

    pub(crate) fn window_has_transient_user_activation(&self, source: OwnerDispatchScope) -> bool {
        self.window_user_activation_state(source).0
    }

    pub(crate) fn window_user_activation_state(&self, source: OwnerDispatchScope) -> (bool, bool) {
        self.current_window_execution_context_owner(source)
            .and_then(|owner| self.close_watcher_managers.get(&owner))
            .map(|manager| manager.user_activation.state())
            .unwrap_or((false, false))
    }

    pub(crate) fn retain_window_user_activation_state(
        &mut self,
        source: OwnerDispatchScope,
    ) -> Option<Rc<WindowUserActivationState>> {
        let owner = self.current_window_execution_context_owner(source)?;
        Some(
            self.close_watcher_managers
                .entry(owner)
                .or_default()
                .user_activation
                .clone(),
        )
    }

    pub(crate) fn consume_close_watcher_history_activation(&mut self, source: OwnerDispatchScope) {
        for target in self.close_watcher_scopes_in_tree(source) {
            if let Some(owner) = self.current_window_execution_context_owner(target)
                && let Some(manager) = self.close_watcher_managers.get_mut(&owner)
            {
                manager.history_action_activation = false;
            }
        }
    }

    fn close_watcher_parent_scope(&self, target: OwnerDispatchScope) -> Option<OwnerDispatchScope> {
        let OwnerDispatchScope::Child(handle) = target else {
            return None;
        };
        Some(
            self.child_browsing_context_parent_handle(handle)
                .map(OwnerDispatchScope::Child)
                .or_else(|| {
                    self.child_browsing_context_popup_owner_id(handle)
                        .map(OwnerDispatchScope::LightweightPopup)
                })
                .unwrap_or(OwnerDispatchScope::Top),
        )
    }

    fn close_watcher_scope_descends_from(
        &self,
        mut target: OwnerDispatchScope,
        ancestor: OwnerDispatchScope,
    ) -> bool {
        loop {
            if target == ancestor {
                return true;
            }
            let Some(parent) = self.close_watcher_parent_scope(target) else {
                return false;
            };
            target = parent;
        }
    }

    fn close_watcher_scopes_in_tree(&self, source: OwnerDispatchScope) -> Vec<OwnerDispatchScope> {
        let mut top = source;
        while let Some(parent) = self.close_watcher_parent_scope(top) {
            top = parent;
        }
        let mut scopes = vec![top];
        scopes.extend(
            self.live_child_browsing_context_owner_snapshots()
                .into_iter()
                .map(|(handle, _)| OwnerDispatchScope::Child(handle))
                .filter(|target| self.close_watcher_scope_descends_from(*target, top)),
        );
        scopes
    }
}
