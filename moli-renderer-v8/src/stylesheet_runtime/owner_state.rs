use std::{collections::HashMap, sync::Arc};

use crate::frame_owner_model::MainDocumentStyleLoadEventBinding;
use crate::module_runtime::NativeModulepreloadLinkClient;

use super::{ConnectedLoadOperation, DomHandle, LinkStyleState};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(in crate::document_runtime) enum StylesheetOwnerCspDisposition {
    #[default]
    Allowed,
    Blocked,
}

impl StylesheetOwnerCspDisposition {
    pub(in crate::document_runtime) fn from_blocked(blocked: bool) -> Self {
        if blocked {
            Self::Blocked
        } else {
            Self::Allowed
        }
    }

    pub(in crate::document_runtime) fn is_blocked(self) -> bool {
        self == Self::Blocked
    }
}

#[derive(Debug)]
enum ConnectedLoadPhase {
    Pending,
    Completed {
        remaining_source_results: usize,
        event_pending: bool,
    },
}

#[derive(Debug)]
struct ConnectedLoadState {
    operation: Arc<ConnectedLoadOperation>,
    phase: ConnectedLoadPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeModulepreloadPhase {
    Pending,
    EventPosted,
}

#[derive(Debug)]
struct NativeModulepreloadState {
    client: Arc<NativeModulepreloadLinkClient>,
    phase: NativeModulepreloadPhase,
}

#[derive(Debug, Default)]
struct StylesheetOwnerRuntimeState {
    // A link client may coexist with its connected @import operation. The
    // connected and modulepreload slots are mutually exclusive processing
    // modes, while CSP disposition deliberately survives their invalidation.
    connected_load: Option<ConnectedLoadState>,
    native_modulepreload: Option<NativeModulepreloadState>,
    link_style_state: Option<LinkStyleState>,
    csp_disposition: StylesheetOwnerCspDisposition,
}

#[derive(Debug, Default)]
pub(in crate::document_runtime) struct StylesheetOwnerRuntimeStates {
    by_owner: HashMap<DomHandle, StylesheetOwnerRuntimeState>,
}

impl StylesheetOwnerRuntimeStates {
    pub(in crate::document_runtime) fn install_pending_operation(
        &mut self,
        operation: Arc<ConnectedLoadOperation>,
    ) {
        let owner = operation.owner;
        let state = self.by_owner.entry(owner).or_default();
        state.native_modulepreload = None;
        state.connected_load = Some(ConnectedLoadState {
            operation,
            phase: ConnectedLoadPhase::Pending,
        });
    }

    pub(in crate::document_runtime) fn install_pending_native_modulepreload(
        &mut self,
        client: Arc<NativeModulepreloadLinkClient>,
    ) {
        let owner = client.owner();
        let state = self.by_owner.entry(owner).or_default();
        state.connected_load = None;
        state.native_modulepreload = Some(NativeModulepreloadState {
            client,
            phase: NativeModulepreloadPhase::Pending,
        });
    }

    pub(in crate::document_runtime) fn pending_native_modulepreload(
        &self,
        owner: DomHandle,
    ) -> Option<&Arc<NativeModulepreloadLinkClient>> {
        let state = self.by_owner.get(&owner)?.native_modulepreload.as_ref()?;
        (state.phase == NativeModulepreloadPhase::Pending).then_some(&state.client)
    }

    pub(in crate::document_runtime) fn pending_operation(
        &self,
        owner: DomHandle,
    ) -> Option<&Arc<ConnectedLoadOperation>> {
        let state = self.by_owner.get(&owner)?.connected_load.as_ref()?;
        matches!(state.phase, ConnectedLoadPhase::Pending).then_some(&state.operation)
    }

    pub(in crate::document_runtime) fn clear_connected_operation(&mut self, owner: DomHandle) {
        if let Some(state) = self.by_owner.get_mut(&owner) {
            state.connected_load = None;
        }
        self.remove_owner_if_empty(owner);
    }

