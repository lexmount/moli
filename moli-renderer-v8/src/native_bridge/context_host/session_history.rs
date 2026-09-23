use super::OwnerDispatchScope;
use moli_session_history::{JointSessionHistory, SessionHistoryContextId};
use std::collections::HashMap;

/// Each traversable owns one history. Lightweight popup Windows share a V8
/// host with their opener, but have independent traversables.
#[derive(Default)]
pub(crate) struct RendererSessionHistories {
    main: JointSessionHistory,
    popups: HashMap<u64, JointSessionHistory>,
    contexts: HashMap<OwnerDispatchScope, SessionHistoryContextId>,
}

impl RendererSessionHistories {
    pub(crate) fn entry_is_current(
        &self,
        owner: OwnerDispatchScope,
        entry: &moli_session_history::SessionHistoryEntry,
    ) -> bool {
        let Some(context) = self.contexts.get(&owner).copied() else {
            return false;
        };
        self.main.entry(context) == Some(entry)
            || self
                .popups
                .values()
                .any(|history| history.entry(context) == Some(entry))
    }

    pub(crate) fn detach(&mut self, owner: OwnerDispatchScope) {
        if let Some(context) = self.contexts.remove(&owner) {
            self.main.detach(context);
            for history in self.popups.values_mut() {
                history.detach(context);
            }
        }
    }
    pub(crate) fn get_mut(&mut self, popup: Option<u64>) -> &mut JointSessionHistory {
        match popup {
            None => &mut self.main,
            Some(id) => self.popups.entry(id).or_default(),
        }
    }

    pub(crate) fn context(&mut self, owner: OwnerDispatchScope) -> SessionHistoryContextId {
        match owner {
            OwnerDispatchScope::Top | OwnerDispatchScope::LightweightPopup(_) => {
                SessionHistoryContextId::ROOT
            }
            OwnerDispatchScope::Child(_) => *self
                .contexts
                .entry(owner)
                .or_insert_with(SessionHistoryContextId::allocate),
        }
    }

    pub(crate) fn owner(
        &self,
        context: SessionHistoryContextId,
        popup: Option<u64>,
    ) -> Option<OwnerDispatchScope> {
        if context == SessionHistoryContextId::ROOT {
            return Some(popup.map_or(
                OwnerDispatchScope::Top,
                OwnerDispatchScope::LightweightPopup,
            ));
        }
        self.contexts
            .iter()
            .find_map(|(owner, id)| (*id == context).then_some(*owner))
    }
}
