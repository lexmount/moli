use super::*;

/// The renderer has already created this exact initial Page synchronously.
/// Protocol adoption consumes its reservation without constructing a second
/// Document or transferring any V8 handle between threads.
#[derive(Clone, Debug)]
pub struct RendererPendingAuxiliaryPage {
    inner: Arc<PendingAuxiliaryPageReservation>,
}

#[derive(Debug)]
struct PendingAuxiliaryPageReservation {
    reservation: RendererPageReservationToken,
    popup_id: u64,
    runtime: Arc<dyn super::page::RendererRuntimeLease>,
}

impl Drop for PendingAuxiliaryPageReservation {
    fn drop(&mut self) {
        // Cancellation is owner-local and idempotent after adoption. No V8
        // handles or PageVm destructors run on the protocol thread.
        self.runtime.cancel_pending_auxiliary_page(self.reservation);
    }
}

impl PartialEq for RendererPendingAuxiliaryPage {
    fn eq(&self, other: &Self) -> bool {
        self.page_reservation() == other.page_reservation() && self.popup_id() == other.popup_id()
    }
}

impl Eq for RendererPendingAuxiliaryPage {}

impl RendererPendingAuxiliaryPage {
    pub fn page_reservation(&self) -> RendererPageReservationToken {
        self.inner.reservation
    }

    pub fn popup_id(&self) -> u64 {
        self.inner.popup_id
    }

    pub fn renderer_runtime(&self) -> JsRuntime {
        self.inner.runtime.clone().into_runtime()
    }
}

pub(crate) struct RendererRelatedInitialEmptyPageInit {
    pub(crate) dom_host: crate::dom::native::DomHost,
    pub(crate) loader: ResourceRequestClient,
    pub(crate) env: PageVmEnvConfig,
    pub(crate) inherited_origin: String,
    pub(crate) inherited_security_token: v8::Global<v8::Value>,
    pub(crate) opener: v8::Global<v8::Object>,
    pub(crate) window: RendererAuxiliaryWindow,
    pub(crate) name: RendererBrowsingContextName,
}

#[derive(Clone)]
pub(crate) struct RendererAuxiliaryPageAllocator {
    owner: owner_local_store::RendererOwnerLocalContext,
    source_page: PageId,
}

impl RendererAuxiliaryPageAllocator {
    pub(in crate::runtime) fn new(
        owner: owner_local_store::RendererOwnerLocalContext,
        source_page: PageId,
    ) -> Self {
        Self { owner, source_page }
    }

    pub(crate) fn stage_in_scope<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        source_environment: &crate::script_vm::RendererPageScriptEnvironment,
        source_bindings: &crate::native_bridge::bindings::NativeBridgeBindings,
        init: RendererRelatedInitialEmptyPageInit,
    ) -> Result<(RendererPendingAuxiliaryPage, v8::Local<'s, v8::Object>)> {
        ensure!(
            source_environment.page_id() == self.source_page.as_u64(),
            "an auxiliary Page must use its exact creator's allocator"
        );
        let page_id = PageId::new(
            self.owner
                .owner_state
                .next_page_id
                .fetch_add(1, Ordering::Relaxed),
        );
        let pending = RendererPendingAuxiliaryPage {
            inner: Arc::new(PendingAuxiliaryPageReservation {
                reservation: RendererPageReservationToken::new(self.owner.local_host_id, page_id),
                popup_id: init.window.id(),
                runtime: self
                    .owner
                    .owner_state
                    .runtime_handle
                    .get()
                    .and_then(super::page::RendererProducerShutdownHandle::upgrade_runtime)
                    .ok_or_else(|| anyhow!("auxiliary creator renderer is shutting down"))?,
            }),
        };
        let environment =
            owner_local_store::stage_related_initial_empty_page_on_bound_owner_local_store(
                &self.owner,
                scope,
                &pending,
                source_environment,
                source_bindings,
                init,
            )?;
        let window = environment.window_proxy_in_scope(scope)?;
        Ok((pending, window))
    }
}
