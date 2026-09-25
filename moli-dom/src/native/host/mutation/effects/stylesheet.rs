use super::{DomHandle, DomMutationEffects};
use crate::native::host::StylesheetCandidateChanges;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomStylesheetOwnerTransitions {
    pub(super) changes: Vec<DomStylesheetOwnerChange>,
}

impl DomStylesheetOwnerTransitions {
    pub fn changes(&self) -> &[DomStylesheetOwnerChange] {
        &self.changes
    }

    fn push(&mut self, change: DomStylesheetOwnerChange) {
        let preserves_transition_order = matches!(
            &change.kind,
            DomStylesheetOwnerChangeKind::Registered
                | DomStylesheetOwnerChangeKind::Unregistered
                | DomStylesheetOwnerChangeKind::TreeConnectionChanged { .. }
        );
        if preserves_transition_order || !self.changes.contains(&change) {
            self.changes.push(change);
        }
    }

    pub(super) fn merge(&mut self, other: Self) {
        for change in other.changes {
            self.push(change);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomStylesheetOwnerChange {
    pub(super) owner: DomHandle,
    pub(super) kind: DomStylesheetOwnerChangeKind,
    pub(super) tree_scopes: DomStylesheetOwnerTreeScopes,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DomStylesheetOwnerTreeScopes {
    pub(super) old: Option<DomHandle>,
    pub(super) current: Option<DomHandle>,
    pub(super) new: Option<DomHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomStylesheetOwnerChangeKind {
    Registered,
    Unregistered,
    Attribute {
        namespace: Option<String>,
        local_name: String,
    },
    Contents,
    /// The parser has finished constructing this style element's children.
    ParsingFinished,
    OwnerDocumentChanged,
    TreeConnectionChanged {
        connected: bool,
    },
}

impl DomStylesheetOwnerChange {
    pub(in crate::native::host) fn registered(owner: DomHandle, tree_scope: DomHandle) -> Self {
        Self {
            owner,
            kind: DomStylesheetOwnerChangeKind::Registered,
            tree_scopes: DomStylesheetOwnerTreeScopes::from_parts(None, None, Some(tree_scope)),
        }
    }

    pub(in crate::native::host) fn unregistered(owner: DomHandle, tree_scope: DomHandle) -> Self {
        Self {
            owner,
            kind: DomStylesheetOwnerChangeKind::Unregistered,
            tree_scopes: DomStylesheetOwnerTreeScopes::from_parts(Some(tree_scope), None, None),
        }
    }

    pub(in crate::native::host) fn owner_document_changed(
        owner: DomHandle,
        current_tree_scope: Option<DomHandle>,
    ) -> Self {
        Self {
            owner,
            kind: DomStylesheetOwnerChangeKind::OwnerDocumentChanged,
            tree_scopes: DomStylesheetOwnerTreeScopes::current(current_tree_scope),
        }
    }

    pub(in crate::native::host) fn tree_connection_changed(
        owner: DomHandle,
        connected: bool,
        current_tree_scope: Option<DomHandle>,
    ) -> Self {
        Self {
            owner,
            kind: DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected },
            tree_scopes: DomStylesheetOwnerTreeScopes::current(current_tree_scope),
        }
    }

    pub fn owner(&self) -> DomHandle {
        self.owner
    }

    pub fn kind(&self) -> &DomStylesheetOwnerChangeKind {
        &self.kind
    }

    pub fn tree_scopes(&self) -> DomStylesheetOwnerTreeScopes {
        self.tree_scopes
    }
}

impl DomStylesheetOwnerTreeScopes {
    pub(in crate::native::host) fn from_parts(
        old: Option<DomHandle>,
        current: Option<DomHandle>,
        new: Option<DomHandle>,
    ) -> Self {
        Self { old, current, new }
    }

    pub(super) fn current(current: Option<DomHandle>) -> Self {
        Self::from_parts(None, current, None)
    }

    pub fn old(self) -> Option<DomHandle> {
        self.old
    }

    pub fn current_scope(self) -> Option<DomHandle> {
        self.current
    }

    pub fn new_scope(self) -> Option<DomHandle> {
        self.new
    }

    pub fn iter(self) -> impl Iterator<Item = DomHandle> {
        [self.old, self.current, self.new].into_iter().flatten()
    }
}

impl DomMutationEffects {
    pub(in crate::native::host::mutation) fn extend_stylesheet_candidate_changes(
        &mut self,
        changes: StylesheetCandidateChanges,
    ) {
        for change in changes.into_owner_changes() {
            self.mark_stylesheet_owner_change(change);
        }
    }

    pub(in crate::native::host::mutation) fn extend_stylesheet_owner_changes(
        &mut self,
        changes: impl IntoIterator<Item = DomStylesheetOwnerChange>,
    ) {
        for change in changes {
            self.mark_stylesheet_owner_change(change);
        }
    }

    pub(in crate::native::host::mutation) fn mark_stylesheet_owner_attribute_change(
        &mut self,
        owner: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
        current_tree_scope: Option<DomHandle>,
    ) {
        self.mark_stylesheet_owner_change(DomStylesheetOwnerChange {
            owner,
            kind: DomStylesheetOwnerChangeKind::Attribute {
                namespace: namespace.map(str::to_owned),
                local_name: local_name.to_owned(),
            },
            tree_scopes: DomStylesheetOwnerTreeScopes::current(current_tree_scope),
        });
    }

    pub(in crate::native::host::mutation) fn mark_stylesheet_owner_contents_change(
        &mut self,
        owner: DomHandle,
        current_tree_scope: Option<DomHandle>,
    ) {
        self.mark_stylesheet_owner_change(DomStylesheetOwnerChange {
            owner,
            kind: DomStylesheetOwnerChangeKind::Contents,
            tree_scopes: DomStylesheetOwnerTreeScopes::current(current_tree_scope),
        });
    }

    pub(in crate::native::host::mutation) fn mark_stylesheet_owner_parsing_finished(
        &mut self,
        owner: DomHandle,
        current_tree_scope: Option<DomHandle>,
    ) {
        self.mark_stylesheet_owner_change(DomStylesheetOwnerChange {
            owner,
            kind: DomStylesheetOwnerChangeKind::ParsingFinished,
            tree_scopes: DomStylesheetOwnerTreeScopes::current(current_tree_scope),
        });
    }

    pub(super) fn mark_stylesheet_owner_change(&mut self, change: DomStylesheetOwnerChange) {
        self.changed = true;
        self.stylesheet_owners.push(change);
    }
}
