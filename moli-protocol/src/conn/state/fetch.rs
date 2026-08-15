use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};
use url::Url;

use super::super::fetch_support::{
    ClaimedSubresourceContinueRequest, DocumentBodySource, InFlightSubresourceFetchRequest,
    PausedDocumentTransfer, PausedDocumentTransfers, PendingFetchAuthNavigation,
    PendingFetchNavigation, PendingFetchResponseBodyStreamRead,
    PendingFetchResponseBodyStreamReadDispatch, PendingFetchResponseBodyStreamReadStart,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchOwnerKind,
    PendingSubresourceFetchRequest, PendingSubresourceFetchResponseRequest,
    ResponseStageUrlMatchPolicy, fetch_subresource_interception_config_for_patterns,
    matching_fetch_pattern,
};
pub(super) use super::super::fetch_support::{
    FetchInterceptionPattern, FetchRequestStage, FetchResourceTypeFilter, OpenBodyStreamError,
};
use super::navigation_outcome::NavigationDispatchState;
use super::runtime_slot::TargetRuntimeSlot;
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use moli_fetch::url_pattern_matches;

#[derive(Debug, Default)]
pub struct TargetFetchState {
    pending_fetch_request_ids: HashSet<String>,
    pending_fetch_navigations: HashMap<String, PendingFetchNavigation>,
    pending_fetch_auth_navigations: HashMap<String, PendingFetchAuthNavigation>,
    pending_fetch_response_transfers: PausedDocumentTransfers,
    pending_subresource_fetches: HashMap<String, PendingSubresourceFetchRequest>,
    in_flight_subresource_fetches: HashMap<u64, InFlightSubresourceFetchRequest>,
    pending_subresource_fetch_auths: HashMap<String, PendingSubresourceFetchAuthRequest>,
    pending_subresource_fetch_responses: HashMap<String, PendingSubresourceFetchResponseRequest>,
}

impl TargetFetchState {
    fn pending_action_matches(
        pending: &PendingSubresourceFetchRequest,
        session_id: Option<&str>,
    ) -> bool {
        pending.action_session_id.as_deref() == session_id
    }

    fn pending_navigation_action_matches(
        pending: &PendingFetchNavigation,
        session_id: Option<&str>,
    ) -> bool {
        pending.interception_session_id.as_deref() == session_id
    }

    fn pending_auth_action_matches(
        pending: &PendingSubresourceFetchAuthRequest,
        session_id: Option<&str>,
    ) -> bool {
        Self::auth_pause_action_matches(
            pending.owner_kind,
            pending.action_session_id.as_deref(),
            session_id,
        )
    }

    fn pending_response_action_matches(
        pending: &PendingSubresourceFetchResponseRequest,
        session_id: Option<&str>,
    ) -> bool {
        pending.action_session_id.as_deref() == session_id
    }

    fn pending_owner_matches_disable(
        owner_session_id: &Option<String>,
        session_id: Option<&str>,
    ) -> bool {
        owner_session_id.is_none() || owner_session_id.as_deref() == session_id
    }

    fn pending_fetch_owner_matches_disable(
        owner_kind: PendingSubresourceFetchOwnerKind,
        owner_session_id: &Option<String>,
        session_id: Option<&str>,
    ) -> bool {
        owner_kind.drains_on_fetch_disable()
            && Self::pending_owner_matches_disable(owner_session_id, session_id)
    }

    fn navigation_owner_matches_disable(
        owner_session_id: &Option<String>,
        session_id: Option<&str>,
    ) -> bool {
        session_id.is_none() || Self::pending_owner_matches_disable(owner_session_id, session_id)
    }

    fn auth_navigation_owner_matches_disable(
        pending: &PendingFetchAuthNavigation,
        session_id: Option<&str>,
    ) -> bool {
        pending.owner_kind.drains_on_fetch_disable()
            && Self::navigation_owner_matches_disable(&pending.owner_session_id, session_id)
    }

    fn auth_navigation_action_matches(
        pending: &PendingFetchAuthNavigation,
        action_session_id: Option<&str>,
    ) -> bool {
        Self::auth_pause_action_matches(
            pending.owner_kind,
            pending.action_session_id.as_deref(),
            action_session_id,
        )
    }

    fn auth_pause_action_matches(
        owner_kind: PendingSubresourceFetchOwnerKind,
        pending_action_session_id: Option<&str>,
        action_session_id: Option<&str>,
    ) -> bool {
        if pending_action_session_id == action_session_id {
            return true;
        }
        // Some Fetch-owned auth pauses are addressed through a target/request-id
        // route instead of a concrete protocol session. Keep strict matching
        // when both sides name sessions, but allow the routed action when either
        // side is target-scoped.
        (pending_action_session_id.is_none() || action_session_id.is_none())
            && matches!(owner_kind, PendingSubresourceFetchOwnerKind::Fetch)
    }

