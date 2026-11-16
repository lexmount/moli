use std::sync::Arc;

#[cfg(feature = "devtools")]
use anyhow::{Result, anyhow, bail, ensure};
use parking_lot::Mutex;
use tokio::sync::oneshot;
use url::Url;

#[cfg(feature = "devtools")]
use crate::RendererPageCommand;
use crate::RendererSyntheticResponseBody;
use crate::types::SubresourceRequestInitiatorType;

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
    internal_id: u64,
    worker_network: bool,
    phase: InterceptionPhase,
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
    Auth(crate::SubresourceAuthCredentials),
    CancelAuth,
    Fail(String),
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
        let (tx, rx) = oneshot::channel();
        (
            Self {
                internal_id,
                worker_network: false,
                phase,
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
        self.sender
            .lock()
            .take()
            .is_some_and(|sender| sender.send(decision).is_ok())
    }

    /// Releasing an interceptor changes native eligibility immediately. It
    /// must not enqueue a Page policy turn behind another session's pause.
    #[cfg(feature = "devtools")]
    pub fn set_target_interception(&self, classes: crate::SubresourceInterceptionClasses) {
        self.target_policy.set_subresource_interception(classes);
    }

    /// Adapter for the existing Fetch command surface. Only the exhaustive
    /// native network command subset is accepted; other Page work stays queued.
    #[cfg(feature = "devtools")]
    pub fn dispatch(&self, command: RendererPageCommand) -> Result<ResourceInterceptionReply> {
        #[cfg(feature = "devtools")]
        use RendererPageCommand as C;
        use ResourceInterceptionDecision as D;
        use ResourceInterceptionReply as R;
        let (id, phase, decision, reply) = match command {
            C::ContinuePendingSubresourceFetch {
                internal_id,
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
            } => (
                internal_id,
                InterceptionPhase::Request,
                D::ContinueRequest {
                    url,
                    method,
                    body,
                    headers,
                    intercept_response,
                    handle_auth_requests,
                },
                R::Started,
            ),
            C::FailPendingSubresourceFetch {
                internal_id,
                error_text,
            } => (
                internal_id,
                InterceptionPhase::Request,
                D::Fail(error_text),
                R::Unit,
            ),
            C::FulfillPendingSubresourceFetch {
                internal_id,
                request_overrides,
                response_code,
                response_headers,
                response_body,
            } => (
                internal_id,
                InterceptionPhase::Request,
                D::Fulfill {
                    request_overrides,
                    status: response_code,
                    headers: response_headers,
                    body: response_body,
                },
                R::Unit,
            ),
            C::ContinuePendingSubresourceResponse {
                internal_id,
                response_code,
                response_headers,
            } => (
                internal_id,
                InterceptionPhase::Response,
                D::ContinueResponse {
                    status: response_code,
                    headers: response_headers,
                },
                R::Unit,
            ),
            C::FailPendingSubresourceResponse {
                internal_id,
                error_text,
            } => (
                internal_id,
                InterceptionPhase::Response,
                D::Fail(error_text),
                R::Unit,
            ),
            C::FulfillPendingSubresourceResponse {
                internal_id,
                response_code,
                response_headers,
                response_body,
            } => (
                internal_id,
                InterceptionPhase::Response,
                D::Fulfill {
                    request_overrides: None,
                    status: response_code,
                    headers: response_headers,
                    body: response_body,
                },
                R::Unit,
            ),
            C::ContinuePendingSubresourceAuth { internal_id, auth } => (
                internal_id,
                InterceptionPhase::Auth,
                D::Auth(auth),
                R::Started,
            ),
            C::CancelPendingSubresourceAuth { internal_id } => {
                (internal_id, InterceptionPhase::Auth, D::CancelAuth, R::Unit)
            }
            C::FailPendingSubresourceAuth {
                internal_id,
                error_text,
            } => (
                internal_id,
                InterceptionPhase::Auth,
                D::Fail(error_text),
                R::Unit,
            ),
            _ => bail!("command cannot run through a native network pause"),
        };
        ensure!(
            id == self.internal_id && phase == self.phase,
            "network pause identity or phase does not match"
        );
        let tx = self
            .sender
            .lock()
            .take()
            .ok_or_else(|| anyhow!("network pause was already resumed"))?;
        tx.send(decision)
            .map_err(|_| anyhow!("network load is no longer waiting"))?;
        Ok(reply)
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
                .dispatch(RendererPageCommand::FailPendingSubresourceResponse {
                    internal_id: 7,
                    error_text: "wrong phase".into(),
                })
                .is_err()
        );
        assert!(
            pause
                .dispatch(RendererPageCommand::FailPendingSubresourceFetch {
                    internal_id: 8,
                    error_text: "wrong request".into(),
                })
                .is_err()
        );
        pause
            .dispatch(RendererPageCommand::FailPendingSubresourceFetch {
                internal_id: 7,
                error_text: "blocked".into(),
            })
            .unwrap();
        assert!(
            matches!(rx.await.unwrap(), ResourceInterceptionDecision::Fail(reason) if reason == "blocked")
        );
        assert!(
            pause
                .dispatch(RendererPageCommand::FailPendingSubresourceFetch {
                    internal_id: 7,
                    error_text: "late".into(),
                })
                .is_err()
        );
    }
}
