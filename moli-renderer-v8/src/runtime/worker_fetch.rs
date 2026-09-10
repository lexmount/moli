use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{
    RendererSyntheticResponseBody,
    network::loads::ResourceLoadLease,
    protocol_types::{
        PendingSubresourceAuthInfo, PendingSubresourceFetchInfo, PendingSubresourceResponseInfo,
        SubresourceAuthCredentials, SubresourceNetworkRequestHandle,
    },
    worker::WorkerMessage,
};

/// Immutable facts about one pause, shared by the native owner and its observers.
/// The resource and its JS continuation remain in the physical Worker.
#[derive(Debug)]
pub enum RendererWorkerFetchStage {
    Request(Box<PendingSubresourceFetchInfo>),
    Auth(Box<PendingSubresourceAuthInfo>),
    Response(Box<PendingSubresourceResponseInfo>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerFetchPhase {
    Request,
    Auth,
    Response,
}

impl RendererWorkerFetchStage {
    pub(crate) fn phase(&self) -> WorkerFetchPhase {
        match self {
            Self::Request(_) => WorkerFetchPhase::Request,
            Self::Auth(_) => WorkerFetchPhase::Auth,
            Self::Response(_) => WorkerFetchPhase::Response,
        }
    }

    pub fn internal_id(&self) -> u64 {
        match self {
            Self::Request(info) => info.internal_id,
            Self::Auth(info) => info.internal_id,
            Self::Response(info) => info.internal_id,
        }
    }
}

/// No protocol identity, Page lookup or executable callback enters a decision.
#[derive(Debug)]
pub enum WorkerFetchDecision {
    ContinueRequest {
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    },
    ContinueResponse {
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    },
    ProvideAuth(SubresourceAuthCredentials),
    CancelAuth,
    Fail(String),
    Fulfill {
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    /// Neutral release when an observer or its policy disappears.
    Release,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum WorkerFetchTarget {
    Fetch(u32),
    Xhr(u32),
    CspReport(u32),
}

#[derive(Debug)]
pub(crate) struct WorkerFetchDecisionDispatch {
    pub(crate) target: WorkerFetchTarget,
    pub(crate) handle: SubresourceNetworkRequestHandle,
    pub(crate) phase: WorkerFetchPhase,
    pub(crate) decision: WorkerFetchDecision,
    pub(crate) reply: Option<oneshot::Sender<Result<(), String>>>,
}

#[derive(Debug)]
struct WorkerFetchControl {
    sender: mpsc::UnboundedSender<WorkerMessage>,
    cancellation: crate::network::loads::ResourceLoadCancellationObserver,
}

#[derive(Debug)]
struct WorkerFetchPauseInner {
    network: super::RendererWorkerNetworkReporter,
    document_url: Url,
    stage: RendererWorkerFetchStage,
    target: WorkerFetchTarget,
    handle: SubresourceNetworkRequestHandle,
    control: Mutex<Option<WorkerFetchControl>>,
}

impl WorkerFetchPauseInner {
    fn decide(
        &self,
        decision: WorkerFetchDecision,
        reply: Option<oneshot::Sender<Result<(), String>>>,
    ) -> Result<(), String> {
        let phase = self.stage.phase();
        let valid = match &decision {
            WorkerFetchDecision::ContinueRequest { .. } => phase == WorkerFetchPhase::Request,
            WorkerFetchDecision::ContinueResponse { .. } => phase == WorkerFetchPhase::Response,
            WorkerFetchDecision::ProvideAuth(_) | WorkerFetchDecision::CancelAuth => {
                phase == WorkerFetchPhase::Auth
            }
            WorkerFetchDecision::Fail(_)
            | WorkerFetchDecision::Fulfill { .. }
            | WorkerFetchDecision::Release => true,
        };
        if !valid {
            return Err("Worker request is paused at a different stage".into());
        }
        let control = self
            .control
            .lock()
            .take()
            .ok_or("Worker request pause is no longer available")?;
        if control.cancellation.is_cancelled() {
            return Err("Worker request was canceled".into());
        }
        control
            .sender
            .send(WorkerMessage::DecideInterceptedRequest(Box::new(
                WorkerFetchDecisionDispatch {
                    target: self.target,
                    handle: self.handle,
                    phase,
                    decision,
                    reply,
                },
            )))
            .map_err(|_| "Worker request owner is unavailable".into())
    }
}

impl Drop for WorkerFetchPauseInner {
    fn drop(&mut self) {
        // The last observer is not a second request owner. A forgotten pause
        // releases its one decision; it cannot retain a Worker thread or Page.
        let _ = self.decide(WorkerFetchDecision::Release, None);
    }
}

/// One single-use stage capability. Clones share the same decision slot, and
/// the sender is fixed before publication rather than resolved on completion.
#[derive(Clone, Debug)]
pub struct RendererWorkerFetchPause(Arc<WorkerFetchPauseInner>);

impl PartialEq for RendererWorkerFetchPause {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RendererWorkerFetchPause {}

impl RendererWorkerFetchPause {
    pub(crate) fn new(
        stage: RendererWorkerFetchStage,
        document_url: Url,
        target: WorkerFetchTarget,
        handle: SubresourceNetworkRequestHandle,
        load: ResourceLoadLease,
        sender: mpsc::UnboundedSender<WorkerMessage>,
        network: super::RendererWorkerNetworkReporter,
    ) -> Self {
        Self(Arc::new(WorkerFetchPauseInner {
            network,
            document_url,
            stage,
            target,
            handle,
            control: Mutex::new(Some(WorkerFetchControl {
                sender,
                cancellation: load.cancellation_observer(),
            })),
        }))
    }

    pub fn stage(&self) -> &RendererWorkerFetchStage {
        &self.0.stage
    }

    pub fn document_url(&self) -> &Url {
        &self.0.document_url
    }

    pub(crate) fn report(
        &self,
        policy_document: Option<(
            super::RendererOwnerLocalHostId,
            super::RendererDocumentToken,
        )>,
    ) -> super::RendererNetworkObservation {
        self.0.network.report_pause(self.clone(), policy_document)
    }

    pub(crate) fn renderer_transport_charge_bytes(&self) -> usize {
        let stage = match self.stage() {
            RendererWorkerFetchStage::Request(info) => info.renderer_transport_charge_bytes(),
            RendererWorkerFetchStage::Auth(info) => info.renderer_transport_charge_bytes(),
            RendererWorkerFetchStage::Response(info) => info.renderer_transport_charge_bytes(),
        };
        stage.saturating_add(self.document_url().as_str().len().saturating_mul(2))
    }

    pub fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.0.handle
    }

    pub fn worker(&self) -> &super::RendererWorkerIdentity {
        self.0.network.identity()
    }

    pub fn is_available(&self) -> bool {
        self.0.control.lock().as_ref().is_some_and(|control| {
            !control.cancellation.is_cancelled() && !control.sender.is_closed()
        })
    }

    pub fn start_decision(
        &self,
        decision: WorkerFetchDecision,
    ) -> Result<PendingWorkerFetchDecision, String> {
        let (reply, completion) = oneshot::channel();
        self.0.decide(decision, Some(reply))?;
        Ok(PendingWorkerFetchDecision { completion })
    }

    pub fn release(&self) {
        let _ = self.0.decide(WorkerFetchDecision::Release, None);
    }

    pub fn invalidate(&self) {
        self.0.control.lock().take();
    }
}

pub struct PendingWorkerFetchDecision {
    completion: oneshot::Receiver<Result<(), String>>,
}

impl PendingWorkerFetchDecision {
    pub async fn wait(self) -> Result<(), String> {
        self.completion
            .await
            .map_err(|_| "Worker request owner is unavailable".to_owned())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_pause() -> (
        crate::network::ResourceRequestClientOwner,
        ResourceLoadLease,
        RendererWorkerFetchPause,
        mpsc::UnboundedReceiver<WorkerMessage>,
    ) {
        let client =
            crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
                .unwrap();
        let load = crate::network::loads::resource_load_lease_for_test(client.handle(), None);
        let handle = SubresourceNetworkRequestHandle::allocate();
        let (sender, receiver) = mpsc::unbounded_channel();
        let document_url = Url::parse("https://worker.test/script.js").unwrap();
        let info = PendingSubresourceFetchInfo {
            internal_id: handle.get(),
            network_request_handle: Some(handle),
            frame_id: None,
            document_url: document_url.clone(),
            url: document_url.join("request").unwrap(),
            websocket_socket_id: None,
            method: "GET".into(),
            request_headers: Vec::new(),
            request_body_bytes: None,
            request_body: None,
            resource_type: crate::protocol_types::SubresourceResourceType::Fetch,
            request_cookie_report: None,
        };
        let pause = RendererWorkerFetchPause::new(
            RendererWorkerFetchStage::Request(Box::new(info)),
            document_url,
            WorkerFetchTarget::Fetch(1),
            handle,
            load.clone(),
            sender,
            crate::runtime::RendererWorkerNetworkReporter::unobserved_for_test(),
        );
        (client, load, pause, receiver)
    }

    #[tokio::test]
    async fn stage_validation_preserves_single_use_authority_and_exact_worker_sender() {
        let (_client, _load, pause, mut receiver) = request_pause();
        let (_other_client, _other_load, other, mut other_receiver) = request_pause();
        assert!(
            pause
                .start_decision(WorkerFetchDecision::ContinueResponse {
                    response_code: None,
                    response_headers: None,
                })
                .is_err()
        );
        assert!(pause.is_available());
        let clone = pause.clone();
        let pending = pause
            .start_decision(WorkerFetchDecision::Fail("expected".into()))
            .unwrap();
        assert!(!clone.is_available());
        assert!(clone.start_decision(WorkerFetchDecision::Release).is_err());
        let WorkerMessage::DecideInterceptedRequest(dispatch) = receiver.try_recv().unwrap() else {
            panic!("decision must reach the original Worker");
        };
        assert_eq!(dispatch.handle, pause.handle());
        assert!(matches!(dispatch.target, WorkerFetchTarget::Fetch(1)));
        assert!(
            matches!(dispatch.decision, WorkerFetchDecision::Fail(ref error) if error == "expected")
        );
        dispatch.reply.unwrap().send(Ok(())).unwrap();
        pending.wait().await.unwrap();
        assert!(
            other.is_available(),
            "same local Fetch id in another Worker is independent"
        );
        assert!(other_receiver.try_recv().is_err());
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn native_rejection_releases_pause_even_while_source_fifo_retains_its_observation() {
        let (_client, _load, pause, mut receiver) = request_pause();
        let observation = pause.report(None);
        assert!(
            !pause.is_available(),
            "an absent native owner must not park the Worker"
        );
        let WorkerMessage::DecideInterceptedRequest(dispatch) = receiver.try_recv().unwrap() else {
            panic!("rejected input must release the physical request");
        };
        assert!(matches!(dispatch.decision, WorkerFetchDecision::Release));
        assert!(observation.committed().await.is_none());
        drop(pause);
        assert!(
            receiver.try_recv().is_err(),
            "release is single-use across all observers"
        );
    }

    #[tokio::test]
    async fn final_observer_drop_releases_but_invalidation_does_not_resume_retired_work() {
        let (_client, _load, pause, mut receiver) = request_pause();
        let clone = pause.clone();
        drop(pause);
        assert!(receiver.try_recv().is_err());
        drop(clone);
        let WorkerMessage::DecideInterceptedRequest(dispatch) = receiver.try_recv().unwrap() else {
            panic!("final observer must release");
        };
        assert!(matches!(dispatch.decision, WorkerFetchDecision::Release));
        let (_client, _load, pause, mut receiver) = request_pause();
        pause.invalidate();
        assert!(!pause.is_available());
        assert!(pause.start_decision(WorkerFetchDecision::Release).is_err());
        drop(pause);
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn load_cancellation_revokes_all_pause_clones_without_resuming_work() {
        let (_client, load, pause, mut receiver) = request_pause();
        let clone = pause.clone();
        load.cancel();
        assert!(!pause.is_available());
        assert!(clone.start_decision(WorkerFetchDecision::Release).is_err());
        drop(pause);
        drop(clone);
        assert!(receiver.try_recv().is_err());
    }
}
