use super::target_session_owner::TargetSessionOwnerMut;
use super::*;
use crate::conn::state::{
    TargetFetchConfig, TargetFetchOwner, TargetFetchState,
    TargetFetchSubresourceInterceptionSnapshot,
};
use crate::conn::{
    CapturedBody, CompletedFetchResponseBodyStreamReadDispatch, FetchInterceptionPattern,
    FetchRequestStage, InFlightSubresourceFetchRequest, PausedDocumentTransfer,
    PendingFetchAuthNavigation, PendingFetchNavigation, PendingFetchResponseBodyStreamRead,
    PendingFetchResponseBodyStreamReadStart, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchRequest, PendingSubresourceFetchResponseRequest, TargetRuntimeSlot,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use crate::domains::network::TargetIoStreamRead;

pub(crate) type SessionOwnerPendingFetchState = (
    Vec<PendingFetchNavigation>,
    Vec<PendingFetchAuthNavigation>,
    Vec<PausedDocumentTransfer>,
    Vec<(String, PendingSubresourceFetchRequest)>,
    Vec<(String, PendingSubresourceFetchAuthRequest)>,
    Vec<(String, PendingSubresourceFetchResponseRequest)>,
);

struct ParkedFetchStateGuard<'a> {
    browser_context: &'a mut BrowserContext,
    target_id: String,
    fetch_state: Option<TargetFetchState>,
}

impl<'a> ParkedFetchStateGuard<'a> {
    fn take(browser_context: &'a mut BrowserContext, target_id: &str) -> Self {
        Self {
            fetch_state: Some(browser_context.take_parked_fetch_state(target_id)),
            browser_context,
            target_id: target_id.to_owned(),
        }
    }

    fn fetch_state_mut(&mut self) -> &mut TargetFetchState {
        self.fetch_state
            .as_mut()
            .expect("parked fetch state should be restored exactly once")
    }

    fn fetch_state(&self) -> &TargetFetchState {
        self.fetch_state
            .as_ref()
            .expect("parked fetch state should be restored exactly once")
    }

    fn open_pending_fetch_response_body_stream(
        &mut self,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(fetch_state) = self.fetch_state.as_mut() else {
            return Ok(None);
        };
        let owner_key = fetch_stream_owner_key(&self.browser_context.id, &self.target_id);
        let Some(target) = self.browser_context.background_target_mut(&self.target_id) else {
            return Ok(None);
        };
        let handle = target_scoped_stream_handle(
            &owner_key,
            target.runtime_slot.allocate_io_stream_handle(),
        );
        fetch_state.open_pending_fetch_response_body_stream(
            &mut target.runtime_slot,
            request_id,
            handle,
        )
    }

    fn start_pending_fetch_response_body_stream_read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let Some(fetch_state) = self.fetch_state.as_mut() else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        fetch_state.start_pending_fetch_response_body_stream_read(handle, offset, size)
    }

    fn finish_pending_fetch_response_body_stream_read(
        &mut self,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let Some(fetch_state) = self.fetch_state.as_mut() else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        let Some(target) = self.browser_context.background_target_mut(&self.target_id) else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        fetch_state
            .finish_pending_fetch_response_body_stream_read(&mut target.runtime_slot, completed)
    }

    fn close_pending_fetch_response_body_stream(&mut self, handle: &str) -> bool {
        let Some(fetch_state) = self.fetch_state.as_mut() else {
            return false;
        };
        if self
            .browser_context
            .background_target(&self.target_id)
            .is_none()
        {
            return false;
        }
        fetch_state.close_pending_fetch_response_body_stream(handle)
    }
}

impl Drop for ParkedFetchStateGuard<'_> {
    fn drop(&mut self) {
        if let Some(fetch_state) = self.fetch_state.take() {
            self.browser_context
                .replace_parked_fetch_state(self.target_id.clone(), fetch_state);
        }
    }
}

struct ParkedPendingFetchOwner<'a> {
    fetch_state: ParkedFetchStateGuard<'a>,
}

impl<'a> ParkedPendingFetchOwner<'a> {
    fn take(browser_context: &'a mut BrowserContext, target_id: &str) -> Self {
        Self {
            fetch_state: ParkedFetchStateGuard::take(browser_context, target_id),
        }
    }

    fn fetch_state_mut(&mut self) -> &mut TargetFetchState {
        self.fetch_state.fetch_state_mut()
    }

    fn fetch_state(&self) -> &TargetFetchState {
        self.fetch_state.fetch_state()
    }
}

struct ParkedFetchBodyStreamOwner<'a> {
    fetch_state: ParkedFetchStateGuard<'a>,
}

impl<'a> ParkedFetchBodyStreamOwner<'a> {
    fn take(browser_context: &'a mut BrowserContext, target_id: &str) -> Self {
        Self {
            fetch_state: ParkedFetchStateGuard::take(browser_context, target_id),
        }
    }

