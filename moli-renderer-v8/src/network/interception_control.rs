use std::sync::Arc;

use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use tokio::sync::oneshot;
use url::Url;

#[cfg(any(test, feature = "devtools"))]
use crate::RendererSyntheticResponseBody;
use crate::types::SubresourceRequestInitiatorType;

#[cfg(feature = "devtools")]
mod devtools;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InterceptionPhase {
    Request,
    Response,
    Auth,
}

/// One native pause, rather than a command capability for a Page or realm.
/// Consuming a decision cannot enter V8, run DOM work, or wait for a Page turn.
#[derive(Clone, Debug)]
pub struct ResourceInterceptionPause {
    #[cfg(test)]
    pub(crate) request_policy_for_test:
        Option<(moli_fetch::RequestMode, moli_fetch::RequestCredentialsMode)>,
    #[cfg(any(test, feature = "devtools"))]
    internal_id: u64,
    worker_network: bool,
    phase: InterceptionPhase,
    #[cfg(feature = "devtools")]
    target_policy: crate::network::PageNetworkPolicy,
    request_redirect: Option<Arc<moli_fetch::RedirectInfo>>,
    request_initiator_type: SubresourceRequestInitiatorType,
    response_body: Option<super::ResourceInterceptionBody>,
    sender: Arc<Mutex<Option<oneshot::Sender<ResourceInterceptionDecision>>>>,
}

impl PartialEq for ResourceInterceptionPause {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sender, &other.sender)
    }
}

impl Eq for ResourceInterceptionPause {}

#[derive(Debug)]
pub(crate) enum ResourceInterceptionDecision {
    ContinueRequest {
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<bytes::Bytes>>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
        intercept_response: bool,
        handle_auth_requests: bool,
    },
    ContinueResponse {
        status: Option<u16>,
        headers: Option<Vec<(String, Vec<u8>)>>,
    },
    #[cfg(any(test, feature = "devtools"))]
    Auth(crate::SubresourceAuthCredentials),
    CancelAuth,
    #[cfg(any(test, feature = "devtools"))]
    Fail(String),
    #[cfg(any(test, feature = "devtools"))]
    Fulfill {
        request_overrides: Option<Box<moli_fetch::RequestOverrides>>,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        body: RendererSyntheticResponseBody,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum ResourceInterceptionReply {
    Started,
    Unit,
}

impl ResourceInterceptionPause {
    pub(crate) fn new(
        internal_id: u64,
        phase: InterceptionPhase,
        target_policy: crate::network::PageNetworkPolicy,
    ) -> (Self, oneshot::Receiver<ResourceInterceptionDecision>) {
        #[cfg(not(feature = "devtools"))]
        let _ = target_policy;
        #[cfg(not(any(test, feature = "devtools")))]
        let _ = internal_id;
        let (tx, rx) = oneshot::channel();
        (
            Self {
                #[cfg(test)]
                request_policy_for_test: None,
                #[cfg(any(test, feature = "devtools"))]
                internal_id,
                worker_network: false,
                phase,
                #[cfg(feature = "devtools")]
                target_policy,
                request_redirect: None,
                request_initiator_type: SubresourceRequestInitiatorType::Script,
                response_body: None,
                sender: Arc::new(Mutex::new(Some(tx))),
            },
            rx,
        )
    }

    pub(crate) fn with_request_redirect(
        mut self,
        redirect: Option<moli_fetch::RedirectInfo>,
    ) -> Self {
        self.request_redirect = redirect.map(Arc::new);
        self
    }

    pub(crate) fn with_worker_network(mut self, worker: bool) -> Self {
        self.worker_network = worker;
        self
    }
    pub fn observes_worker_network(&self) -> bool {
        self.worker_network
    }
    pub fn request_redirect(&self) -> Option<&moli_fetch::RedirectInfo> {
        self.request_redirect.as_deref()
    }

    pub(crate) fn with_request_initiator_type(
        mut self,
        initiator: SubresourceRequestInitiatorType,
    ) -> Self {
        self.request_initiator_type = initiator;
        self
    }

    pub fn request_initiator_type(&self) -> SubresourceRequestInitiatorType {
        self.request_initiator_type
    }

    pub(crate) fn with_response_body(mut self, body: super::ResourceInterceptionBody) -> Self {
        self.response_body = Some(body);
        self
    }

    pub fn response_body(&self) -> Option<&super::ResourceInterceptionBody> {
        self.response_body.as_ref()
    }

    /// Resume an unobserved native load without constructing a protocol command.
    pub(crate) fn resume_unobserved(&self) -> bool {
        let decision = match self.phase {
            InterceptionPhase::Request => ResourceInterceptionDecision::ContinueRequest {
                url: None,
                method: None,
                body: None,
                headers: None,
                intercept_response: false,
                handle_auth_requests: false,
            },
            InterceptionPhase::Response => ResourceInterceptionDecision::ContinueResponse {
                status: None,
                headers: None,
            },
            InterceptionPhase::Auth => ResourceInterceptionDecision::CancelAuth,
        };
        self.send_decision(decision).is_ok()
    }

    fn send_decision(&self, decision: ResourceInterceptionDecision) -> Result<()> {
        let sender = self
            .sender
            .lock()
            .take()
            .ok_or_else(|| anyhow!("network pause was already resumed"))?;
        sender
            .send(decision)
            .map_err(|_| anyhow!("network load is no longer waiting"))
    }

    #[cfg(test)]
    pub(crate) fn send_test_decision(
        &self,
        internal_id: u64,
        phase: InterceptionPhase,
        decision: ResourceInterceptionDecision,
    ) {
        assert_eq!((self.internal_id, self.phase), (internal_id, phase));
        self.send_decision(decision).expect("native test decision");
    }
}

static NEXT_SUBRESOURCE_REQUEST_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);
pub(crate) fn next_subresource_request_id() -> u64 {
    NEXT_SUBRESOURCE_REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

static NEXT_SUBRESOURCE_NETWORK_HANDLE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);
pub(crate) fn next_subresource_network_handle() -> crate::types::SubresourceNetworkRequestHandle {
    crate::types::SubresourceNetworkRequestHandle::new(
        NEXT_SUBRESOURCE_NETWORK_HANDLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "devtools")]
    use crate::{RendererDevToolsPageCommand, RendererPageCommand};

