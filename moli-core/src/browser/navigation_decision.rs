use parking_lot::Mutex;
use std::sync::{Arc, Weak};
use tokio::sync::{oneshot, watch};
use url::Url;

use super::web_contents::{InitialDocumentBuildKey, NavigationInterceptionPermit};
use moli_renderer_v8::RendererPreparedDocumentInspectionEndpoint;

/// An optional observer's lifetime, not ownership of a Browser transaction.
/// Dropping it releases outstanding decisions to their neutral fallback.
pub struct NavigationDecisionProvider {
    pub(super) _alive: watch::Sender<()>,
}

/// A concrete pre-execution boundary. Inspection configuration is sent directly
/// to the restricted renderer endpoint; it is never stored in Browser state.
#[derive(Clone)]
pub enum NavigationDecisionStage {
    InitialDocumentReserved {
        key: InitialDocumentBuildKey,
    },
    InitialDocument {
        key: InitialDocumentBuildKey,
        inspection: RendererPreparedDocumentInspectionEndpoint,
    },
    Request {
        url: Url,
        method: String,
        headers: Vec<(String, String)>,
        opening: std::sync::Weak<crate::page::RendererPopupOpening>,
    },
    Auth {
        response: Box<moli_fetch::ResponseHead>,
        observations: moli_fetch::NetworkObservationJournal,
    },
    Response {
        response: Box<moli_fetch::ResponseHead>,
        observations: moli_fetch::NetworkObservationJournal,
    },
    PreparedDocument {
        renderer: super::RendererPageResidenceIdentity,
        inspection: RendererPreparedDocumentInspectionEndpoint,
    },
}

