use super::super::JsContextHost;
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{FrameDocumentTaskOwner, FrameRealmId, FrameRealmMaterializationRequest},
    stylesheet_blocking::{connected_preload_like_link_url, link_rel_includes_token},
};

pub(in crate::native_bridge::context_host) struct PendingChildParserPreload {
    child: DomHandle,
    document: DomHandle,
    owner: FrameDocumentTaskOwner,
    realm: FrameRealmId,
    link: DomHandle,
}

impl JsContextHost {
    #[cfg(test)]
    pub(crate) fn pending_child_parser_preload_count_for_test(&self) -> usize {
        self.pending_child_parser_preloads.len()
    }

    fn is_current_child_parser_preload(
        &self,
        child: DomHandle,
        document: DomHandle,
        owner: FrameDocumentTaskOwner,
        link: DomHandle,
    ) -> bool {
        self.frame_owner_store
            .child_document_task_owner_is_current(child, owner)
            && self.child_browsing_context_document_handle(child) == Some(document)
            && self.dom_host().owner_document_handle(link) == Some(document)
            && self.dom_host().is_connected(link)
            && self
                .dom_host()
                .node(link)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| {
                    element.is_html_element("link")
                        && element.attribute("rel").is_some_and(|rel| {
                            link_rel_includes_token(rel, "preload")
                                && !link_rel_includes_token(rel, "modulepreload")
                                && !link_rel_includes_token(rel, "stylesheet")
                        })
                })
    }

    pub(in crate::native_bridge::context_host) fn queue_child_parser_discovered_preloads(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child: DomHandle,
        document: DomHandle,
        links: &[DomHandle],
    ) {
        let Some(owner) = self.current_child_document_task_owner(child) else {
            return;
        };
        for &link in links {
            if !self.is_current_child_parser_preload(child, document, owner, link)
                || connected_preload_like_link_url(self.dom_host(), link).is_none()
            {
                continue;
            }
            if self.connected_style_document_realm(owner).is_some() {
                self.start_child_parser_preload(scope, child, document, owner, link);
                continue;
            }
            if self
                .pending_child_parser_preloads
                .iter()
                .any(|pending| pending.owner == owner && pending.link == link)
            {
                continue;
            }
            let Some(request) =
                self.request_child_frame_realm_materialization_for_owner(child, owner)
            else {
                continue;
            };
            if matches!(
                request,
                FrameRealmMaterializationRequest::AlreadyMaterialized { .. }
            ) {
                self.start_child_parser_preload(scope, child, document, owner, link);
            } else {
                self.pending_child_parser_preloads
                    .push(PendingChildParserPreload {
                        child,
                        document,
                        owner,
                        realm: request.realm_id(),
                        link,
                    });
            }
        }
    }

    fn start_child_parser_preload(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child: DomHandle,
        document: DomHandle,
        owner: FrameDocumentTaskOwner,
        link: DomHandle,
    ) {
        if !self.is_current_child_parser_preload(child, document, owner, link) {
            return;
        }
        let Some(url) = connected_preload_like_link_url(self.dom_host(), link) else {
            return;
        };
        let Some(check) = self.preload_link_csp_check(link, &url) else {
            return;
        };
        let runtime = self.runtime;
        let host_ptr = self as *mut JsContextHost;
        // The policy snapshot is complete before the mutable runtime borrow.
        // Lifecycle commit below only accesses FrameOwnerStore.
        unsafe { &mut *runtime }.prime_parser_preload_link(scope, host_ptr, link, check);
    }

    pub(crate) fn promote_child_parser_preloads_after_realm_materialization(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) {
        let realm = self.connected_style_document_realm(owner);
        for pending in std::mem::take(&mut self.pending_child_parser_preloads) {
            if pending.child != child || pending.owner != owner {
                self.pending_child_parser_preloads.push(pending);
            } else if realm == Some(pending.realm) {
                self.start_child_parser_preload(
                    scope,
                    child,
                    pending.document,
                    owner,
                    pending.link,
                );
            }
        }
    }

    pub(crate) fn discard_child_parser_preloads(
        &mut self,
        child: DomHandle,
        owner: Option<FrameDocumentTaskOwner>,
    ) -> usize {
        let before = self.pending_child_parser_preloads.len();
        self.pending_child_parser_preloads.retain(|pending| {
            pending.child != child || owner.is_some_and(|owner| pending.owner != owner)
        });
        before - self.pending_child_parser_preloads.len()
    }
}
