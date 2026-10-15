//! A Document's native Window association is distinct from a Node wrapper's
//! creation Realm and from permission to execute a callback in that Window.

use super::{
    DomHandle, JsContextHost, OwnerDispatchScope, WindowEnvironmentSettings,
    WindowExecutionContextIdentity, WindowExecutionContextOwner,
};
use std::collections::HashMap;

/// Allocate the Document provenance ledger only when an inert factory is used;
/// ordinary Hosts retain a single optional pointer, not a resident map header.
#[derive(Default)]
pub(super) struct InertDocumentExecutionContexts {
    documents: HashMap<DomHandle, Option<DocumentExecutionContext>>,
}

/// Scalar provenance only. Live execution remains authorized by the existing
/// Window/Realm registry; this record never keeps a V8 Context or Host alive.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DocumentExecutionContext {
    owner: WindowExecutionContextOwner,
    dispatch_scope: OwnerDispatchScope,
}

impl From<WindowExecutionContextIdentity> for DocumentExecutionContext {
    fn from(identity: WindowExecutionContextIdentity) -> Self {
        Self {
            owner: identity.owner(),
            dispatch_scope: identity.dispatch_scope(),
        }
    }
}

impl JsContextHost {
    pub(crate) fn document_execution_context_for_node(
        &self,
        node: DomHandle,
    ) -> Option<DocumentExecutionContext> {
        let document = self.dom_host().owner_document_handle(node)?;
        if let Some(source) = self
            .inert_document_execution_contexts
            .as_ref()
            .and_then(|sources| sources.documents.get(&document))
        {
            // Inert documents retain the association even after the Window
            // is destroyed. The original framed Document instead loses its
            // execution context when it leaves the current frame membership.
            return *source;
        }
        if self.page_context_resources_closed {
            return None;
        }
        let dispatch_scope = self.owner_dispatch_scope_for_node(document)?;
        Some(DocumentExecutionContext {
            owner: self.current_window_execution_context_owner(dispatch_scope)?,
            dispatch_scope,
        })
    }

    pub(crate) fn current_realm_document_execution_context(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<DocumentExecutionContext> {
        self.current_runtime_window_execution_context_identity(scope)
            .map(Into::into)
            .or_else(|| {
                WindowEnvironmentSettings::for_current_realm(scope).and_then(|settings| {
                    self.document_execution_context_for_node(settings.document_handle())
                })
            })
    }

    /// DOMParser owns a Window, rather than borrowing the current Document's
    /// execution context. A retained parser can still construct inert DOM
    /// after that Window's framed Document has shut down.
    pub(crate) fn current_realm_window_for_inert_document(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<DocumentExecutionContext> {
        self.current_runtime_window_execution_context_identity(scope)
            .map(Into::into)
            .or_else(|| {
                WindowEnvironmentSettings::for_current_realm(scope)
                    .map(|settings| settings.execution_context_identity().into())
            })
    }

    pub(crate) fn set_inert_document_execution_context(
        &mut self,
        document: DomHandle,
        source: Option<DocumentExecutionContext>,
    ) {
        debug_assert!(
            self.dom_host()
                .node(document)
                .is_some_and(|node| node.is_document())
        );
        self.inert_document_execution_contexts
            .get_or_insert_with(Default::default)
            .documents
            .insert(document, source);
    }

    pub(super) fn forget_inert_document_execution_context(&mut self, document: DomHandle) {
        if let Some(sources) = self.inert_document_execution_contexts.as_mut() {
            sources.documents.remove(&document);
            if sources.documents.is_empty() {
                self.inert_document_execution_contexts = None;
            }
        }
    }

    pub(crate) fn node_event_execution_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        node: DomHandle,
    ) -> Option<v8::Local<'s, v8::Context>> {
        let source = self.document_execution_context_for_node(node)?;
        if !self.window_execution_context_owner_is_current(source.owner, source.dispatch_scope) {
            return None;
        }
        let (_, context) =
            self.window_execution_context(scope, source.owner, source.dispatch_scope)?;
        let identity = self.window_execution_context_identity_for_access_check(context)?;
        self.window_execution_context_identity_is_current(identity)
            .then_some(context)
    }
}