    pub(in crate::document_runtime) fn clear_async_operations(&mut self, owner: DomHandle) {
        if let Some(state) = self.by_owner.get_mut(&owner) {
            state.connected_load = None;
            state.native_modulepreload = None;
        }
        self.remove_owner_if_empty(owner);
    }

    pub(in crate::document_runtime) fn pending_operations(
        &self,
    ) -> Vec<Arc<ConnectedLoadOperation>> {
        self.by_owner
            .values()
            .filter_map(|state| {
                let connected = state.connected_load.as_ref()?;
                matches!(connected.phase, ConnectedLoadPhase::Pending)
                    .then(|| Arc::clone(&connected.operation))
            })
            .collect()
    }

    pub(in crate::document_runtime) fn accept_completion(
        &mut self,
        operation: &Arc<ConnectedLoadOperation>,
        remaining_source_results: usize,
        event_pending: bool,
    ) -> bool {
        let Some(connected) = self
            .by_owner
            .get_mut(&operation.owner)
            .and_then(|state| state.connected_load.as_mut())
        else {
            return false;
        };
        if !matches!(connected.phase, ConnectedLoadPhase::Pending)
            || !ConnectedLoadOperation::ptr_eq(&connected.operation, operation)
        {
            return false;
        }
        connected.phase = ConnectedLoadPhase::Completed {
            remaining_source_results,
            event_pending,
        };
        self.remove_completed_operation_if_finished(operation.owner);
        true
    }

    pub(in crate::document_runtime) fn accepts_source_result(
        &self,
        operation: &Arc<ConnectedLoadOperation>,
    ) -> bool {
        self.by_owner
            .get(&operation.owner)
            .and_then(|state| state.connected_load.as_ref())
            .is_some_and(|connected| {
                ConnectedLoadOperation::ptr_eq(&connected.operation, operation)
                    && matches!(
                        connected.phase,
                        ConnectedLoadPhase::Completed {
                            remaining_source_results: 1..,
                            ..
                        }
                    )
            })
    }

    pub(in crate::document_runtime) fn consume_source_result(
        &mut self,
        operation: &Arc<ConnectedLoadOperation>,
    ) -> bool {
        let Some(connected) = self
            .by_owner
            .get_mut(&operation.owner)
            .and_then(|state| state.connected_load.as_mut())
        else {
            return false;
        };
        if !ConnectedLoadOperation::ptr_eq(&connected.operation, operation) {
            return false;
        }
        let ConnectedLoadPhase::Completed {
            remaining_source_results,
            ..
        } = &mut connected.phase
        else {
            return false;
        };
        if *remaining_source_results == 0 {
            return false;
        }
        *remaining_source_results -= 1;
        self.remove_completed_operation_if_finished(operation.owner);
        true
    }

    pub(in crate::document_runtime) fn accept_native_modulepreload_completion(
        &mut self,
        client: &Arc<NativeModulepreloadLinkClient>,
    ) -> bool {
        let Some(state) = self
            .by_owner
            .get_mut(&client.owner())
            .and_then(|state| state.native_modulepreload.as_mut())
        else {
            return false;
        };
        if state.phase != NativeModulepreloadPhase::Pending
            || !NativeModulepreloadLinkClient::ptr_eq(&state.client, client)
        {
            return false;
        }
        state.phase = NativeModulepreloadPhase::EventPosted;
        true
    }

    pub(in crate::document_runtime) fn replace_link_state(
        &mut self,
        owner: DomHandle,
        state: LinkStyleState,
    ) -> Option<LinkStyleState> {
        let owner_state = self.by_owner.entry(owner).or_default();
        owner_state.native_modulepreload = None;
        owner_state.link_style_state.replace(state)
    }

    pub(in crate::document_runtime) fn link_state(
        &self,
        owner: DomHandle,
    ) -> Option<&LinkStyleState> {
        self.by_owner.get(&owner)?.link_style_state.as_ref()
    }

    pub(in crate::document_runtime) fn link_state_mut(
        &mut self,
        owner: DomHandle,
    ) -> Option<&mut LinkStyleState> {
        self.by_owner.get_mut(&owner)?.link_style_state.as_mut()
    }

