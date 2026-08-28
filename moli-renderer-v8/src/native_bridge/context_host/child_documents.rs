mod async_policy;
mod bootstrap_commit;
mod commit;
mod frame_owner_resource_timing;
mod initial_empty;
mod lifecycle;
mod live_parser;
mod loads;
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

pub(in crate::native_bridge::context_host) use frame_owner_resource_timing::{
    ChildDocumentNavigationInitiator, CompletedFrameOwnerResourceTiming,
    PendingFrameOwnerResourceTiming,
};
pub(super) use initial_empty::ChildInitialEmptyDocumentInit;
pub(crate) use loads::{ChildDocumentLoadApplication, ChildDocumentLoadBodyActivity};
pub(in crate::native_bridge::context_host) use parser_store::ChildDocumentParserStore;

fn configure_child_document_navigation_request(
    request: moli_fetch::Request,
    initiator_url: &Url,
    browser_context: &moli_cookie_jar::BrowserCookieFacadeContext,
) -> moli_fetch::Request {
    request
        .with_browser_site_context(browser_context.clone())
        .with_subframe_navigation_cookie_context()
        .with_initiator_url(initiator_url)
}

impl super::JsContextHost {
    fn inferred_child_document_navigation_referrer(
        &self,
        handle: crate::document_runtime::DomHandle,
        target_url: &Url,
    ) -> String {
        let source_policy = self
            .child_browsing_context_parent_handle(handle)
            .and_then(|parent| self.child_browsing_context_policy_container_snapshot(parent))
            .unwrap_or_else(|| self.document_policy_container().clone());
        let raw_source_url = self.document_url_for_child_context(handle);
        let source_url = if raw_source_url.scheme() == "about" {
            Url::parse(&source_policy.document_referrer).unwrap_or(raw_source_url)
        } else {
            raw_source_url
        };
        moli_fetch::navigation_referrer_value(
            &source_url,
            target_url,
            None,
            source_policy.referrer_policy.as_deref(),
        )
        .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub(super) struct PendingChildDocumentNavigation {
    pub(super) target: ChildDocumentNavigationFetchTarget,
    pub(super) target_url: Url,
    pub(super) resource_loader: crate::network::navigation::NavigationResourceLoader,
    pub(super) reserved_service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    pub(super) document_credentialless: bool,
    pub(super) credentialless_storage_nonce: Option<OpaqueOriginNonce>,
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
