use crate::conn::DrainedFetchRequests;
use anyhow::{Context as _, Result};

use super::resource_target::ResourceTargetStateMut;
use super::*;
use crate::automation::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use crate::conn::IoStreamState;
use crate::conn::state::{
    TargetFetchConfig, TargetFetchOwner, TargetFetchSubresourceInterceptionSnapshot,
};
use crate::conn::{
    CommandOwnerScope, CompletedFetchResponseBodyStreamReadDispatch, FetchInterceptionPattern,
    FetchRequestStage, PausedDocumentTransfer, PendingFetchAuthNavigation, PendingFetchNavigation,
    PendingFetchResponseBodyStreamRead, PendingFetchResponseBodyStreamReadStart,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest,
};

#[derive(Debug, thiserror::Error)]
#[error("RequestNotFound")]
struct SubresourceRequestNotFound;

type FetchDisableStateWithSubresourceConfig = (
    DrainedFetchRequests,
    moli_core::page::SubresourceInterceptionClasses,
    bool,
);

fn fetch_stream_owner_key(browser_context_id: &str, target_id: &str) -> String {
    format!("{browser_context_id}:{target_id}")
}

fn target_scoped_stream_handle(owner_key: &str, handle: String) -> String {
    format!("{owner_key}:{handle}")
}

#[derive(Debug)]
struct TargetScopedStreamOwner {
    browser_context_id: String,
    target_id: String,
}

fn target_scoped_stream_owner_from_handle(handle: &str) -> Option<TargetScopedStreamOwner> {
    let (owner_key, stream_id) = handle.rsplit_once(':')?;
    if stream_id.is_empty() {
        return None;
    }
    let (browser_context_id, target_id) = owner_key.split_once(':')?;
    if browser_context_id.is_empty() || target_id.is_empty() {
        return None;
    }
    Some(TargetScopedStreamOwner {
        browser_context_id: browser_context_id.to_owned(),
        target_id: target_id.to_owned(),
    })
}

fn remove_network_intercept_from_browser_context(
    browser_context: &mut BrowserContext,
    intercept_id: &str,
) -> Result<Option<Option<moli_core::page::PendingPageCommand>>> {
    for target in browser_context.page_targets.iter_mut() {
        let target = ResourceTargetStateMut::for_page(
            &browser_context.id,
            target,
            moli_page_types::DevToolsSessionKey::Primary,
        );
        if target.fetch_owner.remove_network_intercept(intercept_id) {
            return target
                .start_fetch_policy_update(target.fetch_owner.subresource_interception_config())
                .map(Some)
                .context("failed to update page fetch interception");
        }
    }

    Ok(None)
}