    fn drain_pending_fetch_navigations_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<PendingFetchNavigation> {
        let request_ids = self
            .pending_fetch_navigations
            .iter()
            .filter(|(_, pending)| {
                Self::navigation_owner_matches_disable(&pending.interception_session_id, session_id)
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        let mut drained = Vec::new();
        for request_id in request_ids {
            if let Some(pending) = self.pending_fetch_navigations.remove(&request_id) {
                self.pending_fetch_request_ids.remove(&request_id);
                drained.push(pending);
            }
        }
        drained
    }

    fn drain_pending_fetch_auth_navigations_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<PendingFetchAuthNavigation> {
        let request_ids = self
            .pending_fetch_auth_navigations
            .iter()
            .filter(|(_, pending)| Self::auth_navigation_owner_matches_disable(pending, session_id))
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        let mut drained = Vec::new();
        for request_id in request_ids {
            if let Some(pending) = self.pending_fetch_auth_navigations.remove(&request_id) {
                self.pending_fetch_request_ids.remove(&request_id);
                drained.push(pending);
            }
        }
        drained
    }

    fn drain_pending_subresource_fetches_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<(String, PendingSubresourceFetchRequest)> {
        let request_ids = self
            .pending_subresource_fetches
            .iter()
            .filter(|(_, pending)| {
                Self::pending_fetch_owner_matches_disable(
                    pending.owner_kind,
                    &pending.owner_session_id,
                    session_id,
                )
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        let mut drained = Vec::new();
        for request_id in request_ids {
            if let Some(pending) = self.pending_subresource_fetches.remove(&request_id) {
                self.pending_fetch_request_ids.remove(&request_id);
                drained.push((request_id, pending));
            }
        }
        drained
    }

    fn drain_pending_subresource_auths_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<(String, PendingSubresourceFetchAuthRequest)> {
        let request_ids = self
            .pending_subresource_fetch_auths
            .iter()
            .filter(|(_, pending)| {
                Self::pending_fetch_owner_matches_disable(
                    pending.owner_kind,
                    &pending.owner_session_id,
                    session_id,
                )
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        let mut drained = Vec::new();
        for request_id in request_ids {
            if let Some(pending) = self.pending_subresource_fetch_auths.remove(&request_id) {
                self.pending_fetch_request_ids.remove(&request_id);
                drained.push((request_id, pending));
            }
        }
        drained
    }

    fn drain_pending_subresource_responses_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<(String, PendingSubresourceFetchResponseRequest)> {
        let request_ids = self
            .pending_subresource_fetch_responses
            .iter()
            .filter(|(_, pending)| {
                Self::pending_fetch_owner_matches_disable(
                    pending.owner_kind,
                    &pending.owner_session_id,
                    session_id,
                )
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        let mut drained = Vec::new();
        for request_id in request_ids {
            if let Some(pending) = self.pending_subresource_fetch_responses.remove(&request_id) {
                self.pending_fetch_request_ids.remove(&request_id);
                drained.push((request_id, pending));
            }
        }
        drained
    }

    fn clear_in_flight_response_stage_for_session(&mut self, session_id: Option<&str>) {
        for in_flight in self.in_flight_subresource_fetches.values_mut() {
            if !Self::pending_fetch_owner_matches_disable(
                in_flight.pending.owner_kind,
                &in_flight.pending.owner_session_id,
                session_id,
            ) {
                continue;
            }
            if let Some(request_id) = in_flight.request_id.take() {
                self.pending_fetch_request_ids.remove(&request_id);
            }
            in_flight.response_stage_url_match_policy = ResponseStageUrlMatchPolicy::AlreadyMatched;
            in_flight.response_stage_blocked_intercepts.clear();
        }
    }

    fn prune_request_stage_chains_for_session(&mut self, session_id: Option<&str>) {
        for pending in self.pending_subresource_fetches.values_mut() {
            if let Some(chain) = pending.request_stage_chain.as_mut() {
                chain.remaining_sessions.retain(|stage| {
                    !stage.owner_kind.drains_on_fetch_disable()
                        || stage.session_id.as_deref() != session_id
                });
            }
        }
    }

    fn prune_response_stage_chains_for_session(&mut self, session_id: Option<&str>) {
        for pending in self.pending_subresource_fetch_responses.values_mut() {
            if let Some(chain) = pending.response_stage_chain.as_mut() {
                chain
                    .remaining_sessions
                    .retain(|stage| stage.session_id.as_deref() != session_id);
            }
        }
    }

    fn prune_auth_stage_chains_for_session(&mut self, session_id: Option<&str>) {
        for pending in self.pending_fetch_auth_navigations.values_mut() {
            if let Some(chain) = pending.auth_stage_chain.as_mut() {
                chain
                    .remaining_sessions
                    .retain(|stage| stage.session_id.as_deref() != session_id);
            }
        }
        for pending in self.pending_subresource_fetch_auths.values_mut() {
            if let Some(chain) = pending.auth_stage_chain.as_mut() {
                chain
                    .remaining_sessions
                    .retain(|stage| stage.session_id.as_deref() != session_id);
            }
        }
    }

    fn request_id_has_pending_bucket(&self, request_id: &str) -> bool {
        self.pending_fetch_navigations.contains_key(request_id)
            || self.pending_fetch_auth_navigations.contains_key(request_id)
            || self
                .pending_fetch_response_transfers
                .contains_request(request_id)
            || self.pending_subresource_fetches.contains_key(request_id)
            || self
                .pending_subresource_fetch_auths
                .contains_key(request_id)
            || self
                .pending_subresource_fetch_responses
                .contains_key(request_id)
            || self
                .in_flight_subresource_fetches
                .values()
                .any(|in_flight| in_flight.request_id.as_deref() == Some(request_id))
    }

    fn clear_orphan_pending_request_ids_for_disable(&mut self, session_id: Option<&str>) {
        if session_id.is_some() {
            return;
        }
        let orphan_request_ids = self
            .pending_fetch_request_ids
            .iter()
            .filter(|request_id| !self.request_id_has_pending_bucket(request_id))
            .cloned()
            .collect::<Vec<_>>();
        for request_id in orphan_request_ids {
            self.pending_fetch_request_ids.remove(&request_id);
        }
    }

    fn drain_pending_request_maps(
        &mut self,
        clear_in_flight: bool,
    ) -> (
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    ) {
        let pending_navigations = std::mem::take(&mut self.pending_fetch_navigations)
            .into_values()
            .collect::<Vec<_>>();
        let pending_auth_navigations = std::mem::take(&mut self.pending_fetch_auth_navigations)
            .into_values()
            .collect::<Vec<_>>();
        let pending_response_navigations = self
            .pending_fetch_response_transfers
            .drain_pending_transfers();
        let pending_subresource_fetches = std::mem::take(&mut self.pending_subresource_fetches)
            .into_iter()
            .collect::<Vec<_>>();
        let pending_subresource_auths = std::mem::take(&mut self.pending_subresource_fetch_auths)
            .into_iter()
            .collect::<Vec<_>>();
        let pending_subresource_responses =
            std::mem::take(&mut self.pending_subresource_fetch_responses)
                .into_iter()
                .collect::<Vec<_>>();
        if clear_in_flight {
            self.in_flight_subresource_fetches.clear();
        }
        for fetch_request_id in pending_navigations
            .iter()
            .map(|pending| pending.fetch_request_id.as_str())
            .chain(
                pending_auth_navigations
                    .iter()
                    .map(|pending| pending.fetch_request_id.as_str()),
            )
            .chain(
                pending_response_navigations
                    .iter()
                    .map(|pending| pending.fetch_request_id()),
            )
            .chain(
                pending_subresource_fetches
                    .iter()
                    .map(|(request_id, _)| request_id.as_str()),
            )
            .chain(
                pending_subresource_auths
                    .iter()
                    .map(|(request_id, _)| request_id.as_str()),
            )
            .chain(
                pending_subresource_responses
                    .iter()
                    .map(|(request_id, _)| request_id.as_str()),
            )
        {
            self.pending_fetch_request_ids.remove(fetch_request_id);
        }
        (
            pending_navigations,
            pending_auth_navigations,
            pending_response_navigations,
            pending_subresource_fetches,
            pending_subresource_auths,
            pending_subresource_responses,
        )
    }

    pub(crate) fn register_pending_fetch_navigation_request(
        &mut self,
        pending: PendingFetchNavigation,
    ) {
        self.pending_fetch_request_ids
            .insert(pending.fetch_request_id.clone());
        self.pending_fetch_navigations
            .insert(pending.fetch_request_id.clone(), pending);
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_navigation(&self) -> bool {
        !self.pending_fetch_navigations.is_empty()
    }

    pub(crate) fn pending_subresource_fetch_response_request(
        &self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<&PendingSubresourceFetchResponseRequest> {
        let pending = self.pending_subresource_fetch_responses.get(request_id)?;
        Self::pending_response_action_matches(pending, session_id).then_some(pending)
    }

    pub(crate) fn mark_pending_subresource_fetch_response_body_taken_as_stream(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> bool {
        let Some(pending) = self.pending_subresource_fetch_responses.get_mut(request_id) else {
            return false;
        };
        if !Self::pending_response_action_matches(pending, session_id) {
            return false;
        }
        pending.response_body_taken_as_stream = true;
        true
    }

    pub(crate) fn consume_pending_request_action(
        &mut self,
        request_id: &str,
    ) -> Result<(), &'static str> {
        // These buckets own request ids that must be completed through a
        // more specific Fetch command path. Keep the internal error string
        // aligned with the Fetch domain wire error for wrong-action ids.
        if self
            .pending_fetch_response_transfers
            .contains_request(request_id)
            || self.pending_fetch_navigations.contains_key(request_id)
            || self.pending_fetch_auth_navigations.contains_key(request_id)
            || self.pending_subresource_fetches.contains_key(request_id)
            || self
                .pending_subresource_fetch_auths
                .contains_key(request_id)
            || self
                .pending_subresource_fetch_responses
                .contains_key(request_id)
        {
            return Err("RequestNotFound");
        }
        if !self.pending_fetch_request_ids.remove(request_id) {
            return Err("RequestNotFound");
        }
        Ok(())
    }

    pub(crate) fn contains_pending_request(&self, request_id: &str) -> bool {
        self.pending_fetch_request_ids.contains(request_id)
    }

    pub(crate) fn take_pending_fetch_navigation(
        &mut self,
        request_id: &str,
    ) -> Option<PendingFetchNavigation> {
        let pending = self.pending_fetch_navigations.remove(request_id)?;
        self.pending_fetch_request_ids.remove(request_id);
        Some(pending)
    }

    pub(crate) fn take_pending_fetch_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchNavigation> {
        let pending = self.pending_fetch_navigations.get(request_id)?;
        if !Self::pending_navigation_action_matches(pending, action_session_id) {
            return None;
        }
        self.take_pending_fetch_navigation(request_id)
    }

    pub(crate) fn take_pending_fetch_auth_navigation(
        &mut self,
        request_id: &str,
    ) -> Option<PendingFetchAuthNavigation> {
        let pending = self.pending_fetch_auth_navigations.remove(request_id)?;
        self.pending_fetch_request_ids.remove(request_id);
        Some(pending)
    }

    pub(crate) fn take_pending_fetch_auth_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchAuthNavigation> {
        let pending = self.pending_fetch_auth_navigations.get(request_id)?;
        if !Self::auth_navigation_action_matches(pending, action_session_id) {
            return None;
        }
        self.take_pending_fetch_auth_navigation(request_id)
    }

    pub(crate) fn register_pending_fetch_auth_navigation(
        &mut self,
        request_id: String,
        pending: PendingFetchAuthNavigation,
    ) {
        self.pending_fetch_request_ids.insert(request_id.clone());
        self.pending_fetch_auth_navigations
            .insert(request_id, pending);
    }

    pub(crate) fn register_pending_fetch_response_navigation(
        &mut self,
        request_id: String,
        document_navigation_token: Option<super::DocumentNavigationToken>,
        navigation: NavigationDispatchState,
        body: DocumentBodySource,
    ) {
        self.pending_fetch_request_ids.insert(request_id.clone());
        self.pending_fetch_response_transfers
            .register_pending_navigation(request_id, document_navigation_token, navigation, body);
    }

    pub(crate) fn take_pending_fetch_response_transfer_for_terminal_action(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        let transfer = self.pending_fetch_response_transfers.take(request_id)?;
        self.pending_fetch_request_ids.remove(request_id);
        Some(transfer)
    }

    pub(crate) fn take_pending_fetch_response_transfer(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.pending_fetch_response_transfers.take(request_id)
    }

    pub(crate) fn register_pending_fetch_response_transfer(
        &mut self,
        request_id: String,
        transfer: PausedDocumentTransfer,
    ) {
        self.pending_fetch_request_ids.insert(request_id.clone());
        self.pending_fetch_response_transfers
            .register(request_id, transfer);
    }

    pub(crate) fn take_pending_fetch_response_body_stream_by_handle(
        &mut self,
        handle: &str,
    ) -> Option<(String, PausedDocumentTransfer)> {
        self.pending_fetch_response_transfers
            .take_body_stream_by_handle(handle)
    }

    pub(crate) fn open_pending_fetch_response_body_stream(
        &mut self,
        runtime_slot: &mut TargetRuntimeSlot,
        request_id: &str,
        handle: String,
    ) -> Result<Option<String>, String> {
        let Some(transfer) = self.take_pending_fetch_response_transfer(request_id) else {
            return Ok(None);
        };
        let opened = match transfer.open_body_stream(handle) {
            Ok(opened) => opened,
            Err(OpenBodyStreamError::NotOpenable(transfer)) => {
                self.register_pending_fetch_response_transfer(request_id.to_owned(), *transfer);
                return Ok(None);
            }
            Err(OpenBodyStreamError::Failed { transfer, message }) => {
                self.register_pending_fetch_response_transfer(request_id.to_owned(), *transfer);
                return Err(message);
            }
        };

        let handle = opened.handle;
        let buffered_bytes = opened.buffered_bytes;
        self.register_pending_fetch_response_transfer(request_id.to_owned(), opened.transfer);
        if let Some(bytes) = buffered_bytes {
            runtime_slot.insert_io_stream(handle.clone(), bytes, 0);
        }

        Ok(Some(handle))
    }

    pub(crate) fn start_pending_fetch_response_body_stream_read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let Some((request_id, transfer)) =
            self.take_pending_fetch_response_body_stream_by_handle(handle)
        else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };

        if let Some(offset) = offset
            && offset != transfer.body_stream_offset().unwrap_or(0)
        {
            self.register_pending_fetch_response_transfer(request_id, transfer);
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

    pub(crate) fn finish_pending_fetch_response_body_stream_read(
        &mut self,
        runtime_slot: &mut TargetRuntimeSlot,
        completed: super::super::fetch_support::CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        let request_id = completed.request_id().to_owned();
        let handle = completed.handle().to_owned();
        match completed.into_completed() {
            Ok((bytes, eof, transfer)) => {
                self.register_pending_fetch_response_transfer(request_id, transfer);
                if eof {
                    runtime_slot.insert_io_stream(handle, Vec::new(), 0);
                }
                PendingFetchResponseBodyStreamRead::Read { bytes, eof }
            }
            Err(completed) => {
                let (transfer, message) = *completed;
                self.register_pending_fetch_response_transfer(request_id, transfer);
                PendingFetchResponseBodyStreamRead::Failed(message)
            }
        }
    }

    pub(crate) fn close_pending_fetch_response_body_stream(&mut self, handle: &str) -> bool {
        let Some((request_id, _)) = self.take_pending_fetch_response_body_stream_by_handle(handle)
        else {
            return false;
        };
        self.pending_fetch_request_ids.remove(&request_id);
        true
    }

    pub(crate) fn take_pending_subresource_fetch_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchRequest> {
        let pending = self.pending_subresource_fetches.get(request_id)?;
        if !Self::pending_action_matches(pending, session_id) {
            return None;
        }
        let pending = self.pending_subresource_fetches.remove(request_id)?;
        self.pending_fetch_request_ids.remove(request_id);
        Some(pending)
    }

    pub(crate) fn take_in_flight_subresource_fetch_request(
        &mut self,
        internal_id: u64,
    ) -> Option<InFlightSubresourceFetchRequest> {
        self.in_flight_subresource_fetches.remove(&internal_id)
    }

    /// Atomically authorize and remove the protocol state correlated with one
    /// renderer continuation.
    ///
    /// The registry can still contain entries owned by a retired loaded-Page
    /// generation. Those entries must remain resident when a continuation for
    /// a replacement Page reuses the same renderer-local `internal_id`;
    /// removing first and validating later silently loses the old paused
    /// request. Matching the exact owner while the maps are borrowed makes
    /// owner mismatch a non-consuming observation.
    pub(crate) fn claim_subresource_continue_request(
        &mut self,
        expected_page_owner: &super::TargetPageResidenceIdentity,
        internal_id: u64,
        session_id: Option<&str>,
        allow_pending_completion: bool,
    ) -> Option<ClaimedSubresourceContinueRequest> {
        let in_flight_matches = self
            .in_flight_subresource_fetches
            .get(&internal_id)
            .and_then(|in_flight| in_flight.pending.installed_page_owner())
            == Some(expected_page_owner);
        if in_flight_matches {
            return self
                .in_flight_subresource_fetches
                .remove(&internal_id)
                .map(ClaimedSubresourceContinueRequest::InFlight);
        }
        if !allow_pending_completion {
            return None;
        }

        let request_id =
            self.pending_subresource_fetches
                .iter()
                .find_map(|(request_id, pending)| {
                    (pending.internal_id == internal_id
                        && Self::pending_action_matches(pending, session_id)
                        && pending.installed_page_owner() == Some(expected_page_owner))
                    .then(|| request_id.clone())
                })?;
        self.take_pending_subresource_fetch_request(&request_id, session_id)
            .map(ClaimedSubresourceContinueRequest::PendingCompletion)
    }

    pub(crate) fn in_flight_subresource_fetch_request_id(&self, internal_id: u64) -> Option<&str> {
        self.in_flight_subresource_fetches
            .get(&internal_id)?
            .request_id
            .as_deref()
    }

    pub(crate) fn in_flight_subresource_fetch_request_page_owner(
        &self,
        internal_id: u64,
    ) -> Option<&super::TargetPageResidenceIdentity> {
        self.in_flight_subresource_fetches
            .get(&internal_id)?
            .pending
            .installed_page_owner()
    }

    pub(crate) fn register_pending_subresource_fetch_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    ) {
        self.pending_fetch_request_ids.insert(request_id.clone());
        self.pending_subresource_fetches.insert(request_id, pending);
    }

    pub(crate) fn register_in_flight_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) {
        self.register_in_flight_subresource_fetch_request_with_response_match_policy_and_intercepts(
            request_id,
            pending,
            ResponseStageUrlMatchPolicy::AlreadyMatched,
            Vec::new(),
        );
    }

    pub(crate) fn register_in_flight_subresource_fetch_request_with_response_match_policy(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
    ) {
        self.register_in_flight_subresource_fetch_request_with_response_match_policy_and_intercepts(
            request_id,
            pending,
            response_stage_url_match_policy,
            Vec::new(),
        );
    }

    pub(crate) fn register_in_flight_response_stage_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) {
        self.register_in_flight_subresource_fetch_request_with_response_match_policy_and_intercepts(
            request_id,
            pending,
            ResponseStageUrlMatchPolicy::AlreadyMatched,
            response_stage_blocked_intercepts,
        );
    }

    fn register_in_flight_subresource_fetch_request_with_response_match_policy_and_intercepts(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) {
        self.in_flight_subresource_fetches.insert(
            pending.internal_id,
            InFlightSubresourceFetchRequest {
                request_id,
                pending,
                response_stage_url_match_policy,
                response_stage_blocked_intercepts,
            },
        );
    }

    pub(crate) fn register_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchAuthRequest,
    ) {
        self.pending_fetch_request_ids.insert(request_id.clone());
        self.pending_subresource_fetch_auths
            .insert(request_id, pending);
    }

    pub(crate) fn register_pending_subresource_fetch_response_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchResponseRequest,
    ) {
        self.pending_fetch_request_ids.insert(request_id.clone());
        self.pending_subresource_fetch_responses
            .insert(request_id, pending);
    }

    pub(crate) fn take_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchAuthRequest> {
        let pending = self.pending_subresource_fetch_auths.get(request_id)?;
        if !Self::pending_auth_action_matches(pending, session_id) {
            return None;
        }
        let pending = self.pending_subresource_fetch_auths.remove(request_id)?;
        self.pending_fetch_request_ids.remove(request_id);
        Some(pending)
    }

    pub(crate) fn take_pending_subresource_fetch_response_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        let pending = self.pending_subresource_fetch_responses.get(request_id)?;
        if !Self::pending_response_action_matches(pending, session_id) {
            return None;
        }
        let pending = self
            .pending_subresource_fetch_responses
            .remove(request_id)?;
        self.pending_fetch_request_ids.remove(request_id);
        Some(pending)
    }

    pub(crate) fn drain_pending_requests(
        &mut self,
    ) -> (
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    ) {
        self.drain_pending_request_maps(true)
    }

    pub(crate) fn drain_pending_requests_for_disable_session(
        &mut self,
        session_id: Option<&str>,
    ) -> (
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    ) {
        let pending_navigations = self.drain_pending_fetch_navigations_for_session(session_id);
        let pending_auth_navigations =
            self.drain_pending_fetch_auth_navigations_for_session(session_id);
        let pending_response_navigations = self
            .pending_fetch_response_transfers
            .drain_pending_transfers_for_session(session_id);
        for pending in &pending_response_navigations {
            self.pending_fetch_request_ids
                .remove(pending.fetch_request_id());
        }
        let pending_subresource_fetches =
            self.drain_pending_subresource_fetches_for_session(session_id);
        let pending_subresource_auths =
            self.drain_pending_subresource_auths_for_session(session_id);
        let pending_subresource_responses =
            self.drain_pending_subresource_responses_for_session(session_id);

        self.clear_in_flight_response_stage_for_session(session_id);
        self.prune_request_stage_chains_for_session(session_id);
        self.prune_response_stage_chains_for_session(session_id);
        self.prune_auth_stage_chains_for_session(session_id);
        self.clear_orphan_pending_request_ids_for_disable(session_id);

        (
            pending_navigations,
            pending_auth_navigations,
            pending_response_navigations,
            pending_subresource_fetches,
            pending_subresource_auths,
            pending_subresource_responses,
        )
    }

    pub(crate) fn clear(&mut self) {
        self.pending_fetch_request_ids.clear();
        self.pending_fetch_navigations.clear();
        self.pending_fetch_auth_navigations.clear();
        self.pending_fetch_response_transfers.clear();
        self.pending_subresource_fetches.clear();
        self.in_flight_subresource_fetches.clear();
        self.pending_subresource_fetch_auths.clear();
        self.pending_subresource_fetch_responses.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.pending_fetch_request_ids.is_empty()
            && self.pending_fetch_navigations.is_empty()
            && self.pending_fetch_auth_navigations.is_empty()
            && self.pending_fetch_response_transfers.is_empty()
            && self.pending_subresource_fetches.is_empty()
            && self.in_flight_subresource_fetches.is_empty()
            && self.pending_subresource_fetch_auths.is_empty()
            && self.pending_subresource_fetch_responses.is_empty()
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        json!({
            "pendingFetchRequestIdCount": self.pending_fetch_request_ids.len(),
            "pendingFetchNavigationCount": self.pending_fetch_navigations.len(),
            "pendingFetchAuthNavigationCount": self.pending_fetch_auth_navigations.len(),
            "pendingFetchResponseTransferPresent": !self.pending_fetch_response_transfers.is_empty(),
            "pendingSubresourceFetchCount": self.pending_subresource_fetches.len(),
            "inFlightSubresourceFetchCount": self.in_flight_subresource_fetches.len(),
            "pendingSubresourceFetchAuthCount": self.pending_subresource_fetch_auths.len(),
            "pendingSubresourceFetchResponseCount": self.pending_subresource_fetch_responses.len(),
            "empty": self.is_empty(),
        })
    }

    #[cfg(test)]
    pub(crate) fn insert_pending_fetch_request_id_for_test(&mut self, request_id: String) {
        self.pending_fetch_request_ids.insert(request_id);
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_request_id_for_test(&self, request_id: &str) -> bool {
        self.pending_fetch_request_ids.contains(request_id)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_navigation_for_test(&self, request_id: &str) -> bool {
        self.pending_fetch_navigations.contains_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_auth_navigation_for_test(&self, request_id: &str) -> bool {
        self.pending_fetch_auth_navigations.contains_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_subresource_fetch_for_test(&self, request_id: &str) -> bool {
        self.pending_subresource_fetches.contains_key(request_id)
    }
}

pub type ParkedFetchState = TargetFetchState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFetchConfig {
    enabled: bool,
    session_id: Option<String>,
    handle_auth_requests: bool,
    request_stage: FetchRequestStage,
    url_pattern: String,
    resource_type_filter: Option<FetchResourceTypeFilter>,
    patterns: Vec<FetchInterceptionPattern>,
    fetch_sessions: BTreeMap<Option<String>, TargetFetchSessionConfig>,
    fetch_session_order: Vec<Option<String>>,
    network_intercepts: BTreeMap<String, TargetNetworkInterceptConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetFetchSessionConfig {
    handle_auth_requests: bool,
    patterns: Vec<FetchInterceptionPattern>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetNetworkInterceptConfig {
    session_id: Option<String>,
    handle_auth_requests: bool,
    auth_url_patterns: Vec<String>,
    patterns: Vec<FetchInterceptionPattern>,
}

impl Default for TargetFetchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            session_id: None,
            handle_auth_requests: false,
            request_stage: FetchRequestStage::Request,
            url_pattern: "*".to_owned(),
            resource_type_filter: None,
            patterns: Vec::new(),
            fetch_sessions: BTreeMap::new(),
            fetch_session_order: Vec::new(),
            network_intercepts: BTreeMap::new(),
        }
    }
}

impl TargetFetchConfig {
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn handle_auth_requests(&self) -> bool {
        self.handle_auth_requests
    }

    #[cfg(test)]
    pub(crate) fn request_stage(&self) -> FetchRequestStage {
        self.request_stage
    }

    #[cfg(test)]
    pub(crate) fn url_pattern(&self) -> &str {
        &self.url_pattern
    }

    #[cfg(test)]
    pub(crate) fn resource_type_filter(&self) -> Option<FetchResourceTypeFilter> {
        self.resource_type_filter
    }

    pub(crate) fn patterns(&self) -> &[FetchInterceptionPattern] {
        &self.patterns
    }

    pub(crate) fn subresource_interception_config(
        &self,
    ) -> (bool, Option<moli_core::page::SubresourceResourceType>) {
        fetch_subresource_interception_config_for_patterns(
            self.enabled || self.handle_auth_requests,
            &self.patterns,
        )
    }

    pub(crate) fn subresource_interception_snapshot(
        &self,
    ) -> TargetFetchSubresourceInterceptionSnapshot {
        TargetFetchSubresourceInterceptionSnapshot {
            enabled: self.enabled,
            event_session_id: self.session_id.clone(),
            handle_auth_requests: self.handle_auth_requests,
            fetch_sessions: self.fetch_sessions.clone(),
            fetch_session_order: self.fetch_session_order.clone(),
            patterns: self.patterns().to_vec(),
            network_intercepts: self.network_intercepts.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn config_snapshot_for_session(&self, session_id: Option<&str>) -> Self {
        let mut snapshot = Self::default();
        let key = session_id.map(str::to_owned);
        if let Some(config) = self.fetch_sessions.get(&key) {
            snapshot.fetch_sessions.insert(key, config.clone());
            snapshot
                .fetch_session_order
                .push(session_id.map(str::to_owned));
        }
        snapshot.network_intercepts = self
            .network_intercepts
            .iter()
            .filter(|(_, intercept)| intercept.session_id.as_deref() == session_id)
            .map(|(intercept_id, intercept)| (intercept_id.clone(), intercept.clone()))
            .collect();
        snapshot.rebuild_effective_state();
        snapshot
    }

    #[cfg(test)]
    pub(crate) fn matching_request_stage(
        &self,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Option<FetchRequestStage> {
        if !self.enabled {
            return None;
        }
        matching_fetch_pattern(&self.patterns, resource_type, url)
            .map(|pattern| pattern.request_stage)
    }

    #[cfg(test)]
    pub(crate) fn matching_document_request_stage(&self, url: &Url) -> Option<FetchRequestStage> {
        self.matching_request_stage(DevToolsNetworkResourceType::Document, url)
    }

    pub(crate) fn matches_auth_required(&self, url: &Url) -> bool {
        matches_auth_required(self.handle_auth_requests, &self.network_intercepts, url)
    }

    pub(crate) fn matching_auth_required_network_intercepts(
        &self,
        url: &Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        matching_auth_required_network_intercepts(&self.network_intercepts, url)
    }

    pub(crate) fn matching_network_intercepts(
        &self,
        request_stage: FetchRequestStage,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        if !self.enabled {
            return Vec::new();
        }
        matching_network_intercepts(&self.network_intercepts, request_stage, resource_type, url)
    }

    pub(crate) fn has_document_response_stage_candidate(&self) -> bool {
        self.enabled
            && self.patterns.iter().any(|pattern| {
                pattern.request_stage == FetchRequestStage::Response
                    && pattern.resource_type_filter.is_none_or(|filter| {
                        filter.matches_resource_type(DevToolsNetworkResourceType::Document)
                    })
            })
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn configure(
        &mut self,
        session_id: Option<String>,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) {
        if !self.fetch_sessions.contains_key(&session_id) {
            self.fetch_session_order.push(session_id.clone());
        }
        self.fetch_sessions.insert(
            session_id,
            TargetFetchSessionConfig {
                handle_auth_requests,
                patterns,
            },
        );
        self.rebuild_effective_state();
    }

    pub(crate) fn remove_fetch_session(&mut self, session_id: Option<&str>) -> bool {
        let key = session_id.map(str::to_owned);
        let removed = self.fetch_sessions.remove(&key).is_some();
        if removed {
            self.fetch_session_order
                .retain(|ordered_session_id| ordered_session_id != &key);
            self.rebuild_effective_state();
        }
        removed
    }

    pub(crate) fn add_network_intercept(
        &mut self,
        intercept_id: String,
        session_id: Option<String>,
        handle_auth_requests: bool,
        auth_url_patterns: Vec<String>,
        patterns: Vec<FetchInterceptionPattern>,
    ) {
        self.network_intercepts.insert(
            intercept_id,
            TargetNetworkInterceptConfig {
                session_id,
                handle_auth_requests,
                auth_url_patterns,
                patterns,
            },
        );
        self.rebuild_effective_state();
    }

    pub(crate) fn remove_network_intercept(&mut self, intercept_id: &str) -> bool {
        let removed = self.network_intercepts.remove(intercept_id).is_some();
        if removed {
            self.rebuild_effective_state();
        }
        removed
    }

    fn rebuild_effective_state(&mut self) {
        self.session_id = self
            .fetch_session_order
            .iter()
            .filter(|session_id| self.fetch_sessions.contains_key(*session_id))
            .find_map(|session_id| session_id.clone())
            .or_else(|| {
                self.network_intercepts
                    .values()
                    .find_map(|intercept| intercept.session_id.clone())
            });
        self.handle_auth_requests = self
            .fetch_sessions
            .values()
            .any(|config| config.handle_auth_requests)
            || self
                .network_intercepts
                .values()
                .any(|intercept| intercept.handle_auth_requests);
        let mut patterns = Vec::new();
        for session_id in &self.fetch_session_order {
            if let Some(config) = self.fetch_sessions.get(session_id) {
                patterns.extend(config.patterns.iter().cloned());
            }
        }
        for intercept in self.network_intercepts.values() {
            patterns.extend(intercept.patterns.iter().cloned());
        }
        let first_pattern = patterns
            .first()
            .cloned()
            .unwrap_or(FetchInterceptionPattern {
                url_pattern: "*".to_owned(),
                resource_type_filter: None,
                request_stage: FetchRequestStage::Request,
            });
        self.enabled = !self.fetch_sessions.is_empty()
            || self
                .network_intercepts
                .values()
                .any(|intercept| !intercept.patterns.is_empty());
        self.request_stage = first_pattern.request_stage;
        self.url_pattern = first_pattern.url_pattern;
        self.resource_type_filter = first_pattern.resource_type_filter;
        self.patterns = patterns;
    }
}

#[derive(Debug, Default)]
pub struct TargetFetchOwner {
    config: TargetFetchConfig,
    pending: TargetFetchState,
}

#[derive(Debug, Clone)]
pub(crate) struct TargetFetchSubresourceInterceptionSnapshot {
    enabled: bool,
    event_session_id: Option<String>,
    handle_auth_requests: bool,
    fetch_sessions: BTreeMap<Option<String>, TargetFetchSessionConfig>,
    fetch_session_order: Vec<Option<String>>,
    patterns: Vec<FetchInterceptionPattern>,
    network_intercepts: BTreeMap<String, TargetNetworkInterceptConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetFetchRequestStageSession {
    pub(crate) session_id: Option<String>,
    pub(crate) owner_kind: PendingSubresourceFetchOwnerKind,
    pub(crate) request_stage: FetchRequestStage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetFetchResponseStageSession {
    pub(crate) session_id: Option<String>,
    pub(crate) owner_kind: PendingSubresourceFetchOwnerKind,
    pub(crate) blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetFetchAuthRequiredSession {
    pub(crate) session_id: Option<String>,
    pub(crate) owner_kind: PendingSubresourceFetchOwnerKind,
    pub(crate) blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

impl TargetFetchSubresourceInterceptionSnapshot {
    fn ordered_fetch_sessions(
        &self,
    ) -> impl Iterator<Item = (&Option<String>, &TargetFetchSessionConfig)> {
        self.fetch_session_order
            .iter()
            .filter_map(|session_id| self.fetch_sessions.get_key_value(session_id))
    }

    pub(crate) fn event_session_id<'a>(
        &'a self,
        default_session_id: Option<&'a str>,
    ) -> Option<&'a str> {
        self.event_session_id.as_deref().or(default_session_id)
    }

    pub(crate) fn matching_request_stage(
        &self,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Option<FetchRequestStage> {
        if !self.enabled {
            return None;
        }
        matching_fetch_pattern(&self.patterns, resource_type, url)
            .map(|pattern| pattern.request_stage)
    }

    pub(crate) fn matching_fetch_request_stage_sessions(
        &self,
        default_session_id: Option<&str>,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Vec<TargetFetchRequestStageSession> {
        if !self.enabled {
            return Vec::new();
        }
        self.ordered_fetch_sessions()
            .filter_map(|(session_id, config)| {
                let request_stage = if config.patterns.iter().any(|pattern| {
                    pattern.request_stage == FetchRequestStage::Request
                        && pattern.matches_request(resource_type, url)
                }) {
                    FetchRequestStage::Request
                } else if config.patterns.iter().any(|pattern| {
                    pattern.request_stage == FetchRequestStage::Response
                        && pattern.matches_request(resource_type, url)
                }) {
                    FetchRequestStage::Response
                } else {
                    return None;
                };
                Some(TargetFetchRequestStageSession {
                    session_id: session_id
                        .clone()
                        .or_else(|| default_session_id.map(str::to_owned)),
                    owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                    request_stage,
                })
            })
            .collect()
    }

    pub(crate) fn matching_request_stage_pause_sessions(
        &self,
        default_session_id: Option<&str>,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Vec<TargetFetchRequestStageSession> {
        let mut sessions = self
            .matching_fetch_request_stage_sessions(default_session_id, resource_type, url)
            .into_iter()
            .filter(|session| session.request_stage == FetchRequestStage::Request)
            .collect::<Vec<_>>();
        if let Some(network_session) =
            self.matching_network_request_stage_session(default_session_id, resource_type, url)
        {
            sessions.push(network_session);
        }
        sessions
    }

    fn matching_network_request_stage_session(
        &self,
        default_session_id: Option<&str>,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Option<TargetFetchRequestStageSession> {
        if !self.enabled {
            return None;
        }
        self.network_intercepts
            .values()
            .find(|intercept| {
                intercept.patterns.iter().any(|pattern| {
                    pattern.request_stage == FetchRequestStage::Request
                        && pattern.matches_request(resource_type, url)
                })
            })
            .map(|intercept| TargetFetchRequestStageSession {
                session_id: intercept
                    .session_id
                    .clone()
                    .or_else(|| default_session_id.map(str::to_owned)),
                owner_kind: PendingSubresourceFetchOwnerKind::NetworkOrBidi,
                request_stage: FetchRequestStage::Request,
            })
    }

    pub(crate) fn matching_fetch_response_stage_sessions(
        &self,
        default_session_id: Option<&str>,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Vec<TargetFetchResponseStageSession> {
        if !self.enabled {
            return Vec::new();
        }
        self.ordered_fetch_sessions()
            .filter(|(_, config)| {
                config.patterns.iter().any(|pattern| {
                    pattern.request_stage == FetchRequestStage::Response
                        && pattern.matches_request(resource_type, url)
                })
            })
            .map(|(session_id, _)| TargetFetchResponseStageSession {
                session_id: session_id
                    .clone()
                    .or_else(|| default_session_id.map(str::to_owned)),
                owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                blocked_intercepts: Vec::new(),
            })
            .collect()
    }

    pub(crate) fn matching_response_stage_pause_sessions(
        &self,
        default_session_id: Option<&str>,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Vec<TargetFetchResponseStageSession> {
        let mut sessions =
            self.matching_fetch_response_stage_sessions(default_session_id, resource_type, url);
        if let Some(network_session) =
            self.matching_network_response_stage_session(default_session_id, resource_type, url)
        {
            sessions.push(network_session);
        }
        sessions
    }

    fn matching_network_response_stage_session(
        &self,
        default_session_id: Option<&str>,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Option<TargetFetchResponseStageSession> {
        if !self.enabled {
            return None;
        }
        let blocked_intercepts = matching_network_intercepts(
            &self.network_intercepts,
            FetchRequestStage::Response,
            resource_type,
            url,
        );
        if blocked_intercepts.is_empty() {
            return None;
        }
        let intercept = self.network_intercepts.values().find(|intercept| {
            intercept.patterns.iter().any(|pattern| {
                pattern.request_stage == FetchRequestStage::Response
                    && pattern.matches_request(resource_type, url)
            })
        })?;
        Some(TargetFetchResponseStageSession {
            session_id: intercept
                .session_id
                .clone()
                .or_else(|| default_session_id.map(str::to_owned)),
            owner_kind: PendingSubresourceFetchOwnerKind::NetworkOrBidi,
            blocked_intercepts,
        })
    }

    pub(crate) fn matching_fetch_auth_required_sessions(
        &self,
        default_session_id: Option<&str>,
    ) -> Vec<TargetFetchAuthRequiredSession> {
        self.ordered_fetch_sessions()
            .filter(|(_, config)| config.handle_auth_requests)
            .map(|(session_id, _)| TargetFetchAuthRequiredSession {
                session_id: session_id
                    .clone()
                    .or_else(|| default_session_id.map(str::to_owned)),
                owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                blocked_intercepts: Vec::new(),
            })
            .collect()
    }

    pub(crate) fn matching_auth_required_pause_sessions(
        &self,
        default_session_id: Option<&str>,
        url: &Url,
    ) -> Vec<TargetFetchAuthRequiredSession> {
        let mut sessions = self.matching_fetch_auth_required_sessions(default_session_id);
        if let Some(network_session) =
            self.matching_network_auth_required_session(default_session_id, url)
        {
            sessions.push(network_session);
        }
        sessions
    }

    fn matching_network_auth_required_session(
        &self,
        default_session_id: Option<&str>,
        url: &Url,
    ) -> Option<TargetFetchAuthRequiredSession> {
        if !self.enabled {
            return None;
        }
        let blocked_intercepts =
            matching_auth_required_network_intercepts(&self.network_intercepts, url);
        if blocked_intercepts.is_empty() {
            return None;
        }
        let intercept = self
            .network_intercepts
            .values()
            .find(|intercept| network_intercept_matches_auth_required(intercept, url))?;
        Some(TargetFetchAuthRequiredSession {
            session_id: intercept
                .session_id
                .clone()
                .or_else(|| default_session_id.map(str::to_owned)),
            owner_kind: PendingSubresourceFetchOwnerKind::NetworkOrBidi,
            blocked_intercepts,
        })
    }

    pub(crate) fn response_stage_owner_kind(
        &self,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Option<PendingSubresourceFetchOwnerKind> {
        if !self.enabled {
            return None;
        }
        if self.fetch_sessions.values().any(|config| {
            matching_fetch_pattern(&config.patterns, resource_type, url)
                .is_some_and(|pattern| pattern.request_stage == FetchRequestStage::Response)
        }) {
            return Some(PendingSubresourceFetchOwnerKind::Fetch);
        }
        (!matching_network_intercepts(
            &self.network_intercepts,
            FetchRequestStage::Response,
            resource_type,
            url,
        )
        .is_empty())
        .then_some(PendingSubresourceFetchOwnerKind::NetworkOrBidi)
    }

    pub(crate) fn response_stage_candidate_owner_kind(
        &self,
        resource_type: DevToolsNetworkResourceType,
    ) -> Option<PendingSubresourceFetchOwnerKind> {
        if !self.enabled {
            return None;
        }
        if self.fetch_sessions.values().any(|config| {
            config.patterns.iter().any(|pattern| {
                pattern.request_stage == FetchRequestStage::Response
                    && pattern
                        .resource_type_filter
                        .is_none_or(|filter| filter.matches_resource_type(resource_type))
            })
        }) {
            return Some(PendingSubresourceFetchOwnerKind::Fetch);
        }
        self.network_intercepts
            .values()
            .any(|intercept| {
                intercept.patterns.iter().any(|pattern| {
                    pattern.request_stage == FetchRequestStage::Response
                        && pattern
                            .resource_type_filter
                            .is_none_or(|filter| filter.matches_resource_type(resource_type))
                })
            })
            .then_some(PendingSubresourceFetchOwnerKind::NetworkOrBidi)
    }

    pub(crate) fn auth_required_owner_kind(
        &self,
        url: &Url,
    ) -> Option<PendingSubresourceFetchOwnerKind> {
        if self
            .fetch_sessions
            .values()
            .any(|config| config.handle_auth_requests)
        {
            return Some(PendingSubresourceFetchOwnerKind::Fetch);
        }
        (!matching_auth_required_network_intercepts(&self.network_intercepts, url).is_empty())
            .then_some(PendingSubresourceFetchOwnerKind::NetworkOrBidi)
    }

    pub(crate) fn matching_network_intercepts(
        &self,
        request_stage: FetchRequestStage,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        if !self.enabled {
            return Vec::new();
        }
        matching_network_intercepts(&self.network_intercepts, request_stage, resource_type, url)
    }

    pub(crate) fn matches_auth_required(&self, url: &Url) -> bool {
        matches_auth_required(self.handle_auth_requests, &self.network_intercepts, url)
    }

    pub(crate) fn matching_auth_required_network_intercepts(
        &self,
        url: &Url,
    ) -> Vec<DevToolsNetworkInterceptId> {
        matching_auth_required_network_intercepts(&self.network_intercepts, url)
    }

    pub(crate) fn has_response_stage_candidate(
        &self,
        resource_type: DevToolsNetworkResourceType,
    ) -> bool {
        self.enabled
            && self.patterns.iter().any(|pattern| {
                pattern.request_stage == FetchRequestStage::Response
                    && pattern
                        .resource_type_filter
                        .is_none_or(|filter| filter.matches_resource_type(resource_type))
            })
    }

    pub(crate) fn matches_response_stage(
        &self,
        resource_type: DevToolsNetworkResourceType,
        url: &Url,
    ) -> bool {
        self.enabled
            && self.patterns.iter().any(|pattern| {
                pattern.request_stage == FetchRequestStage::Response
                    && pattern.matches_request(resource_type, url)
            })
    }
}

fn matching_network_intercepts(
    intercepts: &BTreeMap<String, TargetNetworkInterceptConfig>,
    request_stage: FetchRequestStage,
    resource_type: DevToolsNetworkResourceType,
    url: &Url,
) -> Vec<DevToolsNetworkInterceptId> {
    intercepts
        .iter()
        .filter(|(_, intercept)| {
            intercept.patterns.iter().any(|pattern| {
                pattern.request_stage == request_stage
                    && pattern.matches_request(resource_type, url)
            })
        })
        .map(|(intercept_id, _)| DevToolsNetworkInterceptId::from(intercept_id.as_str()))
        .collect()
}

fn matches_auth_required(
    handle_auth_requests: bool,
    intercepts: &BTreeMap<String, TargetNetworkInterceptConfig>,
    url: &Url,
) -> bool {
    if !handle_auth_requests {
        return false;
    }
    if intercepts.is_empty() {
        return true;
    }
    intercepts
        .values()
        .any(|intercept| network_intercept_matches_auth_required(intercept, url))
}

fn matching_auth_required_network_intercepts(
    intercepts: &BTreeMap<String, TargetNetworkInterceptConfig>,
    url: &Url,
) -> Vec<DevToolsNetworkInterceptId> {
    intercepts
        .iter()
        .filter(|(_, intercept)| network_intercept_matches_auth_required(intercept, url))
        .map(|(intercept_id, _)| DevToolsNetworkInterceptId::from(intercept_id.as_str()))
        .collect()
}

fn network_intercept_matches_auth_required(
    intercept: &TargetNetworkInterceptConfig,
    url: &Url,
) -> bool {
    intercept.handle_auth_requests
        && intercept
            .auth_url_patterns
            .iter()
            .any(|pattern| url_pattern_matches(pattern, url.as_str()))
}

impl TargetFetchOwner {
    pub(crate) fn configure(
        &mut self,
        session_id: Option<String>,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) {
        self.config
            .configure(session_id, handle_auth_requests, patterns);
    }

    pub(crate) fn add_network_intercept(
        &mut self,
        intercept_id: String,
        session_id: Option<String>,
        handle_auth_requests: bool,
        auth_url_patterns: Vec<String>,
        patterns: Vec<FetchInterceptionPattern>,
    ) {
        self.config.add_network_intercept(
            intercept_id,
            session_id,
            handle_auth_requests,
            auth_url_patterns,
            patterns,
        );
    }

    pub(crate) fn remove_network_intercept(&mut self, intercept_id: &str) -> bool {
        self.config.remove_network_intercept(intercept_id)
    }

    pub(crate) fn reset_config(&mut self) {
        self.config.reset();
    }

    pub(crate) fn remove_fetch_session(&mut self, session_id: Option<&str>) -> bool {
        self.config.remove_fetch_session(session_id)
    }

    pub(crate) fn replace_config(&mut self, config: TargetFetchConfig) {
        self.config = config;
    }

    pub(crate) fn config_snapshot(&self) -> TargetFetchConfig {
        self.config.clone()
    }

    #[cfg(test)]
    pub(crate) fn config_snapshot_for_session(
        &self,
        session_id: Option<&str>,
    ) -> TargetFetchConfig {
        self.config.config_snapshot_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn is_enabled(&self) -> bool {
        self.config.is_enabled()
    }

    pub(crate) fn contains_pending_request(&self, request_id: &str) -> bool {
        self.pending.contains_pending_request(request_id)
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        json!({
            "enabled": self.config.enabled,
            "patternCount": self.config.patterns.len(),
            "hasSession": self.config.session_id.is_some(),
            "handleAuthRequests": self.config.handle_auth_requests,
            "pending": self.pending.moli_memory_diagnostics(),
        })
    }

    #[cfg(test)]
    pub(crate) fn event_session_id<'a>(
        &'a self,
        default_session_id: Option<&'a str>,
    ) -> Option<&'a str> {
        self.config.session_id().or(default_session_id)
    }

    #[cfg(test)]
    pub(crate) fn handle_auth_requests(&self) -> bool {
        self.config.handle_auth_requests()
    }

    #[cfg(test)]
    pub(crate) fn subresource_interception_snapshot(
        &self,
    ) -> TargetFetchSubresourceInterceptionSnapshot {
        self.config.subresource_interception_snapshot()
    }

    pub(crate) fn subresource_interception_config(
        &self,
    ) -> (bool, Option<moli_core::page::SubresourceResourceType>) {
        self.config.subresource_interception_config()
    }

    #[cfg(test)]
    pub(crate) fn matching_document_request_stage(&self, url: &Url) -> Option<FetchRequestStage> {
        self.config.matching_document_request_stage(url)
    }

    pub(crate) fn register_pending_fetch_navigation_request(
        &mut self,
        pending: PendingFetchNavigation,
    ) {
        self.pending
            .register_pending_fetch_navigation_request(pending);
    }

    pub(crate) fn drop_active_fetch_response_body_streams(&mut self) {
        for request_id in self
            .pending
            .pending_fetch_response_transfers
            .drop_active_body_streams()
        {
            self.pending.pending_fetch_request_ids.remove(&request_id);
        }
    }

    pub(crate) fn consume_pending_request_action(
        &mut self,
        request_id: &str,
    ) -> Result<(), &'static str> {
        self.pending.consume_pending_request_action(request_id)
    }

    pub(crate) fn take_pending_fetch_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchNavigation> {
        self.pending
            .take_pending_fetch_navigation_for_action_session(request_id, action_session_id)
    }

    pub(crate) fn take_pending_fetch_auth_navigation_for_action_session(
        &mut self,
        request_id: &str,
        action_session_id: Option<&str>,
    ) -> Option<PendingFetchAuthNavigation> {
        self.pending
            .take_pending_fetch_auth_navigation_for_action_session(request_id, action_session_id)
    }

    pub(crate) fn register_pending_fetch_auth_navigation(
        &mut self,
        request_id: String,
        pending: PendingFetchAuthNavigation,
    ) {
        self.pending
            .register_pending_fetch_auth_navigation(request_id, pending);
    }

    pub(crate) fn register_pending_fetch_response_navigation(
        &mut self,
        request_id: String,
        document_navigation_token: Option<super::DocumentNavigationToken>,
        navigation: NavigationDispatchState,
        body: DocumentBodySource,
    ) {
        self.pending.register_pending_fetch_response_navigation(
            request_id,
            document_navigation_token,
            navigation,
            body,
        );
    }

    pub(crate) fn take_pending_fetch_response_transfer_for_terminal_action(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.pending
            .take_pending_fetch_response_transfer_for_terminal_action(request_id)
    }

    pub(crate) fn take_pending_fetch_response_transfer(
        &mut self,
        request_id: &str,
    ) -> Option<PausedDocumentTransfer> {
        self.pending
            .take_pending_fetch_response_transfer(request_id)
    }

    pub(crate) fn register_pending_fetch_response_transfer(
        &mut self,
        request_id: String,
        transfer: PausedDocumentTransfer,
    ) {
        self.pending
            .register_pending_fetch_response_transfer(request_id, transfer);
    }

    pub(crate) fn pending_subresource_fetch_response_request(
        &self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<&PendingSubresourceFetchResponseRequest> {
        self.pending
            .pending_subresource_fetch_response_request(request_id, session_id)
    }

    pub(crate) fn mark_pending_subresource_fetch_response_body_taken_as_stream(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> bool {
        self.pending
            .mark_pending_subresource_fetch_response_body_taken_as_stream(request_id, session_id)
    }

    pub(crate) fn take_pending_subresource_fetch_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchRequest> {
        self.pending
            .take_pending_subresource_fetch_request(request_id, session_id)
    }

    pub(crate) fn take_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchAuthRequest> {
        self.pending
            .take_pending_subresource_fetch_auth_request(request_id, session_id)
    }

    pub(crate) fn take_pending_subresource_fetch_response_request(
        &mut self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<PendingSubresourceFetchResponseRequest> {
        self.pending
            .take_pending_subresource_fetch_response_request(request_id, session_id)
    }

    pub(crate) fn take_in_flight_subresource_fetch_request(
        &mut self,
        internal_id: u64,
    ) -> Option<InFlightSubresourceFetchRequest> {
        self.pending
            .take_in_flight_subresource_fetch_request(internal_id)
    }

    pub(crate) fn claim_subresource_continue_request(
        &mut self,
        expected_page_owner: &super::TargetPageResidenceIdentity,
        internal_id: u64,
        session_id: Option<&str>,
        allow_pending_completion: bool,
    ) -> Option<ClaimedSubresourceContinueRequest> {
        self.pending.claim_subresource_continue_request(
            expected_page_owner,
            internal_id,
            session_id,
            allow_pending_completion,
        )
    }

    pub(crate) fn in_flight_subresource_fetch_request_id(&self, internal_id: u64) -> Option<&str> {
        self.pending
            .in_flight_subresource_fetch_request_id(internal_id)
    }

    pub(crate) fn in_flight_subresource_fetch_request_page_owner(
        &self,
        internal_id: u64,
    ) -> Option<&super::TargetPageResidenceIdentity> {
        self.pending
            .in_flight_subresource_fetch_request_page_owner(internal_id)
    }

    pub(crate) fn register_pending_subresource_fetch_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    ) {
        self.pending
            .register_pending_subresource_fetch_request(request_id, pending);
    }

    pub(crate) fn register_in_flight_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
    ) {
        self.pending
            .register_in_flight_subresource_fetch_request(request_id, pending);
    }

    pub(crate) fn register_in_flight_response_stage_subresource_fetch_request(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    ) {
        self.pending
            .register_in_flight_response_stage_subresource_fetch_request(
                request_id,
                pending,
                response_stage_blocked_intercepts,
            );
    }

    pub(crate) fn register_in_flight_subresource_fetch_request_with_response_match_policy(
        &mut self,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
    ) {
        self.pending
            .register_in_flight_subresource_fetch_request_with_response_match_policy(
                request_id,
                pending,
                response_stage_url_match_policy,
            );
    }

    pub(crate) fn register_pending_subresource_fetch_auth_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchAuthRequest,
    ) {
        self.pending
            .register_pending_subresource_fetch_auth_request(request_id, pending);
    }

    pub(crate) fn register_pending_subresource_fetch_response_request(
        &mut self,
        request_id: String,
        pending: PendingSubresourceFetchResponseRequest,
    ) {
        self.pending
            .register_pending_subresource_fetch_response_request(request_id, pending);
    }

    pub(crate) fn open_pending_fetch_response_body_stream(
        &mut self,
        runtime_slot: &mut TargetRuntimeSlot,
        request_id: &str,
        handle: String,
    ) -> Result<Option<String>, String> {
        self.pending
            .open_pending_fetch_response_body_stream(runtime_slot, request_id, handle)
    }

    pub(crate) fn start_pending_fetch_response_body_stream_read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        self.pending
            .start_pending_fetch_response_body_stream_read(handle, offset, size)
    }

    pub(crate) fn finish_pending_fetch_response_body_stream_read(
        &mut self,
        runtime_slot: &mut TargetRuntimeSlot,
        completed: super::super::fetch_support::CompletedFetchResponseBodyStreamReadDispatch,
    ) -> PendingFetchResponseBodyStreamRead {
        self.pending
            .finish_pending_fetch_response_body_stream_read(runtime_slot, completed)
    }

    pub(crate) fn close_pending_fetch_response_body_stream(&mut self, handle: &str) -> bool {
        self.pending
            .close_pending_fetch_response_body_stream(handle)
    }

    #[cfg(test)]
    pub(crate) fn has_in_flight_subresource_fetches_for_test(&self) -> bool {
        !self.pending.in_flight_subresource_fetches.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn has_pending_subresource_fetch_for_test(&self, request_id: &str) -> bool {
        self.pending
            .pending_subresource_fetches
            .contains_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_navigation_for_test(&self, request_id: &str) -> bool {
        self.pending
            .pending_fetch_navigations
            .contains_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_request_id_for_test(&self, request_id: &str) -> bool {
        self.pending.pending_fetch_request_ids.contains(request_id)
    }

    #[cfg(test)]
    pub(crate) fn register_pending_fetch_request_id_for_test(&mut self, request_id: String) {
        self.pending.pending_fetch_request_ids.insert(request_id);
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_auth_navigation_for_test(&self, request_id: &str) -> bool {
        self.pending
            .pending_fetch_auth_navigations
            .contains_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch_state_for_test(&self) -> bool {
        !self.pending.pending_fetch_request_ids.is_empty()
            || !self.pending.pending_fetch_navigations.is_empty()
            || !self.pending.pending_fetch_auth_navigations.is_empty()
            || !self.pending.pending_subresource_fetches.is_empty()
            || !self.pending.pending_subresource_fetch_auths.is_empty()
            || !self.pending.pending_subresource_fetch_responses.is_empty()
            || !self.pending.pending_fetch_response_transfers.is_empty()
            || !self.pending.in_flight_subresource_fetches.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn pending_fetch_response_transfer_is_pending_for_test(
        &self,
        request_id: &str,
    ) -> bool {
        self.pending
            .pending_fetch_response_transfers
            .get(request_id)
            .is_some_and(PausedDocumentTransfer::is_pending)
    }

    #[cfg(test)]
    pub(crate) fn pending_fetch_response_prepared_renderer_agent_for_test(
        &self,
        request_id: &str,
    ) -> Option<moli_core::page::RendererDevToolsAgentToken> {
        self.pending
            .pending_fetch_response_transfers
            .get(request_id)
            .and_then(PausedDocumentTransfer::prepared_renderer_agent_token)
    }

    #[cfg(test)]
    pub(crate) fn active_fetch_response_body_stream_request_id_for_test(
        &self,
        handle: &str,
    ) -> Option<&str> {
        self.pending
            .pending_fetch_response_transfers
            .active_body_stream_request_id(handle)
    }

    pub(crate) fn drain_pending_requests(
        &mut self,
    ) -> (
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    ) {
        self.pending.drain_pending_requests()
    }

    pub(crate) fn drain_pending_requests_for_disable_session(
        &mut self,
        session_id: Option<&str>,
    ) -> (
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    ) {
        self.pending
            .drain_pending_requests_for_disable_session(session_id)
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
    }

    pub(crate) fn take_pending_state(&mut self) -> TargetFetchState {
        std::mem::take(&mut self.pending)
    }

    pub(crate) fn replace_pending_state(&mut self, state: TargetFetchState) {
        self.pending = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        CapturedBody, FetchAuthChallenge, NavigationResultProjection,
        PendingSubresourceFetchAuthStage, PendingSubresourceFetchAuthStageChain,
        PendingSubresourceFetchRequestStage, PendingSubresourceFetchRequestStageChain,
    };
    use moli_core::page::SubresourceResourceType;

    fn test_url(path: &str) -> Url {
        Url::parse(&format!("https://example.test/{path}")).unwrap()
    }

    fn test_page_owner() -> crate::conn::TargetPageResidenceIdentity {
        crate::conn::TargetPageResidenceIdentity::new_for_test(
            "BID-fetch-state".to_owned(),
            Some("TID-fetch-state".to_owned()),
            1,
        )
    }

    fn pending_subresource_fetch(
        internal_id: u64,
        owner_session_id: Option<&str>,
    ) -> PendingSubresourceFetchRequest {
        pending_subresource_fetch_with_owner_kind(
            internal_id,
            owner_session_id,
            PendingSubresourceFetchOwnerKind::Fetch,
        )
    }

    fn pending_subresource_fetch_with_owner_kind(
        internal_id: u64,
        owner_session_id: Option<&str>,
        owner_kind: PendingSubresourceFetchOwnerKind,
    ) -> PendingSubresourceFetchRequest {
        PendingSubresourceFetchRequest {
            residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(
                test_page_owner(),
            ),
            owner_session_id: owner_session_id.map(str::to_owned),
            action_session_id: owner_session_id.map(str::to_owned),
            owner_kind,
            internal_id,
            network_request_id: format!("NETWORK-{internal_id}"),
            network_request_handle: None,
            frame_id: "FRAME-1".to_owned(),
            document_url: test_url("page"),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            request_stage_chain: None,
        }
    }

    fn pending_subresource_auth(
        internal_id: u64,
        owner_session_id: Option<&str>,
    ) -> PendingSubresourceFetchAuthRequest {
        pending_subresource_auth_with_owner_kind(
            internal_id,
            owner_session_id,
            PendingSubresourceFetchOwnerKind::Fetch,
        )
    }

    fn pending_subresource_auth_with_owner_kind(
        internal_id: u64,
        owner_session_id: Option<&str>,
        owner_kind: PendingSubresourceFetchOwnerKind,
    ) -> PendingSubresourceFetchAuthRequest {
        PendingSubresourceFetchAuthRequest {
            page_owner: test_page_owner(),
            owner_session_id: owner_session_id.map(str::to_owned),
            action_session_id: owner_session_id.map(str::to_owned),
            owner_kind,
            internal_id,
            network_request_id: format!("NETWORK-AUTH-{internal_id}"),
            network_request_handle: None,
            frame_id: "FRAME-1".to_owned(),
            document_url: test_url("page"),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            url: test_url("auth"),
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_cookie_report: None,
            challenge: FetchAuthChallenge {
                origin: "https://example.test".to_owned(),
                source: "Server".to_owned(),
                scheme: "basic".to_owned(),
                realm: "test".to_owned(),
            },
            intercept_response: false,
            auth_stage_chain: None,
        }
    }

    fn pending_subresource_response(
        internal_id: u64,
        owner_session_id: Option<&str>,
    ) -> PendingSubresourceFetchResponseRequest {
        pending_subresource_response_with_owner_kind(
            internal_id,
            owner_session_id,
            PendingSubresourceFetchOwnerKind::Fetch,
        )
    }

    fn pending_subresource_response_with_owner_kind(
        internal_id: u64,
        owner_session_id: Option<&str>,
        owner_kind: PendingSubresourceFetchOwnerKind,
    ) -> PendingSubresourceFetchResponseRequest {
        PendingSubresourceFetchResponseRequest {
            page_owner: test_page_owner(),
            owner_session_id: owner_session_id.map(str::to_owned),
            action_session_id: owner_session_id.map(str::to_owned),
            owner_kind,
            internal_id,
            network_request_id: format!("NETWORK-RESP-{internal_id}"),
            network_request_handle: None,
            frame_id: "FRAME-1".to_owned(),
            document_url: test_url("page"),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            url: test_url("response"),
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_cookie_report: None,
            response_status: 200,
            response_headers: Vec::new(),
            response_head_overridden: false,
            response_body_taken_as_stream: false,
            response_body: CapturedBody::from_bytes(Vec::new()),
            response_stage_chain: None,
        }
    }

    fn pending_fetch_auth_navigation(
        request_id: &str,
        owner_session_id: Option<&str>,
    ) -> PendingFetchAuthNavigation {
        PendingFetchAuthNavigation {
            owner_session_id: owner_session_id.map(str::to_owned),
            action_session_id: owner_session_id.map(str::to_owned),
            interception_session_id: owner_session_id.map(str::to_owned),
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            fetch_request_id: request_id.to_owned(),
            response_stage_request_id: request_id.to_owned(),
            document_navigation_token: None,
            navigation: NavigationDispatchState {
                navigate_id: Some(1),
                navigate_session_id: owner_session_id.map(str::to_owned),
                result_projection: NavigationResultProjection::Cdp(
                    json!({"frameId": "FRAME-1", "loaderId": "LOADER-1"}),
                ),
                frame_id: "FRAME-1".to_owned(),
                session_id: owner_session_id.map(str::to_owned),
                request_id: Some(format!("NETWORK-{request_id}")),
                loader_id: "LOADER-1".to_owned(),
                request_announced: false,
                requested_url: test_url("auth-page"),
                request_method: "GET".to_owned(),
                request_body: None,
                request_body_bytes: None,
                request_headers: Vec::new(),
                request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
                timestamp: 0.0,
                source_document_security: Default::default(),
            },
            request_cookie_report: None,
            auth_response: PendingFetchAuthNavigation::test_auth_response(test_url("auth-page")),
            challenge: FetchAuthChallenge {
                origin: "https://example.test".to_owned(),
                source: "Server".to_owned(),
                scheme: "basic".to_owned(),
                realm: "test".to_owned(),
            },
            intercept_response: false,
            response_stage_url_match_policy: ResponseStageUrlMatchPolicy::AlreadyMatched,
            auth_stage_chain: None,
        }
    }

    #[test]
    fn disable_session_drain_preserves_other_session_subresource_pending_state() {
        let mut owner = TargetFetchOwner::default();
        owner.register_pending_subresource_fetch_request(
            "FETCH-A".to_owned(),
            pending_subresource_fetch(1, Some("SID-A")),
        );

        let mut pending_b = pending_subresource_fetch(2, Some("SID-B"));
        pending_b.request_stage_chain = Some(Box::new(PendingSubresourceFetchRequestStageChain {
            url: test_url("api"),
            method: "GET".to_owned(),
            headers: Vec::new(),
            body: None,
            request_cookie_report: None,
            remaining_sessions: vec![
                PendingSubresourceFetchRequestStage {
                    session_id: Some("SID-A".to_owned()),
                    owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                    request_id: "FETCH-A-LATER".to_owned(),
                    blocked_intercepts: Vec::new(),
                },
                PendingSubresourceFetchRequestStage {
                    session_id: Some("SID-C".to_owned()),
                    owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                    request_id: "FETCH-C-LATER".to_owned(),
                    blocked_intercepts: Vec::new(),
                },
            ],
        }));
        owner.register_pending_subresource_fetch_request("FETCH-B".to_owned(), pending_b);

        owner.register_pending_subresource_fetch_auth_request(
            "AUTH-A".to_owned(),
            pending_subresource_auth(10, Some("SID-A")),
        );
        owner.register_pending_subresource_fetch_auth_request(
            "AUTH-B".to_owned(),
            pending_subresource_auth(11, Some("SID-B")),
        );
        owner.register_pending_subresource_fetch_response_request(
            "RESP-A".to_owned(),
            pending_subresource_response(20, Some("SID-A")),
        );
        owner.register_pending_subresource_fetch_response_request(
            "RESP-B".to_owned(),
            pending_subresource_response(21, Some("SID-B")),
        );
        owner.register_in_flight_subresource_fetch_request_with_response_match_policy(
            Some("IN-FLIGHT-A".to_owned()),
            pending_subresource_fetch(30, Some("SID-A")),
            ResponseStageUrlMatchPolicy::MatchFinalUrl,
        );
        owner.register_in_flight_subresource_fetch_request_with_response_match_policy(
            Some("IN-FLIGHT-B".to_owned()),
            pending_subresource_fetch(31, Some("SID-B")),
            ResponseStageUrlMatchPolicy::MatchFinalUrl,
        );

        let (
            pending_navigations,
            pending_auth_navigations,
            pending_response_navigations,
            pending_fetches,
            pending_auths,
            pending_responses,
        ) = owner.drain_pending_requests_for_disable_session(Some("SID-A"));

        assert!(pending_navigations.is_empty());
        assert!(pending_auth_navigations.is_empty());
        assert!(pending_response_navigations.is_empty());
        assert_eq!(pending_fetches.len(), 1);
        assert_eq!(pending_fetches[0].0, "FETCH-A");
        assert_eq!(pending_auths.len(), 1);
        assert_eq!(pending_auths[0].0, "AUTH-A");
        assert_eq!(pending_responses.len(), 1);
        assert_eq!(pending_responses[0].0, "RESP-A");

        assert!(!owner.has_pending_fetch_request_id_for_test("FETCH-A"));
        assert!(!owner.has_pending_fetch_request_id_for_test("AUTH-A"));
        assert!(!owner.has_pending_fetch_request_id_for_test("RESP-A"));
        assert!(owner.has_pending_fetch_request_id_for_test("FETCH-B"));
        assert!(owner.has_pending_fetch_request_id_for_test("AUTH-B"));
        assert!(owner.has_pending_fetch_request_id_for_test("RESP-B"));

        assert_eq!(owner.in_flight_subresource_fetch_request_id(30), None);
        assert_eq!(
            owner.in_flight_subresource_fetch_request_id(31),
            Some("IN-FLIGHT-B")
        );
        let in_flight_a = owner
            .take_in_flight_subresource_fetch_request(30)
            .expect("disabled session in-flight entry should remain for renderer completion");
        assert_eq!(
            in_flight_a.response_stage_url_match_policy,
            ResponseStageUrlMatchPolicy::AlreadyMatched
        );

        let pending_b = owner
            .take_pending_subresource_fetch_request("FETCH-B", Some("SID-B"))
            .expect("other session request-stage pending should remain");
        let remaining_sessions = pending_b
            .request_stage_pause_state()
            .expect("other session request-stage chain should remain")
            .remaining_sessions
            .iter()
            .map(|stage| stage.session_id.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(remaining_sessions, vec![Some("SID-C")]);
        assert!(
            owner
                .take_pending_subresource_fetch_auth_request("AUTH-B", Some("SID-B"))
                .is_some()
        );
        assert!(
            owner
                .take_pending_subresource_fetch_response_request("RESP-B", Some("SID-B"))
                .is_some()
        );
    }

    #[test]
    fn disable_session_drain_prunes_navigation_auth_stage_chains() {
        let mut owner = TargetFetchOwner::default();
        let mut pending = pending_fetch_auth_navigation("AUTH-B", Some("SID-B"));
        pending.auth_stage_chain = Some(Box::new(PendingSubresourceFetchAuthStageChain {
            remaining_sessions: vec![
                PendingSubresourceFetchAuthStage {
                    session_id: Some("SID-A".to_owned()),
                    owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                    request_id: "AUTH-A-LATER".to_owned(),
                    blocked_intercepts: Vec::new(),
                },
                PendingSubresourceFetchAuthStage {
                    session_id: Some("SID-C".to_owned()),
                    owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                    request_id: "AUTH-C-LATER".to_owned(),
                    blocked_intercepts: Vec::new(),
                },
            ],
        }));
        owner.register_pending_fetch_auth_navigation("AUTH-B".to_owned(), pending);

        let (_, pending_auth_navigations, _, _, _, _) =
            owner.drain_pending_requests_for_disable_session(Some("SID-A"));

        assert!(
            pending_auth_navigations.is_empty(),
            "other-session active auth navigation should remain pending"
        );
        assert!(owner.has_pending_fetch_request_id_for_test("AUTH-B"));

        let pending = owner
            .take_pending_fetch_auth_navigation_for_action_session("AUTH-B", Some("SID-B"))
            .expect("other-session auth navigation should remain actionable");
        let remaining_sessions = pending
            .auth_stage_pause_state()
            .expect("remaining auth chain should stay attached")
            .remaining_sessions
            .iter()
            .map(|stage| stage.session_id.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(remaining_sessions, vec![Some("SID-C")]);
    }

    #[test]
    fn disable_session_drain_preserves_same_session_network_or_bidi_pending_state() {
        let mut owner = TargetFetchOwner::default();
        owner.register_pending_subresource_fetch_request(
            "FETCH-OWNED".to_owned(),
            pending_subresource_fetch(1, Some("SID-A")),
        );

        let mut network_owned_pending = pending_subresource_fetch_with_owner_kind(
            2,
            Some("SID-A"),
            PendingSubresourceFetchOwnerKind::NetworkOrBidi,
        );
        network_owned_pending.request_stage_chain =
            Some(Box::new(PendingSubresourceFetchRequestStageChain {
                url: test_url("api"),
                method: "GET".to_owned(),
                headers: Vec::new(),
                body: None,
                request_cookie_report: None,
                remaining_sessions: vec![
                    PendingSubresourceFetchRequestStage {
                        session_id: Some("SID-A".to_owned()),
                        owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
                        request_id: "FETCH-STAGE-SAME-SESSION".to_owned(),
                        blocked_intercepts: Vec::new(),
                    },
                    PendingSubresourceFetchRequestStage {
                        session_id: Some("SID-A".to_owned()),
                        owner_kind: PendingSubresourceFetchOwnerKind::NetworkOrBidi,
                        request_id: "NETWORK-STAGE-SAME-SESSION".to_owned(),
                        blocked_intercepts: Vec::new(),
                    },
                ],
            }));
        owner.register_pending_subresource_fetch_request(
            "NETWORK-OWNED".to_owned(),
            network_owned_pending,
        );

        owner.register_pending_subresource_fetch_auth_request(
            "AUTH-FETCH".to_owned(),
            pending_subresource_auth(10, Some("SID-A")),
        );
        owner.register_pending_subresource_fetch_auth_request(
            "AUTH-NETWORK".to_owned(),
            pending_subresource_auth_with_owner_kind(
                11,
                Some("SID-A"),
                PendingSubresourceFetchOwnerKind::NetworkOrBidi,
            ),
        );
        owner.register_pending_subresource_fetch_response_request(
            "RESP-FETCH".to_owned(),
            pending_subresource_response(20, Some("SID-A")),
        );
        owner.register_pending_subresource_fetch_response_request(
            "RESP-NETWORK".to_owned(),
            pending_subresource_response_with_owner_kind(
                21,
                Some("SID-A"),
                PendingSubresourceFetchOwnerKind::NetworkOrBidi,
            ),
        );
        owner.register_in_flight_subresource_fetch_request_with_response_match_policy(
            Some("IN-FLIGHT-FETCH".to_owned()),
            pending_subresource_fetch(30, Some("SID-A")),
            ResponseStageUrlMatchPolicy::MatchFinalUrl,
        );
        owner.register_in_flight_subresource_fetch_request_with_response_match_policy(
            Some("IN-FLIGHT-NETWORK".to_owned()),
            pending_subresource_fetch_with_owner_kind(
                31,
                Some("SID-A"),
                PendingSubresourceFetchOwnerKind::NetworkOrBidi,
            ),
            ResponseStageUrlMatchPolicy::MatchFinalUrl,
        );

        let (_, _, _, pending_fetches, pending_auths, pending_responses) =
            owner.drain_pending_requests_for_disable_session(Some("SID-A"));

        assert_eq!(pending_fetches.len(), 1);
        assert_eq!(pending_fetches[0].0, "FETCH-OWNED");
        assert_eq!(pending_auths.len(), 1);
        assert_eq!(pending_auths[0].0, "AUTH-FETCH");
        assert_eq!(pending_responses.len(), 1);
        assert_eq!(pending_responses[0].0, "RESP-FETCH");

        assert!(!owner.has_pending_fetch_request_id_for_test("FETCH-OWNED"));
        assert!(!owner.has_pending_fetch_request_id_for_test("AUTH-FETCH"));
        assert!(!owner.has_pending_fetch_request_id_for_test("RESP-FETCH"));
        assert!(owner.has_pending_fetch_request_id_for_test("NETWORK-OWNED"));
        assert!(owner.has_pending_fetch_request_id_for_test("AUTH-NETWORK"));
        assert!(owner.has_pending_fetch_request_id_for_test("RESP-NETWORK"));

        assert_eq!(owner.in_flight_subresource_fetch_request_id(30), None);
        assert_eq!(
            owner.in_flight_subresource_fetch_request_id(31),
            Some("IN-FLIGHT-NETWORK")
        );

        let network_owned_pending = owner
            .take_pending_subresource_fetch_request("NETWORK-OWNED", Some("SID-A"))
            .expect("same-session Network/BiDi-owned pending should remain");
        let remaining_sessions = network_owned_pending
            .request_stage_pause_state()
            .expect("Network/BiDi-owned pending chain should remain")
            .remaining_sessions
            .iter()
            .map(|stage| (stage.session_id.as_deref(), stage.owner_kind))
            .collect::<Vec<_>>();
        assert_eq!(
            remaining_sessions,
            vec![(
                Some("SID-A"),
                PendingSubresourceFetchOwnerKind::NetworkOrBidi
            )]
        );
        assert!(
            owner
                .take_pending_subresource_fetch_auth_request("AUTH-NETWORK", Some("SID-A"))
                .is_some()
        );
        assert!(
            owner
                .take_pending_subresource_fetch_response_request("RESP-NETWORK", Some("SID-A"))
                .is_some()
        );
    }
}
