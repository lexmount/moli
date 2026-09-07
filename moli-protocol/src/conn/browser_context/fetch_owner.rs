use super::target_session_owner::TargetSessionOwnerMut;
use super::*;
use crate::conn::fetch_support::OpenBodyStreamError;
use crate::conn::state::{
    ClaimedNavigationRequest, InterceptedNavigationResponse, NavigationInterceptionPermit,
    NavigationRequestInterception,
};
use crate::conn::state::{
    TargetFetchConfig, TargetFetchOwner, TargetFetchSubresourceInterceptionSnapshot,
};
use crate::conn::{
    CapturedBody, ClaimedFetchNavigation, ClaimedFetchResponseNavigation, CommandOwnerScope,
    CompletedFetchResponseBodyStreamReadDispatch, FetchInterceptionPattern, FetchRequestStage,
    InFlightSubresourceFetchRequest, PausedDocumentTransfer, PendingFetchAuthNavigation,
    PendingFetchNavigation, PendingFetchResponseBodyStreamRead,
    PendingFetchResponseBodyStreamReadDispatch, PendingFetchResponseBodyStreamReadStart,
    PendingFetchResponseNavigation, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchRequest, PendingSubresourceFetchResponseRequest, TargetRuntimeSlot,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use crate::domains::network::TargetIoStreamRead;

impl CdpConnection {
    pub(crate) fn pause_navigation_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        navigation: crate::conn::NavigationId,
        request: NavigationRequestInterception,
    ) -> Result<NavigationInterceptionPermit, String> {
        let (browser_context_id, target_id) = self
            .resolved_page_owner_identity_for_owner(owner)
            .ok_or("navigation WebContents unavailable")?;
        self.browser_context_by_id_mut(&browser_context_id)
            .ok_or("navigation BrowserContext unavailable")?
            .pause_navigation_request_for_target(&target_id, navigation, request)
    }

    pub(crate) fn take_navigation_request(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<ClaimedNavigationRequest> {
        self.browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find_map(|context| context.take_navigation_request(permit))
    }

    pub(crate) fn pause_navigation_auth(
        &mut self,
        response: InterceptedNavigationResponse<moli_fetch::RawResponse>,
    ) -> Result<NavigationInterceptionPermit, String> {
        // Browser identity was frozen at load admission, before the fetch.
        // Session routing and the current Target/loader are not authority here.
        self.browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.owns_web_contents(response.web_contents()))
            .ok_or("navigation BrowserContext unavailable")?
            .pause_navigation_auth(response)
    }

    pub(crate) fn take_navigation_auth(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<InterceptedNavigationResponse<moli_fetch::RawResponse>> {
        self.browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find_map(|context| context.take_navigation_auth(permit))
    }

    pub(crate) fn take_navigation_response(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<PausedDocumentTransfer> {
        self.browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find_map(|context| context.take_navigation_response(permit))
    }
}

pub(crate) type SessionOwnerPendingFetchState = (
    Vec<PendingFetchNavigation>,
    Vec<PendingFetchAuthNavigation>,
    Vec<PendingFetchResponseNavigation>,
    Vec<(String, PendingSubresourceFetchRequest)>,
    Vec<(String, PendingSubresourceFetchAuthRequest)>,
    Vec<(String, PendingSubresourceFetchResponseRequest)>,
);

struct SessionPendingFetchOwner<'a>(&'a mut TargetFetchOwner);

impl std::ops::Deref for SessionPendingFetchOwner<'_> {
    type Target = TargetFetchOwner;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl std::ops::DerefMut for SessionPendingFetchOwner<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

impl SessionPendingFetchOwner<'_> {
    fn in_flight_subresource_fetch_request_identity(
        &self,
        internal_id: u64,
    ) -> Option<(String, crate::conn::TargetPageResidenceIdentity)> {
        Some((
            self.in_flight_subresource_fetch_request_id(internal_id)?
                .to_owned(),
            self.in_flight_subresource_fetch_request_page_owner(internal_id)?
                .clone(),
        ))
    }
}

fn runtime_slot_for_target_scoped_stream_mut<'a>(
    browser_context: &'a mut BrowserContext,
    target_id: &str,
) -> Option<&'a mut TargetRuntimeSlot> {
    browser_context
        .page_target_mut(target_id)
        .map(|target| &mut target.runtime_slot)
}

fn restore_response_transfer_for_target(
    context: &mut BrowserContext,
    target_id: &str,
    request_id: &str,
    permit: NavigationInterceptionPermit,
    transfer: PausedDocumentTransfer,
) -> bool {
    if context
        .restore_navigation_response(permit, transfer)
        .is_ok()
    {
        return true;
    }
    if let Some(target) = context.page_target_mut(target_id) {
        target
            .fetch_owner
            .take_pending_fetch_response_navigation_for_terminal_action(request_id);
    }
    false
}