    fn open_pending_fetch_response_body_stream(
        &mut self,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        self.fetch_state
            .open_pending_fetch_response_body_stream(request_id)
    }

    fn start_pending_fetch_response_body_stream_read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        self.fetch_state
            .start_pending_fetch_response_body_stream_read(handle, offset, size)
    }

    fn finish_pending_fetch_response_body_stream_read(
        &mut self,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        self.fetch_state
            .finish_pending_fetch_response_body_stream_read(completed)
    }

    fn close_pending_fetch_response_body_stream(&mut self, handle: &str) -> bool {
        self.fetch_state
            .close_pending_fetch_response_body_stream(handle)
    }
}

enum SessionPendingFetchOwner<'a> {
    Active(&'a mut TargetFetchOwner),
    Parked(Box<ParkedPendingFetchOwner<'a>>),
}

enum SessionFetchBodyStreamOwner<'a> {
    Active {
        owner_key: String,
        fetch_owner: &'a mut TargetFetchOwner,
        runtime_slot: &'a mut TargetRuntimeSlot,
    },
    Parked(Box<ParkedFetchBodyStreamOwner<'a>>),
}

impl SessionFetchBodyStreamOwner<'_> {
    fn open_pending_fetch_response_body_stream(
        &mut self,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        match self {
            Self::Active {
                owner_key,
                fetch_owner,
                runtime_slot,
            } => {
                let handle = target_scoped_stream_handle(
                    owner_key,
                    runtime_slot.allocate_io_stream_handle(),
                );
                fetch_owner.open_pending_fetch_response_body_stream(
                    runtime_slot,
                    request_id,
                    handle,
                )
            }
            Self::Parked(owner) => owner.open_pending_fetch_response_body_stream(request_id),
        }
    }

    fn start_pending_fetch_response_body_stream_read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        match self {
            Self::Active { fetch_owner, .. } => {
                fetch_owner.start_pending_fetch_response_body_stream_read(handle, offset, size)
            }
            Self::Parked(owner) => {
                owner.start_pending_fetch_response_body_stream_read(handle, offset, size)
            }
        }
    }

    fn finish_pending_fetch_response_body_stream_read(
        &mut self,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        match self {
            Self::Active {
                fetch_owner,
                runtime_slot,
                ..
            } => {
                fetch_owner.finish_pending_fetch_response_body_stream_read(runtime_slot, completed)
            }
            Self::Parked(owner) => owner.finish_pending_fetch_response_body_stream_read(completed),
        }
    }

    fn close_pending_fetch_response_body_stream(&mut self, handle: &str) -> bool {
        match self {
            Self::Active { fetch_owner, .. } => {
                fetch_owner.close_pending_fetch_response_body_stream(handle)
            }
            Self::Parked(owner) => owner.close_pending_fetch_response_body_stream(handle),
        }
    }
}

fn fetch_body_stream_owner_for_target_mut<'a>(
    browser_context: &'a mut BrowserContext,
    target_id: &str,
) -> Option<SessionFetchBodyStreamOwner<'a>> {
    let is_active_target = browser_context.active_target_id() == Some(target_id)
        || (target_id == "active" && browser_context.active_target_id().is_none());
    if is_active_target {
        let owner_key = fetch_stream_owner_key(&browser_context.id, target_id);
        let active_target = &mut browser_context.active_target;
        return Some(SessionFetchBodyStreamOwner::Active {
            owner_key,
            fetch_owner: &mut active_target.fetch_owner,
            runtime_slot: &mut active_target.runtime_slot,
        });
    }
    browser_context.background_target(target_id)?;
    Some(SessionFetchBodyStreamOwner::Parked(Box::new(
        ParkedFetchBodyStreamOwner::take(browser_context, target_id),
    )))
}

fn runtime_slot_for_target_scoped_stream_mut<'a>(
    browser_context: &'a mut BrowserContext,
    target_id: &str,
) -> Option<&'a mut TargetRuntimeSlot> {
    let is_active_target = browser_context.active_target_id() == Some(target_id)
        || (target_id == "active" && browser_context.active_target_id().is_none());
    if is_active_target {
        return Some(&mut browser_context.active_target.runtime_slot);
    }
    browser_context
        .background_target_mut(target_id)
        .map(|target| &mut target.runtime_slot)
}

