use super::BrowserContext;
use crate::browser::DocumentHandle;

impl BrowserContext {
    pub fn worker_fetch_pause(
        &self,
        pause: crate::page::RendererWorkerFetchPause,
    ) -> Option<crate::browser::WorkerFetchPause> {
        self.network_requests.worker_pause(&pause)
    }

    pub fn start_worker_fetch_decision(
        &mut self,
        pause: crate::browser::WorkerFetchPause,
        decision: crate::page::WorkerFetchDecision,
    ) -> Result<crate::page::PendingWorkerFetchDecision, String> {
        self.network_requests
            .start_worker_decision(&pause, decision)
    }

    pub(in crate::browser) fn worker_fetch_policy_document(
        &self,
        worker: &crate::page::RendererWorkerIdentity,
        policy_document: Option<(
            crate::RendererOwnerLocalHostId,
            crate::page::RendererDocumentToken,
        )>,
    ) -> Option<DocumentHandle> {
        let (renderer, root) = if let Some((owner, document)) = policy_document {
            (
                crate::browser::RendererPageResidenceIdentity::from_parts(owner, document.page_id),
                Some(document),
            )
        } else {
            let mut worker = worker.clone();
            loop {
                match worker {
                    crate::page::RendererWorkerIdentity::Dedicated(instance) => {
                        match &self.dedicated_workers.get(&instance)?.info.owner {
                            crate::page::RendererDedicatedWorkerOwner::Document {
                                owner_local_host_id,
                                page_id,
                            } => {
                                break (
                                    crate::browser::RendererPageResidenceIdentity::from_parts(
                                        *owner_local_host_id,
                                        *page_id,
                                    ),
                                    None,
                                );
                            }
                            crate::page::RendererDedicatedWorkerOwner::Worker(parent) => {
                                worker = parent.clone()
                            }
                        }
                    }
                    crate::page::RendererWorkerIdentity::Shared(instance) => {
                        self.shared_workers.get(&instance)?;
                        let (owner, document) = self
                            .renderer_runtime()
                            .shared_worker_client_document(instance)?;
                        break (
                            crate::browser::RendererPageResidenceIdentity::from_parts(
                                owner,
                                document.page_id,
                            ),
                            Some(document),
                        );
                    }
                    crate::page::RendererWorkerIdentity::Service { .. } => return None,
                }
            }
        };
        let document = self.network_document_for_renderer(renderer)?;
        // Only a live policy holder can pause work. A reserved or replaced
        // Document is not an alternative owner for a surviving SharedWorker.
        let contents = self.web_contents(document.web_contents()).ok()?;
        if self.document_handle(document.web_contents()).ok()? != Some(document)
            || !contents.fetch_subresource_interception().0
            || root.is_some_and(|root| {
                self.document_lifecycle_snapshot(document)
                    .ok()
                    .flatten()
                    .is_none_or(|snapshot| snapshot.document != root)
            })
        {
            return None;
        }
        Some(document)
    }

    pub(in crate::browser) fn install_network_handler(
        &self,
        handler: impl Fn(crate::page::RendererNetworkInput) + Send + Sync + 'static,
    ) {
        self.renderer_runtime().install_network_handler(handler);
    }

    pub(in crate::browser) fn network_document_for_renderer(
        &self,
        renderer: crate::browser::RendererPageResidenceIdentity,
    ) -> Option<DocumentHandle> {
        self.web_contents_handles().find_map(|handle| {
            let contents = self.web_contents(handle).ok()?;
            if let Some(document) = contents.main_frame.current_document.as_ref()
                && crate::browser::RendererPageResidenceIdentity::from_page(&document.page)
                    == renderer
            {
                return Some(DocumentHandle::new(handle, document.id));
            }
            contents
                .navigation()
                .reserved_document_for_renderer(renderer)
                .map(|document| DocumentHandle::new(handle, document))
        })
    }
}