impl std::fmt::Debug for NavigationDecisionStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InitialDocumentReserved { key } => {
                f.debug_tuple("InitialDocumentReserved").field(key).finish()
            }
            Self::InitialDocument { key, .. } => {
                f.debug_tuple("InitialDocument").field(key).finish()
            }
            Self::Request {
                url,
                method,
                headers,
                ..
            } => f
                .debug_struct("Request")
                .field("url", url)
                .field("method", method)
                .field("headers", headers)
                .finish(),
            Self::Auth { response, .. } => f.debug_tuple("Auth").field(response).finish(),
            Self::Response { response, .. } => f.debug_tuple("Response").field(response).finish(),
            Self::PreparedDocument { renderer, .. } => f
                .debug_struct("PreparedDocument")
                .field("renderer", renderer)
                .finish(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NavigationDecisionSnapshot {
    pub permit: NavigationInterceptionPermit,
    pub stage: NavigationDecisionStage,
}

#[derive(Debug)]
pub enum NavigationDecision {
    Continue,
    Request {
        url: Url,
        method: String,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
    },
    Fulfill {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Authenticate {
        credentials: crate::page::SubresourceAuthCredentials,
        response: Box<super::web_contents::PausedDocumentTransfer>,
    },
    Response {
        transfer: Box<super::web_contents::PausedDocumentTransfer>,
        status: Option<u16>,
        headers: Vec<(String, String)>,
    },
    Cancel,
}

/// Stored in the exact pending navigation's mutually exclusive pause slot.
/// Cancellation/supersession drops the sender and wakes the Browser driver.
pub(in crate::browser) struct PendingNavigationDecision {
    permit: NavigationInterceptionPermit,
    completion: Arc<NavigationDecisionCompletion>,
    state: PendingDecisionState,
}

enum DecisionResource<T> {
    Available(T),
    Claimed,
}

impl<T> DecisionResource<T> {
    fn take(&mut self) -> Option<T> {
        match std::mem::replace(self, Self::Claimed) {
            Self::Available(value) => Some(value),
            Self::Claimed => None,
        }
    }
}

struct PendingResponseDecision {
    head: Box<moli_fetch::ResponseHead>,
    observations: moli_fetch::NetworkObservationJournal,
    transfer: DecisionResource<super::web_contents::PausedDocumentTransfer>,
}

enum PendingDecisionState {
    InitialDocumentReserved {
        key: InitialDocumentBuildKey,
    },
    InitialDocument {
        key: InitialDocumentBuildKey,
        inspection: RendererPreparedDocumentInspectionEndpoint,
    },
    Request {
        request: DecisionResource<Box<super::web_contents::NavigationRequestInterception>>,
        opening: Weak<crate::page::RendererPopupOpening>,
    },
    Auth(Box<PendingResponseDecision>),
    Response(Box<PendingResponseDecision>),
    PreparedDocument {
        renderer: super::RendererPageResidenceIdentity,
        inspection: RendererPreparedDocumentInspectionEndpoint,
    },
}

impl std::fmt::Debug for PendingNavigationDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingNavigationDecision")
            .field("permit", &self.permit)
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl PendingNavigationDecision {
    pub fn new(
        permit: NavigationInterceptionPermit,
        stage: NavigationDecisionStage,
        sender: oneshot::Sender<NavigationDecision>,
    ) -> Result<Self, String> {
        let state = match stage {
            NavigationDecisionStage::InitialDocumentReserved { key } => {
                PendingDecisionState::InitialDocumentReserved { key }
            }
            NavigationDecisionStage::InitialDocument { key, inspection } => {
                PendingDecisionState::InitialDocument { key, inspection }
            }
            NavigationDecisionStage::PreparedDocument {
                renderer,
                inspection,
            } => PendingDecisionState::PreparedDocument {
                renderer,
                inspection,
            },
            NavigationDecisionStage::Request {
                url,
                method,
                headers,
                opening,
            } => PendingDecisionState::Request {
                request: DecisionResource::Available(Box::new(
                    super::web_contents::NavigationRequestInterception::new(
                        url,
                        method,
                        None,
                        headers,
                        super::NavigationRequestLoadPolicy::DocumentInitiated,
                    ),
                )),
                opening,
            },
            NavigationDecisionStage::Auth { .. } | NavigationDecisionStage::Response { .. } => {
                return Err("response decision requires its transfer at admission".into());
            }
        };
        Ok(Self {
            permit,
            completion: NavigationDecisionCompletion::new(sender),
            state,
        })
    }

    pub fn with_response(
        permit: NavigationInterceptionPermit,
        stage: NavigationDecisionStage,
        sender: oneshot::Sender<NavigationDecision>,
        transfer: super::web_contents::PausedDocumentTransfer,
    ) -> Result<Self, String> {
        let state = match stage {
            NavigationDecisionStage::Auth {
                response,
                observations,
            } => PendingDecisionState::Auth(Box::new(PendingResponseDecision {
                head: response,
                observations,
                transfer: DecisionResource::Available(transfer),
            })),
            NavigationDecisionStage::Response {
                response,
                observations,
            } => PendingDecisionState::Response(Box::new(PendingResponseDecision {
                head: response,
                observations,
                transfer: DecisionResource::Available(transfer),
            })),
            _ => return Err("body supplied for a non-response decision".into()),
        };
        Ok(Self {
            permit,
            completion: NavigationDecisionCompletion::new(sender),
            state,
        })
    }

    pub fn permit(&self) -> NavigationInterceptionPermit {
        self.permit
    }

    pub fn snapshot(&self) -> Option<NavigationDecisionSnapshot> {
        let stage = match &self.state {
            PendingDecisionState::InitialDocumentReserved { key } => {
                NavigationDecisionStage::InitialDocumentReserved { key: *key }
            }
            PendingDecisionState::InitialDocument { key, inspection } => {
                NavigationDecisionStage::InitialDocument {
                    key: *key,
                    inspection: inspection.clone(),
                }
            }
            PendingDecisionState::PreparedDocument {
                renderer,
                inspection,
            } => NavigationDecisionStage::PreparedDocument {
                renderer: *renderer,
                inspection: inspection.clone(),
            },
            PendingDecisionState::Request {
                request: DecisionResource::Available(request),
                opening,
            } => request.decision_stage(opening.clone()),
            PendingDecisionState::Auth(response) | PendingDecisionState::Response(response)
                if matches!(response.transfer, DecisionResource::Available(_)) =>
            {
                if matches!(self.state, PendingDecisionState::Auth(_)) {
                    NavigationDecisionStage::Auth {
                        response: response.head.clone(),
                        observations: response.observations.clone(),
                    }
                } else {
                    NavigationDecisionStage::Response {
                        response: response.head.clone(),
                        observations: response.observations.clone(),
                    }
                }
            }
            _ => return None,
        };
        Some(NavigationDecisionSnapshot {
            permit: self.permit,
            stage,
        })
    }

    pub fn take_request(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<super::web_contents::ClaimedNavigationRequest> {
        if self.permit != permit {
            return None;
        }
        let PendingDecisionState::Request { request, .. } = &mut self.state else {
            return None;
        };
        Some(super::web_contents::ClaimedNavigationRequest::new_native(
            permit,
            *request.take()?,
            self.completion.claim(),
        ))
    }

    pub fn take_response(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<super::web_contents::PausedDocumentTransfer> {
        if self.permit != permit {
            return None;
        }
        let (PendingDecisionState::Auth(response) | PendingDecisionState::Response(response)) =
            &mut self.state
        else {
            return None;
        };
        let mut transfer = response.transfer.take()?;
        transfer.claim_native(self.completion.claim_response());
        Some(transfer)
    }

    pub fn restore_response(
        &mut self,
        permit: NavigationInterceptionPermit,
        mut transfer: super::web_contents::PausedDocumentTransfer,
    ) -> Result<(), Box<super::web_contents::PausedDocumentTransfer>> {
        if permit == self.permit
            && let PendingDecisionState::Auth(response) | PendingDecisionState::Response(response) =
                &mut self.state
            && matches!(response.transfer, DecisionResource::Claimed)
        {
            transfer.release_native_claim();
            response.transfer = DecisionResource::Available(transfer);
            return Ok(());
        }
        Err(Box::new(transfer))
    }

    pub fn accepts(
        &self,
        permit: NavigationInterceptionPermit,
        decision: &NavigationDecision,
    ) -> bool {
        permit == self.permit
            && match decision {
                NavigationDecision::Authenticate { .. } => {
                    matches!(self.state, PendingDecisionState::Auth(_))
                }
                NavigationDecision::Request { .. } => {
                    matches!(self.state, PendingDecisionState::Request { .. })
                }
                NavigationDecision::Response { .. } => matches!(
                    self.state,
                    PendingDecisionState::Auth(_) | PendingDecisionState::Response(_)
                ),
                NavigationDecision::Fulfill { .. } => matches!(
                    self.state,
                    PendingDecisionState::Request { .. } | PendingDecisionState::Response(_)
                ),
                NavigationDecision::Continue | NavigationDecision::Cancel => true,
            }
    }

    pub fn resolve(mut self, mut decision: NavigationDecision) -> bool {
        if matches!(decision, NavigationDecision::Continue)
            && let PendingDecisionState::Auth(response) | PendingDecisionState::Response(response) =
                &mut self.state
        {
            decision = match response.transfer.take() {
                Some(transfer) => NavigationDecision::Response {
                    transfer: Box::new(transfer),
                    status: None,
                    headers: Vec::new(),
                },
                None => NavigationDecision::Cancel,
            };
        }
        match &mut decision {
            NavigationDecision::Response { transfer, .. } => transfer.release_native_claim(),
            NavigationDecision::Authenticate { response, .. } => response.release_native_claim(),
            _ => {}
        }
        self.completion.send(decision)
    }
}

#[derive(Debug)]
pub(in crate::browser) struct NavigationDecisionCompletion {
    sender: Mutex<Option<oneshot::Sender<NavigationDecision>>>,
}

impl NavigationDecisionCompletion {
    pub fn new(sender: oneshot::Sender<NavigationDecision>) -> Arc<Self> {
        Arc::new(Self {
            sender: Mutex::new(Some(sender)),
        })
    }

    pub fn send(&self, decision: NavigationDecision) -> bool {
        self.sender
            .lock()
            .take()
            .is_some_and(|sender| sender.send(decision).is_ok())
    }

    pub fn claim(self: &Arc<Self>) -> NavigationDecisionClaim {
        NavigationDecisionClaim {
            completion: Arc::downgrade(self),
            fallback: NavigationDecisionFallback::Continue,
        }
    }

    pub fn claim_response(self: &Arc<Self>) -> NavigationDecisionClaim {
        NavigationDecisionClaim {
            completion: Arc::downgrade(self),
            fallback: NavigationDecisionFallback::Cancel,
        }
    }
}

/// A dropped command releases only its original decision. The weak reference
/// cannot keep a canceled/superseded pending navigation alive.
#[derive(Debug)]
pub(in crate::browser) struct NavigationDecisionClaim {
    completion: Weak<NavigationDecisionCompletion>,
    fallback: NavigationDecisionFallback,
}

#[derive(Debug)]
enum NavigationDecisionFallback {
    Continue,
    Cancel,
}

impl NavigationDecisionClaim {
    pub fn disarm(mut self) {
        self.completion = Weak::new();
    }
}

impl Drop for NavigationDecisionClaim {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.upgrade() {
            completion.send(match self.fallback {
                NavigationDecisionFallback::Continue => NavigationDecision::Continue,
                NavigationDecisionFallback::Cancel => NavigationDecision::Cancel,
            });
        }
    }
}