impl SessionPendingFetchOwner<'_> {
    fn register_pending_fetch_navigation_request(&mut self, pending: PendingFetchNavigation) {
        match self {
            Self::Active(owner) => owner.register_pending_fetch_navigation_request(pending),
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .register_pending_fetch_navigation_request(pending),
        }
    }

    fn consume_pending_request_action(&mut self, request_id: &str) -> Result<(), &'static str> {
        match self {
            Self::Active(owner) => owner.consume_pending_request_action(request_id),
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .consume_pending_request_action(request_id),
        }
    }

    fn take_pending_fetch_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchNavigation> {
        match self {
            Self::Active(owner) => owner
                .take_pending_fetch_navigation_for_action_session(request_id, action_session_id),
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_pending_fetch_navigation_for_action_session(request_id, action_session_id),
        }
    }

    fn take_pending_fetch_auth_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchAuthNavigation> {
        match self {
            Self::Active(owner) => owner.take_pending_fetch_auth_navigation_for_action_session(
                request_id,
                action_session_id,
            ),
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_pending_fetch_auth_navigation_for_action_session(
                    request_id,
                    action_session_id,
                ),
        }
    }

    fn register_pending_fetch_auth_navigation(
        &mut self,
        request_id: String,
        pending: PendingFetchAuthNavigation,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_pending_fetch_auth_navigation(request_id, pending);
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_pending_fetch_auth_navigation(request_id, pending);
            }
        }
    }

    fn register_pending_fetch_response_navigation(
        &mut self,
        request_id: String,
        document_navigation_token: Option<crate::conn::DocumentNavigationToken>,
        navigation: crate::conn::NavigationDispatchState,
        body: crate::conn::DocumentBodySource,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_pending_fetch_response_navigation(
                    request_id,
                    document_navigation_token,
                    navigation,
                    body,
                );
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_pending_fetch_response_navigation(
                        request_id,
                        document_navigation_token,
                        navigation,
                        body,
                    );
            }
        }
    }

    fn take_pending_fetch_response_transfer_for_terminal_action(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        match self {
            Self::Active(owner) => {
                owner.take_pending_fetch_response_transfer_for_terminal_action(request_id)
            }
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_pending_fetch_response_transfer_for_terminal_action(request_id),
        }
    }

    fn take_pending_fetch_response_transfer(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        match self {
            Self::Active(owner) => owner.take_pending_fetch_response_transfer(request_id),
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_pending_fetch_response_transfer(request_id),
        }
    }

    fn register_pending_fetch_response_transfer(
        &mut self,
        request_id: String,
        transfer: PausedDocumentTransfer,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_pending_fetch_response_transfer(request_id, transfer);
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_pending_fetch_response_transfer(request_id, transfer);
            }
        }
    }

    fn pending_subresource_fetch_response_request(
        &self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        match self {
            Self::Active(owner) => owner
                .pending_subresource_fetch_response_request(request_id, session_id)
                .cloned(),
            Self::Parked(owner) => owner
                .fetch_state()
                .pending_subresource_fetch_response_request(request_id, session_id)
                .cloned(),
        }
    }

    fn mark_pending_subresource_fetch_response_body_taken_as_stream(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> bool {
        match self {
            Self::Active(owner) => owner
                .mark_pending_subresource_fetch_response_body_taken_as_stream(
                    request_id, session_id,
                ),
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .mark_pending_subresource_fetch_response_body_taken_as_stream(
                    request_id, session_id,
                ),
        }
    }

    fn take_pending_subresource_fetch_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchRequest> {
        match self {
            Self::Active(owner) => {
                owner.take_pending_subresource_fetch_request(request_id, session_id)
            }
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_pending_subresource_fetch_request(request_id, session_id),
        }
    }

    fn take_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchAuthRequest> {
        match self {
            Self::Active(owner) => {
                owner.take_pending_subresource_fetch_auth_request(request_id, session_id)
            }
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_pending_subresource_fetch_auth_request(request_id, session_id),
        }
    }

    fn take_pending_subresource_fetch_response_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        match self {
            Self::Active(owner) => {
                owner.take_pending_subresource_fetch_response_request(request_id, session_id)
            }
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_pending_subresource_fetch_response_request(request_id, session_id),
        }
    }

    fn take_in_flight_subresource_fetch_request(
        &mut self,
        internal_id: u64,
    ) -> Option<InFlightSubresourceFetchRequest> {
        match self {
            Self::Active(owner) => owner.take_in_flight_subresource_fetch_request(internal_id),
            Self::Parked(owner) => owner
                .fetch_state_mut()
                .take_in_flight_subresource_fetch_request(internal_id),
        }
    }

    fn claim_subresource_continue_request(
        &mut self,
        expected_page_owner: &crate::conn::TargetPageResidenceIdentity,
        internal_id: u64,
        session_id: Option<&str>,
        allow_pending_completion: bool,
    ) -> Option<crate::conn::ClaimedSubresourceContinueRequest> {
        match self {
            Self::Active(owner) => owner.claim_subresource_continue_request(
                expected_page_owner,
                internal_id,
                session_id,
                allow_pending_completion,
            ),
            Self::Parked(owner) => owner.fetch_state_mut().claim_subresource_continue_request(
                expected_page_owner,
                internal_id,
                session_id,
                allow_pending_completion,
            ),
        }
    }

    fn in_flight_subresource_fetch_request_identity(
        &mut self,
        internal_id: u64,
    ) -> Option<(String, crate::conn::TargetPageResidenceIdentity)> {
        match self {
            Self::Active(owner) => Some((
                owner
                    .in_flight_subresource_fetch_request_id(internal_id)?
                    .to_owned(),
                owner
                    .in_flight_subresource_fetch_request_page_owner(internal_id)?
                    .clone(),
            )),
            Self::Parked(owner) => Some((
                owner
                    .fetch_state()
                    .in_flight_subresource_fetch_request_id(internal_id)?
                    .to_owned(),
                owner
                    .fetch_state()
                    .in_flight_subresource_fetch_request_page_owner(internal_id)?
                    .clone(),
            )),
        }
    }

    fn register_pending_subresource_fetch_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_pending_subresource_fetch_request(request_id, pending);
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_pending_subresource_fetch_request(request_id, pending);
            }
        }
    }

    fn register_in_flight_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_in_flight_subresource_fetch_request(request_id, pending);
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_in_flight_subresource_fetch_request(request_id, pending);
            }
        }
    }

    fn register_in_flight_response_stage_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_in_flight_response_stage_subresource_fetch_request(
                    request_id,
                    pending,
                    response_stage_blocked_intercepts,
                );
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_in_flight_response_stage_subresource_fetch_request(
                        request_id,
                        pending,
                        response_stage_blocked_intercepts,
                    );
            }
        }
    }

    fn register_in_flight_subresource_fetch_request_with_response_match_policy(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_url_match_policy: crate::conn::ResponseStageUrlMatchPolicy,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_in_flight_subresource_fetch_request_with_response_match_policy(
                    request_id,
                    pending,
                    response_stage_url_match_policy,
                );
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_in_flight_subresource_fetch_request_with_response_match_policy(
                        request_id,
                        pending,
                        response_stage_url_match_policy,
                    );
            }
        }
    }

    fn register_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchAuthRequest,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_pending_subresource_fetch_auth_request(request_id, pending);
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_pending_subresource_fetch_auth_request(request_id, pending);
            }
        }
    }

    fn register_pending_subresource_fetch_response_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchResponseRequest,
    ) {
        match self {
            Self::Active(owner) => {
                owner.register_pending_subresource_fetch_response_request(request_id, pending);
            }
            Self::Parked(owner) => {
                owner
                    .fetch_state_mut()
                    .register_pending_subresource_fetch_response_request(request_id, pending);
            }
        }
    }
}

