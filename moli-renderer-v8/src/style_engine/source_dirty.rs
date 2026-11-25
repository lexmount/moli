use std::cell::RefCell;

use indexmap::IndexSet;

use crate::document_runtime::DomHandle;

use super::source_id::{StyleScopeId, StyleSourceId};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StyleSourceDirtyScopeSnapshot {
    records: Vec<StyleSourceDirtyScopeRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleSourceDirtyScopeRecord {
    scope_id: StyleScopeId,
    reason: StyleSourceDirtyReason,
    source_ids: IndexSet<StyleSourceId>,
    scoped_roots: IndexSet<DomHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum StyleSourceDirtyReason {
    DocumentStyleSheets,
    DocumentAdoptedStyleSheets,
    OwnerStyleSheet,
    LinkedStyleSheet,
    ShadowRootAdoptedStyleSheets,
    CustomPropertyRegistration,
}

#[derive(Default)]
pub(super) struct StyleSourceDirtyScopes {
    records: RefCell<Vec<StyleSourceDirtyScopeRecord>>,
}

impl StyleSourceDirtyScopes {
    pub(super) fn record_scope(
        &self,
        scope_id: StyleScopeId,
        reason: StyleSourceDirtyReason,
        source_ids: impl IntoIterator<Item = StyleSourceId>,
        roots: impl IntoIterator<Item = DomHandle>,
    ) {
        let scoped_roots = roots.into_iter().collect::<IndexSet<_>>();
        self.records.borrow_mut().push(StyleSourceDirtyScopeRecord {
            scope_id,
            reason,
            source_ids: source_ids.into_iter().collect(),
            scoped_roots,
        });
    }

    pub(super) fn snapshot(&self) -> StyleSourceDirtyScopeSnapshot {
        StyleSourceDirtyScopeSnapshot {
            records: self.records.borrow().clone(),
        }
    }

    pub(super) fn clear(&self) {
        self.records.borrow_mut().clear();
    }
}

impl StyleSourceDirtyScopeSnapshot {
    pub(super) fn requires_retained_style_update(&self) -> bool {
        !self.records.is_empty()
    }

    pub(super) fn scoped_roots_vec(&self) -> Vec<DomHandle> {
        self.scoped_roots().into_iter().collect()
    }

    pub(super) fn source_ids_vec(&self) -> Vec<StyleSourceId> {
        self.source_ids().into_iter().collect()
    }

    pub(super) fn scope_ids_vec(&self) -> Vec<StyleScopeId> {
        self.scope_ids().into_iter().collect()
    }

    pub(super) fn reasons_vec(&self) -> Vec<StyleSourceDirtyReason> {
        self.reasons().into_iter().collect()
    }

    pub(super) fn records_vec(&self) -> Vec<StyleSourceDirtyScopeRecord> {
        self.records.clone()
    }

    pub(super) fn refreshes_document_stylesheets(&self, document: DomHandle) -> bool {
        self.records.iter().any(|record| {
            record.scope_id == StyleScopeId::Document(document)
                && !matches!(
                    record.reason,
                    StyleSourceDirtyReason::CustomPropertyRegistration
                )
        })
    }

    pub(super) fn dirty_shadow_roots(&self) -> IndexSet<DomHandle> {
        self.records
            .iter()
            .filter_map(|record| match record.scope_id {
                StyleScopeId::ShadowRoot(root) => Some(root),
                StyleScopeId::Document(_) => None,
            })
            .collect()
    }

    pub(super) fn refreshes_custom_property_registrations(&self, document: DomHandle) -> bool {
        self.records.iter().any(|record| {
            record.scope_id == StyleScopeId::Document(document)
                && record.reason == StyleSourceDirtyReason::CustomPropertyRegistration
        })
    }

    pub(super) fn scoped_roots(&self) -> IndexSet<DomHandle> {
        self.records
            .iter()
            .flat_map(|record| record.scoped_roots.iter().copied())
            .collect()
    }

    fn source_ids(&self) -> IndexSet<StyleSourceId> {
        self.records
            .iter()
            .flat_map(|record| record.source_ids.iter().cloned())
            .collect()
    }

    fn scope_ids(&self) -> IndexSet<StyleScopeId> {
        self.records.iter().map(|record| record.scope_id).collect()
    }

    fn reasons(&self) -> IndexSet<StyleSourceDirtyReason> {
        self.records.iter().map(|record| record.reason).collect()
    }
}