    #[tokio::test]
    async fn unobserved_pauses_resume_natively_and_only_once() {
        for phase in [
            InterceptionPhase::Request,
            InterceptionPhase::Response,
            InterceptionPhase::Auth,
        ] {
            let (pause, receiver) = ResourceInterceptionPause::new(
                7,
                phase,
                crate::network::PageNetworkPolicy::default(),
            );
            let clone = pause.clone();
            assert!(pause.resume_unobserved());
            assert!(!clone.resume_unobserved());
            assert!(matches!(
                (phase, receiver.await.unwrap()),
                (
                    InterceptionPhase::Request,
                    ResourceInterceptionDecision::ContinueRequest {
                        url: None,
                        method: None,
                        body: None,
                        headers: None,
                        intercept_response: false,
                        handle_auth_requests: false,
                    }
                ) | (
                    InterceptionPhase::Response,
                    ResourceInterceptionDecision::ContinueResponse {
                        status: None,
                        headers: None,
                    }
                ) | (
                    InterceptionPhase::Auth,
                    ResourceInterceptionDecision::CancelAuth
                )
            ));
        }
    }

    #[tokio::test]
    #[cfg(feature = "devtools")]
    async fn decisions_are_native_one_shot_and_phase_scoped() {
        let (pause, rx) = ResourceInterceptionPause::new(
            7,
            InterceptionPhase::Request,
            crate::network::PageNetworkPolicy::default(),
        );
        assert!(
            pause
                .dispatch(RendererPageCommand::DevTools(
                    RendererDevToolsPageCommand::FailPendingSubresourceResponse {
                        internal_id: 7,
                        error_text: "wrong phase".into(),
                    }
                ))
                .is_err()
        );
        assert!(
            pause
                .dispatch(RendererPageCommand::DevTools(
                    RendererDevToolsPageCommand::FailPendingSubresourceFetch {
                        internal_id: 8,
                        error_text: "wrong request".into(),
                    }
                ))
                .is_err()
        );
        pause
            .dispatch(RendererPageCommand::DevTools(
                RendererDevToolsPageCommand::FailPendingSubresourceFetch {
                    internal_id: 7,
                    error_text: "blocked".into(),
                },
            ))
            .unwrap();
        assert!(
            matches!(rx.await.unwrap(), ResourceInterceptionDecision::Fail(reason) if reason == "blocked")
        );
        assert!(
            pause
                .dispatch(RendererPageCommand::DevTools(
                    RendererDevToolsPageCommand::FailPendingSubresourceFetch {
                        internal_id: 7,
                        error_text: "late".into(),
                    }
                ))
                .is_err()
        );
    }
}
