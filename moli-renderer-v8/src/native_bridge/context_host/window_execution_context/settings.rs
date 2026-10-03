use super::super::JsContextHost;
use super::super::OwnerDispatchScope;
use super::WindowExecutionContextIdentity;
use crate::document_runtime::DomHandle;
use moli_url::WebOrigin;
use std::rc::Rc;
use url::Url;

/// Document-backed settings for pure APIs in a retained Window realm.
///
/// Unlike an execution-context binding, these survive owner retirement. They
/// grant no authority to dispatch tasks or start network requests. The native
/// Document remains in the host's DOM arena after iframe removal; retaining its
/// identity lets subsequent calls observe live base changes without consulting
/// a reused iframe handle. No V8 Global is held here, so this slot does not keep
/// its own Context alive.
pub(crate) struct WindowEnvironmentSettings {
    document: DomHandle,
    origin: WebOrigin,
    execution_identity: WindowExecutionContextIdentity,
}

impl WindowEnvironmentSettings {
    pub(crate) fn bind_current_child_document(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        child_handle: DomHandle,
    ) -> Option<()> {
        // A reused element can point to a new Window. Only a currently
        // registered realm may change its associated Document; a retained old
        // realm keeps its settings even when the same handle becomes live.
        let execution_identity = host
            .current_runtime_window_execution_context_identity_for_dispatch_scope(
                scope,
                OwnerDispatchScope::Child(child_handle),
            )?;
        let document = host.child_browsing_context_document_handle(child_handle)?;
        let owner = host.frame_owner_current_child_snapshot(child_handle)?;
        let origin = WebOrigin::from_serialized(&owner.settings.origin);
        let context = scope.get_current_context();
        let _previous = context.set_slot(Rc::new(Self {
            document,
            origin,
            execution_identity,
        }));
        Some(())
    }

    pub(crate) fn for_current_realm(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<Self>> {
        scope.get_current_context().get_slot::<Self>()
    }

    pub(crate) fn document_handle(&self) -> DomHandle {
        self.document
    }

    pub(crate) fn api_base_url(&self, host: &JsContextHost) -> Option<Url> {
        let document = host.dom_host().node(self.document)?.as_document()?;
        // Preserve the existing bootstrap view while an initial empty Document
        // still stands in for a locally available child snapshot. An explicit
        // base on the associated Document always wins. Consult the live route
        // only for this exact realm and Document, never a reused element.
        if document.base_element_url().is_none()
            && host.window_execution_context_identity_is_current(self.execution_identity)
            && let OwnerDispatchScope::Child(handle) = self.execution_identity.dispatch_scope()
            && host.child_browsing_context_document_handle(handle) == Some(self.document)
            && host
                .frame_owner_store
                .current_child_document_creation_kind(handle)
                .is_some_and(|kind| kind.is_initial_empty())
        {
            return host.child_browsing_context_base_url(handle);
        }
        Some(document.base_url().clone())
    }

    pub(crate) fn origin(&self) -> &WebOrigin {
        &self.origin
    }
}
