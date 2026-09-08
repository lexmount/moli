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

/// Admission chooses a resource-bearing variant; this tag is not retained as
/// a second stage or published independently from the actual response.
pub(in crate::browser) enum ResponseInterceptionStage {
    Auth,
    Response,
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