impl CdpConnection {
    fn resource_target_for_scoped_stream(
        &mut self,
        session_id: Option<&str>,
        stream_owner: &TargetScopedStreamOwner,
    ) -> Option<ResourceTargetStateMut<'_>> {
        let owner = CommandOwnerScope::capture(self, session_id);
        let state = self.resource_target_state_mut_for_owner(&owner)?;
        (state.browser_context_id == stream_owner.browser_context_id
            && state.target_id == stream_owner.target_id)
            .then_some(state)
    }

    pub(crate) fn fetch_command_wait_lifetime_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<crate::conn::CommandWaitLifetime> {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)?
            .command_wait_lifetime(owner.session_id())
    }

    pub(crate) fn target_fetch_subresource_interception_snapshot_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetFetchSubresourceInterceptionSnapshot> {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)
            .map(|config| config.subresource_interception_snapshot())
    }

    pub(crate) fn target_fetch_subresource_interception_snapshot_for_target(
        &self,
        target_id: &str,
    ) -> Option<TargetFetchSubresourceInterceptionSnapshot> {
        self.target_fetch_config_for_target(target_id)
            .map(|config| config.subresource_interception_snapshot())
    }

    pub(crate) fn target_fetch_event_session_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)
            .and_then(|config| config.session_id().map(str::to_owned))
    }

    pub(crate) fn target_fetch_handle_auth_requests_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)
            .is_some_and(|config| config.handle_auth_requests())
    }

    pub(crate) fn target_fetch_matches_auth_required_for_owner(
        &self,
        owner: &CommandOwnerScope,
        url: &url::Url,
    ) -> bool {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)
            .is_some_and(|config| config.matches_auth_required(url))
    }

    pub(crate) fn target_fetch_matching_auth_required_network_intercepts_for_owner(
        &self,
        owner: &CommandOwnerScope,
        url: &url::Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)
            .map(|config| config.matching_auth_required_network_intercepts(url))
            .unwrap_or_default()
    }

    pub(crate) fn target_fetch_matching_auth_required_network_intercepts_for_target(
        &self,
        target_id: &str,
        url: &url::Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        self.target_fetch_config_for_target(target_id)
            .map(|config| config.matching_auth_required_network_intercepts(url))
            .unwrap_or_default()
    }

    pub(crate) fn target_fetch_matching_network_intercepts_for_target(
        &self,
        target_id: &str,
        request_stage: FetchRequestStage,
        resource_type: DevToolsNetworkResourceType,
        url: &url::Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        self.target_fetch_config_for_target(target_id)
            .map(|config| config.matching_network_intercepts(request_stage, resource_type, url))
            .unwrap_or_default()
    }

    fn target_fetch_config_for_target(&self, target_id: &str) -> Option<TargetFetchConfig> {
        match self.target_session_route_for_target_id(target_id)? {
            CdpSessionRoute::PageTarget {
                browser_context_id,
                target_id,
                ..
            } => self
                .browser_context_by_id(&browser_context_id)?
                .page_target(&target_id)
                .map(|target| target.fetch_owner.config_snapshot()),
            CdpSessionRoute::Browser
            | CdpSessionRoute::BrowserContext { .. }
            | CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => None,
        }
    }

    pub(crate) fn allocate_pending_subresource_fetch_request_ids_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<(String, String)> {
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .runtime_session_owner_slot_mut_for_owner(owner)
            .map(|runtime_slot| {
                runtime_slot
                    .request_id_allocator()
                    .allocate_pending_subresource_fetch_request_ids(
                        &mut network_request_id_allocator,
                    )
            });
        self.network_request_id_allocator = network_request_id_allocator;
        result
    }

    pub(crate) fn allocate_fetch_navigation_request_id_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        self.runtime_session_owner_slot_mut_for_owner(owner)
            .map(|runtime_slot| {
                runtime_slot
                    .request_id_allocator()
                    .allocate_fetch_navigation_request_id()
            })
    }

    #[cfg(test)]
    pub(crate) fn open_io_stream_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<String> {
        self.open_io_stream_body_source_for_session_owner(
            session_id,
            crate::conn::CapturedBody::from_bytes_spooled(bytes),
        )
    }

    pub(crate) fn open_io_stream_body_source_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        body: impl Into<crate::conn::ProtocolResponseBody>,
    ) -> Result<String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.open_io_stream_body_source_for_owner(&owner, body)
    }

    pub(crate) fn open_io_stream_body_source_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        body: impl Into<crate::conn::ProtocolResponseBody>,
    ) -> Result<String> {
        let Some(mut owner) = self.resource_target_state_mut_for_owner(owner) else {
            return Err(crate::conn::NoDocumentLoaded.into());
        };
        Ok(owner.open_scoped_io_stream_body_source(body))
    }

    pub(crate) fn io_stream_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
    ) -> Option<IoStreamState> {
        let Some(owner) = target_scoped_stream_owner_from_handle(handle) else {
            return self
                .runtime_session_owner_slot(session_id)
                .and_then(|slot| slot.io_stream(handle));
        };
        self.resource_target_for_scoped_stream(session_id, &owner)?
            .runtime_slot
            .io_stream(handle)
    }

    pub(crate) fn register_synthetic_websocket_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        network_request_id: String,
        socket_id: u64,
    ) -> bool {
        let Some(runtime_slot) = self.runtime_session_owner_slot_mut_for_owner(owner) else {
            return false;
        };
        runtime_slot.register_synthetic_websocket_request(
            request_id,
            network_request_id,
            socket_id,
        );
        true
    }

    pub(crate) fn synthetic_websocket_socket_id_for_session_owner(
        &self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Option<u64> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.synthetic_websocket_socket_id_for_owner(&owner, request_id)
    }

    pub(crate) fn synthetic_websocket_socket_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Option<u64> {
        self.runtime_session_owner_slot_for_owner(owner)
            .and_then(|runtime_slot| {
                runtime_slot.synthetic_websocket_socket_id_for_request(request_id)
            })
    }

    pub(crate) fn pending_fetch_request_session_route(
        &self,
        request_id: &str,
    ) -> Option<CdpSessionRoute> {
        self.browser_contexts()
            .find_map(|browser_context| pending_fetch_request_route(browser_context, request_id))
    }

    pub(crate) fn subresource_residence_is_current(
        &self,
        residence: &crate::conn::PendingSubresourceFetchResidence,
    ) -> bool {
        match residence {
            crate::conn::PendingSubresourceFetchResidence::Worker {
                browser_context_id,
                target_id,
                ..
            } => self
                .browser_context_by_id(browser_context_id)
                .and_then(|context| context.worker_resource_target(target_id))
                .is_some(),
            _ => residence.page_owner().is_none_or(|page| {
                self.subresource_document_is_current(page, residence.source_document())
            }),
        }
    }
    pub(crate) fn start_subresource_command(
        &self,
        owner: &CommandOwnerScope,
        residence: &crate::conn::PendingSubresourceFetchResidence,
        command: moli_core::RendererPageCommand,
    ) -> Result<moli_core::page::PendingResourceCommand> {
        if let crate::conn::PendingSubresourceFetchResidence::Worker {
            browser_context_id,
            target_id,
            ..
        } = residence
        {
            anyhow::ensure!(
                self.resource_target_state_for_owner(owner)
                    .is_some_and(|state| {
                        state.is_worker
                            && state.browser_context_id == browser_context_id
                            && state.target_id == target_id
                    }),
                "RequestNotFound"
            );
            return residence.start_resource_command(command);
        }
        self.start_subresource_document_command(
            owner,
            residence.page_owner().context(SubresourceRequestNotFound)?,
            residence.document_command_target(),
            command,
        )
    }

    pub(crate) fn start_subresource_document_command(
        &self,
        owner: &CommandOwnerScope,
        page_owner: &crate::conn::TargetPageResidenceIdentity,
        target: Option<&moli_core::page::PageDocumentCommandTarget>,
        command: moli_core::RendererPageCommand,
    ) -> Result<moli_core::page::PendingResourceCommand> {
        let target = target.context(SubresourceRequestNotFound)?;
        if !self.subresource_document_is_current_for_owner(
            owner,
            page_owner,
            Some(target.document()),
        ) {
            return Err(SubresourceRequestNotFound.into());
        }
        target.start_resource_command(command)
    }

    pub(crate) fn subresource_document_is_current(
        &self,
        page_owner: &crate::conn::TargetPageResidenceIdentity,
        document: Option<moli_core::RendererDocumentLifecycleIdentity>,
    ) -> bool {
        self.target_page_output_residence_is_current(page_owner)
            && document.is_none_or(|document| {
                let owner = CommandOwnerScope::for_page_residence(page_owner);
                self.target_root_document_output_identity_for_owner(&owner, document)
                    .is_some_and(|binding| binding.attachment().page_owner() == page_owner)
            })
    }

    pub(crate) fn subresource_document_is_current_for_owner(
        &self,
        owner: &CommandOwnerScope,
        page: &crate::conn::TargetPageResidenceIdentity,
        document: Option<moli_core::RendererDocumentLifecycleIdentity>,
    ) -> bool {
        self.target_owner_identity_for_owner(owner)
            .is_some_and(|(context, target)| {
                context == page.browser_context_id() && target.as_deref() == page.target_id()
            })
            && self.subresource_document_is_current(page, document)
    }

    pub(crate) fn pending_subresource_fetch_request_residence_is_current(
        &self,
        pending: &PendingSubresourceFetchRequest,
    ) -> bool {
        self.subresource_residence_is_current(&pending.residence)
    }

    pub(crate) fn claim_subresource_continue_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        expected_page_owner: Option<&crate::conn::TargetPageResidenceIdentity>,
        internal_id: u64,
        allow_pending_completion: bool,
    ) -> Option<crate::conn::ClaimedSubresourceContinueRequest> {
        if expected_page_owner
            .is_some_and(|page| !self.target_page_output_residence_is_current(page))
        {
            return None;
        }
        let state = self.resource_target_state_mut_for_owner(owner)?;
        if expected_page_owner.is_none() && !state.is_worker() {
            return None;
        }
        state.fetch_owner.claim_subresource_continue_request(
            expected_page_owner,
            internal_id,
            owner.session_id(),
            allow_pending_completion,
        )
    }

    pub(crate) fn consume_pending_request_action_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Option<Result<(), &'static str>> {
        Some(
            self.resource_target_state_mut_for_owner(owner)?
                .fetch_owner
                .consume_pending_request_action(request_id),
        )
    }

    pub(crate) fn take_pending_fetch_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingFetchNavigation> {
        self.resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .take_pending_fetch_navigation_for_action_session(request_id, action_session_id)
    }

    pub(crate) fn take_pending_fetch_auth_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingFetchAuthNavigation> {
        self.resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .take_pending_fetch_auth_navigation_for_action_session(request_id, action_session_id)
    }

    pub(crate) fn register_pending_fetch_auth_navigation_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        pending: PendingFetchAuthNavigation,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.register_pending_fetch_auth_navigation_for_owner(&owner, request_id, pending)
    }

    pub(crate) fn register_pending_fetch_auth_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        pending: PendingFetchAuthNavigation,
    ) -> bool {
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|owner| {
                owner
                    .fetch_owner
                    .register_pending_fetch_auth_navigation(request_id, pending);
                true
            })
    }

    pub(crate) fn take_fetch_response_transfer_for_cancellation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .take_fetch_response_transfer_for_cancellation(request_id)
    }

    pub(crate) fn start_pending_fetch_response_body_read_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Option<crate::conn::PendingDocumentBodyRead> {
        self.resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .start_pending_fetch_response_body_read(request_id)
    }

    pub(crate) fn take_available_fetch_response_transfer_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .take_available_fetch_response_transfer(request_id)
    }

    pub(crate) fn register_pending_fetch_response_transfer_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        transfer: PausedDocumentTransfer,
    ) -> bool {
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_pending_fetch_response_transfer(request_id, transfer)
            })
    }

    pub(crate) fn finish_pending_fetch_response_body_read_for_owner<T>(
        &mut self,
        owner: &CommandOwnerScope,
        completed: crate::conn::CompletedDocumentBodyRead<T>,
    ) -> Option<Result<T>> {
        self.resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .finish_pending_fetch_response_body_read(completed)
    }

    pub(crate) fn pending_subresource_fetch_response_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.resource_target_state_mut_for_owner(owner)?
            .pending_subresource_fetch_response_request(request_id, action_session_id)
    }

    pub(crate) fn mark_pending_subresource_fetch_response_body_taken_as_stream_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> bool {
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|owner| {
                owner
                    .fetch_owner
                    .mark_pending_subresource_fetch_response_body_taken_as_stream(
                        request_id,
                        action_session_id,
                    )
            })
    }

    pub(crate) fn open_pending_fetch_response_body_stream_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Result<Option<String>> {
        let Some(mut owner) = self.resource_target_state_mut(session_id) else {
            return Ok(None);
        };
        owner.open_pending_fetch_response_body_stream(request_id)
    }

    pub(crate) fn start_pending_fetch_response_body_stream_read_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let Some(owner) = self.resource_target_state_mut(session_id) else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        owner
            .fetch_owner
            .start_pending_fetch_response_body_stream_read(handle, offset, size)
    }

    pub(crate) fn start_pending_fetch_response_body_stream_read_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let Some(stream_owner) = target_scoped_stream_owner_from_handle(handle) else {
            return self.start_pending_fetch_response_body_stream_read_for_session_owner(
                session_id, handle, offset, size,
            );
        };
        let Some(owner) = self.resource_target_for_scoped_stream(session_id, &stream_owner) else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        owner
            .fetch_owner
            .start_pending_fetch_response_body_stream_read(handle, offset, size)
    }

    pub(crate) fn finish_pending_fetch_response_body_stream_read_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let Some(mut owner) = self.resource_target_state_mut(session_id) else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        owner.finish_pending_fetch_response_body_stream_read(completed)
    }

    pub(crate) fn finish_pending_fetch_response_body_stream_read_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let stream_owner = target_scoped_stream_owner_from_handle(&completed.handle);
        let Some(stream_owner) = stream_owner else {
            return self.finish_pending_fetch_response_body_stream_read_for_session_owner(
                session_id, completed,
            );
        };
        let Some(mut owner) = self.resource_target_for_scoped_stream(session_id, &stream_owner)
        else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        owner.finish_pending_fetch_response_body_stream_read(completed)
    }

    pub(crate) fn close_pending_fetch_response_body_stream_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
    ) -> bool {
        self.resource_target_state_mut(session_id)
            .is_some_and(|owner| {
                owner
                    .fetch_owner
                    .close_pending_fetch_response_body_stream(handle)
            })
    }

    pub(crate) fn close_pending_fetch_response_body_stream_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
    ) -> bool {
        let Some(stream_owner) = target_scoped_stream_owner_from_handle(handle) else {
            return self
                .close_pending_fetch_response_body_stream_for_session_owner(session_id, handle);
        };
        self.resource_target_for_scoped_stream(session_id, &stream_owner)
            .is_some_and(|owner| {
                owner
                    .fetch_owner
                    .close_pending_fetch_response_body_stream(handle)
            })
    }

    pub(crate) fn close_io_stream_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
    ) -> bool {
        let Some(stream_owner) = target_scoped_stream_owner_from_handle(handle) else {
            return self
                .runtime_session_owner_slot_mut(session_id)
                .is_some_and(|runtime_slot| runtime_slot.close_io_stream(handle));
        };
        self.resource_target_for_scoped_stream(session_id, &stream_owner)
            .is_some_and(|owner| owner.runtime_slot.close_io_stream(handle))
    }

    #[cfg(test)]
    pub(crate) fn take_pending_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchRequest> {
        let pending = self
            .resource_target_state_mut(session_id)?
            .fetch_owner
            .take_pending_subresource_fetch_request(request_id, session_id)?;
        self.pending_subresource_fetch_request_residence_is_current(&pending)
            .then_some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchRequest> {
        let pending = self
            .resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .take_pending_subresource_fetch_request(request_id, action_session_id)?;
        self.pending_subresource_fetch_request_residence_is_current(&pending)
            .then_some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_auth_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchAuthRequest> {
        let pending = self
            .resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .take_pending_subresource_fetch_auth_request(request_id, action_session_id)?;
        self.subresource_residence_is_current(&pending.residence)
            .then_some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_response_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        let pending = self
            .resource_target_state_mut_for_owner(owner)?
            .fetch_owner
            .take_pending_subresource_fetch_response_request(request_id, action_session_id)?;
        self.subresource_residence_is_current(&pending.residence)
            .then_some(pending)
    }

    pub(crate) fn rollback_in_flight_subresource_fetch_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
        network_request_id: &str,
    ) {
        if let Some(owner) = self.resource_target_state_mut_for_owner(owner) {
            owner
                .fetch_owner
                .rollback_in_flight_subresource_fetch_request(internal_id, network_request_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn in_flight_subresource_fetch_request_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
        internal_id: u64,
    ) -> Option<String> {
        let target = self.resource_target_state_for_owner(owner)?;
        let (request_id, page_owner) =
            in_flight_subresource_fetch_request_identity(target.fetch_owner, internal_id)?;
        self.target_page_residence_identity_is_current(&page_owner)
            .then_some(request_id)
    }

    pub(crate) fn paused_subresource_fetch_response_request_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
        internal_id: u64,
    ) -> Option<String> {
        let target = self.resource_target_state_for_owner(owner)?;
        let (request_id, page_owner) =
            in_flight_subresource_fetch_request_identity(target.fetch_owner, internal_id)?;
        (target
            .fetch_owner
            .subresource_response_is_paused(&request_id)
            && self.target_page_residence_identity_is_current(&page_owner))
        .then_some(request_id)
    }

    pub(crate) fn register_pending_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.register_pending_subresource_fetch_request_for_owner(&owner, request_id, pending)
    }

    pub(crate) fn register_pending_subresource_fetch_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if !self.pending_subresource_fetch_request_residence_is_current(&pending) {
            return false;
        }
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_pending_subresource_fetch_request(request_id, pending)
            })
    }

    pub(crate) fn register_in_flight_subresource_fetch_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if !self.pending_subresource_fetch_request_residence_is_current(&pending) {
            return false;
        }
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_in_flight_subresource_fetch_request(request_id, pending)
            })
    }

    pub(crate) fn register_in_flight_response_stage_subresource_fetch_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) -> bool {
        if !self.pending_subresource_fetch_request_residence_is_current(&pending) {
            return false;
        }
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_in_flight_response_stage_subresource_fetch_request(
                    request_id,
                    pending,
                    response_stage_blocked_intercepts,
                );
                true
            })
    }

    pub(crate) fn register_in_flight_deferred_response_stage_subresource_fetch_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if !self.pending_subresource_fetch_request_residence_is_current(&pending) {
            return false;
        }
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_in_flight_subresource_fetch_request_with_response_match_policy(
                    request_id,
                    pending,
                    crate::conn::ResponseStageUrlMatchPolicy::MatchFinalUrl,
                )
            })
    }

    pub(crate) fn register_pending_subresource_fetch_auth_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        pending: PendingSubresourceFetchAuthRequest,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.register_pending_subresource_fetch_auth_request_for_owner(&owner, request_id, pending)
    }

    pub(crate) fn register_pending_subresource_fetch_auth_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        pending: PendingSubresourceFetchAuthRequest,
    ) -> bool {
        if !self.subresource_residence_is_current(&pending.residence) {
            return false;
        }
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_pending_subresource_fetch_auth_request(request_id, pending)
            })
    }

    pub(crate) fn register_pending_subresource_fetch_response_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        pending: PendingSubresourceFetchResponseRequest,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.register_pending_subresource_fetch_response_request_for_owner(
            &owner, request_id, pending,
        )
    }

    pub(crate) fn register_pending_subresource_fetch_response_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        pending: PendingSubresourceFetchResponseRequest,
    ) -> bool {
        if !self.subresource_residence_is_current(&pending.residence) {
            return false;
        }
        self.resource_target_state_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_pending_subresource_fetch_response_request(request_id, pending)
            })
    }

    pub(crate) fn start_enable_fetch_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Result<Option<moli_core::page::PendingPageCommand>> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.start_enable_fetch_for_owner(&owner, handle_auth_requests, patterns)
    }

    pub(crate) fn start_enable_fetch_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Result<Option<moli_core::page::PendingPageCommand>> {
        let owner = self
            .resource_target_state_mut_for_owner(command_owner)
            .ok_or(crate::conn::BrowserContextNotLoaded)?;
        owner.fetch_owner.configure(
            command_owner.session_id().map(str::to_owned),
            handle_auth_requests,
            patterns,
        );
        owner
            .start_fetch_policy_update(owner.fetch_owner.subresource_interception_config())
            .context("failed to update page fetch interception")
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_add_network_intercept_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
        intercept_session_id: Option<String>,
        intercept_id: String,
        handle_auth_requests: bool,
        auth_url_patterns: Vec<String>,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Result<Option<moli_core::page::PendingPageCommand>> {
        let Some(owner) = self.resource_target_state_mut_for_owner(command_owner) else {
            return Err(crate::conn::BrowserContextNotLoaded.into());
        };
        owner.fetch_owner.add_network_intercept(
            intercept_id,
            intercept_session_id,
            handle_auth_requests,
            auth_url_patterns,
            patterns,
        );
        owner
            .start_fetch_policy_update(owner.fetch_owner.subresource_interception_config())
            .context("failed to update page fetch interception")
    }

    pub(crate) fn start_remove_network_intercept_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
        intercept_id: &str,
        allow_global_lookup: bool,
    ) -> Result<Option<moli_core::page::PendingPageCommand>> {
        let Some(owner) = self.resource_target_state_mut_for_owner(command_owner) else {
            return Err(crate::conn::BrowserContextNotLoaded.into());
        };
        if !owner.fetch_owner.remove_network_intercept(intercept_id) {
            if allow_global_lookup {
                return self.start_remove_network_intercept_from_any_target(intercept_id);
            }
            return Err(crate::conn::NetworkInterceptNotFound.into());
        }
        owner
            .start_fetch_policy_update(owner.fetch_owner.subresource_interception_config())
            .context("failed to update page fetch interception")
    }

    fn start_remove_network_intercept_from_any_target(
        &mut self,
        intercept_id: &str,
    ) -> Result<Option<moli_core::page::PendingPageCommand>> {
        if let Some(browser_context) = self.browser_context.as_mut()
            && let Some(pending) =
                remove_network_intercept_from_browser_context(browser_context, intercept_id)?
        {
            return Ok(pending);
        }
        for browser_context in &mut self.inactive_browser_contexts {
            if let Some(pending) =
                remove_network_intercept_from_browser_context(browser_context, intercept_id)?
            {
                return Ok(pending);
            }
        }
        Err(crate::conn::NetworkInterceptNotFound.into())
    }

    pub(crate) fn start_disable_fetch_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<(
        DrainedFetchRequests,
        Option<Result<moli_core::page::PendingPageCommand>>,
    )> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.start_disable_fetch_for_owner(&owner)
    }

    pub(crate) fn start_disable_fetch_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
    ) -> Option<(
        DrainedFetchRequests,
        Option<Result<moli_core::page::PendingPageCommand>>,
    )> {
        let mut target = self.resource_target_state_mut_for_owner(command_owner)?;
        let (pending, classes, update_required) = target
            .reset_fetch_config_for_session_and_drain_pending_state(command_owner.session_id());
        let policy = if update_required {
            target.start_fetch_cleanup_policy(&pending, classes)
        } else {
            None
        };
        Some((pending, policy))
    }

    pub(crate) fn start_disable_target_fetch_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
    ) -> Option<(
        DrainedFetchRequests,
        Option<Result<moli_core::page::PendingPageCommand>>,
    )> {
        let mut target = self.resource_target_state_mut_for_owner(command_owner)?;
        let (pending, classes, update_required) =
            target.disable_target_fetch_and_drain_pending_state();
        let policy = if update_required {
            target.start_fetch_cleanup_policy(&pending, classes)
        } else {
            None
        };
        Some((pending, policy))
    }

    pub(crate) fn take_pending_fetch_state_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<DrainedFetchRequests> {
        Some(
            self.resource_target_state_mut_for_owner(owner)?
                .fetch_owner
                .drain_pending_requests(),
        )
    }

    pub(crate) fn target_fetch_pending_state_is_empty_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<bool> {
        let (context_id, target_id) = self.target_owner_identity_for_owner(owner)?;
        Some(
            self.browser_context_by_id(&context_id)?
                .page_target(target_id.as_deref()?)?
                .fetch_owner
                .pending_state()
                .is_empty(),
        )
    }

    pub(crate) fn take_superseded_intercepted_document_navigations(
        &mut self,
        owner: &CommandOwnerScope,
        current: &crate::conn::DocumentNavigationToken,
    ) -> Vec<crate::conn::NavigationDispatchState> {
        let Some(target) = self.resource_target_state_mut_for_owner(owner) else {
            return Vec::new();
        };
        let retired = target
            .fetch_owner
            .take_superseded_document_navigations(current);
        retired
            .into_iter()
            .map(|(token, navigation)| {
                if let Some(token) = token {
                    let finish = target
                        .runtime_slot
                        .finish_renderer_document_navigation(&token)
                        .expect("a newly admitted navigation keeps the renderer channel open");
                    // The replacement was registered before retiring its predecessors.
                    // Finishing old work must not replay calls yet.
                    debug_assert!(finish.renderer_call_replacements.is_none());
                }
                navigation
            })
            .collect()
    }
}