    pub(in crate::document_runtime) fn link_states(
        &self,
    ) -> impl Iterator<Item = (DomHandle, &LinkStyleState)> {
        self.by_owner
            .iter()
            .filter_map(|(owner, state)| state.link_style_state.as_ref().map(|link| (*owner, link)))
    }

    pub(in crate::document_runtime) fn accepts_stylesheet_link_client(
        &self,
        load: &Arc<super::StylesheetLinkClient>,
    ) -> bool {
        self.link_state(load.owner())
            .is_some_and(|state| super::StylesheetLinkClient::ptr_eq(state.active_load(), load))
    }

    pub(in crate::document_runtime) fn invalidate_owner_operations(&mut self, owner: DomHandle) {
        if let Some(state) = self.by_owner.get_mut(&owner) {
            state.connected_load = None;
            state.native_modulepreload = None;
            state.link_style_state = None;
        }
        self.remove_owner_if_empty(owner);
    }

    #[cfg(test)]
    pub(in crate::document_runtime) fn remove_owner(&mut self, owner: DomHandle) {
        self.by_owner.remove(&owner);
    }

    pub(in crate::document_runtime) fn set_csp_disposition(
        &mut self,
        owner: DomHandle,
        disposition: StylesheetOwnerCspDisposition,
    ) -> bool {
        if disposition == StylesheetOwnerCspDisposition::Allowed {
            let Some(state) = self.by_owner.get_mut(&owner) else {
                return false;
            };
            let changed = state.csp_disposition != disposition;
            state.csp_disposition = disposition;
            self.remove_owner_if_empty(owner);
            return changed;
        }
        let state = self.by_owner.entry(owner).or_default();
        let changed = state.csp_disposition != disposition;
        state.csp_disposition = disposition;
        changed
    }

    pub(in crate::document_runtime) fn csp_disposition(
        &self,
        owner: DomHandle,
    ) -> StylesheetOwnerCspDisposition {
        self.by_owner
            .get(&owner)
            .map(|state| state.csp_disposition)
            .unwrap_or_default()
    }

    pub(in crate::document_runtime) fn has_lifecycle_state(&self, owner: DomHandle) -> bool {
        self.by_owner.get(&owner).is_some_and(|state| {
            state.connected_load.is_some()
                || state.native_modulepreload.is_some()
                || state.link_style_state.is_some()
        })
    }

    pub(in crate::document_runtime) fn cancelable_load_event_bindings(
        &self,
        owner: DomHandle,
    ) -> Vec<MainDocumentStyleLoadEventBinding> {
        let Some(state) = self.by_owner.get(&owner) else {
            return Vec::new();
        };
        let mut bindings = Vec::new();
        if let Some(connected) = &state.connected_load
            && matches!(connected.phase, ConnectedLoadPhase::Pending)
            && let Some(binding) = connected.operation.load_event_binding()
        {
            bindings.push(binding);
        }
        if let Some(link) = &state.link_style_state
            && let Some(binding) = link.cancelable_load_event_binding()
        {
            bindings.push(binding);
        }
        bindings
    }

    #[cfg(test)]
    pub(in crate::document_runtime) fn has_owner(&self, owner: DomHandle) -> bool {
        self.by_owner.contains_key(&owner)
    }

    #[cfg(test)]
    pub(in crate::document_runtime) fn has_pending_operation(&self) -> bool {
        self.by_owner.values().any(|state| {
            state
                .connected_load
                .as_ref()
                .is_some_and(|connected| matches!(connected.phase, ConnectedLoadPhase::Pending))
                || state
                    .native_modulepreload
                    .as_ref()
                    .is_some_and(|native| native.phase == NativeModulepreloadPhase::Pending)
        })
    }

    pub(in crate::document_runtime) fn has_pending_connected_operation(&self) -> bool {
        self.by_owner.values().any(|state| {
            state
                .connected_load
                .as_ref()
                .is_some_and(|connected| matches!(connected.phase, ConnectedLoadPhase::Pending))
        })
    }

    pub(in crate::document_runtime) fn has_pending_link_state(&self) -> bool {
        self.link_states().any(|(_, state)| state.is_pending())
    }

