mod async_policy;
mod bootstrap_commit;
mod commit;
mod encoding;
mod frame_owner_resource_timing;
mod initial_empty;
mod lifecycle;
mod live_parser;
mod loads;
mod navigation_admission;
mod parser_preloads;
mod parser_store;
mod snapshots;

use crate::{
    document_script_scheduler::FrameDocumentClassicScriptSchedulerWork,
    frame_owner_model::{
        ChildDocumentNavigationFetchTarget, FrameDocumentInteractiveLifecycleAction,
        FrameDocumentOwnerTransition,
    },
};
use moli_storage_key::OpaqueOriginNonce;
use url::Url;

pub(crate) enum LocalChildDocumentPreparation {
    /// No local snapshot is available; ordinary fetching/failure handling applies.
    Unavailable,
    /// A local response is available, but must not replace the active Document.
    IgnoredNavigation,
    Ready(Box<super::ChildBrowsingContextSnapshot>),
    /// Restore the historical local resource and its policy container intact;
    /// current iframe attributes apply to new navigations, not this resource.
    Historical(Box<super::ChildBrowsingContextSnapshot>),
}

impl LocalChildDocumentPreparation {
    /// Read-only projections need only a snapshot. Navigation commits must
    /// inspect the disposition so an ignored response cannot clear a Document.
    pub(crate) fn into_snapshot(self) -> Option<super::ChildBrowsingContextSnapshot> {
        match self {
            Self::Ready(snapshot) | Self::Historical(snapshot) => Some(*snapshot),
            Self::Unavailable | Self::IgnoredNavigation => None,
        }
    }
}

pub(in crate::native_bridge::context_host) use frame_owner_resource_timing::{
    ChildDocumentNavigationInitiator, CompletedFrameOwnerResourceTiming,
    PendingFrameOwnerResourceTiming,
};
pub(super) use initial_empty::ChildInitialEmptyDocumentInit;
pub(crate) use loads::{ChildDocumentLoadApplication, ChildDocumentLoadBodyActivity};
pub(in crate::native_bridge::context_host) use parser_preloads::PendingChildParserPreload;
pub(in crate::native_bridge::context_host) use parser_store::ChildDocumentParserStore;

fn configure_child_document_navigation_request(
    request: moli_fetch::Request,
    initiator_url: &Url,
    source: Option<&crate::runtime::RendererNavigationInitiator>,
    browser_context: &moli_cookie_jar::BrowserCookieFacadeContext,
) -> moli_fetch::Request {
    let request = request
        .with_browser_site_context(browser_context.clone())
        .with_subframe_navigation_cookie_context()
        .with_initiator_url(initiator_url);
    if let Some(source) = source {
        // A srcdoc URL cannot recover its native origin or its referrer
        // source. Both belong to the initiating Document's admission snapshot.
        request
            .with_request_origin(source.origin())
            .with_fetch_referrer(source.url().as_str())
            .expect("navigation initiator retains a parsed URL")
    } else {
        request.with_request_origin(moli_url::WebOrigin::from_url(initiator_url))
    }
}

#[derive(Debug, Clone)]
pub(super) struct PendingChildDocumentNavigation {
    pub(super) admitted_history_entry: Option<moli_session_history::SessionHistoryEntry>,
    pub(super) target: ChildDocumentNavigationFetchTarget,
    pub(super) target_url: Url,
    pub(super) resource_loader: crate::network::navigation::NavigationResourceLoader,
    pub(super) reserved_service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    pub(super) document_credentialless: bool,
    pub(super) credentialless_storage_nonce: Option<OpaqueOriginNonce>,
    pub(super) initiator: ChildDocumentNavigationInitiator,
    pub(super) frame_owner_resource_timing: Option<PendingFrameOwnerResourceTiming>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildDocumentCommitState {
    Ready,
    Pending,
}

pub(super) struct ChildDocumentCommitResult {
    pub(super) state: ChildDocumentCommitState,
    pub(super) initial_classic_ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
    pub(super) parser_stop_action: Option<FrameDocumentInteractiveLifecycleAction>,
    pub(super) owner_transition: Option<FrameDocumentOwnerTransition>,
}

impl ChildDocumentCommitResult {
    pub(super) fn ready(
        initial_classic_ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
    ) -> Self {
        Self {
            state: ChildDocumentCommitState::Ready,
            initial_classic_ready_work,
            parser_stop_action: None,
            owner_transition: None,
        }
    }

    pub(super) fn from_install(install: commit::ChildDocumentInstallResult) -> Self {
        Self {
            state: ChildDocumentCommitState::Ready,
            initial_classic_ready_work: install.initial_classic_ready_work,
            parser_stop_action: install.parser_stop_action,
            owner_transition: Some(install.owner_transition),
        }
    }

    pub(super) fn pending() -> Self {
        Self {
            state: ChildDocumentCommitState::Pending,
            initial_classic_ready_work: None,
            parser_stop_action: None,
            owner_transition: None,
        }
    }
}
