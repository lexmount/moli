use crate::{document_runtime::DomHandle, dom::native::DomHost};

use moli_selector::stylo_shadow_root_host_participates_in_style_scope as shadow_root_host_participates_in_style_scope;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StyleSourceId {
    pub(super) scope_id: StyleScopeId,
    pub(super) kind: StyleSourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum StyleScopeId {
    Document(DomHandle),
    ShadowRoot(DomHandle),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum StyleSourceKind {
    OwnerStyleSheet { owner: DomHandle },
    LinkedStyleSheet { owner: DomHandle },
    DocumentAdoptedStyleSheet { client_id: u64 },
    ShadowRootAdoptedStyleSheet { client_id: u64 },
}

impl StyleSourceId {
    pub(crate) fn owner_style_sheet(host: &DomHost, owner: DomHandle) -> Option<Self> {
        Some(Self {
            scope_id: StyleScopeId::for_stylesheet_owner(host, owner)?,
            kind: StyleSourceKind::OwnerStyleSheet { owner },
        })
    }

    pub(crate) fn linked_style_sheet(host: &DomHost, owner: DomHandle) -> Option<Self> {
        Some(Self {
            scope_id: StyleScopeId::for_stylesheet_owner(host, owner)?,
            kind: StyleSourceKind::LinkedStyleSheet { owner },
        })
    }

    pub(crate) fn document_adopted_style_sheet(document: DomHandle, client_id: u64) -> Self {
        Self {
            scope_id: StyleScopeId::Document(document),
            kind: StyleSourceKind::DocumentAdoptedStyleSheet { client_id },
        }
    }

    pub(crate) fn shadow_root_adopted_style_sheet(root: DomHandle, client_id: u64) -> Self {
        Self {
            scope_id: StyleScopeId::ShadowRoot(root),
            kind: StyleSourceKind::ShadowRootAdoptedStyleSheet { client_id },
        }
    }
}

impl StyleScopeId {
    fn for_stylesheet_owner(host: &DomHost, owner: DomHandle) -> Option<Self> {
        if let Some(root) = host.containing_shadow_root(owner)
            && shadow_root_host_participates_in_style_scope(host, root)
        {
            return Some(Self::ShadowRoot(root));
        }
        host.owner_document_handle(owner).map(Self::Document)
    }

    pub(super) fn style_roots(self, host: &DomHost) -> Vec<DomHandle> {
        match self {
            Self::Document(document) => {
                if host.node(document).is_some() {
                    vec![document]
                } else {
                    Vec::new()
                }
            }
            Self::ShadowRoot(root) => {
                if host.node(root).is_none()
                    || !shadow_root_host_participates_in_style_scope(host, root)
                {
                    return Vec::new();
                }
                let mut roots = vec![root];
                if let Some(shadow_host) = host.shadow_root_host(root) {
                    roots.push(shadow_host);
                }
                roots
            }
        }
    }
}
