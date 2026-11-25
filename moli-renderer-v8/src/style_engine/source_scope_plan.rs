use indexmap::IndexSet;

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    source_dirty::StyleSourceDirtyReason,
    source_id::{StyleScopeId, StyleSourceId},
};

#[derive(Default)]
pub(super) struct StyleSourceScopeCleanupPlan {
    full_documents: IndexSet<DomHandle>,
    scoped_entries_by_document: Vec<StyleSourceScopeCleanupPlanEntry>,
}

pub(super) struct StyleSourceScopeCleanupPlanEntry {
    document: DomHandle,
    scope_id: StyleScopeId,
    reason: StyleSourceDirtyReason,
    source_ids: IndexSet<StyleSourceId>,
    scoped_roots: IndexSet<DomHandle>,
}

impl StyleSourceScopeCleanupPlan {
    pub(super) fn document_adopted_stylesheets(
        document: DomHandle,
        source_ids: impl IntoIterator<Item = StyleSourceId>,
    ) -> Self {
        let mut plan = Self::default();
        plan.add_scoped_roots(
            document,
            StyleScopeId::Document(document),
            StyleSourceDirtyReason::DocumentAdoptedStyleSheets,
            source_ids,
            [document],
        );
        plan
    }

    pub(super) fn owner_stylesheet(host: &DomHost, owner: DomHandle) -> Self {
        let mut plan = Self::default();
        plan.add_owner_source(
            host,
            owner,
            StyleSourceDirtyReason::OwnerStyleSheet,
            StyleSourceId::owner_style_sheet(host, owner),
        );
        plan
    }

    pub(super) fn linked_stylesheet_owners(
        host: &DomHost,
        owners: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        let mut plan = Self::default();
        for owner in owners {
            plan.add_owner_source(
                host,
                owner,
                StyleSourceDirtyReason::LinkedStyleSheet,
                StyleSourceId::linked_style_sheet(host, owner),
            );
        }
        plan
    }

    pub(super) fn shadow_root_adopted_stylesheets(
        host: &DomHost,
        root: DomHandle,
        source_ids: impl IntoIterator<Item = StyleSourceId>,
    ) -> Self {
        let mut plan = Self::default();
        let Some(document) = owner_document_for_source_owner(host, root) else {
            return plan;
        };
        let roots = shadow_root_scope_roots(host, root);
        if roots.is_empty() {
            plan.add_full_document(document);
        } else {
            plan.add_scoped_roots(
                document,
                StyleScopeId::ShadowRoot(root),
                StyleSourceDirtyReason::ShadowRootAdoptedStyleSheets,
                source_ids,
                roots,
            );
        }
        plan
    }

    pub(super) fn full_documents(&self) -> impl Iterator<Item = DomHandle> + '_ {
        self.full_documents.iter().copied()
    }

    pub(super) fn scoped_entries_by_document(
        self,
    ) -> impl Iterator<Item = StyleSourceScopeCleanupPlanEntry> {
        let full_documents = self.full_documents;
        self.scoped_entries_by_document
            .into_iter()
            .filter(move |entry| !full_documents.contains(&entry.document))
    }

    fn add_owner_source(
        &mut self,
        host: &DomHost,
        owner: DomHandle,
        reason: StyleSourceDirtyReason,
        source_id: Option<StyleSourceId>,
    ) {
        let Some(document) = owner_document_for_source_owner(host, owner) else {
            return;
        };
        let Some(source_id) = source_id else {
            self.add_full_document(document);
            return;
        };
        let roots = source_id.scope_id.style_roots(host);
        if roots.is_empty() {
            self.add_full_document(document);
        } else {
            self.add_scoped_roots(document, source_id.scope_id, reason, [source_id], roots);
        }
    }

    fn add_full_document(&mut self, document: DomHandle) {
        self.full_documents.insert(document);
    }

    fn add_scoped_roots(
        &mut self,
        document: DomHandle,
        scope_id: StyleScopeId,
        reason: StyleSourceDirtyReason,
        source_ids: impl IntoIterator<Item = StyleSourceId>,
        roots: impl IntoIterator<Item = DomHandle>,
    ) {
        if self.full_documents.contains(&document) {
            return;
        }
        let entry = scoped_entry_for_document_scope_and_reason(
            &mut self.scoped_entries_by_document,
            document,
            scope_id,
            reason,
        );
        entry.source_ids.extend(source_ids);
        entry.scoped_roots.extend(roots);
    }
}

impl StyleSourceScopeCleanupPlanEntry {
    pub(super) fn scope_id(&self) -> StyleScopeId {
        self.scope_id
    }

    pub(super) fn reason(&self) -> StyleSourceDirtyReason {
        self.reason
    }

    pub(super) fn document(&self) -> DomHandle {
        self.document
    }

    pub(super) fn source_ids(&self) -> impl Iterator<Item = StyleSourceId> + '_ {
        self.source_ids.iter().cloned()
    }

    pub(super) fn roots(&self) -> &IndexSet<DomHandle> {
        &self.scoped_roots
    }
}

fn owner_document_for_source_owner(host: &DomHost, owner: DomHandle) -> Option<DomHandle> {
    host.owner_document_handle(owner)
}

fn scoped_entry_for_document_scope_and_reason(
    scoped_entries_by_document: &mut Vec<StyleSourceScopeCleanupPlanEntry>,
    document: DomHandle,
    scope_id: StyleScopeId,
    reason: StyleSourceDirtyReason,
) -> &mut StyleSourceScopeCleanupPlanEntry {
    if let Some(index) = scoped_entries_by_document.iter().position(|entry| {
        entry.document == document && entry.scope_id == scope_id && entry.reason == reason
    }) {
        return &mut scoped_entries_by_document[index];
    }
    scoped_entries_by_document.push(StyleSourceScopeCleanupPlanEntry {
        document,
        scope_id,
        reason,
        source_ids: IndexSet::new(),
        scoped_roots: IndexSet::new(),
    });
    scoped_entries_by_document
        .last_mut()
        .expect("scoped entry was just pushed and must exist")
}

fn shadow_root_scope_roots(host: &DomHost, root: DomHandle) -> IndexSet<DomHandle> {
    let mut roots = IndexSet::new();
    if host.is_shadow_root(root) {
        roots.insert(root);
        if let Some(shadow_host) = host.shadow_root_host(root) {
            roots.insert(shadow_host);
        }
    }
    roots
}
