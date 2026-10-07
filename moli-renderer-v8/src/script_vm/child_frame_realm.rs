use std::collections::BTreeMap;

use super::ChildFrameRealmRecord;
use crate::frame_owner_model::{FrameRealmId, LocalWindowId};

#[derive(Default)]
pub(super) struct ChildFrameRealmStore {
    realms_by_owner_realm_id: BTreeMap<FrameRealmId, ChildFrameRealmRecord>,
    owner_realm_ids_by_context_id: BTreeMap<i64, FrameRealmId>,
}

impl ChildFrameRealmStore {
    pub(super) fn len(&self) -> usize {
        self.realms_by_owner_realm_id.len()
    }

    pub(super) fn execution_context_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.owner_realm_ids_by_context_id.keys().copied()
    }

    pub(super) fn values(
        &self,
    ) -> std::collections::btree_map::Values<'_, FrameRealmId, ChildFrameRealmRecord> {
        self.realms_by_owner_realm_id.values()
    }

    pub(super) fn values_mut(&mut self) -> impl Iterator<Item = &mut ChildFrameRealmRecord> {
        self.realms_by_owner_realm_id.values_mut()
    }

    pub(super) fn iter_by_execution_context_id(
        &self,
    ) -> impl Iterator<Item = (i64, &ChildFrameRealmRecord)> + '_ {
        self.owner_realm_ids_by_context_id
            .iter()
            .filter_map(|(context_id, realm_id)| {
                self.realms_by_owner_realm_id
                    .get(realm_id)
                    .map(|realm| (*context_id, realm))
            })
    }

    pub(super) fn get(&self, context_id: &i64) -> Option<&ChildFrameRealmRecord> {
        let realm_id = self.owner_realm_ids_by_context_id.get(context_id)?;
        self.realms_by_owner_realm_id.get(realm_id)
    }

    pub(super) fn owner_realm_id_for_context_id(&self, context_id: i64) -> Option<FrameRealmId> {
        self.owner_realm_ids_by_context_id.get(&context_id).copied()
    }

    pub(super) fn context_for_owner_realm_id(
        &self,
        realm_id: FrameRealmId,
    ) -> Option<&ChildFrameRealmRecord> {
        self.realms_by_owner_realm_id.get(&realm_id)
    }

    pub(super) fn context_id_for_owner_realm_id(&self, realm_id: FrameRealmId) -> Option<i64> {
        self.realms_by_owner_realm_id
            .get(&realm_id)
            .map(|realm| realm.inspector_execution_context_id)
    }

    pub(super) fn context_id_for_local_window_id(
        &self,
        local_window_id: LocalWindowId,
    ) -> Option<i64> {
        self.realms_by_owner_realm_id
            .values()
            .find(|realm| realm.local_window_id == local_window_id)
            .map(|realm| realm.inspector_execution_context_id)
    }

    pub(super) fn contains_key(&self, context_id: &i64) -> bool {
        self.owner_realm_ids_by_context_id.contains_key(context_id)
    }

    pub(super) fn insert(&mut self, context_id: i64, realm: ChildFrameRealmRecord) {
        debug_assert_eq!(
            context_id, realm.inspector_execution_context_id,
            "child FrameRealm context id index must match the record"
        );
        let owner_realm_id = realm.owner_realm_id;
        assert!(
            !self.realms_by_owner_realm_id.contains_key(&owner_realm_id),
            "child FrameRealm owner already has a live context"
        );
        assert!(
            !self.owner_realm_ids_by_context_id.contains_key(&context_id),
            "child FrameRealm reused a live execution context id"
        );
        assert!(
            self.realms_by_owner_realm_id
                .insert(owner_realm_id, realm)
                .is_none()
        );
        assert!(
            self.owner_realm_ids_by_context_id
                .insert(context_id, owner_realm_id)
                .is_none()
        );
    }

    pub(super) fn remove(&mut self, context_id: &i64) -> Option<ChildFrameRealmRecord> {
        let owner_realm_id = self.owner_realm_ids_by_context_id.remove(context_id)?;
        self.realms_by_owner_realm_id.remove(&owner_realm_id)
    }
}