fn pending_fetch_request_route(
    browser_context: &BrowserContext,
    request_id: &str,
) -> Option<CdpSessionRoute> {
    browser_context.page_targets.iter().find_map(|target| {
        target
            .fetch_owner
            .contains_pending_request(request_id)
            .then(|| CdpSessionRoute::PageTarget {
                browser_context_id: browser_context.id.clone(),
                target_id: target.target_id().to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            })
    })
}

fn in_flight_subresource_fetch_request_identity(
    fetch: &TargetFetchOwner,
    internal_id: u64,
) -> Option<(String, crate::conn::TargetPageResidenceIdentity)> {
    Some((
        fetch
            .in_flight_subresource_fetch_request_id(internal_id)?
            .to_owned(),
        fetch
            .in_flight_subresource_fetch_request_page_owner(internal_id)?
            .clone(),
    ))
}

impl ResourceTargetStateMut<'_> {
    fn bind_subresource_request(
        &mut self,
        handle: Option<moli_core::page::SubresourceNetworkRequestHandle>,
        network_request_id: &str,
        owner_session_id: &mut Option<String>,
        action_session_id: &mut Option<String>,
    ) {
        if let Some(handle) = handle {
            self.runtime_slot_for_command()
                .record_subresource_request_id_for_handle_if_absent(
                    handle,
                    network_request_id.to_owned(),
                );
        }
        if owner_session_id.is_none() {
            *owner_session_id = self.command_session_id.clone();
        }
        if action_session_id.is_none() {
            *action_session_id = owner_session_id.clone();
        }
    }

    fn allocate_scoped_stream_handle(&mut self) -> String {
        let owner_key = fetch_stream_owner_key(self.browser_context_id, self.target_id);
        target_scoped_stream_handle(&owner_key, self.runtime_slot.allocate_io_stream_handle())
    }

    fn open_scoped_io_stream_body_source(
        &mut self,
        body: impl Into<crate::conn::ProtocolResponseBody>,
    ) -> String {
        let handle = self.allocate_scoped_stream_handle();
        self.runtime_slot
            .insert_io_stream_body_source(handle.clone(), body, 0);
        handle
    }

    fn register_pending_fetch_response_transfer(
        &mut self,
        request_id: String,
        transfer: PausedDocumentTransfer,
    ) -> bool {
        if let Some(token) = transfer.navigation_token()
            && let Some(cancellation) = transfer.cancellation_handle()
        {
            self.runtime_slot
                .page_slot_mut()
                .add_document_navigation_cancellation(token, cancellation);
        }
        self.fetch_owner
            .register_pending_fetch_response_transfer(request_id, transfer);
        true
    }

    fn pending_subresource_fetch_response_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.fetch_owner
            .pending_subresource_fetch_response_request(request_id, session_id)
            .cloned()
    }

    pub(super) fn register_pending_subresource_fetch_request(
        &mut self,
        request_id: String,
        mut pending: PendingSubresourceFetchRequest,
    ) -> bool {
        self.bind_subresource_request(
            pending.network_request_handle,
            &pending.network_request_id,
            &mut pending.owner_session_id,
            &mut pending.action_session_id,
        );
        self.fetch_owner
            .register_pending_subresource_fetch_request(request_id, pending);
        true
    }

    fn register_in_flight_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        mut pending: PendingSubresourceFetchRequest,
    ) -> bool {
        self.bind_subresource_request(
            pending.network_request_handle,
            &pending.network_request_id,
            &mut pending.owner_session_id,
            &mut pending.action_session_id,
        );
        self.fetch_owner
            .register_in_flight_subresource_fetch_request(request_id, pending);
        true
    }

    fn register_in_flight_response_stage_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        mut pending: PendingSubresourceFetchRequest,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) -> bool {
        self.bind_subresource_request(
            pending.network_request_handle,
            &pending.network_request_id,
            &mut pending.owner_session_id,
            &mut pending.action_session_id,
        );
        self.fetch_owner
            .register_in_flight_response_stage_subresource_fetch_request(
                request_id,
                pending,
                response_stage_blocked_intercepts,
            );
        true
    }

    fn register_in_flight_subresource_fetch_request_with_response_match_policy(
        &mut self,
        request_id: Option<String>,
        mut pending: PendingSubresourceFetchRequest,
        response_stage_url_match_policy: crate::conn::ResponseStageUrlMatchPolicy,
    ) -> bool {
        self.bind_subresource_request(
            pending.network_request_handle,
            &pending.network_request_id,
            &mut pending.owner_session_id,
            &mut pending.action_session_id,
        );
        self.fetch_owner
            .register_in_flight_subresource_fetch_request_with_response_match_policy(
                request_id,
                pending,
                response_stage_url_match_policy,
            );
        true
    }

    fn register_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: String,
        mut pending: PendingSubresourceFetchAuthRequest,
    ) -> bool {
        self.bind_subresource_request(
            pending.network_request_handle,
            &pending.network_request_id,
            &mut pending.owner_session_id,
            &mut pending.action_session_id,
        );
        self.fetch_owner
            .register_pending_subresource_fetch_auth_request(request_id, pending);
        true
    }

    fn register_pending_subresource_fetch_response_request(
        &mut self,
        request_id: String,
        mut pending: PendingSubresourceFetchResponseRequest,
    ) -> bool {
        self.bind_subresource_request(
            pending.network_request_handle,
            &pending.network_request_id,
            &mut pending.owner_session_id,
            &mut pending.action_session_id,
        );
        self.fetch_owner
            .register_pending_subresource_fetch_response_request(request_id, pending);
        true
    }

    fn open_pending_fetch_response_body_stream(
        &mut self,
        request_id: &str,
    ) -> Result<Option<String>> {
        let handle = self.allocate_scoped_stream_handle();
        self.fetch_owner.open_pending_fetch_response_body_stream(
            self.runtime_slot,
            request_id,
            handle,
        )
    }

    fn finish_pending_fetch_response_body_stream_read(
        &mut self,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        self.fetch_owner
            .finish_pending_fetch_response_body_stream_read(self.runtime_slot, completed)
    }

    pub(super) fn reset_fetch_config_for_session_and_drain_pending_state(
        &mut self,
        session_id: Option<&str>,
    ) -> FetchDisableStateWithSubresourceConfig {
        let previous = self.fetch_owner.subresource_interception_config();
        let removed = self.fetch_owner.remove_fetch_session(session_id);
        let classes = self.fetch_owner.subresource_interception_config();
        let pending = if removed {
            self.fetch_owner
                .drain_pending_requests_for_disable_session(session_id)
        } else {
            DrainedFetchRequests::default()
        };
        (pending, classes, removed && previous != classes)
    }

    fn disable_target_fetch_and_drain_pending_state(
        &mut self,
    ) -> FetchDisableStateWithSubresourceConfig {
        let previous = self.fetch_owner.subresource_interception_config();
        self.fetch_owner.reset_config();
        let classes = self.fetch_owner.subresource_interception_config();
        let pending = self.fetch_owner.drain_pending_requests();
        (pending, classes, previous != classes)
    }

    fn start_fetch_cleanup_policy(
        &self,
        pending: &DrainedFetchRequests,
        classes: moli_core::page::SubresourceInterceptionClasses,
    ) -> Option<Result<moli_core::page::PendingPageCommand>> {
        if !self.is_worker() {
            // A native pause already carries the target's shared eligibility.
            // Update it without queuing an ACK behind a synchronous load.
            let DrainedFetchRequests {
                subresource_requests: requests,
                subresource_auth_requests: auths,
                subresource_responses: responses,
                ..
            } = pending;
            let targets = requests
                .iter()
                .filter_map(|(_, request)| request.document_command_target())
                .chain(
                    auths
                        .iter()
                        .filter_map(|(_, request)| request.residence.document_command_target()),
                )
                .chain(
                    responses
                        .iter()
                        .filter_map(|(_, request)| request.residence.document_command_target()),
                );
            let mut updated = false;
            for target in targets {
                updated |= target.update_native_interception_policy(classes);
            }
            if updated {
                return None;
            }
        }
        self.start_fetch_policy_update(classes).transpose()
    }
}
