use std::collections::HashMap;

use moli_layout::LayoutWorkingSet;
#[cfg(test)]
use moli_layout::LayoutWorkingSetStats;

use crate::{document_runtime::DomHandle, native_bridge::WindowDocumentTaskTarget};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NativeLayoutWorkingSetOwner {
    pub(crate) document: DomHandle,
    pub(crate) root: DomHandle,
    pub(crate) target: WindowDocumentTaskTarget,
}

pub(super) struct NativeDocumentWorkingSet {
    owner: NativeLayoutWorkingSetOwner,
    pub(super) working_set: LayoutWorkingSet<DomHandle>,
}

/// Realtime's mutable computation state, separate from published geometry.
/// No instance is installed on a Mock or OnDemand Host.
#[derive(Default)]
pub(crate) struct NativeLayoutWorkingSets {
    documents: HashMap<DomHandle, NativeDocumentWorkingSet>,
}

impl NativeLayoutWorkingSets {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn retire_owner(&mut self, owner: crate::native_bridge::WindowDocumentOwner) {
        self.documents
            .retain(|_, entry| entry.owner.target.owner() != owner);
    }

    #[cfg(test)]
    pub(crate) fn stats(&self, document: DomHandle) -> Option<LayoutWorkingSetStats> {
        self.documents
            .get(&document)
            .map(|entry| entry.working_set.stats())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.documents.len()
    }
}

/// A publication owns this transaction until its exact input check succeeds.
/// Previous entries are moved out one at a time while recursive passes lend
/// the remaining map to their children. Only successfully visited documents
/// enter `completed`; hidden, rootless and retired children are dropped even
/// when their Window remains alive.
pub(crate) struct NativeLayoutWorkingSetPass {
    owners: HashMap<DomHandle, NativeLayoutWorkingSetOwner>,
    previous: NativeLayoutWorkingSets,
    completed: NativeLayoutWorkingSets,
    completed_charge: usize,
    max_estimated_retained_bytes: usize,
}

impl NativeLayoutWorkingSetPass {
    pub(crate) fn new(
        previous: NativeLayoutWorkingSets,
        owners: impl IntoIterator<Item = NativeLayoutWorkingSetOwner>,
    ) -> Self {
        Self {
            owners: owners
                .into_iter()
                .map(|owner| (owner.document, owner))
                .collect(),
            previous,
            completed: NativeLayoutWorkingSets::default(),
            completed_charge: 0,
            max_estimated_retained_bytes: 64 * 1024 * 1024,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_retention_limit_for_test(&mut self, max_estimated_retained_bytes: usize) {
        self.max_estimated_retained_bytes = max_estimated_retained_bytes;
    }

    pub(super) fn take_document(
        &mut self,
        document: DomHandle,
        root: DomHandle,
    ) -> Option<NativeDocumentWorkingSet> {
        let owner = *self
            .owners
            .get(&document)
            .filter(|owner| owner.root == root)?;
        Some(
            self.previous
                .documents
                .remove(&document)
                .filter(|entry| entry.owner == owner)
                .unwrap_or_else(|| NativeDocumentWorkingSet {
                    owner,
                    working_set: LayoutWorkingSet::default(),
                }),
        )
    }

    pub(super) fn complete_document(&mut self, mut entry: NativeDocumentWorkingSet) {
        // This is an admission charge, not an allocator or shared-font byte
        // limit. Apply it across the complete Page frame tree rather than
        // multiplying a per-document allowance by the number of iframes.
        let stats = entry.working_set.stats();
        let charge = self
            .completed_charge
            .saturating_add(stats.estimated_retained_bytes);
        if stats.retained_boxes == 0 || charge > self.max_estimated_retained_bytes {
            entry.working_set.clear();
            return;
        }
        self.completed_charge = charge;
        self.completed.documents.insert(entry.owner.document, entry);
    }

    pub(crate) fn finish(self) -> NativeLayoutWorkingSets {
        self.completed
    }
}
