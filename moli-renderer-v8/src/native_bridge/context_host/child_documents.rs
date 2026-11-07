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
pub(crate) use loads::{
    ChildDocumentFetch, ChildDocumentLoadApplication, ChildDocumentLoadBodyActivity,
};
pub(in crate::native_bridge::context_host) use parser_store::ChildDocumentParserStore;
pub(in crate::native_bridge::context_host) use snapshots::child_document_content_type_from_headers;

fn configure_child_document_navigation_request(
    request: moli_fetch::Request,
    initiator_url: &Url,
    browser_context: &moli_cookie_jar::BrowserCookieFacadeContext,
) -> moli_fetch::Request {
    request
        .with_browser_site_context(browser_context.clone())
        .with_subframe_navigation_cookie_context()
        .with_initiator_url(initiator_url)
        .with_request_origin(moli_url::WebOrigin::from_url(initiator_url))
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
    pub(super) frame_owner_resource_timing: Option<PendingFrameOwnerResourceTiming>,
    /// The Fetch pause holding this navigation's request, if any.
    pub(super) paused_fetch: Option<u64>,
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

impl super::JsContextHost {
    fn child_document_navigation_referrer_policy(
        &self,
        handle: crate::document_runtime::DomHandle,
        initiator: ChildDocumentNavigationInitiator,
    ) -> String {
        let owner = self.owner_dispatch_scope_for_node(handle);
        if owner
            .and_then(|owner| self.window_access_origin_for_dispatch_scope(owner))
            .is_some_and(|origin| origin.serialized_origin() == "null")
        {
            return "no-referrer".to_owned();
        }
        let source_policy = self
            .dom_host()
            .owner_document_handle(handle)
            .and_then(|document| {
                crate::context_bootstrap::document_referrer_policy_for_native_document(
                    self, document,
                )
            });
        let owner_policy = (initiator == ChildDocumentNavigationInitiator::FrameOwnerElement
            && self.dom_host().is_html_element_named(handle, "iframe"))
        .then(|| self.dom_host().get_attribute(handle, "referrerpolicy"))
        .flatten();
        let owner_policy = owner_policy
            .as_deref()
            .map(crate::native_bridge::element::canonical_referrer_policy_value)
            .filter(|policy| !policy.is_empty());
        moli_fetch::effective_referrer_policy(owner_policy, source_policy.as_deref()).to_owned()
    }
}
