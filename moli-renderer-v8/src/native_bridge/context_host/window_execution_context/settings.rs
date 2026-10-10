use super::super::JsContextHost;
use super::super::OwnerDispatchScope;
use super::super::window_security_tokens::WindowAccessOrigin;
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
    access_origin: WindowAccessOrigin,
    domain_owner_document: DomHandle,
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
        let access_origin = host.child_window_access_origin(child_handle)?;
        let domain_owner_document = host
            .child_document_domain_owner_document(child_handle)
            .unwrap_or(document);
        let context = scope.get_current_context();
        let _previous = context.set_slot(Rc::new(Self {
            document,
            origin,
            access_origin,
            domain_owner_document,
            execution_identity,
        }));
        Some(())
    }

    pub(crate) fn for_current_realm(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<Self>> {
        scope.get_current_context().get_slot::<Self>()
    }

    pub(crate) fn bind_current_main_document_for_retirement(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
    ) -> Option<()> {
        let context = scope.get_current_context();
        let execution_identity =
            host.window_execution_context_identity_for_access_check(context)?;
        if execution_identity.dispatch_scope() != OwnerDispatchScope::Top
            || !host.window_execution_context_identity_is_current(execution_identity)
        {
            return None;
        }
        let access_origin = host.main_window_access_origin()?;
        let _previous = context.set_slot(Rc::new(Self {
            document: host.document_handle(),
            origin: WebOrigin::from_serialized(&host.main_document_security_origin()),
            access_origin,
            domain_owner_document: host.document_handle(),
            execution_identity,
        }));
        Some(())
    }

    /// Pure DOM access still uses the outgoing Document's origin after its
    /// task registry entry is retired. This grants no execution authority.
    pub(in crate::native_bridge::context_host) fn can_access_current_window(
        &self,
        host: &JsContextHost,
        target_host: &JsContextHost,
        target: WindowExecutionContextIdentity,
    ) -> bool {
        if !target_host.window_execution_context_identity_is_current(target) {
            return false;
        }
        if self.execution_identity.grants_universal_access() {
            return true;
        }
        // Retiring a resource loader must not turn an inherited origin into
        // the origin of its about:blank URL. Keep that origin, while observing
        // domain changes on the exact Document that owns the shared origin.
        let mut origin = self.access_origin.clone();
        if let WindowAccessOrigin::Tuple {
            document_domain, ..
        } = &mut origin
        {
            if self.domain_owner_document == host.document_handle() {
                *document_domain = host.document_domain_override.borrow().clone();
            } else if let Some(handle) =
                host.child_browsing_context_handle_for_stored_document(self.domain_owner_document)
            {
                *document_domain = host.child_browsing_context_document_domain_override(handle);
            }
        }
        let Some(target_origin) =
            target_host.window_access_origin_for_dispatch_scope(target.dispatch_scope())
        else {
            return false;
        };
        // Opaque owner IDs are scoped to a host, whereas tuple origins can be
        // compared across Pages sharing the browser's V8 isolate.
        if !std::ptr::eq(host, target_host)
            && (matches!(origin, WindowAccessOrigin::Opaque { .. })
                || matches!(target_origin, WindowAccessOrigin::Opaque { .. }))
        {
            return false;
        }
        origin.can_access(&target_origin)
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