impl TargetSessionOwnerMut<'_> {
    fn open_scoped_io_stream_body_source(&mut self, body: CapturedBody) -> Result<String, String> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let target_id = browser_context
                    .active_target_id_owned()
                    .unwrap_or_else(|| "active".to_owned());
                let owner_key = fetch_stream_owner_key(&browser_context.id, &target_id);
                let runtime_slot = &mut browser_context.active_target.runtime_slot;
                let handle = target_scoped_stream_handle(
                    &owner_key,
                    runtime_slot.allocate_io_stream_handle(),
                );
                runtime_slot.insert_io_stream_body_source(handle.clone(), body, 0);
                Ok(handle)
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let owner_key = fetch_stream_owner_key(&browser_context.id, target_id);
                let Some(target) = browser_context.background_target_mut(target_id) else {
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
            Self::NoLoadedBrowserContext => Err("NoDocumentLoaded".to_owned()),
        }
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
    if browser_context
        .active_target
        .fetch_owner
        .remove_network_intercept(intercept_id)
    {
        let (subresource_enabled, subresource_resource_type) = browser_context
            .active_target
            .fetch_owner
            .subresource_interception_config();
        let Some(page) = browser_context.active_target.runtime_slot.loaded_page_mut() else {
            return Ok(Some(None));
        };
        return page
            .start_set_fetch_subresource_interception(
                subresource_enabled,
                subresource_resource_type,
            )
            .map(Some)
            .map(Some)
            .map_err(|error| format!("failed to update page fetch interception: {error}"));
    }

    let target_ids = browser_context
        .background_targets
        .iter()
        .map(|target| target.target_id().to_owned())
        .collect::<Vec<_>>();
    for target_id in target_ids {
        if browser_context
            .parked_page_session_state(&target_id)
            .is_none()
        {
            continue;
        }
        let removed = browser_context.mutate_parked_page_session_state(&target_id, |state| {
            state.fetch_config.remove_network_intercept(intercept_id)
        });
        if removed {
            return Ok(Some(None));
        }
    }

    Ok(None)
}

impl CdpConnection {
    pub(crate) fn target_fetch_subresource_interception_snapshot_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetFetchSubresourceInterceptionSnapshot> {
        self.target_session_owner_aggregate_fetch_config(session_id)
            .map(|config| config.subresource_interception_snapshot())
    }

    pub(crate) fn target_fetch_subresource_interception_snapshot_for_target(
        &self,
        target_id: &str,
    ) -> Option<TargetFetchSubresourceInterceptionSnapshot> {
        self.target_fetch_config_for_target(target_id)
            .map(|config| config.subresource_interception_snapshot())
    }