impl TargetSessionOwnerMut<'_> {
    fn open_scoped_io_stream_body_source(&mut self, body: CapturedBody) -> Result<String, String> {
        let owner_key = fetch_stream_owner_key(&self.browser_context.id, &self.target_id);
        let Some(target) = self.browser_context.page_target_mut(&self.target_id) else {
            return Err("NoDocumentLoaded".to_owned());
        };
        let handle = target_scoped_stream_handle(
            &owner_key,
            target.runtime_slot.allocate_io_stream_handle(),
        );
        target
            .runtime_slot
            .insert_io_stream_body_source(handle.clone(), body, 0);
        Ok(handle)
    }
}

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

fn target_scoped_stream_owner_matches_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
    owner: &TargetScopedStreamOwner,
) -> bool {
    let Some((browser_context_id, target_id)) = conn.target_owner_identity_for_session(session_id)
    else {
        return false;
    };
    if browser_context_id != owner.browser_context_id {
        return false;
    }
    target_id.unwrap_or_else(|| "active".to_owned()) == owner.target_id
}

fn remove_network_intercept_from_browser_context(
    browser_context: &mut BrowserContext,
    intercept_id: &str,
) -> Result<Option<Option<moli_core::page::PendingPageCommand>>, String> {
    let target_ids = browser_context
        .page_targets
        .iter()
        .map(|target| target.target_id().to_owned())
        .collect::<Vec<_>>();
    for target_id in target_ids {
        let target = browser_context
            .page_target_mut(&target_id)
            .expect("target must remain registered");
        if target.fetch_owner.remove_network_intercept(intercept_id) {
            let (enabled, resource_type) = target.fetch_owner.subresource_interception_config();
            return browser_context
                .start_target_fetch_interception_update(&target_id, enabled, resource_type)
                .map(Some)
                .map_err(|error| format!("failed to update page fetch interception: {error}"));
        }
    }

    Ok(None)
}

impl CdpConnection {
    pub(crate) fn finish_removed_network_interception(
        &mut self,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<(), String> {
        // BiDi's global removal may refer to a background or inactive page.
        // Observe only the physical origin of the completion, not the command
        // caller's selected page. Retirement does not undo the installed policy.
        for context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            if context.observe_renderer_page_state(completion.page_state()) {
                break;
            }
        }
        completion
            .into_unit_page_command_turn()
            .map(drop)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn target_fetch_subresource_interception_snapshot_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetFetchSubresourceInterceptionSnapshot> {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)
            .map(|config| config.subresource_interception_snapshot())
    }

    pub(crate) fn target_fetch_interception_config_after_session_disposal(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        self.target_session_owner_aggregate_fetch_config_for_owner(owner)?
            .subresource_interception_config_after_removing_fetch_session(owner.session_id())
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
    ) -> Result<(String, String), String> {
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
    ) -> Result<String, String> {
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
    ) -> Result<String, String> {
        self.open_io_stream_body_source_for_session_owner(
            session_id,
            CapturedBody::from_bytes_spooled(bytes),
        )
    }

