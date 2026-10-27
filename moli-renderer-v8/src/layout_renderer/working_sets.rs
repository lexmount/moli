use std::collections::{HashMap, HashSet};

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

/// Realtime computation state follows exact Document ownership. Mock and
/// OnDemand never install this storage; published geometry remains separate.
#[derive(Default)]
pub(crate) struct NativeLayoutWorkingSets {
    documents: HashMap<DomHandle, NativeDocumentWorkingSet>,
}

impl NativeLayoutWorkingSets {
    pub(crate) fn clear(&mut self) {
        self.documents = HashMap::new();
    }

    pub(crate) fn retire_all_documents(&mut self) {
        self.clear();
    }

    pub(crate) fn retire_owner(&mut self, owner: crate::native_bridge::WindowDocumentOwner) {
        self.documents
            .retain(|_, entry| entry.owner.target.owner() != owner);
    }

    pub(crate) fn prepare_pass(
        &mut self,
        owners: impl IntoIterator<Item = NativeLayoutWorkingSetOwner>,
    ) -> NativeLayoutWorkingSetPass {
        let owners: HashMap<_, _> = owners
            .into_iter()
            .map(|owner| (owner.document, owner))
            .collect();
        self.documents
            .retain(|document, entry| owners.get(document) == Some(&entry.owner));
        NativeLayoutWorkingSetPass {
            owners,
            previous: std::mem::take(&mut self.documents),
            completed: HashMap::new(),
            visited: HashSet::new(),
        }
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

/// A failed or abandoned screen pass drops its checked-out computation state.
/// Only successful, owner-validated publication installs completed documents.
pub(crate) struct NativeLayoutWorkingSetPass {
    owners: HashMap<DomHandle, NativeLayoutWorkingSetOwner>,
    previous: HashMap<DomHandle, NativeDocumentWorkingSet>,
    completed: HashMap<DomHandle, NativeDocumentWorkingSet>,
    visited: HashSet<DomHandle>,
}

impl NativeLayoutWorkingSetPass {
    pub(super) fn take_document(
        &mut self,
        document: DomHandle,
        root: DomHandle,
    ) -> Option<NativeDocumentWorkingSet> {
        let owner = *self.owners.get(&document)?;
        if owner.root != root || !self.visited.insert(document) {
            return None;
        }
        Some(
            self.previous
                .remove(&document)
                .unwrap_or_else(|| NativeDocumentWorkingSet {
                    owner,
                    working_set: LayoutWorkingSet::default(),
                }),
        )
    }

    pub(super) fn complete_document(&mut self, entry: NativeDocumentWorkingSet) {
        let document = entry.owner.document;
        if self.owners.get(&document) != Some(&entry.owner)
            || !self.visited.contains(&document)
            || entry.working_set.stats().retained_boxes == 0
        {
            return;
        }
        assert!(
            self.completed.insert(document, entry).is_none(),
            "a document completes only once per layout pass"
        );
    }

    pub(crate) fn finish(self, destination: &mut NativeLayoutWorkingSets) {
        assert!(
            destination.documents.is_empty(),
            "pass owns the checked-out state"
        );
        // Unvisited old entries are dropped along with `previous`; hidden or
        // rootless frames do not survive merely because an old tree existed.
        destination.documents = self.completed;
    }
}