    pub(crate) fn target_fetch_event_session_id_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.target_session_owner_aggregate_fetch_config(session_id)
            .and_then(|config| config.session_id().map(str::to_owned))
    }

    pub(crate) fn target_fetch_handle_auth_requests_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.target_session_owner_aggregate_fetch_config(session_id)
            .is_some_and(|config| config.handle_auth_requests())
    }

    pub(crate) fn target_fetch_matches_auth_required_for_session_owner(
        &self,
        session_id: Option<&str>,
        url: &url::Url,
    ) -> bool {
        self.target_session_owner_aggregate_fetch_config(session_id)
            .is_some_and(|config| config.matches_auth_required(url))
    }

    pub(crate) fn target_fetch_matching_auth_required_network_intercepts_for_session_owner(
        &self,
        session_id: Option<&str>,
        url: &url::Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        self.target_session_owner_aggregate_fetch_config(session_id)
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
            CdpSessionRoute::ActiveTarget {
                browser_context_id, ..
            } => self
                .browser_context_by_id(&browser_context_id)
                .map(|browser_context| browser_context.active_target.fetch_owner.config_snapshot()),
            CdpSessionRoute::BackgroundTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .parked_page_session_state(&target_id)
                .map(|state| state.fetch_config.clone()),
            CdpSessionRoute::AuxiliaryTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .background_target(&target_id)
                .and_then(|_| {
                    self.browser_context_by_id(&browser_context_id)?
                        .parked_page_session_state(&target_id)
                        .map(|state| state.fetch_config.clone())
                }),
            CdpSessionRoute::Browser
            | CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => None,
        }
    }

    pub(crate) fn allocate_pending_subresource_fetch_request_ids_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<(String, String), String> {
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .runtime_session_owner_slot_mut(session_id)
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

    pub(crate) fn allocate_fetch_navigation_request_id_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<String, String> {
        self.runtime_session_owner_slot_mut(session_id)
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
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
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

    pub(crate) fn register_synthetic_websocket_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        network_request_id: String,
        socket_id: u64,
    ) -> bool {
        let Ok(runtime_slot) = self.runtime_session_owner_slot_mut(session_id) else {
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
        self.runtime_session_owner_slot(session_id)
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
        session_id: Option<&str>,
        pending: &PendingSubresourceFetchRequest,
    ) -> bool {
        pending.installed_page_owner().is_none_or(|owner| {
            self.target_page_residence_identity_is_current_for_session(session_id, owner)
        })
    }

    fn installed_subresource_fetch_request_is_current(
        &self,
        session_id: Option<&str>,
        pending: &PendingSubresourceFetchRequest,
    ) -> bool {
        pending.installed_page_owner().is_some_and(|owner| {
            self.target_page_residence_identity_is_current_for_session(session_id, owner)
        })
    }

    pub(crate) fn claim_subresource_continue_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        expected_page_owner: &crate::conn::TargetPageResidenceIdentity,
        internal_id: u64,
        allow_pending_completion: bool,
    ) -> Option<crate::conn::ClaimedSubresourceContinueRequest> {
        if !self
            .target_page_residence_identity_is_current_for_session(session_id, expected_page_owner)
        {
            return None;
        }
        self.target_session_owner_mut(session_id)?
            .claim_subresource_continue_request(
                expected_page_owner,
                internal_id,
                session_id,
                allow_pending_completion,
            )
    }

    pub(crate) fn consume_pending_request_action_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Option<Result<(), &'static str>> {
        self.target_session_owner_mut(session_id)?
            .consume_pending_request_action(request_id)
    }

    pub(crate) fn take_pending_fetch_navigation_for_session_owner(
        &mut self,
        owner_session_id: Option<&str>,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingFetchNavigation> {
        self.target_session_owner_mut(owner_session_id)?
            .take_pending_fetch_navigation_for_action_session(request_id, action_session_id)
    }

    pub(crate) fn take_pending_fetch_auth_navigation_for_action_session_owner(
        &mut self,
        owner_session_id: Option<&str>,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingFetchAuthNavigation> {
        self.target_session_owner_mut(owner_session_id)?
            .take_pending_fetch_auth_navigation_for_action_session(request_id, action_session_id)
    }

    pub(crate) fn register_pending_fetch_auth_navigation_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        pending: PendingFetchAuthNavigation,
    ) -> bool {
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.register_pending_fetch_auth_navigation(request_id, pending)
            })
    }

    pub(crate) fn register_pending_fetch_response_navigation_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        document_navigation_token: Option<crate::conn::DocumentNavigationToken>,
        navigation: crate::conn::NavigationDispatchState,
        body: crate::conn::DocumentBodySource,
    ) -> bool {
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.register_pending_fetch_response_navigation(
                    request_id,
                    document_navigation_token,
                    navigation,
                    body,
                )
            })
    }

    pub(crate) fn take_pending_fetch_response_transfer_for_terminal_action_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.target_session_owner_mut(session_id)?
            .take_pending_fetch_response_transfer_for_terminal_action(request_id)
    }

    pub(crate) fn take_pending_fetch_response_transfer_for_body_read_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.target_session_owner_mut(session_id)?
            .take_pending_fetch_response_transfer(request_id)
    }

    pub(crate) fn register_pending_fetch_response_transfer_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        transfer: PausedDocumentTransfer,
    ) -> bool {
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.register_pending_fetch_response_transfer(request_id, transfer)
            })
    }

    pub(crate) fn pending_subresource_fetch_response_request_for_action_session_owner(
        &mut self,
        owner_session_id: Option<&str>,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.target_session_owner_mut(owner_session_id)?
            .pending_subresource_fetch_response_request(request_id, action_session_id)
    }

    pub(crate) fn mark_pending_subresource_fetch_response_body_taken_as_stream_for_action_session_owner(
        &mut self,
        owner_session_id: Option<&str>,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> bool {
        self.target_session_owner_mut(owner_session_id)
            .is_some_and(|mut owner| {
                owner.mark_pending_subresource_fetch_response_body_taken_as_stream(
                    request_id,
                    action_session_id,
                )
            })
    }

    pub(crate) fn open_pending_fetch_response_body_stream_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
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
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        owner.start_pending_fetch_response_body_stream_read(handle, offset, size)
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
        let Some(browser_context) =
            self.browser_context_by_id_mut(&stream_owner.browser_context_id)
        else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        let Some(mut owner) =
            fetch_body_stream_owner_for_target_mut(browser_context, &stream_owner.target_id)
        else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        owner.start_pending_fetch_response_body_stream_read(handle, offset, size)
    }

    pub(crate) fn finish_pending_fetch_response_body_stream_read_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        owner.finish_pending_fetch_response_body_stream_read(completed)
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
        let Some(browser_context) =
            self.browser_context_by_id_mut(&stream_owner.browser_context_id)
        else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        let Some(mut owner) =
            fetch_body_stream_owner_for_target_mut(browser_context, &stream_owner.target_id)
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
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| owner.close_pending_fetch_response_body_stream(handle))
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
        let Some(browser_context) =
            self.browser_context_by_id_mut(&stream_owner.browser_context_id)
        else {
            return false;
        };
        fetch_body_stream_owner_for_target_mut(browser_context, &stream_owner.target_id)
            .is_some_and(|mut owner| owner.close_pending_fetch_response_body_stream(handle))
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
        self.pending_subresource_fetch_request_residence_is_current(session_id, &pending)
            .then_some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_request_for_action_session_owner(
        &mut self,
        owner_session_id: Option<&str>,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchRequest> {
        let pending = self
            .target_session_owner_mut(owner_session_id)?
            .take_pending_subresource_fetch_request(request_id, action_session_id)?;
        self.pending_subresource_fetch_request_residence_is_current(owner_session_id, &pending)
            .then_some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_auth_request_for_action_session_owner(
        &mut self,
        owner_session_id: Option<&str>,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchAuthRequest> {
        let pending = self
            .target_session_owner_mut(owner_session_id)?
            .take_pending_subresource_fetch_auth_request(request_id, action_session_id)?;
        self.target_page_residence_identity_is_current_for_session(
            owner_session_id,
            &pending.page_owner,
        )
        .then_some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_response_request_for_action_session_owner(
        &mut self,
        owner_session_id: Option<&str>,
        action_session_id: Option<&str>,
        request_id: &str,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        let pending = self
            .target_session_owner_mut(owner_session_id)?
            .take_pending_subresource_fetch_response_request(request_id, action_session_id)?;
        self.target_page_residence_identity_is_current_for_session(
            owner_session_id,
            &pending.page_owner,
        )
        .then_some(pending)
    }

    pub(crate) fn take_in_flight_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
    ) -> Option<InFlightSubresourceFetchRequest> {
        let in_flight = self
            .target_session_owner_mut(session_id)?
            .take_in_flight_subresource_fetch_request(internal_id)?;
        self.installed_subresource_fetch_request_is_current(session_id, &in_flight.pending)
            .then_some(in_flight)
    }

    pub(crate) fn in_flight_subresource_fetch_request_id_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
    ) -> Option<String> {
        let (request_id, page_owner) = self
            .target_session_owner_mut(session_id)?
            .in_flight_subresource_fetch_request_identity(internal_id)?;
        self.target_page_residence_identity_is_current_for_session(session_id, &page_owner)
            .then_some(request_id)
    }

    pub(crate) fn register_pending_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if !self.pending_subresource_fetch_request_residence_is_current(session_id, &pending) {
            return false;
        }
        self.record_pending_subresource_network_request_identity(session_id, &pending);
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.register_pending_subresource_fetch_request(request_id, pending)
            })
    }

    pub(crate) fn register_in_flight_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if !self.installed_subresource_fetch_request_is_current(session_id, &pending) {
            return false;
        }
        self.record_pending_subresource_network_request_identity(session_id, &pending);
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.register_in_flight_subresource_fetch_request(request_id, pending)
            })
    }

    pub(crate) fn register_in_flight_response_stage_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) -> bool {
        if !self.installed_subresource_fetch_request_is_current(session_id, &pending) {
            return false;
        }
        self.record_pending_subresource_network_request_identity(session_id, &pending);
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.register_in_flight_response_stage_subresource_fetch_request(
                    request_id,
                    pending,
                    response_stage_blocked_intercepts,
                );
                true
            })
    }

    pub(crate) fn register_in_flight_deferred_response_stage_subresource_fetch_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) -> bool {
        if !self.installed_subresource_fetch_request_is_current(session_id, &pending) {
            return false;
        }
        self.record_pending_subresource_network_request_identity(session_id, &pending);
        self.target_session_owner_mut(session_id)
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
        if !self
            .target_page_residence_identity_is_current_for_session(session_id, &pending.page_owner)
        {
            return false;
        }
        self.record_pending_subresource_auth_network_request_identity(session_id, &pending);
        self.target_session_owner_mut(session_id)
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
        if !self
            .target_page_residence_identity_is_current_for_session(session_id, &pending.page_owner)
        {
            return false;
        }
        self.record_pending_subresource_response_network_request_identity(session_id, &pending);
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.register_pending_subresource_fetch_response_request(request_id, pending)
            })
    }

    fn record_pending_subresource_network_request_identity(
        &mut self,
        session_id: Option<&str>,
        pending: &PendingSubresourceFetchRequest,
    ) {
        let Some(handle) = pending.network_request_handle else {
            return;
        };
        if let Ok(runtime_slot) = self.runtime_session_owner_slot_mut(session_id) {
            runtime_slot.record_subresource_request_id_for_handle_if_absent(
                handle,
                pending.network_request_id.clone(),
            );
        }
    }

    fn record_pending_subresource_auth_network_request_identity(
        &mut self,
        session_id: Option<&str>,
        pending: &PendingSubresourceFetchAuthRequest,
    ) {
        let Some(handle) = pending.network_request_handle else {
            return;
        };
        if let Ok(runtime_slot) = self.runtime_session_owner_slot_mut(session_id) {
            runtime_slot.record_subresource_request_id_for_handle_if_absent(
                handle,
                pending.network_request_id.clone(),
            );
        }
    }

    fn record_pending_subresource_response_network_request_identity(
        &mut self,
        session_id: Option<&str>,
        pending: &PendingSubresourceFetchResponseRequest,
    ) {
        let Some(handle) = pending.network_request_handle else {
            return;
        };
        if let Ok(runtime_slot) = self.runtime_session_owner_slot_mut(session_id) {
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
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some((subresource_enabled, subresource_resource_type)) = owner.configure_fetch(
            session_id.map(str::to_owned),
            handle_auth_requests,
            patterns,
        ) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some(page) = owner
            .runtime_slot_mut()
            .and_then(TargetRuntimeSlot::loaded_page_mut)
        else {
            return Ok(None);
        };
        page.start_set_fetch_subresource_interception(
            subresource_enabled,
            subresource_resource_type,
        )
        .map(Some)
        .map_err(|error| format!("failed to update page fetch interception: {error}"))
    }

    pub(crate) fn start_add_network_intercept_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        intercept_session_id: Option<String>,
        intercept_id: String,
        handle_auth_requests: bool,
        auth_url_patterns: Vec<String>,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some((subresource_enabled, subresource_resource_type)) = owner.add_network_intercept(
            intercept_id,
            intercept_session_id,
            handle_auth_requests,
            auth_url_patterns,
            patterns,
        ) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some(page) = owner
            .runtime_slot_mut()
            .and_then(TargetRuntimeSlot::loaded_page_mut)
        else {
            return Ok(None);
        };
        page.start_set_fetch_subresource_interception(
            subresource_enabled,
            subresource_resource_type,
        )
        .map(Some)
        .map_err(|error| format!("failed to update page fetch interception: {error}"))
    }

    pub(crate) fn start_remove_network_intercept_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        intercept_id: &str,
        allow_global_lookup: bool,
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
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
        let Some(page) = owner
            .runtime_slot_mut()
            .and_then(TargetRuntimeSlot::loaded_page_mut)
        else {
            return Ok(None);
        };
        page.start_set_fetch_subresource_interception(
            subresource_enabled,
            subresource_resource_type,
        )
        .map(Some)
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
    ) -> Result<
        Option<(
            SessionOwnerPendingFetchState,
            Option<moli_core::page::PendingPageCommand>,
        )>,
        String,
    > {
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return Ok(None);
        };
        let Some((pending, (subresource_enabled, subresource_resource_type), page_update_required)) =
            owner.reset_fetch_config_for_session_and_drain_pending_state(session_id)
        else {
            return Ok(None);
        };
        let page_command = if page_update_required
            && let Some(page) = owner
                .runtime_slot_mut()
                .and_then(TargetRuntimeSlot::loaded_page_mut)
        {
            Some(
                page.start_set_fetch_subresource_interception(
                    subresource_enabled,
                    subresource_resource_type,
                )
                .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        Ok(Some((pending, page_command)))
    }

    pub(crate) fn take_pending_fetch_state_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<SessionOwnerPendingFetchState> {
        self.target_session_owner_mut(session_id)?
            .drain_fetch_pending_state()
    }
}

