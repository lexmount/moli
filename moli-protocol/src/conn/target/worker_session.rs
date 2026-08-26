use crate::conn::{
    CdpConnection, DedicatedWorkerTargetState, ServiceWorkerTargetState, SharedWorkerTargetState,
    TargetSharedWorkerProtocolAttachmentIdentity,
};

use super::CdpSessionRoute;

impl CdpConnection {
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

    /// Captures the exact renderer worker and protocol attachment addressed by
    /// `session_id`.
    ///
    /// The attachment scope lives with the SharedWorker target's per-session
    /// state. Holding this weak identity across a publication-capture boundary does
    /// not keep a normally detached session alive and cannot be rebound by a
    /// later current-session lookup.
    pub(crate) fn shared_worker_protocol_attachment_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetSharedWorkerProtocolAttachmentIdentity> {
        let session_id = session_id?;
        let CdpSessionRoute::SharedWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id(&browser_context_id)?
            .shared_worker_target(&target_id)?
            .protocol_attachment_identity(&browser_context_id, session_id)
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