    pub(in crate::document_runtime) fn len(&self) -> usize {
        self.by_owner.len()
    }

    #[cfg(test)]
    pub(in crate::document_runtime) fn is_empty(&self) -> bool {
        self.by_owner.is_empty()
    }

    pub(in crate::document_runtime) fn consume_operation_event(
        &mut self,
        operation: &Arc<ConnectedLoadOperation>,
    ) {
        let owner = operation.owner;
        let Some(ConnectedLoadState {
            operation: current,
            phase: ConnectedLoadPhase::Completed { event_pending, .. },
        }) = self
            .by_owner
            .get_mut(&owner)
            .and_then(|state| state.connected_load.as_mut())
        else {
            return;
        };
        if !ConnectedLoadOperation::ptr_eq(current, operation) {
            return;
        }
        *event_pending = false;
        self.remove_completed_operation_if_finished(owner);
    }

    pub(in crate::document_runtime) fn consume_link_event(
        &mut self,
        load: &Arc<super::StylesheetLinkClient>,
    ) -> bool {
        self.by_owner
            .get_mut(&load.owner())
            .and_then(|state| state.link_style_state.as_mut())
            .is_some_and(|state| state.consume_posted_event(load))
    }

    pub(in crate::document_runtime) fn consume_native_modulepreload_event(
        &mut self,
        client: &Arc<NativeModulepreloadLinkClient>,
    ) {
        let owner = client.owner();
        let matches = self
            .by_owner
            .get(&owner)
            .and_then(|state| state.native_modulepreload.as_ref())
            .is_some_and(|state| {
                state.phase == NativeModulepreloadPhase::EventPosted
                    && NativeModulepreloadLinkClient::ptr_eq(&state.client, client)
            });
        if matches && let Some(state) = self.by_owner.get_mut(&owner) {
            state.native_modulepreload = None;
        }
        self.remove_owner_if_empty(owner);
    }

    fn remove_completed_operation_if_finished(&mut self, owner: DomHandle) {
        let finished = self
            .by_owner
            .get(&owner)
            .and_then(|state| state.connected_load.as_ref())
            .is_some_and(|connected| {
                matches!(
                    connected.phase,
                    ConnectedLoadPhase::Completed {
                        remaining_source_results: 0,
                        event_pending: false,
                    }
                )
            });
        if finished && let Some(state) = self.by_owner.get_mut(&owner) {
            state.connected_load = None;
        }
        self.remove_owner_if_empty(owner);
    }

    fn remove_owner_if_empty(&mut self, owner: DomHandle) {
        let empty = self.by_owner.get(&owner).is_some_and(|state| {
            state.connected_load.is_none()
                && state.native_modulepreload.is_none()
                && state.link_style_state.is_none()
                && state.csp_disposition == StylesheetOwnerCspDisposition::Allowed
        });
        if empty {
            self.by_owner.remove(&owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_disposition_survives_operation_invalidation() {
        let owner = DomHandle::new(7);
        let mut states = StylesheetOwnerRuntimeStates::default();

        assert!(states.set_csp_disposition(owner, StylesheetOwnerCspDisposition::Blocked));
        states.invalidate_owner_operations(owner);

        assert_eq!(
            states.csp_disposition(owner),
            StylesheetOwnerCspDisposition::Blocked
        );
        assert!(!states.has_lifecycle_state(owner));
        assert!(states.has_owner(owner));
    }

    #[test]
    fn allowing_or_removing_owner_drops_standalone_csp_state() {
        let owner = DomHandle::new(11);
        let mut states = StylesheetOwnerRuntimeStates::default();

        states.set_csp_disposition(owner, StylesheetOwnerCspDisposition::Blocked);
        assert!(states.set_csp_disposition(owner, StylesheetOwnerCspDisposition::Allowed));
        assert!(!states.has_owner(owner));

        states.set_csp_disposition(owner, StylesheetOwnerCspDisposition::Blocked);
        states.remove_owner(owner);
        assert_eq!(
            states.csp_disposition(owner),
            StylesheetOwnerCspDisposition::Allowed
        );
    }
}