fn pending_fetch_request_route(
    browser_context: &BrowserContext,
    request_id: &str,
) -> Option<CdpSessionRoute> {
    if browser_context
        .active_target
        .fetch_owner
        .contains_pending_request(request_id)
    {
        return Some(CdpSessionRoute::ActiveTarget {
            browser_context_id: browser_context.id.clone(),
            target_id: browser_context.active_target_id_owned(),
        });
    }

    browser_context
        .background_targets
        .iter()
        .find_map(|target| {
            browser_context
                .target_parking
                .fetch_state(target.target_id())
                .is_some_and(|state| state.contains_pending_request(request_id))
                .then(|| CdpSessionRoute::BackgroundTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target.target_id().to_owned(),
                })
        })
}

impl TargetSessionOwnerMut<'_> {
    fn session_id(&self) -> Option<&str> {
        match self {
            Self::ActiveTarget { session_id, .. } | Self::BackgroundTarget { session_id, .. } => {
                session_id.as_deref()
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    fn pending_fetch_owner_mut(&mut self) -> Option<SessionPendingFetchOwner<'_>> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(SessionPendingFetchOwner::Active(
                &mut browser_context.active_target.fetch_owner,
            )),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => Some(SessionPendingFetchOwner::Parked(Box::new(
                ParkedPendingFetchOwner::take(browser_context, target_id),
            ))),
            Self::NoLoadedBrowserContext => None,
        }
    }

    fn fetch_body_stream_owner_mut(&mut self) -> Option<SessionFetchBodyStreamOwner<'_>> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let target_id = browser_context
                    .active_target_id_owned()
                    .unwrap_or_else(|| "active".to_owned());
                fetch_body_stream_owner_for_target_mut(browser_context, &target_id)
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => fetch_body_stream_owner_for_target_mut(browser_context, target_id),
            Self::NoLoadedBrowserContext => None,
        }
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

    fn register_pending_fetch_response_navigation(
        &mut self,
        request_id: String,
        document_navigation_token: Option<crate::conn::DocumentNavigationToken>,
        navigation: crate::conn::NavigationDispatchState,
        body: crate::conn::DocumentBodySource,
    ) -> bool {
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_pending_fetch_response_navigation(
            request_id,
            document_navigation_token,
            navigation,
            body,
        );
        true
    }

    fn take_pending_fetch_response_transfer_for_terminal_action(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.pending_fetch_owner_mut()?
            .take_pending_fetch_response_transfer_for_terminal_action(request_id)
    }

    fn take_pending_fetch_response_transfer(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.pending_fetch_owner_mut()?
            .take_pending_fetch_response_transfer(request_id)
    }

    fn register_pending_fetch_response_transfer(
        &mut self,
        request_id: String,
        transfer: PausedDocumentTransfer,
    ) -> bool {
        let Some(mut owner) = self.pending_fetch_owner_mut() else {
            return false;
        };
        owner.register_pending_fetch_response_transfer(request_id, transfer);
        true
    }

    fn pending_subresource_fetch_response_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.pending_fetch_owner_mut()?
            .pending_subresource_fetch_response_request(request_id, session_id)
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

    fn open_pending_fetch_response_body_stream(
        &mut self,
        request_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(mut owner) = self.fetch_body_stream_owner_mut() else {
            return Ok(None);
        };
        owner.open_pending_fetch_response_body_stream(request_id)
    }

    fn start_pending_fetch_response_body_stream_read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let Some(mut owner) = self.fetch_body_stream_owner_mut() else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        owner.start_pending_fetch_response_body_stream_read(handle, offset, size)
    }

    fn finish_pending_fetch_response_body_stream_read(
        &mut self,
        completed: CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let Some(mut owner) = self.fetch_body_stream_owner_mut() else {
            return PendingFetchResponseBodyStreamRead::NotFound;
        };
        owner.finish_pending_fetch_response_body_stream_read(completed)
    }

    fn close_pending_fetch_response_body_stream(&mut self, handle: &str) -> bool {
        self.fetch_body_stream_owner_mut()
            .is_some_and(|mut owner| owner.close_pending_fetch_response_body_stream(handle))
    }
}
