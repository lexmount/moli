use anyhow::{Result, anyhow};

use crate::{
    capture_resources::{CaptureResourceEndpoint, CaptureResourceFetch, CaptureResourceKind},
    document_runtime::DomHandle,
    native_bridge::{OwnerDispatchScope, WindowDocumentTaskTarget},
    types::PendingCaptureResource,
};

use super::ScriptVm;

impl ScriptVm {
    /// Start one capture-only resource with the discovery owner's authority.
    /// Context entry is synchronous and never checkpoints or pumps Page tasks.
    pub(crate) fn start_capture_resource(
        &mut self,
        document: DomHandle,
        target: WindowDocumentTaskTarget,
        kind: CaptureResourceKind,
        url: url::Url,
        entry_id: usize,
        endpoint: CaptureResourceEndpoint,
    ) -> Result<Option<u64>> {
        let child_realm = {
            let host = self._context_host.borrow();
            if host
                .capture_layout_identity_for_document(document)
                .is_none_or(|identity| identity.target != target)
            {
                return Err(anyhow!(
                    "capture resource discovery owner is no longer current"
                ));
            }
            match target.dispatch_scope() {
                OwnerDispatchScope::Top => None,
                OwnerDispatchScope::Child(child) => Some(
                    host.frame_owner_current_child_snapshot(child)
                        .filter(|snapshot| snapshot.document_handle == document)
                        .and_then(|snapshot| snapshot.realm_id)
                        .ok_or_else(|| {
                            anyhow!("capture child Document has no current request realm")
                        })?,
                ),
                OwnerDispatchScope::LightweightPopup(_) => {
                    return Err(anyhow!(
                        "capture resource request requires a frame Document realm"
                    ));
                }
            }
        };
        let start = move |scope: &mut v8::PinScope<'_, '_>,
                          host_ptr: *mut crate::native_bridge::JsContextHost| {
            let host = unsafe { &mut *host_ptr };
            // Selecting a context is not itself permission to use a later
            // Document installed at the same frame address.
            if host
                .capture_layout_identity_for_document(document)
                .is_none_or(|identity| identity.target != target)
            {
                return Err(anyhow!(
                    "capture resource owner changed before request start"
                ));
            }
            let service_worker_client_id =
                host.service_worker_client_id_for_subresource_owner(target.dispatch_scope());
            crate::network_host::start_capture_resource_fetch(
                scope,
                host,
                url,
                PendingCaptureResource {
                    kind,
                    target,
                    service_worker_client_id,
                    entry_id,
                    endpoint,
                },
            )
            .map_err(anyhow::Error::msg)
        };
        if let Some(realm) = child_realm {
            self.with_frame_realm_scope(realm, start)
        } else {
            self.with_default_context_scope(start)
        }
    }

    pub(crate) fn cancel_capture_resource_fetches(
        &mut self,
        fetches: &[CaptureResourceFetch],
        endpoint: &CaptureResourceEndpoint,
    ) {
        self._context_host
            .borrow_mut()
            .cancel_capture_resource_fetches(fetches, endpoint);
    }
}