    pub(crate) fn open_io_stream_body_source_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        body: CapturedBody,
    ) -> Result<String, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.open_io_stream_body_source_for_owner(&owner, body)
    }

    pub(crate) fn open_io_stream_body_source_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        body: CapturedBody,
    ) -> Result<String, String> {
        let Some(mut owner) = self.target_session_owner_mut_for_owner(owner) else {
            return Err("NoDocumentLoaded".to_owned());
        };
        owner.open_scoped_io_stream_body_source(body)
    }

    pub(crate) fn read_io_stream_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> Option<TargetIoStreamRead> {
        let Some(owner) = target_scoped_stream_owner_from_handle(handle) else {
            return self
                .runtime_session_owner_slot_mut(session_id)
                .ok()
                .and_then(|runtime_slot| runtime_slot.read_io_stream(handle, offset, size));
        };
        if !target_scoped_stream_owner_matches_session(self, session_id, &owner) {
            return None;
        }
        let browser_context = self.browser_context_by_id_mut(&owner.browser_context_id)?;
        runtime_slot_for_target_scoped_stream_mut(browser_context, &owner.target_id)?
            .read_io_stream(handle, offset, size)
    }

    pub(crate) fn register_synthetic_websocket_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        network_request_id: String,
        socket_id: u64,
    ) -> bool {
        let Ok(runtime_slot) = self.runtime_session_owner_slot_mut_for_owner(owner) else {
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
            .ok()
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

    pub(crate) fn pending_subresource_fetch_request_residence_is_current(
        &self,
        pending: &PendingSubresourceFetchRequest,
    ) -> bool {
        pending
            .installed_page_owner()
            .is_none_or(|owner| self.target_page_residence_identity_is_current(owner))
    }

    fn installed_subresource_fetch_request_is_current(
        &self,
        pending: &PendingSubresourceFetchRequest,
    ) -> bool {
        pending
            .installed_page_owner()
            .is_some_and(|owner| self.target_page_residence_identity_is_current(owner))
    }

    pub(crate) fn claim_subresource_continue_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        expected_page_owner: &crate::conn::TargetPageResidenceIdentity,
        internal_id: u64,
        allow_pending_completion: bool,
    ) -> Option<crate::conn::ClaimedSubresourceContinueRequest> {
        if !self.target_page_residence_identity_is_current(expected_page_owner) {
            return None;
        }
        self.target_session_owner_mut_for_owner(owner)?
            .claim_subresource_continue_request(
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
        self.target_session_owner_mut_for_owner(owner)?
            .consume_pending_request_action(request_id)
    }

    pub(crate) fn take_pending_fetch_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<ClaimedFetchNavigation> {
        let pending = self
            .target_session_owner_mut_for_owner(owner)?
            .take_pending_fetch_navigation_for_action_session(request_id, action_session_id)?;
        let request = self.take_navigation_request(pending.navigation_permit);
        Some(ClaimedFetchNavigation::new(pending, request))
    }

    pub(crate) fn take_pending_fetch_auth_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingFetchAuthNavigation> {
        self.target_session_owner_mut_for_owner(owner)?
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
        self.target_session_owner_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_pending_fetch_auth_navigation(request_id, pending)
            })
    }

    pub(crate) fn register_pending_fetch_response_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: String,
        document_navigation_token: Option<crate::conn::NavigationId>,
        navigation: crate::conn::NavigationDispatchState,
        body: crate::conn::DocumentBodySource,
    ) -> bool {
        let Some((browser_context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(owner)
        else {
            return false;
        };
        let Some(document_navigation_token) = document_navigation_token else {
            return false;
        };
        let transfer = PausedDocumentTransfer::pending(navigation.request_load_policy, body);
        let Some(context) = self.browser_context_by_id_mut(&browser_context_id) else {
            return false;
        };
        let Ok(permit) = context.pause_navigation_response_for_target(
            &target_id,
            document_navigation_token,
            transfer,
        ) else {
            return false;
        };
        let Some(target) = context.page_target_mut(&target_id) else {
            drop(context.take_navigation_response(permit));
            return false;
        };
        target
            .fetch_owner
            .register_pending_fetch_response_navigation(
                request_id,
                PendingFetchResponseNavigation::new(navigation, permit),
            );
        true
    }

    pub(crate) fn take_pending_fetch_response_transfer_for_terminal_action_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Option<ClaimedFetchResponseNavigation> {
        let (browser_context_id, target_id) = self.resolved_page_owner_identity_for_owner(owner)?;
        let context = self.browser_context_by_id_mut(&browser_context_id)?;
        let pending = context
            .page_target_mut(&target_id)?
            .fetch_owner
            .take_pending_fetch_response_navigation_for_terminal_action(request_id)?;
        let transfer = context.take_navigation_response(pending.permit);
        Some(ClaimedFetchResponseNavigation::new(
            request_id.to_owned(),
            pending,
            transfer,
        ))
    }

    pub(crate) fn take_pending_fetch_response_transfer_for_body_read_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        let (browser_context_id, target_id) = self.resolved_page_owner_identity_for_owner(owner)?;
        let context = self.browser_context_by_id_mut(&browser_context_id)?;
        let permit = context
            .page_target(&target_id)?
            .fetch_owner
            .pending_fetch_response_navigation(request_id)?
            .permit;
        context.take_navigation_response(permit)
    }

    pub(crate) fn restore_pending_fetch_response_transfer_for_body_read_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        request_id: &str,
        transfer: PausedDocumentTransfer,
    ) -> bool {
        let Some((browser_context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(owner)
        else {
            return false;
        };
        let Some(context) = self.browser_context_by_id_mut(&browser_context_id) else {
            return false;
        };
        let Some(permit) = context
            .page_target(&target_id)
            .and_then(|target| {
                target
                    .fetch_owner
                    .pending_fetch_response_navigation(request_id)
            })
            .map(|pending| pending.permit)
        else {
            return false;
        };
        restore_response_transfer_for_target(context, &target_id, request_id, permit, transfer)
    }

    pub(crate) fn restore_pending_fetch_response_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        claimed: ClaimedFetchResponseNavigation,
    ) -> bool {
        let Some((request_id, pending, transfer)) = claimed.into_restore_parts() else {
            return false;
        };
        let Some((browser_context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(owner)
        else {
            return false;
        };
        let Some(context) = self.browser_context_by_id_mut(&browser_context_id) else {
            return false;
        };
        let permit = pending.permit;
        if context
            .restore_navigation_response(permit, transfer)
            .is_ok()
        {
            let Some(target) = context.page_target_mut(&target_id) else {
                drop(context.take_navigation_response(permit));
                return false;
            };
            target
                .fetch_owner
                .register_pending_fetch_response_navigation(request_id, pending);
            return true;
        }
        false
    }

    pub(crate) fn pending_subresource_fetch_response_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.target_session_owner_mut_for_owner(owner)?
            .pending_subresource_fetch_response_request(request_id, action_session_id)
    }

    pub(crate) fn mark_pending_subresource_fetch_response_body_taken_as_stream_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> bool {
        self.target_session_owner_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.mark_pending_subresource_fetch_response_body_taken_as_stream(
                    request_id,
                    action_session_id,
                )
            })
    }

    fn open_pending_fetch_response_body_stream_for_target(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(context) = self.browser_context_by_id_mut(browser_context_id) else {
            return Ok(None);
        };
        let owner_key = fetch_stream_owner_key(browser_context_id, target_id);
        let Some((permit, handle)) = context.page_target_mut(target_id).and_then(|target| {
            let permit = target
                .fetch_owner
                .pending_fetch_response_navigation(request_id)?
                .permit;
            let handle = target_scoped_stream_handle(
                &owner_key,
                target.runtime_slot.allocate_io_stream_handle(),
            );
            Some((permit, handle))
        }) else {
            return Ok(None);
        };
        let Some(transfer) = context.take_navigation_response(permit) else {
            return Ok(None);
        };
        let opened = match transfer.open_body_stream(handle) {
            Ok(opened) => opened,
            Err(OpenBodyStreamError::NotOpenable(transfer)) => {
                restore_response_transfer_for_target(
                    context, target_id, request_id, permit, *transfer,
                );
                return Ok(None);
            }
            Err(OpenBodyStreamError::Failed { transfer, message }) => {
                restore_response_transfer_for_target(
                    context, target_id, request_id, permit, *transfer,
                );
                return Err(message);
            }
        };
        let crate::conn::PendingFetchResponseOpenedBodyStream {
            handle,
            buffered_bytes,
            transfer,
        } = opened;
        if !restore_response_transfer_for_target(context, target_id, request_id, permit, transfer) {
            return Ok(None);
        }
        let Some(target) = context.page_target_mut(target_id) else {
            return Ok(None);
        };
        if let Some(bytes) = buffered_bytes {
            target
                .runtime_slot
                .insert_io_stream(handle.clone(), bytes, 0);
        } else if !target
            .fetch_owner
            .set_pending_fetch_response_body_stream_handle(request_id, Some(handle.clone()))
        {
            return Ok(None);
        }
        Ok(Some(handle))
    }

    fn start_pending_fetch_response_body_stream_read_for_target(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let Some(context) = self.browser_context_by_id_mut(browser_context_id) else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        let Some((request_id, permit)) = context.page_target(target_id).and_then(|target| {
            target
                .fetch_owner
                .pending_fetch_response_body_stream(handle)
                .map(|(request_id, pending)| (request_id.to_owned(), pending.permit))
        }) else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        let Some(transfer) = context.take_navigation_response(permit) else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        if let Some(offset) = offset
            && offset != transfer.body_stream_offset().unwrap_or(0)
        {
            restore_response_transfer_for_target(context, target_id, &request_id, permit, transfer);
            return PendingFetchResponseBodyStreamReadStart::OffsetNotSupported;
        }
        PendingFetchResponseBodyStreamReadStart::Pending(Box::new(
            PendingFetchResponseBodyStreamReadDispatch::new(
                request_id,
                handle.to_owned(),
                transfer,
                size,
            ),
        ))
    }

    fn finish_pending_fetch_response_body_stream_read_for_target(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let request_id = completed.request_id().to_owned();
        let handle = completed.handle().to_owned();
        let Some(context) = self.browser_context_by_id_mut(browser_context_id) else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        let Some(permit) = context
            .page_target(target_id)
            .and_then(|target| {
                target
                    .fetch_owner
                    .pending_fetch_response_navigation(&request_id)
            })
            .filter(|pending| pending.active_body_stream_handle() == Some(handle.as_str()))
            .map(|pending| pending.permit)
        else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        match completed.into_completed() {
            Ok((bytes, eof, transfer)) => {
                if !restore_response_transfer_for_target(
                    context,
                    target_id,
                    &request_id,
                    permit,
                    transfer,
                ) {
                    return PendingFetchResponseBodyStreamRead::NotFound;
                }
                if eof && let Some(target) = context.page_target_mut(target_id) {
                    target
                        .fetch_owner
                        .set_pending_fetch_response_body_stream_handle(&request_id, None);
                    target.runtime_slot.insert_io_stream(handle, Vec::new(), 0);
                }
                PendingFetchResponseBodyStreamRead::Read { bytes, eof }
            }
            Err(completed) => {
                let (transfer, message) = *completed;
                if !restore_response_transfer_for_target(
                    context,
                    target_id,
                    &request_id,
                    permit,
                    transfer,
                ) {
                    return PendingFetchResponseBodyStreamRead::NotFound;
                }
                PendingFetchResponseBodyStreamRead::Failed(message)
            }
        }
    }

    fn close_pending_fetch_response_body_stream_for_target(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        handle: &str,
    ) -> bool {
        let Some(context) = self.browser_context_by_id_mut(browser_context_id) else {
            return false;
        };
        let Some(request_id) = context.page_target(target_id).and_then(|target| {
            target
                .fetch_owner
                .pending_fetch_response_body_stream(handle)
                .map(|(request_id, _)| request_id.to_owned())
        }) else {
            return false;
        };
        let Some(pending) = context.page_target_mut(target_id).and_then(|target| {
            target
                .fetch_owner
                .take_pending_fetch_response_navigation_for_terminal_action(&request_id)
        }) else {
            return false;
        };
        drop(context.take_navigation_response(pending.permit));
        true
    }

    pub(crate) fn open_pending_fetch_response_body_stream_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        let Some((browser_context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(&owner)
        else {
            return Ok(None);
        };
        self.open_pending_fetch_response_body_stream_for_target(
            &browser_context_id,
            &target_id,
            request_id,
        )
    }

    pub(crate) fn start_pending_fetch_response_body_stream_read_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let owner = CommandOwnerScope::capture(self, session_id);
        let Some((browser_context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(&owner)
        else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        self.start_pending_fetch_response_body_stream_read_for_target(
            &browser_context_id,
            &target_id,
            handle,
            offset,
            size,
        )
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
        if !target_scoped_stream_owner_matches_session(self, session_id, &stream_owner) {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        }
        self.start_pending_fetch_response_body_stream_read_for_target(
            &stream_owner.browser_context_id,
            &stream_owner.target_id,
            handle,
            offset,
            size,
        )
    }

    pub(crate) fn finish_pending_fetch_response_body_stream_read_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let owner = CommandOwnerScope::capture(self, session_id);
        let Some((browser_context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(&owner)
        else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        self.finish_pending_fetch_response_body_stream_read_for_target(
            &browser_context_id,
            &target_id,
            completed,
        )
    }

    pub(crate) fn finish_pending_fetch_response_body_stream_read_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let stream_owner = target_scoped_stream_owner_from_handle(completed.handle());
        let Some(stream_owner) = stream_owner else {
            return self.finish_pending_fetch_response_body_stream_read_for_session_owner(
                session_id, completed,
            );
        };
        if !target_scoped_stream_owner_matches_session(self, session_id, &stream_owner) {
            return PendingFetchResponseBodyStreamRead::NotFound;
        }
        self.finish_pending_fetch_response_body_stream_read_for_target(
            &stream_owner.browser_context_id,
            &stream_owner.target_id,
            completed,
        )
    }

    pub(crate) fn close_pending_fetch_response_body_stream_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        let Some((browser_context_id, target_id)) =
            self.resolved_page_owner_identity_for_owner(&owner)
        else {
            return false;
        };
        self.close_pending_fetch_response_body_stream_for_target(
            &browser_context_id,
            &target_id,
            handle,
        )
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
        if !target_scoped_stream_owner_matches_session(self, session_id, &stream_owner) {
            return false;
        }
        self.close_pending_fetch_response_body_stream_for_target(
            &stream_owner.browser_context_id,
            &stream_owner.target_id,
            handle,
        )
    }

    pub(crate) fn close_io_stream_for_stream_owner(
        &mut self,
        session_id: Option<&str>,
        handle: &str,
    ) -> bool {
        let Some(stream_owner) = target_scoped_stream_owner_from_handle(handle) else {
            return self
                .runtime_session_owner_slot_mut(session_id)
                .is_ok_and(|runtime_slot| runtime_slot.close_io_stream(handle));
        };
        if !target_scoped_stream_owner_matches_session(self, session_id, &stream_owner) {
            return false;
        }
        let Some(browser_context) =
            self.browser_context_by_id_mut(&stream_owner.browser_context_id)
        else {
            return false;
        };
        runtime_slot_for_target_scoped_stream_mut(browser_context, &stream_owner.target_id)
            .is_some_and(|runtime_slot| runtime_slot.close_io_stream(handle))
    }

    #[cfg(test)]
    pub(crate) fn take_pending_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchRequest> {
        let pending = self
            .target_session_owner_mut(session_id)?
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
            .target_session_owner_mut_for_owner(owner)?
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
            .target_session_owner_mut_for_owner(owner)?
            .take_pending_subresource_fetch_auth_request(request_id, action_session_id)?;
        self.target_page_residence_identity_is_current(&pending.page_owner)
            .then_some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_response_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        let pending = self
            .target_session_owner_mut_for_owner(owner)?
            .take_pending_subresource_fetch_response_request(request_id, action_session_id)?;
        self.target_page_residence_identity_is_current(&pending.page_owner)
            .then_some(pending)
    }

    pub(crate) fn take_in_flight_subresource_fetch_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
    ) -> Option<InFlightSubresourceFetchRequest> {
        let in_flight = self
            .target_session_owner_mut_for_owner(owner)?
            .take_in_flight_subresource_fetch_request(internal_id)?;
        self.installed_subresource_fetch_request_is_current(&in_flight.pending)
            .then_some(in_flight)
    }

    pub(crate) fn in_flight_subresource_fetch_request_id_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
    ) -> Option<String> {
        let (request_id, page_owner) = self
            .target_session_owner_mut_for_owner(owner)?
            .in_flight_subresource_fetch_request_identity(internal_id)?;
        self.target_page_residence_identity_is_current(&page_owner)
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
        self.record_pending_subresource_network_request_identity_for_owner(owner, &pending);
        self.target_session_owner_mut_for_owner(owner)
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
        if !self.installed_subresource_fetch_request_is_current(&pending) {
            return false;
        }
        self.record_pending_subresource_network_request_identity_for_owner(owner, &pending);
        self.target_session_owner_mut_for_owner(owner)
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
        if !self.installed_subresource_fetch_request_is_current(&pending) {
            return false;
        }
        self.record_pending_subresource_network_request_identity_for_owner(owner, &pending);
        self.target_session_owner_mut_for_owner(owner)
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
        if !self.installed_subresource_fetch_request_is_current(&pending) {
            return false;
        }
        self.record_pending_subresource_network_request_identity_for_owner(owner, &pending);
        self.target_session_owner_mut_for_owner(owner)
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
        if !self.target_page_residence_identity_is_current(&pending.page_owner) {
            return false;
        }
        self.record_pending_subresource_auth_network_request_identity_for_owner(owner, &pending);
        self.target_session_owner_mut_for_owner(owner)
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
        if !self.target_page_residence_identity_is_current(&pending.page_owner) {
            return false;
        }
        self.record_pending_subresource_response_network_request_identity_for_owner(
            owner, &pending,
        );
        self.target_session_owner_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.register_pending_subresource_fetch_response_request(request_id, pending)
            })
    }

    fn record_pending_subresource_network_request_identity_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        pending: &PendingSubresourceFetchRequest,
    ) {
        let Some(handle) = pending.network_request_handle else {
            return;
        };
        if let Ok(runtime_slot) = self.runtime_session_owner_slot_mut_for_owner(owner) {
            runtime_slot.record_subresource_request_id_for_handle_if_absent(
                handle,
                pending.network_request_id.clone(),
            );
        }
    }

    fn record_pending_subresource_auth_network_request_identity_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        pending: &PendingSubresourceFetchAuthRequest,
    ) {
        let Some(handle) = pending.network_request_handle else {
            return;
        };
        if let Ok(runtime_slot) = self.runtime_session_owner_slot_mut_for_owner(owner) {
            runtime_slot.record_subresource_request_id_for_handle_if_absent(
                handle,
                pending.network_request_id.clone(),
            );
        }
    }

    fn record_pending_subresource_response_network_request_identity_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        pending: &PendingSubresourceFetchResponseRequest,
    ) {
        let Some(handle) = pending.network_request_handle else {
            return;
        };
        if let Ok(runtime_slot) = self.runtime_session_owner_slot_mut_for_owner(owner) {
            runtime_slot.record_subresource_request_id_for_handle_if_absent(
                handle,
                pending.network_request_id.clone(),
            );
        }
    }

    pub(crate) fn start_enable_fetch_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.start_enable_fetch_for_owner(&owner, handle_auth_requests, patterns)
    }

    pub(crate) fn start_enable_fetch_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
        let Some(mut owner) = self.target_session_owner_mut_for_owner(command_owner) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let (subresource_enabled, subresource_resource_type) = owner.configure_fetch(
            command_owner.session_id().map(str::to_owned),
            handle_auth_requests,
            patterns,
        );
        owner
            .browser_context
            .start_target_fetch_interception_update(
                &owner.target_id,
                subresource_enabled,
                subresource_resource_type,
            )
            .map_err(|error| format!("failed to update page fetch interception: {error}"))
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
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
        let Some(mut owner) = self.target_session_owner_mut_for_owner(command_owner) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let (subresource_enabled, subresource_resource_type) = owner.add_network_intercept(
            intercept_id,
            intercept_session_id,
            handle_auth_requests,
            auth_url_patterns,
            patterns,
        );
        owner
            .browser_context
            .start_target_fetch_interception_update(
                &owner.target_id,
                subresource_enabled,
                subresource_resource_type,
            )
            .map_err(|error| format!("failed to update page fetch interception: {error}"))
    }

    pub(crate) fn start_remove_network_intercept_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
        intercept_id: &str,
        allow_global_lookup: bool,
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
        let Some(mut owner) = self.target_session_owner_mut_for_owner(command_owner) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some((subresource_enabled, subresource_resource_type)) =
            owner.remove_network_intercept(intercept_id)
        else {
            if allow_global_lookup {
                return self.start_remove_network_intercept_from_any_target(intercept_id);
            }
            return Err("NetworkInterceptNotFound".to_owned());
        };
        owner
            .browser_context
            .start_target_fetch_interception_update(
                &owner.target_id,
                subresource_enabled,
                subresource_resource_type,
            )
            .map_err(|error| format!("failed to update page fetch interception: {error}"))
    }

    fn start_remove_network_intercept_from_any_target(
        &mut self,
        intercept_id: &str,
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
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
        Err("NetworkInterceptNotFound".to_owned())
    }

    pub(crate) fn start_disable_fetch_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<(
        SessionOwnerPendingFetchState,
        Result<Option<moli_core::page::PendingPageCommand>, String>,
    )> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.start_disable_fetch_for_owner(&owner, true)
    }

    pub(crate) fn start_dispose_fetch_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        renderer_policy_reconciled: bool,
    ) -> Option<(
        SessionOwnerPendingFetchState,
        Result<Option<moli_core::page::PendingPageCommand>, String>,
    )> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.start_disable_fetch_for_owner(&owner, !renderer_policy_reconciled)
    }

    fn start_disable_fetch_for_owner(
        &mut self,
        command_owner: &CommandOwnerScope,
        enqueue_renderer_update: bool,
    ) -> Option<(
        SessionOwnerPendingFetchState,
        Result<Option<moli_core::page::PendingPageCommand>, String>,
    )> {
        let mut owner = self.target_session_owner_mut_for_owner(command_owner)?;
        let (pending, (subresource_enabled, subresource_resource_type), removed) = owner
            .reset_fetch_config_for_session_and_drain_pending_state(command_owner.session_id());
        // Explicit Fetch.disable and a failed renderer finalization must
        // reinstall the current aggregate even on a retry. Successful session
        // finalization has already applied this source-free value through the
        // renderer lifecycle interrupt, so only publish it to the Browser
        // owner here and never wait behind active JavaScript.
        let page_command = if enqueue_renderer_update {
            owner
                .browser_context
                .start_target_fetch_interception_update(
                    &owner.target_id,
                    subresource_enabled,
                    subresource_resource_type,
                )
        } else if removed {
            owner
                .browser_context
                .install_target_fetch_interception_policy(
                    &owner.target_id,
                    subresource_enabled,
                    subresource_resource_type,
                )
                .map(|()| None)
        } else {
            Ok(None)
        };
        Some((pending, page_command))
    }

    pub(crate) fn take_pending_fetch_state_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<SessionOwnerPendingFetchState> {
        Some(
            self.target_session_owner_mut_for_owner(owner)?
                .drain_fetch_pending_state(),
        )
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

impl TargetSessionOwnerMut<'_> {
    fn session_id(&self) -> Option<&str> {
        self.command_session_id.as_deref()
    }

    fn pending_fetch_owner_mut(&mut self) -> Option<SessionPendingFetchOwner<'_>> {
        Some(SessionPendingFetchOwner(
            &mut self
                .browser_context
                .page_target_mut(&self.target_id)?
                .fetch_owner,
        ))
    }

    pub(super) fn register_pending_fetch_navigation_request(
        &mut self,
        pending: PendingFetchNavigation,
    ) -> Option<()> {
        self.pending_fetch_owner_mut()?
            .register_pending_fetch_navigation_request(pending);
        Some(())
    }

    fn consume_pending_request_action(
        &mut self,
        request_id: &str,
    ) -> Option<Result<(), &'static str>> {
        Some(
            self.pending_fetch_owner_mut()?
                .consume_pending_request_action(request_id),
        )
    }

    fn take_pending_fetch_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchNavigation> {
        self.pending_fetch_owner_mut()?
            .take_pending_fetch_navigation_for_action_session(request_id, action_session_id)
    }

    fn take_pending_fetch_auth_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchAuthNavigation> {
        self.pending_fetch_owner_mut()?
            .take_pending_fetch_auth_navigation_for_action_session(request_id, action_session_id)
    }

    fn register_pending_fetch_auth_navigation(
        &mut self,
        request_id: String,
        pending: PendingFetchAuthNavigation,
    ) -> bool {
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_pending_fetch_auth_navigation(request_id, pending);
        true
    }

    fn pending_subresource_fetch_response_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.pending_fetch_owner_mut()?
            .pending_subresource_fetch_response_request(request_id, session_id)
            .cloned()
    }

    fn mark_pending_subresource_fetch_response_body_taken_as_stream(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> bool {
        self.pending_fetch_owner_mut().is_some_and(|mut owner| {
            owner.mark_pending_subresource_fetch_response_body_taken_as_stream(
                request_id, session_id,
            )
        })
    }

    fn take_pending_subresource_fetch_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchRequest> {
        self.pending_fetch_owner_mut()?
            .take_pending_subresource_fetch_request(request_id, session_id)
    }

    fn take_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchAuthRequest> {
        self.pending_fetch_owner_mut()?
            .take_pending_subresource_fetch_auth_request(request_id, session_id)
    }

    fn take_pending_subresource_fetch_response_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.pending_fetch_owner_mut()?
            .take_pending_subresource_fetch_response_request(request_id, session_id)
    }

    fn take_in_flight_subresource_fetch_request(
        &mut self,
        internal_id: u64,
    ) -> Option<InFlightSubresourceFetchRequest> {
        self.pending_fetch_owner_mut()?
            .take_in_flight_subresource_fetch_request(internal_id)
    }

    fn claim_subresource_continue_request(
        &mut self,
        expected_page_owner: &crate::conn::TargetPageResidenceIdentity,
        internal_id: u64,
        session_id: Option<&str>,
        allow_pending_completion: bool,
    ) -> Option<crate::conn::ClaimedSubresourceContinueRequest> {
        self.pending_fetch_owner_mut()?
            .claim_subresource_continue_request(
                expected_page_owner,
                internal_id,
                session_id,
                allow_pending_completion,
            )
    }

    fn in_flight_subresource_fetch_request_identity(
        &mut self,
        internal_id: u64,
    ) -> Option<(String, crate::conn::TargetPageResidenceIdentity)> {
        self.pending_fetch_owner_mut()?
            .in_flight_subresource_fetch_request_identity(internal_id)
    }

    pub(super) fn register_pending_subresource_fetch_request(
        &mut self,
        request_id: String,
        mut pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if pending.owner_session_id.is_none() {
            pending.owner_session_id = self.session_id().map(str::to_owned);
        }
        if pending.action_session_id.is_none() {
            pending.action_session_id = pending.owner_session_id.clone();
        }
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_pending_subresource_fetch_request(request_id, pending);
        true
    }

    fn register_in_flight_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        mut pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if pending.owner_session_id.is_none() {
            pending.owner_session_id = self.session_id().map(str::to_owned);
        }
        if pending.action_session_id.is_none() {
            pending.action_session_id = pending.owner_session_id.clone();
        }
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_in_flight_subresource_fetch_request(request_id, pending);
        true
    }

    fn register_in_flight_response_stage_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        mut pending: PendingSubresourceFetchRequest,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) -> bool {
        if pending.owner_session_id.is_none() {
            pending.owner_session_id = self.session_id().map(str::to_owned);
        }
        if pending.action_session_id.is_none() {
            pending.action_session_id = pending.owner_session_id.clone();
        }
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_in_flight_response_stage_subresource_fetch_request(
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
        if pending.owner_session_id.is_none() {
            pending.owner_session_id = self.session_id().map(str::to_owned);
        }
        if pending.action_session_id.is_none() {
            pending.action_session_id = pending.owner_session_id.clone();
        }
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_in_flight_subresource_fetch_request_with_response_match_policy(
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
        if pending.owner_session_id.is_none() {
            pending.owner_session_id = self.session_id().map(str::to_owned);
        }
        if pending.action_session_id.is_none() {
            pending.action_session_id = pending.owner_session_id.clone();
        }
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_pending_subresource_fetch_auth_request(request_id, pending);
        true
    }

    fn register_pending_subresource_fetch_response_request(
        &mut self,
        request_id: String,
        mut pending: PendingSubresourceFetchResponseRequest,
    ) -> bool {
        if pending.owner_session_id.is_none() {
            pending.owner_session_id = self.session_id().map(str::to_owned);
        }
        if pending.action_session_id.is_none() {
            pending.action_session_id = pending.owner_session_id.clone();
        }
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_pending_subresource_fetch_response_request(request_id, pending);
        true
    }
}
