use crate::conn::{
    CdpConnection, DedicatedWorkerTargetState, RendererPageResidenceIdentity,
    ServiceWorkerTargetState, SharedWorkerTargetState,
    TargetServiceWorkerProtocolAttachmentIdentity, TargetSharedWorkerProtocolAttachmentIdentity,
};
use moli_core::RendererOutputResidenceIdentity;

use super::CdpSessionRoute;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetWorkerProtocolAttachmentIdentity {
    SharedOrDedicated(TargetSharedWorkerProtocolAttachmentIdentity),
    Service(TargetServiceWorkerProtocolAttachmentIdentity),
}

impl TargetWorkerProtocolAttachmentIdentity {
    pub(crate) fn session_id(&self) -> &str {
        match self {
            Self::SharedOrDedicated(identity) => identity.session_id(),
            Self::Service(identity) => identity.session_id(),
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        match self {
            Self::SharedOrDedicated(identity) => identity.target_id(),
            Self::Service(identity) => identity.target_id(),
        }
    }

    pub(crate) fn is_current(&self) -> bool {
        match self {
            Self::SharedOrDedicated(identity) => identity.is_current(),
            Self::Service(identity) => identity.is_current(),
        }
    }
}

impl CdpConnection {
    /// Captures the exact live Worker attachment addressed by one renderer
    /// output stream and DevTools session.
    ///
    /// A browser-context stream owner is intentionally insufficient here: a
    /// context can contain many workers and Pages. Matching the concrete
    /// residence prevents a late response from following the active Page or a
    /// replacement Worker that happens to reuse connection-local ids.
    pub(crate) fn worker_protocol_attachment_identity_for_renderer_output(
        &self,
        session_id: &str,
        residence: RendererOutputResidenceIdentity,
    ) -> Option<TargetWorkerProtocolAttachmentIdentity> {
        match self.session_route(Some(session_id))? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let RendererOutputResidenceIdentity::SharedWorker {
                    browser_context_runtime_id,
                    instance_id,
                } = residence
                else {
                    return None;
                };
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                if !browser_context
                    .routes_renderer_browser_context_runtime(browser_context_runtime_id)
                {
                    return None;
                }
                let target = browser_context.shared_worker_target(&target_id)?;
                if target.renderer_instance_id.as_u64() != instance_id {
                    return None;
                }
                target
                    .protocol_attachment_identity(&browser_context_id, session_id)
                    .map(TargetWorkerProtocolAttachmentIdentity::SharedOrDedicated)
            }
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let renderer_page = RendererPageResidenceIdentity::from_residence(residence)?;
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                let target = browser_context.dedicated_worker_target(&target_id)?;
                let owner_target_id = target.owner_page.target_id()?;
                let owner_target = browser_context.page_target(owner_target_id)?;
                if !owner_target
                    .runtime_slot()
                    .routes_current_renderer_page_owner(
                        renderer_page,
                        target.owner_page.page_attachment_id(),
                    )
                {
                    return None;
                }
                target
                    .protocol_attachment_identity(&browser_context_id, session_id)
                    .map(TargetWorkerProtocolAttachmentIdentity::SharedOrDedicated)
            }
            CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let RendererOutputResidenceIdentity::ServiceWorker {
                    browser_context_runtime_id,
                    version_id,
                } = residence
                else {
                    return None;
                };
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                if !browser_context
                    .routes_renderer_browser_context_runtime(browser_context_runtime_id)
                {
                    return None;
                }
                let target = browser_context.service_worker_target(&target_id)?;
                if target.renderer_version_id != version_id {
                    return None;
                }
                target
                    .protocol_attachment_identity(&browser_context_id, session_id)
                    .map(TargetWorkerProtocolAttachmentIdentity::Service)
            }
            CdpSessionRoute::Browser
            | CdpSessionRoute::BrowserContext { .. }
            | CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::PageTarget { .. } => None,
        }
    }

    pub(crate) fn shared_worker_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&SharedWorkerTargetState> {
        let session_id = session_id?;
        match self.session_route(Some(session_id))? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .shared_worker_target(&target_id),
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .dedicated_worker_target(&target_id)
                .map(|target| &target.inner),
            _ => None,
        }
    }

    pub(crate) fn shared_worker_target_for_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut SharedWorkerTargetState> {
        let session_id = session_id?;
        match self.session_route(Some(session_id))? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id_mut(&browser_context_id)?
                .shared_worker_target_mut(&target_id),
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id_mut(&browser_context_id)?
                .dedicated_worker_target_mut(&target_id)
                .map(|target| &mut target.inner),
            _ => None,
        }
    }

    pub(crate) fn dedicated_worker_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&DedicatedWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id(&browser_context_id)?
            .dedicated_worker_target(&target_id)
    }

    pub(crate) fn dedicated_worker_target_for_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut DedicatedWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id_mut(&browser_context_id)?
            .dedicated_worker_target_mut(&target_id)
    }

    pub(crate) fn service_worker_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&ServiceWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id(&browser_context_id)?
            .service_worker_target(&target_id)
    }

    pub(crate) fn service_worker_target_for_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut ServiceWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id_mut(&browser_context_id)?
            .service_worker_target_mut(&target_id)
    }
}
