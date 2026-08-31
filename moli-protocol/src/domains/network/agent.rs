use std::collections::{HashMap, HashSet};

use moli_bounded_buffer::{BoundedByteBuffer, ByteLimits, InsertOutcome};
use moli_core::page::{ScriptNetworkOutputItem, SubresourceNetworkRequestHandle};

use crate::conn::{CapturedBody, ConnectionNetworkRequestIdAllocator};
use crate::devtools_runtime::DevToolsNetworkDataType;

use super::{
    PendingNetworkBacklogDeliverySnapshot, PendingSubresourceNetworkActivity,
    PendingSubresourceNetworkActivitySession, PendingWebSocketNetworkActivity,
    PendingWebSocketNetworkActivitySession, TargetNetworkBacklogPreparedDelivery,
    TargetNetworkBacklogRequestIdResolver, TargetNetworkOutputQueue, TargetSubresourcePlanOutput,
};

const RESPONSE_BODY_BUFFER_MAX_TOTAL_BYTES: usize = 20_000_000;
const RESPONSE_BODY_BUFFER_MAX_ENTRY_BYTES: usize = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererSubresourceTeardownDisposition {
    RequiresDocumentTerminal,
    DetachedKeepalive,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetNetworkAgentState {
    primary_enabled: bool,
    auxiliary_event_session_ids: Vec<String>,
    output_queue: TargetNetworkOutputQueue,
    active_renderer_subresource_requests:
        HashMap<SubresourceNetworkRequestHandle, RendererSubresourceTeardownDisposition>,
    artifacts: TargetNetworkArtifacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollectedNetworkDataArtifact {
    pub(crate) request_id: String,
    pub(crate) data_type: DevToolsNetworkDataType,
    pub(crate) body: CapturedBody,
    pub(crate) collector_ids: Vec<String>,
    pub(crate) collection_was_gated: bool,
}

struct NetworkBacklogRequestIdPlan<'a> {
    subresource_artifacts: &'a mut SubresourceNetworkArtifacts,
    websocket_artifacts: &'a mut WebSocketNetworkArtifacts,
    preferred_request_id: NetworkBacklogPreferredRequestIdBudget,
    request_id_allocator: &'a mut ConnectionNetworkRequestIdAllocator,
}

impl<'a> NetworkBacklogRequestIdPlan<'a> {
    fn new(
        subresource_artifacts: &'a mut SubresourceNetworkArtifacts,
        websocket_artifacts: &'a mut WebSocketNetworkArtifacts,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        request_id_allocator: &'a mut ConnectionNetworkRequestIdAllocator,
    ) -> Self {
        let preferred_request_id = NetworkBacklogPreferredRequestIdBudget::new(
            preferred_request_id,
            subresource_artifacts,
        );
        Self {
            subresource_artifacts,
            websocket_artifacts,
            preferred_request_id,
            request_id_allocator,
        }
    }

    fn request_id_for_subresource_output(
        &mut self,
        output: &TargetSubresourcePlanOutput,
    ) -> String {
        if let Some(socket_id) = output.websocket_socket_id()
            && let Some(request_id) = self.websocket_artifacts.request_id_for_socket(socket_id)
        {
            return request_id.to_owned();
        }
        if let Some(handle) = output.request_handle()
            && let Some(request_id) = self.subresource_artifacts.request_id_for_handle(handle)
        {
            return request_id.to_owned();
        }
        let request_id = self
            .preferred_request_id
            .take_for_new_subresource(output.request_handle())
            .unwrap_or_else(|| self.request_id_allocator.allocate_request_id());
        if let Some(handle) = output.request_handle() {
            self.subresource_artifacts
                .set_request_id_for_handle_if_absent(handle, request_id.clone());
        }
        if let Some(socket_id) = output.websocket_socket_id() {
            self.websocket_artifacts
                .set_request_id_for_socket_if_absent(socket_id, request_id.clone());
        }
        request_id
    }

    fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String {
        if let Some(request_id) = self.websocket_artifacts.request_id_for_socket(socket_id) {
            return request_id.to_owned();
        }
        let request_id = self.request_id_allocator.allocate_request_id();
        self.websocket_artifacts
            .set_request_id_for_socket_if_absent(socket_id, request_id.clone());
        request_id
    }
}

impl TargetNetworkBacklogRequestIdResolver for NetworkBacklogRequestIdPlan<'_> {
    fn request_id_for_subresource_output(
        &mut self,
        output: &TargetSubresourcePlanOutput,
    ) -> String {
        NetworkBacklogRequestIdPlan::request_id_for_subresource_output(self, output)
    }

    fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String {
        NetworkBacklogRequestIdPlan::request_id_for_websocket_socket(self, socket_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkBacklogPreferredRequestId<'a> {
    ContextualSubresource(&'a str),
}

impl<'a> NetworkBacklogPreferredRequestId<'a> {
    pub(crate) fn contextual_subresource(request_id: &'a str) -> Self {
        Self::ContextualSubresource(request_id)
    }

    fn value(self) -> &'a str {
        match self {
            Self::ContextualSubresource(request_id) => request_id,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NetworkBacklogPreferredRequestIdBudget {
    value: Option<String>,
    bound_request_handle: Option<SubresourceNetworkRequestHandle>,
}

impl NetworkBacklogPreferredRequestIdBudget {
    fn new(
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        subresource_artifacts: &SubresourceNetworkArtifacts,
    ) -> Self {
        let value = preferred_request_id.map(|request_id| request_id.value().to_owned());
        // A contextual id belongs to the request whose Fetch pause produced
        // it. Once that request has a stable handle, reserve the id for that
        // handle instead of letting an older uncorrelated backlog item consume
        // it merely because that item is delivered first.
        let bound_request_handle = value
            .as_deref()
            .and_then(|request_id| subresource_artifacts.request_handle_for_request_id(request_id));
        Self {
            value,
            bound_request_handle,
        }
    }

    fn take_for_new_subresource(
        &mut self,
        request_handle: Option<SubresourceNetworkRequestHandle>,
    ) -> Option<String> {
        if self
            .bound_request_handle
            .is_some_and(|expected_handle| Some(expected_handle) != request_handle)
        {
            return None;
        }
        self.value.take()
    }
}

impl TargetNetworkAgentState {
    /// Moves the Document-owned live queue and request correlations into a
    /// short-lived predecessor state while retaining target/session policy and
    /// target-scoped body/stream artifacts for the replacement Document.
    ///
    /// A renderer Page can publish its final cancellation batch after protocol
    /// has installed its successor Page. Keeping the predecessor queue intact
    /// lets those terminal facts reuse the request ids announced before the
    /// commit without making the successor replay historical requests.
    pub(crate) fn rotate_document_for_replacement(&mut self) -> RetiringTargetNetworkAgentState {
        RetiringTargetNetworkAgentState {
            output_queue: std::mem::take(&mut self.output_queue),
            active_renderer_subresource_requests: std::mem::take(
                &mut self.active_renderer_subresource_requests,
            ),
            subresource_network_artifacts: std::mem::take(
                &mut self.artifacts.subresource_network_artifacts,
            ),
            websocket_network_artifacts: std::mem::take(
                &mut self.artifacts.websocket_network_artifacts,
            ),
        }
    }

    pub(crate) fn primary_events_enabled(&self) -> bool {
        self.primary_enabled
    }

    pub(crate) fn set_primary_events_enabled(&mut self, enabled: bool) {
        self.primary_enabled = enabled;
    }

    #[cfg(test)]
    pub(crate) fn enable_primary_events(&mut self) {
        self.set_primary_events_enabled(true);
    }

    pub(crate) fn disable_primary_events(&mut self) {
        self.set_primary_events_enabled(false);
    }

    pub(crate) fn reset_output_queue(&mut self) {
        self.output_queue.reset();
        self.artifacts
            .subresource_network_artifacts
            .clear_request_ids();
        self.artifacts
            .websocket_network_artifacts
            .clear_request_ids();
    }

    /// Applies one source-bound live renderer fact and updates the protocol's
    /// authoritative in-flight request set at the same boundary.
    ///
    /// Network-idle must be derived from concrete request lifecycle facts, not
    /// from a later renderer snapshot. The exact Document check is performed
    /// by the owning runtime slot before this method is called.
    pub(crate) fn ingest_renderer_output_item(
        &mut self,
        item: &ScriptNetworkOutputItem,
        document_loader_id: &str,
    ) {
        update_active_renderer_subresource_requests(
            &mut self.active_renderer_subresource_requests,
            item,
        );
        self.output_queue
            .append_renderer_output_item_for_loader(item, document_loader_id);
    }

    /// Applies one renderer fact and freezes exactly the delivery range that
    /// fact appended for the listeners enabled at this ingress boundary.
    ///
    /// This deliberately does not start from the sessions' emitted cursors.
    /// Several concrete renderer publications can reach ordered ingress before
    /// the first one is projected, so those cursors may still point before an
    /// earlier prepared token. Starting from them would duplicate that earlier
    /// fact in every later token. The concrete stream FIFO instead guarantees
    /// these disjoint ranges are projected in the same order they are prepared.
    pub(crate) fn ingest_renderer_output_item_and_prepare_live_delivery(
        &mut self,
        item: &ScriptNetworkOutputItem,
        document_loader_id: &str,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> TargetNetworkBacklogPreparedDelivery {
        let subresource_start = self.output_queue_subresource_record_count();
        let websocket_event_start = self.output_queue_websocket_event_count();
        self.ingest_renderer_output_item(item, document_loader_id);

        let sessions = self.event_session_ids(trigger_session_id, primary_session_id);
        let subresource_activity = PendingSubresourceNetworkActivity::from_sessions(
            sessions
                .iter()
                .cloned()
                .map(|session_id| {
                    PendingSubresourceNetworkActivitySession::new(session_id, subresource_start)
                })
                .collect(),
        );
        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(
            sessions
                .into_iter()
                .map(|session_id| {
                    PendingWebSocketNetworkActivitySession::new(
                        session_id,
                        subresource_start,
                        websocket_event_start,
                    )
                })
                .collect(),
        );
        let mut request_ids = NetworkBacklogRequestIdPlan::new(
            &mut self.artifacts.subresource_network_artifacts,
            &mut self.artifacts.websocket_network_artifacts,
            preferred_request_id,
            request_id_allocator,
        );
        self.output_queue.backlog_prepared_delivery_for_activity(
            subresource_activity,
            websocket_activity,
            &mut request_ids,
        )
    }

    pub(crate) fn renderer_subresources_are_idle(&self) -> bool {
        self.active_renderer_subresource_requests.is_empty()
    }

    fn output_queue_subresource_record_count(&self) -> usize {
        self.output_queue.subresource_record_count()
    }

    fn output_queue_websocket_event_count(&self) -> usize {
        self.output_queue.websocket_event_count()
    }

    pub(crate) fn enable_auxiliary_events(&mut self, session_id: &str) {
        if !self
            .auxiliary_event_session_ids
            .iter()
            .any(|enabled_session_id| enabled_session_id == session_id)
        {
            self.auxiliary_event_session_ids.push(session_id.to_owned());
        }
    }

    pub(crate) fn disable_auxiliary_events(&mut self, session_id: &str) -> bool {
        let previous_len = self.auxiliary_event_session_ids.len();
        self.auxiliary_event_session_ids
            .retain(|enabled_session_id| enabled_session_id != session_id);
        self.auxiliary_event_session_ids.len() != previous_len
    }

    pub(crate) fn remove_auxiliary_session(&mut self, session_id: &str) {
        self.disable_auxiliary_events(session_id);
        self.remove_session_observation_cursor(Some(session_id));
    }

    pub(crate) fn has_event_listeners(&self) -> bool {
        self.primary_enabled || !self.auxiliary_event_session_ids.is_empty()
    }

    pub(crate) fn auxiliary_events_enabled_for_session(&self, session_id: &str) -> bool {
        self.auxiliary_event_session_ids
            .iter()
            .any(|enabled_session_id| enabled_session_id == session_id)
    }

    pub(crate) fn event_session_ids(
        &self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        let mut session_ids = Vec::new();
        let primary_event_session_id = primary_session_id.or(trigger_session_id);
        if self.primary_enabled {
            session_ids.push(primary_event_session_id.map(str::to_owned));
        }
        for session_id in &self.auxiliary_event_session_ids {
            if !self.primary_enabled || Some(session_id.as_str()) != primary_event_session_id {
                session_ids.push(Some(session_id.clone()));
            }
        }
        session_ids
    }

    pub(crate) fn pending_subresource_activity(
        &self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
    ) -> Option<PendingSubresourceNetworkActivity> {
        let sessions = self
            .event_session_ids(trigger_session_id, primary_session_id)
            .into_iter()
            .map(|session_id| {
                let start_index = self
                    .artifacts
                    .subresource_network_artifacts
                    .emitted_record_count_for_session(session_id.as_deref());
                PendingSubresourceNetworkActivitySession::new(session_id, start_index)
            })
            .collect::<Vec<_>>();
        PendingSubresourceNetworkActivity::from_sessions(sessions)
    }

    pub(crate) fn pending_websocket_activity(
        &self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
    ) -> Option<PendingWebSocketNetworkActivity> {
        let sessions = self
            .event_session_ids(trigger_session_id, primary_session_id)
            .into_iter()
            .map(|session_id| {
                let record_start_index = self
                    .artifacts
                    .websocket_network_artifacts
                    .emitted_record_count_for_session(session_id.as_deref());
                let event_start_index = self
                    .artifacts
                    .websocket_network_artifacts
                    .emitted_event_count_for_session(session_id.as_deref());
                PendingWebSocketNetworkActivitySession::new(
                    session_id,
                    record_start_index,
                    event_start_index,
                )
            })
            .collect::<Vec<_>>();
        PendingWebSocketNetworkActivity::from_sessions(sessions)
    }

    pub(crate) fn pending_network_backlog_delivery_snapshot(
        &mut self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        let mut backlog = self.backlog_prepared_delivery(
            trigger_session_id,
            primary_session_id,
            preferred_request_id,
            request_id_allocator,
        );
        self.pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
    }

    pub(crate) fn pending_network_backlog_delivery_snapshot_from_backlog(
        &mut self,
        backlog: &mut TargetNetworkBacklogPreparedDelivery,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        self.output_queue
            .pending_network_backlog_delivery_snapshot_from_backlog(backlog)
    }

    pub(crate) fn backlog_prepared_delivery(
        &mut self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> TargetNetworkBacklogPreparedDelivery {
        let subresource_activity =
            self.pending_subresource_activity(trigger_session_id, primary_session_id);
        let websocket_activity =
            self.pending_websocket_activity(trigger_session_id, primary_session_id);
        let mut request_ids = NetworkBacklogRequestIdPlan::new(
            &mut self.artifacts.subresource_network_artifacts,
            &mut self.artifacts.websocket_network_artifacts,
            preferred_request_id,
            request_id_allocator,
        );
        self.output_queue.backlog_prepared_delivery_for_activity(
            subresource_activity,
            websocket_activity,
            &mut request_ids,
        )
    }

    pub(crate) fn initialize_session_observation_cursor_at_output_tail(
        &mut self,
        session_id: Option<&str>,
    ) {
        let subresource_record_count = self.output_queue_subresource_record_count();
        let websocket_event_count = self.output_queue_websocket_event_count();
        self.artifacts
            .subresource_network_artifacts
            .set_session_cursor(session_id, subresource_record_count);
        self.artifacts
            .websocket_network_artifacts
            .set_session_cursors(session_id, subresource_record_count, websocket_event_count);
    }

    pub(crate) fn remove_session_observation_cursor(&mut self, session_id: Option<&str>) {
        self.artifacts
            .subresource_network_artifacts
            .remove_session_cursor(session_id);
        self.artifacts
            .websocket_network_artifacts
            .remove_session_cursors(session_id);
    }

    pub(crate) fn collected_network_data_artifacts(&self) -> Vec<CollectedNetworkDataArtifact> {
        self.artifacts.collected_network_data_artifacts()
    }

    pub(crate) fn record_subresource_request_id_for_handle_if_absent(
        &mut self,
        handle: SubresourceNetworkRequestHandle,
        request_id: String,
    ) {
        self.artifacts
            .subresource_network_artifacts
            .set_request_id_for_handle_if_absent(handle, request_id);
    }

    pub(crate) fn record_fetch_pause_announced_request_id(&mut self, request_id: String) {
        self.artifacts
            .subresource_network_artifacts
            .record_fetch_pause_announced_request_id(request_id);
    }

    pub(crate) fn take_fetch_pause_announced_request_id(&mut self, request_id: &str) -> bool {
        self.artifacts
            .subresource_network_artifacts
            .take_fetch_pause_announced_request_id(request_id)
    }

    pub(crate) fn clear_fetch_pause_announced_request_id(&mut self, request_id: &str) {
        self.artifacts
            .subresource_network_artifacts
            .clear_fetch_pause_announced_request_id(request_id);
    }

    pub(crate) fn request_id_for_subresource_handle(
        &mut self,
        handle: SubresourceNetworkRequestHandle,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> String {
        if let Some(request_id) = self
            .artifacts
            .subresource_network_artifacts
            .request_id_for_handle(handle)
        {
            return request_id.to_owned();
        }
        let request_id = request_id_allocator.allocate_request_id();
        self.artifacts
            .subresource_network_artifacts
            .set_request_id_for_handle_if_absent(handle, request_id.clone());
        request_id
    }

    pub(crate) fn claim_completed_subresource_request_id(&mut self, request_id: &str) -> bool {
        self.artifacts
            .subresource_network_artifacts
            .claim_completed_request_id(request_id)
    }

    #[cfg(test)]
    pub(crate) fn allocate_network_request_id(&mut self) -> String {
        self.artifacts.allocate_network_request_id()
    }

    #[cfg(test)]
    pub(crate) fn set_session_observation_cursor_at_counts_for_test(
        &mut self,
        session_id: Option<&str>,
        subresource_count: usize,
        websocket_count: usize,
    ) {
        self.artifacts.set_session_observation_cursor_at_counts(
            session_id,
            subresource_count,
            websocket_count,
        );
    }

    #[cfg(test)]
    pub(crate) fn emitted_subresource_record_count_for_session_for_test(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.artifacts
            .emitted_subresource_record_count_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn emitted_websocket_event_count_for_session_for_test(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.artifacts
            .emitted_websocket_event_count_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn artifacts_are_default_for_test(&self) -> bool {
        self.artifacts == TargetNetworkArtifacts::default()
    }

    #[cfg(test)]
    pub(crate) fn record_captured_response_body(
        &mut self,
        request_id: String,
        response_body: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.artifacts
            .body_artifacts
            .insert(request_id, response_body, session_ids);
    }

    pub(crate) fn record_captured_response_body_source_with_collector_scope(
        &mut self,
        request_id: String,
        response_body: CapturedBody,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.artifacts
            .body_artifacts
            .insert_captured_body_with_collector_scope(
                request_id,
                response_body,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn record_captured_request_body_with_collector_scope(
        &mut self,
        request_id: String,
        request_body: Vec<u8>,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.artifacts
            .body_artifacts
            .insert_request_body_with_collector_scope(
                request_id,
                request_body,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    #[cfg(test)]
    pub(crate) fn record_pending_response_body(
        &mut self,
        request_id: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.record_pending_response_body_with_collector_scope(
            request_id,
            session_ids,
            std::iter::empty::<String>(),
            false,
        );
    }

    pub(crate) fn record_pending_response_body_with_collector_scope(
        &mut self,
        request_id: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.artifacts
            .body_artifacts
            .insert_pending_with_collector_scope(
                request_id,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn record_failed_response_body_with_collector_scope(
        &mut self,
        request_id: String,
        error_text: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.artifacts
            .body_artifacts
            .insert_failed_with_collector_scope(
                request_id,
                error_text,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn captured_response_body(&self, request_id: &str) -> Option<&CapturedResponseBody> {
        self.artifacts
            .body_artifacts
            .captured_response_body(request_id)
    }

    pub(crate) fn captured_request_body(&self, request_id: &str) -> Option<&CapturedRequestBody> {
        self.artifacts
            .body_artifacts
            .captured_request_body(request_id)
    }

    pub(crate) fn clear_captured_response_bodies(&mut self) {
        self.artifacts
            .body_artifacts
            .clear_captured_response_bodies();
    }

    pub(crate) fn clear_body_artifacts(&mut self) {
        self.artifacts.body_artifacts.clear_session_scoped();
    }

    pub(crate) fn remove_captured_response_body_visibility_for_session(
        &mut self,
        session_id: Option<&str>,
    ) {
        self.artifacts
            .body_artifacts
            .remove_session_visibility(session_id);
    }

    pub(crate) fn allocate_io_stream_handle(&mut self) -> String {
        self.artifacts.body_artifacts.allocate_io_stream_handle()
    }

    pub(crate) fn insert_io_stream(&mut self, handle: String, bytes: Vec<u8>, offset: usize) {
        self.artifacts
            .body_artifacts
            .insert_stream(handle, bytes, offset);
    }

    pub(crate) fn insert_io_stream_body_source(
        &mut self,
        handle: String,
        body: CapturedBody,
        offset: usize,
    ) {
        self.artifacts
            .body_artifacts
            .insert_stream_body_source(handle, body, offset);
    }

    pub(crate) fn read_io_stream(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> Option<TargetIoStreamRead> {
        self.artifacts
            .body_artifacts
            .read_io_stream(handle, offset, size)
    }

    pub(crate) fn close_io_stream(&mut self, handle: &str) -> bool {
        self.artifacts.body_artifacts.close_io_stream(handle)
    }

    pub(crate) fn mark_subresource_records_emitted(
        &mut self,
        session_id: Option<&str>,
        start_index: usize,
        record_count: usize,
    ) {
        self.artifacts.subresource_network_artifacts.mark_emitted(
            session_id,
            start_index,
            record_count,
        );
    }

    pub(crate) fn reset_subresource_cursor(&mut self) {
        self.artifacts.subresource_network_artifacts.reset_cursor();
    }

    pub(crate) fn mark_websocket_activity_emitted(
        &mut self,
        session_id: Option<&str>,
        record_start_index: usize,
        record_count: usize,
        event_start_index: usize,
        event_count: usize,
    ) {
        self.artifacts.websocket_network_artifacts.mark_emitted(
            session_id,
            record_start_index,
            record_count,
            event_start_index,
            event_count,
        );
    }

    pub(crate) fn mark_network_backlog_delivery_snapshot_emitted(
        &mut self,
        snapshot: &PendingNetworkBacklogDeliverySnapshot,
    ) {
        for cursor in snapshot.subresource_cursor_advances() {
            self.mark_subresource_records_emitted(
                cursor.session_id(),
                cursor.start_index(),
                cursor.record_count(),
            );
        }
        for cursor in snapshot.websocket_cursor_advances() {
            self.mark_websocket_activity_emitted(
                cursor.session_id(),
                cursor.record_start_index(),
                cursor.record_count(),
                cursor.event_start_index(),
                cursor.event_count(),
            );
        }
    }

    pub(crate) fn clear_websocket_request_ids(&mut self) {
        self.artifacts
            .websocket_network_artifacts
            .clear_request_ids();
    }

    pub(crate) fn clear_websocket_artifacts(&mut self) {
        self.artifacts.websocket_network_artifacts.clear_all();
    }

    pub(crate) fn register_synthetic_websocket_request(
        &mut self,
        request_id: String,
        network_request_id: String,
        socket_id: u64,
    ) {
        self.artifacts
            .websocket_network_artifacts
            .register_synthetic_request(request_id, network_request_id, socket_id);
    }

    pub(crate) fn synthetic_websocket_socket_id_for_request(
        &self,
        request_id: &str,
    ) -> Option<u64> {
        self.artifacts
            .websocket_network_artifacts
            .synthetic_socket_id_for_request(request_id)
    }

    #[cfg(test)]
    fn record_websocket_request_id_for_socket_if_absent(
        &mut self,
        socket_id: u64,
        request_id: String,
    ) {
        self.artifacts
            .record_websocket_request_id_for_socket_if_absent(socket_id, request_id);
    }

    #[cfg(test)]
    fn websocket_request_id_for_socket_or_allocate(&mut self, socket_id: u64) -> String {
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        self.websocket_request_id_for_socket_or_allocate_with(socket_id, &mut request_id_allocator)
    }

    #[cfg(test)]
    fn websocket_request_id_for_socket_or_allocate_with(
        &mut self,
        socket_id: u64,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> String {
        self.artifacts
            .request_id_for_websocket_socket(socket_id, request_id_allocator)
    }

    pub(crate) fn clear_session_scoped_observation_artifacts(&mut self) {
        self.artifacts.clear_session_scoped_observation_artifacts();
    }

    pub(crate) fn reset_all_target_scoped_artifacts(&mut self) {
        self.artifacts.reset_all_target_scoped_artifacts();
    }

    #[cfg(test)]
    pub(crate) fn has_auxiliary_events_for_session(&self, session_id: &str) -> bool {
        self.auxiliary_events_enabled_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn has_captured_response_body(&self, request_id: &str) -> bool {
        self.artifacts.body_artifacts.contains_body_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn captured_response_bodies_empty(&self) -> bool {
        self.artifacts
            .body_artifacts
            .captured_response_bodies_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_next_network_request_sequence_for_test(&mut self, sequence: u64) {
        self.artifacts
            .request_id_allocator
            .set_next_sequence_for_test(sequence);
    }

    #[cfg(test)]
    pub(crate) fn next_network_request_sequence_for_test(&self) -> u64 {
        self.artifacts.request_id_allocator.next_sequence_for_test()
    }

    #[cfg(test)]
    pub(crate) fn io_streams_empty(&self) -> bool {
        self.artifacts.body_artifacts.io_streams_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_next_io_stream_sequence_for_test(&mut self, sequence: u64) {
        self.artifacts
            .body_artifacts
            .set_next_stream_id_for_test(sequence);
    }

    #[cfg(test)]
    pub(crate) fn next_io_stream_sequence_for_test(&self) -> u64 {
        self.artifacts.body_artifacts.next_stream_id_for_test()
    }

    #[cfg(test)]
    pub(crate) fn set_subresource_emitted_record_count_for_test(&mut self, count: usize) {
        self.artifacts
            .subresource_network_artifacts
            .set_emitted_record_count_for_test(count);
    }

    #[cfg(test)]
    pub(crate) fn subresource_emitted_record_count_for_test(&self) -> usize {
        self.artifacts
            .subresource_network_artifacts
            .emitted_record_count()
    }
}

/// Network lifecycle state retained only until a replaced renderer Page has
/// published and closed its final output stream.
#[derive(Debug)]
pub(crate) struct RetiringTargetNetworkAgentState {
    output_queue: TargetNetworkOutputQueue,
    active_renderer_subresource_requests:
        HashMap<SubresourceNetworkRequestHandle, RendererSubresourceTeardownDisposition>,
    subresource_network_artifacts: SubresourceNetworkArtifacts,
    websocket_network_artifacts: WebSocketNetworkArtifacts,
}

impl RetiringTargetNetworkAgentState {
    pub(crate) fn ingest_renderer_output_item_and_prepare_live_delivery(
        &mut self,
        item: &ScriptNetworkOutputItem,
        document_loader_id: &str,
        event_session_ids: Vec<Option<String>>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> TargetNetworkBacklogPreparedDelivery {
        update_active_renderer_subresource_requests(
            &mut self.active_renderer_subresource_requests,
            item,
        );

        let subresource_start = self.output_queue.subresource_record_count();
        let websocket_event_start = self.output_queue.websocket_event_count();
        self.output_queue
            .append_renderer_output_item_for_loader(item, document_loader_id);
        let subresource_activity = PendingSubresourceNetworkActivity::from_sessions(
            event_session_ids
                .iter()
                .cloned()
                .map(|session_id| {
                    PendingSubresourceNetworkActivitySession::new(session_id, subresource_start)
                })
                .collect(),
        );
        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(
            event_session_ids
                .into_iter()
                .map(|session_id| {
                    PendingWebSocketNetworkActivitySession::new(
                        session_id,
                        subresource_start,
                        websocket_event_start,
                    )
                })
                .collect(),
        );
        let mut request_ids = NetworkBacklogRequestIdPlan::new(
            &mut self.subresource_network_artifacts,
            &mut self.websocket_network_artifacts,
            preferred_request_id,
            request_id_allocator,
        );
        self.output_queue.backlog_prepared_delivery_for_activity(
            subresource_activity,
            websocket_activity,
            &mut request_ids,
        )
    }

    pub(crate) fn unterminated_document_bound_request_diagnostics(
        &self,
    ) -> Vec<(u64, Option<&str>)> {
        let mut handles = self
            .active_renderer_subresource_requests
            .iter()
            .filter_map(|(&handle, &disposition)| {
                (disposition == RendererSubresourceTeardownDisposition::RequiresDocumentTerminal)
                    .then_some(handle)
            })
            .collect::<Vec<_>>();
        handles.sort_unstable_by_key(|handle| handle.get());
        handles
            .into_iter()
            .map(|handle| {
                (
                    handle.get(),
                    self.subresource_network_artifacts
                        .request_id_for_handle(handle),
                )
            })
            .collect()
    }
}

fn update_active_renderer_subresource_requests(
    active_requests: &mut HashMap<
        SubresourceNetworkRequestHandle,
        RendererSubresourceTeardownDisposition,
    >,
    item: &ScriptNetworkOutputItem,
) {
    match item {
        ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
            let disposition = if request.keepalive() {
                RendererSubresourceTeardownDisposition::DetachedKeepalive
            } else {
                RendererSubresourceTeardownDisposition::RequiresDocumentTerminal
            };
            active_requests.insert(request.handle(), disposition);
        }
        ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
            active_requests.remove(&body.handle());
        }
        ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
            if let Some(handle) = record.request_handle() {
                active_requests.remove(&handle);
            }
        }
        ScriptNetworkOutputItem::SubresourceResponseStarted(_)
        | ScriptNetworkOutputItem::SubresourceDataReceived(_)
        | ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
        | ScriptNetworkOutputItem::WebSocketNetworkEvent(_)
        | ScriptNetworkOutputItem::WebSocketLifecycleEvent(_) => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedResponseBody {
    state: CapturedResponseBodyState,
    session_ids: HashSet<Option<String>>,
    collector_ids: HashSet<String>,
    collection_was_gated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CapturedResponseBodyState {
    Pending,
    Ready(CapturedBody),
    Failed(String),
    Evicted,
}

impl CapturedResponseBody {
    pub(crate) fn pending_with_collector_scope(
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) -> Self {
        let collector_ids = collector_ids.into_iter().collect::<HashSet<_>>();
        Self {
            state: CapturedResponseBodyState::Pending,
            session_ids: session_ids.into_iter().collect(),
            collection_was_gated: collection_was_gated || !collector_ids.is_empty(),
            collector_ids,
        }
    }

    pub(crate) fn from_captured_body_with_collector_scope(
        body: CapturedBody,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) -> Self {
        let collector_ids = collector_ids.into_iter().collect::<HashSet<_>>();
        Self {
            state: CapturedResponseBodyState::Ready(body),
            session_ids: session_ids.into_iter().collect(),
            collection_was_gated: collection_was_gated || !collector_ids.is_empty(),
            collector_ids,
        }
    }

    pub(crate) fn failed_with_collector_scope(
        error_text: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) -> Self {
        let collector_ids = collector_ids.into_iter().collect::<HashSet<_>>();
        Self {
            state: CapturedResponseBodyState::Failed(error_text),
            session_ids: session_ids.into_iter().collect(),
            collection_was_gated: collection_was_gated || !collector_ids.is_empty(),
            collector_ids,
        }
    }

    #[cfg(test)]
    pub(crate) fn body(&self) -> String {
        match &self.state {
            CapturedResponseBodyState::Ready(body) => {
                body.materialize_lossy_string().unwrap_or_default()
            }
            CapturedResponseBodyState::Pending
            | CapturedResponseBodyState::Failed(_)
            | CapturedResponseBodyState::Evicted => String::new(),
        }
    }

    pub(crate) fn body_bytes_limited(&self, limit: usize) -> anyhow::Result<Vec<u8>> {
        match &self.state {
            CapturedResponseBodyState::Ready(body) => body.materialize_bytes_limited(limit),
            CapturedResponseBodyState::Pending => {
                anyhow::bail!("No data found for resource with given identifier")
            }
            CapturedResponseBodyState::Failed(_) => {
                anyhow::bail!("No data found for resource with given identifier")
            }
            CapturedResponseBodyState::Evicted => {
                anyhow::bail!("Request content was evicted from inspector cache")
            }
        }
    }

    pub(crate) fn is_visible_to_session(&self, session_id: Option<&str>) -> bool {
        self.session_ids
            .contains(&session_id.map(std::borrow::ToOwned::to_owned))
    }

    pub(crate) fn was_collected_by(&self, collector_id: &str) -> bool {
        self.collector_ids.contains(collector_id)
    }

    pub(crate) fn collector_ids(&self) -> &HashSet<String> {
        &self.collector_ids
    }

    fn collected_network_data_artifact(
        &self,
        request_id: &str,
    ) -> Option<CollectedNetworkDataArtifact> {
        if self.collector_ids.is_empty() {
            return None;
        }
        let CapturedResponseBodyState::Ready(body) = &self.state else {
            return None;
        };
        Some(CollectedNetworkDataArtifact {
            request_id: request_id.to_owned(),
            data_type: DevToolsNetworkDataType::Response,
            body: body.clone(),
            collector_ids: sorted_collector_ids(&self.collector_ids),
            collection_was_gated: self.collection_was_gated,
        })
    }

    pub(crate) fn remove_session_visibility(&mut self, session_id: Option<&str>) -> bool {
        self.session_ids
            .remove(&session_id.map(std::borrow::ToOwned::to_owned));
        !self.session_ids.is_empty()
    }

    fn mark_evicted(&mut self) {
        self.state = CapturedResponseBodyState::Evicted;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedRequestBody {
    body: CapturedBody,
    session_ids: HashSet<Option<String>>,
    collector_ids: HashSet<String>,
    collection_was_gated: bool,
}

impl CapturedRequestBody {
    pub(crate) fn new_with_collector_scope(
        body: Vec<u8>,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) -> Self {
        let collector_ids = collector_ids.into_iter().collect::<HashSet<_>>();
        Self {
            body: CapturedBody::from_bytes(body),
            session_ids: session_ids.into_iter().collect(),
            collection_was_gated: collection_was_gated || !collector_ids.is_empty(),
            collector_ids,
        }
    }

    pub(crate) fn body_bytes_limited(&self, limit: usize) -> anyhow::Result<Vec<u8>> {
        self.body.materialize_bytes_limited(limit)
    }

    pub(crate) fn is_visible_to_session(&self, session_id: Option<&str>) -> bool {
        self.session_ids
            .contains(&session_id.map(std::borrow::ToOwned::to_owned))
    }

    pub(crate) fn was_collected_by(&self, collector_id: &str) -> bool {
        self.collector_ids.contains(collector_id)
    }

    pub(crate) fn collector_ids(&self) -> &HashSet<String> {
        &self.collector_ids
    }

    fn collected_network_data_artifact(
        &self,
        request_id: &str,
    ) -> Option<CollectedNetworkDataArtifact> {
        if self.collector_ids.is_empty() {
            return None;
        }
        Some(CollectedNetworkDataArtifact {
            request_id: request_id.to_owned(),
            data_type: DevToolsNetworkDataType::Request,
            body: self.body.clone(),
            collector_ids: sorted_collector_ids(&self.collector_ids),
            collection_was_gated: self.collection_was_gated,
        })
    }

    pub(crate) fn remove_session_visibility(&mut self, session_id: Option<&str>) -> bool {
        self.session_ids
            .remove(&session_id.map(std::borrow::ToOwned::to_owned));
        !self.session_ids.is_empty()
    }
}

fn sorted_collector_ids(collector_ids: &HashSet<String>) -> Vec<String> {
    let mut collector_ids = collector_ids.iter().cloned().collect::<Vec<_>>();
    collector_ids.sort();
    collector_ids
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CapturedRequestBodyStore {
    bodies: HashMap<String, CapturedRequestBody>,
}

impl CapturedRequestBodyStore {
    pub(crate) fn insert_with_collector_scope(
        &mut self,
        request_id: String,
        body: Vec<u8>,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.bodies.insert(
            request_id,
            CapturedRequestBody::new_with_collector_scope(
                body,
                session_ids,
                collector_ids,
                collection_was_gated,
            ),
        );
    }

    pub(crate) fn get(&self, request_id: &str) -> Option<&CapturedRequestBody> {
        self.bodies.get(request_id)
    }

    fn collected_network_data_artifacts(&self) -> Vec<CollectedNetworkDataArtifact> {
        self.bodies
            .iter()
            .filter_map(|(request_id, body)| body.collected_network_data_artifact(request_id))
            .collect()
    }

    pub(crate) fn clear(&mut self) {
        self.bodies.clear();
    }

    pub(crate) fn remove_session_visibility(&mut self, session_id: Option<&str>) {
        self.bodies
            .retain(|_, body| body.remove_session_visibility(session_id));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedResponseBodyStore {
    /// Terminal entries without retained payloads. Ready entries live only in
    /// `buffered_bodies`, making the bounded buffer the single body owner.
    bodies: HashMap<String, CapturedResponseBody>,
    buffered_bodies: BoundedByteBuffer<String, CapturedResponseBody>,
}

impl Default for CapturedResponseBodyStore {
    fn default() -> Self {
        Self::with_limits(ByteLimits::new(
            RESPONSE_BODY_BUFFER_MAX_TOTAL_BYTES,
            RESPONSE_BODY_BUFFER_MAX_ENTRY_BYTES,
        ))
    }
}

impl CapturedResponseBodyStore {
    fn with_limits(limits: ByteLimits) -> Self {
        Self {
            bodies: HashMap::new(),
            buffered_bodies: BoundedByteBuffer::new(limits),
        }
    }

    #[cfg(test)]
    pub(crate) fn insert(
        &mut self,
        request_id: String,
        body: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.insert_captured_body_with_collector_scope(
            request_id,
            CapturedBody::from_string(body),
            session_ids,
            std::iter::empty::<String>(),
            false,
        );
    }

    pub(crate) fn insert_captured_body_with_collector_scope(
        &mut self,
        request_id: String,
        body: CapturedBody,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        let byte_len = body.len();
        let captured = CapturedResponseBody::from_captured_body_with_collector_scope(
            body,
            session_ids,
            collector_ids,
            collection_was_gated,
        );
        self.bodies.remove(&request_id);
        match self.buffered_bodies.insert(request_id, captured, byte_len) {
            InsertOutcome::Stored { evicted } => {
                for (evicted_request_id, mut evicted_body) in evicted {
                    evicted_body.mark_evicted();
                    self.bodies.insert(evicted_request_id, evicted_body);
                }
            }
            InsertOutcome::Rejected {
                key: request_id,
                value: mut rejected_body,
            } => {
                rejected_body.mark_evicted();
                self.bodies.insert(request_id, rejected_body);
            }
        }
    }

    pub(crate) fn insert_pending_with_collector_scope(
        &mut self,
        request_id: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        if self.bodies.contains_key(&request_id)
            || self.buffered_bodies.contains_key(request_id.as_str())
        {
            return;
        }
        self.bodies.insert(
            request_id,
            CapturedResponseBody::pending_with_collector_scope(
                session_ids,
                collector_ids,
                collection_was_gated,
            ),
        );
    }

    pub(crate) fn insert_failed_with_collector_scope(
        &mut self,
        request_id: String,
        error_text: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.buffered_bodies.remove(&request_id);
        self.bodies.insert(
            request_id,
            CapturedResponseBody::failed_with_collector_scope(
                error_text,
                session_ids,
                collector_ids,
                collection_was_gated,
            ),
        );
    }

    pub(crate) fn get(&self, request_id: &str) -> Option<&CapturedResponseBody> {
        self.buffered_bodies
            .get(request_id)
            .or_else(|| self.bodies.get(request_id))
    }

    fn collected_network_data_artifacts(&self) -> Vec<CollectedNetworkDataArtifact> {
        self.buffered_bodies
            .iter()
            .chain(self.bodies.iter())
            .filter_map(|(request_id, body)| body.collected_network_data_artifact(request_id))
            .collect()
    }

    pub(crate) fn clear(&mut self) {
        self.bodies.clear();
        self.buffered_bodies.clear();
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.bodies.is_empty() && self.buffered_bodies.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, request_id: &str) -> bool {
        self.bodies.contains_key(request_id) || self.buffered_bodies.contains_key(request_id)
    }

    pub(crate) fn remove_session_visibility(&mut self, session_id: Option<&str>) {
        self.bodies
            .retain(|_, body| body.remove_session_visibility(session_id));

        let buffered_request_ids = self
            .buffered_bodies
            .iter()
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in buffered_request_ids {
            let retain = self
                .buffered_bodies
                .get_mut(request_id.as_str())
                .is_some_and(|body| body.remove_session_visibility(session_id));
            if !retain {
                self.buffered_bodies.remove(request_id.as_str());
            }
        }
    }

    #[cfg(test)]
    fn buffered_body_bytes(&self) -> usize {
        self.buffered_bodies.used_bytes()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TargetNetworkArtifacts {
    request_id_allocator: TargetNetworkRequestIdAllocator,
    body_artifacts: TargetNetworkBodyArtifacts,
    subresource_network_artifacts: SubresourceNetworkArtifacts,
    websocket_network_artifacts: WebSocketNetworkArtifacts,
}

impl TargetNetworkArtifacts {
    #[cfg(test)]
    pub(crate) fn allocate_network_request_id(&mut self) -> String {
        self.request_id_allocator.allocate_request_id()
    }

    #[cfg(test)]
    fn set_session_observation_cursor_at_counts(
        &mut self,
        session_id: Option<&str>,
        subresource_record_count: usize,
        websocket_event_count: usize,
    ) {
        self.subresource_network_artifacts
            .set_session_cursor(session_id, subresource_record_count);
        self.websocket_network_artifacts.set_session_cursors(
            session_id,
            subresource_record_count,
            websocket_event_count,
        );
    }

    fn collected_network_data_artifacts(&self) -> Vec<CollectedNetworkDataArtifact> {
        self.body_artifacts.collected_network_data_artifacts()
    }

    #[cfg(test)]
    pub(crate) fn emitted_subresource_record_count_for_session(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.subresource_network_artifacts
            .emitted_record_count_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn emitted_websocket_event_count_for_session(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.websocket_network_artifacts
            .emitted_event_count_for_session(session_id)
    }

    #[cfg(test)]
    fn request_id_for_websocket_socket(
        &mut self,
        socket_id: u64,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> String {
        if let Some(request_id) = self.websocket_request_id_for_socket(socket_id) {
            return request_id.to_owned();
        }
        let request_id = request_id_allocator.allocate_request_id();
        self.record_websocket_request_id_for_socket_if_absent(socket_id, request_id.clone());
        request_id
    }

    #[cfg(test)]
    fn websocket_request_id_for_socket(&self, socket_id: u64) -> Option<&str> {
        self.websocket_network_artifacts
            .request_id_for_socket(socket_id)
    }

    #[cfg(test)]
    fn record_websocket_request_id_for_socket_if_absent(
        &mut self,
        socket_id: u64,
        request_id: String,
    ) {
        self.websocket_network_artifacts
            .set_request_id_for_socket_if_absent(socket_id, request_id);
    }

    pub(crate) fn clear_session_scoped_observation_artifacts(&mut self) {
        self.body_artifacts.clear_session_scoped();
        self.subresource_network_artifacts.clear_request_ids();
        self.websocket_network_artifacts.clear_all();
    }

    pub(crate) fn reset_all_target_scoped_artifacts(&mut self) {
        self.request_id_allocator.reset();
        self.body_artifacts.reset_all();
        self.subresource_network_artifacts.reset_cursor();
        self.subresource_network_artifacts.clear_request_ids();
        self.websocket_network_artifacts.clear_all();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TargetNetworkRequestIdAllocator {
    next_sequence: u64,
}

impl TargetNetworkRequestIdAllocator {
    #[cfg(test)]
    pub(crate) fn allocate_sequence(&mut self) -> u64 {
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("network request id sequence exhausted");
        self.next_sequence
    }

    #[cfg(test)]
    pub(crate) fn allocate_request_id(&mut self) -> String {
        format!("REQ-{}", self.allocate_sequence())
    }

    pub(crate) fn reset(&mut self) {
        self.next_sequence = 0;
    }

    #[cfg(test)]
    pub(crate) fn set_next_sequence_for_test(&mut self, sequence: u64) {
        self.next_sequence = sequence;
    }

    #[cfg(test)]
    pub(crate) fn next_sequence_for_test(&self) -> u64 {
        self.next_sequence
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TargetNetworkBodyArtifacts {
    captured_request_bodies: CapturedRequestBodyStore,
    captured_response_bodies: CapturedResponseBodyStore,
    io_stream_artifacts: TargetIoStreamArtifacts,
}

impl TargetNetworkBodyArtifacts {
    #[cfg(test)]
    pub(crate) fn insert(
        &mut self,
        request_id: String,
        response_body: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.captured_response_bodies
            .insert(request_id, response_body, session_ids);
    }

    pub(crate) fn insert_captured_body_with_collector_scope(
        &mut self,
        request_id: String,
        response_body: CapturedBody,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.captured_response_bodies
            .insert_captured_body_with_collector_scope(
                request_id,
                response_body,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn insert_request_body_with_collector_scope(
        &mut self,
        request_id: String,
        request_body: Vec<u8>,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.captured_request_bodies.insert_with_collector_scope(
            request_id,
            request_body,
            session_ids,
            collector_ids,
            collection_was_gated,
        );
    }

    pub(crate) fn insert_pending_with_collector_scope(
        &mut self,
        request_id: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.captured_response_bodies
            .insert_pending_with_collector_scope(
                request_id,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn insert_failed_with_collector_scope(
        &mut self,
        request_id: String,
        error_text: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.captured_response_bodies
            .insert_failed_with_collector_scope(
                request_id,
                error_text,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn captured_response_body(&self, request_id: &str) -> Option<&CapturedResponseBody> {
        self.captured_response_bodies.get(request_id)
    }

    pub(crate) fn captured_request_body(&self, request_id: &str) -> Option<&CapturedRequestBody> {
        self.captured_request_bodies.get(request_id)
    }

    pub(crate) fn collected_network_data_artifacts(&self) -> Vec<CollectedNetworkDataArtifact> {
        let mut artifacts = self
            .captured_request_bodies
            .collected_network_data_artifacts();
        artifacts.extend(
            self.captured_response_bodies
                .collected_network_data_artifacts(),
        );
        artifacts
    }

    pub(crate) fn clear_captured_response_bodies(&mut self) {
        self.captured_response_bodies.clear();
    }

    pub(crate) fn remove_session_visibility(&mut self, session_id: Option<&str>) {
        self.captured_request_bodies
            .remove_session_visibility(session_id);
        self.captured_response_bodies
            .remove_session_visibility(session_id);
    }

    pub(crate) fn allocate_io_stream_handle(&mut self) -> String {
        self.io_stream_artifacts.allocate_handle()
    }

    pub(crate) fn insert_stream(&mut self, handle: String, bytes: Vec<u8>, offset: usize) {
        self.io_stream_artifacts
            .insert_stream(handle, bytes, offset);
    }

    pub(crate) fn insert_stream_body_source(
        &mut self,
        handle: String,
        body: CapturedBody,
        offset: usize,
    ) {
        self.io_stream_artifacts
            .insert_stream_body_source(handle, body, offset);
    }

    pub(crate) fn read_io_stream(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> Option<TargetIoStreamRead> {
        self.io_stream_artifacts.read(handle, offset, size)
    }

    pub(crate) fn close_io_stream(&mut self, handle: &str) -> bool {
        self.io_stream_artifacts.remove(handle).is_some()
    }

    pub(crate) fn clear_session_scoped(&mut self) {
        self.captured_request_bodies.clear();
        self.captured_response_bodies.clear();
        self.io_stream_artifacts.clear_streams();
    }

    pub(crate) fn reset_all(&mut self) {
        self.captured_request_bodies.clear();
        self.captured_response_bodies.clear();
        self.io_stream_artifacts.reset_all();
    }

    #[cfg(test)]
    pub(crate) fn contains_body_key(&self, request_id: &str) -> bool {
        self.captured_response_bodies.contains_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn captured_response_bodies_empty(&self) -> bool {
        self.captured_response_bodies.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn io_streams_empty(&self) -> bool {
        self.io_stream_artifacts.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_next_stream_id_for_test(&mut self, next_stream_id: u64) {
        self.io_stream_artifacts
            .set_next_stream_id_for_test(next_stream_id);
    }

    #[cfg(test)]
    pub(crate) fn next_stream_id_for_test(&self) -> u64 {
        self.io_stream_artifacts.next_stream_id_for_test()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SubresourceNetworkArtifacts {
    emitted_record_counts_by_session: HashMap<Option<String>, usize>,
    request_ids_by_handle: HashMap<SubresourceNetworkRequestHandle, String>,
    completed_request_ids: HashSet<String>,
    fetch_pause_announced_request_ids: HashSet<String>,
}

impl SubresourceNetworkArtifacts {
    #[cfg(test)]
    pub(crate) fn emitted_record_count(&self) -> usize {
        self.emitted_record_counts_by_session
            .values()
            .copied()
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn emitted_record_count_for_session(&self, session_id: Option<&str>) -> usize {
        self.emitted_record_counts_by_session
            .get(&session_id.map(str::to_owned))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn mark_emitted(
        &mut self,
        session_id: Option<&str>,
        start_index: usize,
        record_count: usize,
    ) {
        let cursor = start_index
            .checked_add(record_count)
            .expect("subresource emitted record cursor exhausted");
        self.set_session_cursor(session_id, cursor);
    }

    pub(crate) fn reset_cursor(&mut self) {
        self.emitted_record_counts_by_session.clear();
    }

    pub(crate) fn request_id_for_handle(
        &self,
        handle: SubresourceNetworkRequestHandle,
    ) -> Option<&str> {
        self.request_ids_by_handle.get(&handle).map(String::as_str)
    }

    fn request_handle_for_request_id(
        &self,
        request_id: &str,
    ) -> Option<SubresourceNetworkRequestHandle> {
        self.request_ids_by_handle
            .iter()
            .find_map(|(&handle, mapped_request_id)| {
                (mapped_request_id == request_id).then_some(handle)
            })
    }

    pub(crate) fn set_request_id_for_handle_if_absent(
        &mut self,
        handle: SubresourceNetworkRequestHandle,
        request_id: String,
    ) {
        self.request_ids_by_handle
            .entry(handle)
            .or_insert(request_id);
    }

    fn record_fetch_pause_announced_request_id(&mut self, request_id: String) {
        self.fetch_pause_announced_request_ids.insert(request_id);
    }

    fn take_fetch_pause_announced_request_id(&mut self, request_id: &str) -> bool {
        self.fetch_pause_announced_request_ids.remove(request_id)
    }

    fn clear_fetch_pause_announced_request_id(&mut self, request_id: &str) {
        self.fetch_pause_announced_request_ids.remove(request_id);
    }

    pub(crate) fn claim_completed_request_id(&mut self, request_id: &str) -> bool {
        self.completed_request_ids.insert(request_id.to_owned())
    }

    pub(crate) fn clear_request_ids(&mut self) {
        self.request_ids_by_handle.clear();
        self.completed_request_ids.clear();
        self.fetch_pause_announced_request_ids.clear();
    }

    pub(crate) fn set_session_cursor(&mut self, session_id: Option<&str>, record_count: usize) {
        self.emitted_record_counts_by_session
            .insert(session_id.map(str::to_owned), record_count);
    }

    pub(crate) fn remove_session_cursor(&mut self, session_id: Option<&str>) {
        self.emitted_record_counts_by_session
            .remove(&session_id.map(str::to_owned));
    }

    #[cfg(test)]
    pub(crate) fn set_emitted_record_count_for_test(&mut self, count: usize) {
        self.set_session_cursor(None, count);
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        ScriptNetworkOutputItem, SubresourceNetworkRecord, SubresourceNetworkRequestHandle,
        SubresourceRequestInitiatorType, SubresourceRequestStarted, SubresourceResourceType,
        WebSocketFrameDirection, WebSocketFrameOpcode, WebSocketNetworkEvent,
    };
    use url::Url;

    use super::super::output_queue::{
        PendingSubresourceNetworkActivity, PendingSubresourceNetworkActivitySession,
        PendingWebSocketNetworkActivity, PendingWebSocketNetworkActivitySession,
    };
    use super::{
        CapturedResponseBodyStore, NetworkBacklogPreferredRequestId, NetworkBacklogRequestIdPlan,
        TargetNetworkAgentState, WebSocketNetworkArtifacts,
    };
    use crate::conn::{CapturedBody, CapturedBodyWriter, ConnectionNetworkRequestIdAllocator};

    #[test]
    fn response_body_store_defaults_to_one_tenth_of_chromium_desktop_budget() {
        let store = CapturedResponseBodyStore::default();

        assert_eq!(
            store.buffered_bodies.limits(),
            moli_bounded_buffer::ByteLimits::new(20_000_000, 2_000_000)
        );
    }

    #[test]
    fn response_body_store_marks_oldest_payload_evicted_when_total_budget_fills() {
        let mut store =
            CapturedResponseBodyStore::with_limits(moli_bounded_buffer::ByteLimits::new(5, 4));
        store.insert_captured_body_with_collector_scope(
            "REQ-first".to_owned(),
            CapturedBody::from_string("aa".to_owned()),
            [None],
            std::iter::empty::<String>(),
            false,
        );
        store.insert_captured_body_with_collector_scope(
            "REQ-second".to_owned(),
            CapturedBody::from_string("bb".to_owned()),
            [None],
            std::iter::empty::<String>(),
            false,
        );
        store.insert_captured_body_with_collector_scope(
            "REQ-third".to_owned(),
            CapturedBody::from_string("ccc".to_owned()),
            [None],
            std::iter::empty::<String>(),
            false,
        );

        assert_eq!(store.buffered_body_bytes(), 5);
        assert_eq!(
            store
                .get("REQ-first")
                .expect("evicted metadata must remain addressable")
                .body_bytes_limited(10)
                .expect_err("oldest payload should be evicted")
                .to_string(),
            "Request content was evicted from inspector cache"
        );
        assert_eq!(
            store
                .get("REQ-second")
                .expect("second response should remain")
                .body_bytes_limited(10)
                .expect("second response body should remain readable"),
            b"bb"
        );
        assert_eq!(
            store
                .get("REQ-third")
                .expect("third response should remain")
                .body_bytes_limited(10)
                .expect("third response body should remain readable"),
            b"ccc"
        );
    }

    #[test]
    fn response_body_store_returns_byte_charge_on_state_and_visibility_removal() {
        let mut store =
            CapturedResponseBodyStore::with_limits(moli_bounded_buffer::ByteLimits::new(8, 4));
        store.insert_captured_body_with_collector_scope(
            "REQ-failed".to_owned(),
            CapturedBody::from_string("body".to_owned()),
            [None],
            std::iter::empty::<String>(),
            false,
        );
        assert_eq!(store.buffered_body_bytes(), 4);
        store.insert_failed_with_collector_scope(
            "REQ-failed".to_owned(),
            "network failed".to_owned(),
            [None],
            std::iter::empty::<String>(),
            false,
        );
        assert_eq!(store.buffered_body_bytes(), 0);

        store.insert_captured_body_with_collector_scope(
            "REQ-session".to_owned(),
            CapturedBody::from_string("abc".to_owned()),
            [Some("SID-1".to_owned())],
            std::iter::empty::<String>(),
            false,
        );
        assert_eq!(store.buffered_body_bytes(), 3);
        store.remove_session_visibility(Some("SID-1"));
        assert_eq!(store.buffered_body_bytes(), 0);
        assert!(store.get("REQ-session").is_none());
    }

    fn subresource_record(url: &str) -> SubresourceNetworkRecord {
        let url = Url::parse(url).expect("test URL should parse");
        SubresourceNetworkRecord::success(
            None,
            Url::parse("https://example.com/").expect("document URL should parse"),
            url.clone(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Script,
            None,
            Vec::new(),
            url,
            200,
            Vec::new(),
            String::new(),
            Vec::new(),
        )
    }

    fn websocket_record(url: &str, socket_id: u64) -> SubresourceNetworkRecord {
        let url = Url::parse(url).expect("test URL should parse");
        SubresourceNetworkRecord::success(
            None,
            Url::parse("https://example.com/").expect("document URL should parse"),
            url.clone(),
            "GET".to_owned(),
            vec![("Sec-WebSocket-Version".to_owned(), "13".to_owned())],
            None,
            SubresourceResourceType::WebSocket,
            None,
            Vec::new(),
            url,
            101,
            vec![("Upgrade".to_owned(), "websocket".to_owned())],
            String::new(),
            Vec::new(),
        )
        .with_websocket_socket_id(socket_id)
    }

    fn websocket_event(socket_id: u64, payload_length: usize) -> WebSocketNetworkEvent {
        WebSocketNetworkEvent::new(
            socket_id,
            Url::parse("https://example.com/").expect("document URL should parse"),
            Url::parse("wss://example.com/socket").expect("websocket URL should parse"),
            WebSocketFrameDirection::Received,
            WebSocketFrameOpcode::Text,
            payload_length,
        )
    }

    fn network_output_items(
        records: &[SubresourceNetworkRecord],
        events: &[WebSocketNetworkEvent],
    ) -> Vec<ScriptNetworkOutputItem> {
        records
            .iter()
            .cloned()
            .map(|record| ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)))
            .chain(
                events
                    .iter()
                    .cloned()
                    .map(ScriptNetworkOutputItem::WebSocketNetworkEvent),
            )
            .collect()
    }

    fn ingest_network_output(
        agent: &mut TargetNetworkAgentState,
        records: &[SubresourceNetworkRecord],
        events: &[WebSocketNetworkEvent],
    ) {
        for item in network_output_items(records, events) {
            agent.ingest_renderer_output_item(&item, "LOADER-1");
        }
    }

    #[test]
    fn handled_terminal_network_record_retires_staged_request_from_idle_accounting() {
        let mut agent = TargetNetworkAgentState::default();
        let handle = SubresourceNetworkRequestHandle::new(17);
        let document_url = Url::parse("https://example.com/").expect("document URL should parse");
        let request_url =
            Url::parse("https://example.com/late-xhr").expect("request URL should parse");
        let started = ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(
            SubresourceRequestStarted::new(
                handle,
                None,
                document_url,
                request_url,
                "GET".to_owned(),
                Vec::new(),
                None,
                SubresourceResourceType::Xhr,
                SubresourceRequestInitiatorType::Script,
                None,
            ),
        ));
        agent.ingest_renderer_output_item(&started, "LOADER-1");
        assert!(!agent.renderer_subresources_are_idle());

        let terminal = ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            subresource_record("https://example.com/late-xhr").with_request_handle(handle),
        ));
        agent.ingest_renderer_output_item(&terminal, "LOADER-1");

        assert!(
            agent.renderer_subresources_are_idle(),
            "a handled all-in-one record is terminal for the staged request with the same handle"
        );
    }

    #[test]
    fn retiring_keepalive_does_not_require_a_document_bound_terminal() {
        fn started(
            handle: SubresourceNetworkRequestHandle,
            keepalive: bool,
        ) -> ScriptNetworkOutputItem {
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(
                SubresourceRequestStarted::new(
                    handle,
                    None,
                    Url::parse("https://example.com/").expect("document URL should parse"),
                    Url::parse("https://example.com/collect").expect("request URL should parse"),
                    "POST".to_owned(),
                    Vec::new(),
                    None,
                    SubresourceResourceType::Fetch,
                    SubresourceRequestInitiatorType::Script,
                    None,
                )
                .with_keepalive(keepalive),
            ))
        }

        let mut keepalive_agent = TargetNetworkAgentState::default();
        keepalive_agent.ingest_renderer_output_item(
            &started(SubresourceNetworkRequestHandle::new(18), true),
            "LOADER-1",
        );
        assert!(
            !keepalive_agent.renderer_subresources_are_idle(),
            "a keepalive remains current-document activity until replacement"
        );
        let retiring_keepalive = keepalive_agent.rotate_document_for_replacement();
        assert!(
            retiring_keepalive
                .unterminated_document_bound_request_diagnostics()
                .is_empty(),
            "a detached keepalive may outlive its Document without a CDP terminal"
        );

        let mut ordinary_agent = TargetNetworkAgentState::default();
        ordinary_agent.ingest_renderer_output_item(
            &started(SubresourceNetworkRequestHandle::new(19), false),
            "LOADER-2",
        );
        let retiring_ordinary = ordinary_agent.rotate_document_for_replacement();
        assert_eq!(
            retiring_ordinary.unterminated_document_bound_request_diagnostics(),
            vec![(19, None)],
            "an ordinary request still requires a terminal before its renderer stream closes"
        );
    }

    fn pending_combined_snapshot(
        agent: &mut TargetNetworkAgentState,
        preferred_request_id: Option<&str>,
    ) -> super::PendingNetworkBacklogDeliverySnapshot {
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        pending_combined_snapshot_with_allocator(
            agent,
            preferred_request_id,
            &mut request_id_allocator,
        )
    }

    fn pending_combined_snapshot_with_allocator(
        agent: &mut TargetNetworkAgentState,
        preferred_request_id: Option<&str>,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> super::PendingNetworkBacklogDeliverySnapshot {
        agent
            .pending_network_backlog_delivery_snapshot(
                None,
                None,
                preferred_request_id.map(NetworkBacklogPreferredRequestId::contextual_subresource),
                request_id_allocator,
            )
            .expect("combined network backlog snapshot should be visible")
    }

    fn contextual_preferred_request_id(request_id: &str) -> NetworkBacklogPreferredRequestId<'_> {
        NetworkBacklogPreferredRequestId::contextual_subresource(request_id)
    }

    fn subresource_request_ids(
        snapshot: &super::PendingNetworkBacklogDeliverySnapshot,
    ) -> Vec<String> {
        snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| {
                item.as_subresource()
                    .map(|output| output.request_id().to_owned())
            })
            .collect()
    }

    fn subresource_urls(snapshot: &super::PendingNetworkBacklogDeliverySnapshot) -> Vec<String> {
        snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| {
                item.as_subresource()
                    .map(|output| output.metadata().url().as_str().to_owned())
            })
            .collect()
    }

    fn websocket_request_ids(
        snapshot: &super::PendingNetworkBacklogDeliverySnapshot,
    ) -> Vec<String> {
        snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| {
                item.as_websocket().map(|output| {
                    output
                        .as_handshake()
                        .map(|output| output.request_id())
                        .or_else(|| output.as_frame().map(|output| output.request_id()))
                        .expect("test WebSocket output should carry a request id")
                        .to_owned()
                })
            })
            .collect()
    }

    fn websocket_handshake_request_ids(
        snapshot: &super::PendingNetworkBacklogDeliverySnapshot,
    ) -> Vec<String> {
        snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| {
                item.as_websocket()
                    .and_then(|output| output.as_handshake())
                    .map(|output| output.request_id().to_owned())
            })
            .collect()
    }

    fn websocket_frame_outputs(
        snapshot: &super::PendingNetworkBacklogDeliverySnapshot,
    ) -> Vec<(String, usize)> {
        snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| {
                item.as_websocket()
                    .and_then(|output| output.as_frame())
                    .map(|output| (output.request_id().to_owned(), output.payload_length()))
            })
            .collect()
    }

    #[test]
    fn body_artifacts_clear_response_bodies_and_io_streams_together() {
        let mut agent = TargetNetworkAgentState::default();

        agent.record_captured_response_body("REQ-1".to_owned(), "response body".to_owned(), [None]);
        agent.insert_io_stream("STREAM-1".to_owned(), b"stream body".to_vec(), 0);
        assert!(agent.has_captured_response_body("REQ-1"));
        assert!(!agent.io_streams_empty());

        agent.clear_session_scoped_observation_artifacts();
        assert!(!agent.has_captured_response_body("REQ-1"));
        assert!(agent.io_streams_empty());

        agent.record_captured_response_body("REQ-2".to_owned(), "response body".to_owned(), [None]);
        agent.insert_io_stream("STREAM-2".to_owned(), b"stream body".to_vec(), 0);
        assert!(agent.has_captured_response_body("REQ-2"));
        assert!(!agent.io_streams_empty());

        agent.reset_all_target_scoped_artifacts();
        assert!(!agent.has_captured_response_body("REQ-2"));
        assert!(agent.io_streams_empty());
    }

    #[test]
    fn output_queue_reset_clears_page_local_subresource_request_handle_bindings() {
        let mut agent = TargetNetworkAgentState::default();
        let handle = SubresourceNetworkRequestHandle::new(5);

        agent.record_subresource_request_id_for_handle_if_absent(handle, "REQ-old".to_owned());
        agent.record_subresource_request_id_for_handle_if_absent(handle, "REQ-stale".to_owned());
        assert_eq!(
            agent
                .artifacts
                .subresource_network_artifacts
                .request_id_for_handle(handle),
            Some("REQ-old")
        );

        agent.reset_output_queue();
        agent.record_subresource_request_id_for_handle_if_absent(handle, "REQ-new".to_owned());
        assert_eq!(
            agent
                .artifacts
                .subresource_network_artifacts
                .request_id_for_handle(handle),
            Some("REQ-new")
        );
    }

    #[test]
    fn contextual_preferred_request_id_skips_earlier_unrelated_records() {
        let mut agent = TargetNetworkAgentState::default();
        let contextual_handle = SubresourceNetworkRequestHandle::new(7);
        agent.enable_primary_events();
        agent.record_subresource_request_id_for_handle_if_absent(
            contextual_handle,
            "REQ-contextual".to_owned(),
        );
        let records = [
            subresource_record("https://example.com/earlier.js"),
            subresource_record("https://example.com/contextual.js")
                .with_request_handle(contextual_handle),
        ];
        ingest_network_output(&mut agent, &records, &[]);

        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let snapshot = pending_combined_snapshot_with_allocator(
            &mut agent,
            Some("REQ-contextual"),
            &mut request_id_allocator,
        );

        assert_eq!(
            subresource_request_ids(&snapshot),
            vec!["REQ-1", "REQ-contextual"],
            "a contextual id already bound to a request handle must not be consumed by an earlier unrelated backlog record"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            1,
            "only the unrelated record should allocate a new request id"
        );
    }

    #[test]
    fn body_artifacts_own_io_stream_read_offsets_and_close() {
        let mut agent = TargetNetworkAgentState::default();
        agent.insert_io_stream("STREAM-1".to_owned(), b"abcdef".to_vec(), 0);

        let first = agent
            .read_io_stream("STREAM-1", None, Some(2))
            .expect("stream should exist");
        assert_eq!(first.bytes, b"ab");
        assert!(!first.eof);

        let second = agent
            .read_io_stream("STREAM-1", Some(4), None)
            .expect("stream should exist");
        assert_eq!(second.bytes, b"ef");
        assert!(second.eof);

        assert!(agent.close_io_stream("STREAM-1"));
        assert!(agent.read_io_stream("STREAM-1", None, None).is_none());
        assert!(!agent.close_io_stream("STREAM-1"));
    }

    #[test]
    fn body_artifacts_read_io_stream_from_captured_body_source() {
        let mut writer = CapturedBodyWriter::new(4);
        writer
            .append(b"captured body source")
            .expect("captured body writer should accept bytes");
        let body = writer
            .finish()
            .expect("captured body writer should finish source");
        let mut agent = TargetNetworkAgentState::default();

        agent.insert_io_stream_body_source("STREAM-source".to_owned(), body, 0);

        let first = agent
            .read_io_stream("STREAM-source", None, Some(8))
            .expect("source-backed stream should exist");
        assert_eq!(first.bytes, b"captured");
        assert!(!first.eof);

        let second = agent
            .read_io_stream("STREAM-source", None, None)
            .expect("source-backed stream should remain readable");
        assert_eq!(second.bytes, b" body source");
        assert!(second.eof);
    }

    #[test]
    fn websocket_request_ids_are_owned_by_network_agent() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();

        agent.record_websocket_request_id_for_socket_if_absent(7, "REQ-bound".to_owned());
        assert_eq!(
            agent.websocket_request_id_for_socket_or_allocate_with(7, &mut request_id_allocator),
            "REQ-bound"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            0,
            "bound request id should not consume a new network id"
        );

        let allocated =
            agent.websocket_request_id_for_socket_or_allocate_with(8, &mut request_id_allocator);
        assert_eq!(allocated, "REQ-1");
        assert_eq!(
            agent.websocket_request_id_for_socket_or_allocate_with(8, &mut request_id_allocator),
            "REQ-1"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            1,
            "subsequent lookups should reuse the socket mapping"
        );
    }

    #[test]
    fn synthetic_websocket_request_registration_updates_reverse_socket_mapping() {
        let mut agent = TargetNetworkAgentState::default();

        agent.register_synthetic_websocket_request("FETCH-1".to_owned(), "REQ-first".to_owned(), 7);
        agent.register_synthetic_websocket_request(
            "FETCH-2".to_owned(),
            "REQ-second".to_owned(),
            7,
        );

        assert_eq!(
            agent.synthetic_websocket_socket_id_for_request("FETCH-1"),
            Some(7)
        );
        assert_eq!(
            agent.synthetic_websocket_socket_id_for_request("FETCH-2"),
            Some(7)
        );
        assert_eq!(
            agent.websocket_request_id_for_socket_or_allocate(7),
            "REQ-second",
            "re-registering a synthetic socket must keep the canonical reverse map in sync"
        );
    }

    #[test]
    fn auxiliary_network_event_sessions_follow_enable_insertion_order() {
        let mut agent = TargetNetworkAgentState::default();

        agent.enable_auxiliary_events("SID-z");
        agent.enable_auxiliary_events("SID-a");
        agent.enable_auxiliary_events("SID-z");

        assert_eq!(
            agent.event_session_ids(Some("SID-primary"), Some("SID-primary")),
            vec![Some("SID-z".to_owned()), Some("SID-a".to_owned())],
            "auxiliary Network listeners should not be sorted by session id"
        );

        agent.enable_primary_events();
        assert_eq!(
            agent.event_session_ids(Some("SID-primary"), Some("SID-primary")),
            vec![
                Some("SID-primary".to_owned()),
                Some("SID-z".to_owned()),
                Some("SID-a".to_owned()),
            ],
            "primary listener remains first, followed by auxiliary enable order"
        );

        assert!(agent.disable_auxiliary_events("SID-z"));
        agent.enable_auxiliary_events("SID-z");
        assert_eq!(
            agent.event_session_ids(Some("SID-primary"), Some("SID-primary")),
            vec![
                Some("SID-primary".to_owned()),
                Some("SID-a".to_owned()),
                Some("SID-z".to_owned()),
            ],
            "disable then re-enable should append the auxiliary session"
        );
    }

    #[test]
    fn consecutive_renderer_live_tokens_own_disjoint_ingress_ranges() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        agent.enable_primary_events();

        let first = ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            subresource_record("https://example.com/first.js"),
        ));
        let second = ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            subresource_record("https://example.com/second.js"),
        ));
        let mut first_delivery = agent.ingest_renderer_output_item_and_prepare_live_delivery(
            &first,
            "LOADER-1",
            None,
            None,
            None,
            &mut request_id_allocator,
        );
        let mut second_delivery = agent.ingest_renderer_output_item_and_prepare_live_delivery(
            &second,
            "LOADER-1",
            None,
            None,
            None,
            &mut request_id_allocator,
        );

        let first_snapshot = agent
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut first_delivery)
            .expect("the first renderer fact should own one live delivery range");
        assert_eq!(
            subresource_urls(&first_snapshot),
            vec!["https://example.com/first.js"]
        );
        agent.mark_network_backlog_delivery_snapshot_emitted(&first_snapshot);

        let second_snapshot = agent
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut second_delivery)
            .expect("the second renderer fact should own one live delivery range");
        assert_eq!(
            subresource_urls(&second_snapshot),
            vec!["https://example.com/second.js"],
            "a later ingress token must not rediscover an earlier unprojected fact from the session cursor"
        );
        agent.mark_network_backlog_delivery_snapshot_emitted(&second_snapshot);
        assert_eq!(
            agent
                .artifacts
                .emitted_subresource_record_count_for_session(None),
            2
        );
    }

    #[test]
    fn websocket_observation_cursors_and_request_ids_are_separate_artifacts() {
        let mut artifacts = WebSocketNetworkArtifacts::default();
        artifacts.set_session_cursors(None, 3, 5);
        artifacts.set_request_id_for_socket_if_absent(7, "REQ-bound".to_owned());

        artifacts.clear_request_ids();

        assert_eq!(
            artifacts.emitted_record_count_for_session(None),
            3,
            "clearing WebSocket request ids must not reset record observation cursors"
        );
        assert_eq!(
            artifacts.emitted_event_count_for_session(None),
            5,
            "clearing WebSocket request ids must not reset frame observation cursors"
        );
        assert_eq!(artifacts.request_id_for_socket(7), None);

        artifacts.clear_all();
        assert_eq!(artifacts.emitted_record_count_for_session(None), 0);
        assert_eq!(artifacts.emitted_event_count_for_session(None), 0);
    }

    #[test]
    fn subresource_delivery_snapshot_binds_websocket_request_id() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        agent.enable_primary_events();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        ingest_network_output(&mut agent, &records, &[]);
        let snapshot = pending_combined_snapshot_with_allocator(
            &mut agent,
            Some("REQ-from-subresource"),
            &mut request_id_allocator,
        );

        assert_eq!(
            subresource_request_ids(&snapshot),
            vec!["REQ-from-subresource"]
        );
        assert_eq!(
            agent.websocket_request_id_for_socket_or_allocate(7),
            "REQ-from-subresource"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            0,
            "subresource output binding should not allocate a second request id"
        );
    }

    #[test]
    fn prepared_backlog_drives_delivery_snapshots_without_recomputing_ranges() {
        let mut agent = TargetNetworkAgentState::default();
        agent.enable_primary_events();
        let records = vec![
            subresource_record("https://example.com/app.js"),
            websocket_record("wss://example.com/socket", 7),
        ];
        let events = [websocket_event(7, 12)];
        ingest_network_output(&mut agent, &records, &events);

        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let mut backlog = agent.backlog_prepared_delivery(
            None,
            None,
            Some(contextual_preferred_request_id("REQ-main")),
            &mut request_id_allocator,
        );
        assert!(
            backlog.has_output(),
            "prepared backlog should retain one visible Network output for typed backlog items"
        );

        let snapshot = agent
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("prepared combined backlog should build a delivery snapshot");
        assert_eq!(snapshot.outputs().len(), 4);
        assert_eq!(
            subresource_request_ids(&snapshot),
            vec!["REQ-main", "REQ-1"],
            "WebSocket subresource metadata should bind a stable owner request id while consuming prepared output"
        );
        assert_eq!(
            websocket_request_ids(&snapshot),
            vec!["REQ-1", "REQ-1"],
            "prepared WebSocket delivery should reuse the request id bound by the subresource snapshot"
        );
        assert!(
            !backlog.has_output(),
            "prepared backlog outputs should be one-shot delivery inputs"
        );
    }

    #[test]
    fn prepared_backlog_drives_combined_network_delivery_snapshot() {
        let mut agent = TargetNetworkAgentState::default();
        agent.enable_primary_events();
        let script_record = subresource_record("https://example.com/app.js");
        let websocket_record = websocket_record("wss://example.com/socket", 7);
        let fetch_record = subresource_record("https://example.com/api");
        let events = [websocket_event(7, 12)];
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(script_record)),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record)),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(events[0].clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(fetch_record)),
        ];
        for item in &items {
            agent.ingest_renderer_output_item(item, "LOADER-1");
        }

        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let mut backlog = agent.backlog_prepared_delivery(
            None,
            None,
            Some(contextual_preferred_request_id("REQ-main")),
            &mut request_id_allocator,
        );
        let snapshot = agent
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("prepared network backlog should build one combined delivery snapshot");

        assert_eq!(snapshot.outputs().len(), 5);
        let output_order = snapshot
            .outputs()
            .into_iter()
            .map(|item| {
                if let Some(output) = item.as_subresource() {
                    return format!("sub:{}", output.request_id());
                }
                let output = item
                    .as_websocket()
                    .expect("combined backlog item should be subresource or WebSocket");
                if let Some(output) = output.as_handshake() {
                    format!("ws-handshake:{}", output.request_id())
                } else if let Some(output) = output.as_frame() {
                    format!("ws-frame:{}", output.request_id())
                } else {
                    unreachable!("test WebSocket item should be handshake or frame")
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            output_order,
            vec![
                "sub:REQ-main",
                "sub:REQ-1",
                "ws-handshake:REQ-1",
                "ws-frame:REQ-1",
                "sub:REQ-2",
            ],
            "combined backlog snapshot should preserve producer delivery order across families"
        );
        let subresource_request_ids = snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| item.as_subresource().map(|output| output.request_id()))
            .collect::<Vec<_>>();
        assert_eq!(
            subresource_request_ids,
            vec!["REQ-main", "REQ-1", "REQ-2"],
            "WebSocket metadata should bind a request id before WebSocket events are materialized"
        );
        let websocket_request_ids = snapshot
            .outputs()
            .into_iter()
            .filter_map(|item| {
                item.as_websocket().map(|output| {
                    output
                        .as_handshake()
                        .map(|output| output.request_id())
                        .or_else(|| output.as_frame().map(|output| output.request_id()))
                        .expect("test WebSocket item should carry a request id")
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            websocket_request_ids,
            vec!["REQ-1", "REQ-1"],
            "WebSocket delivery should reuse the request id bound by subresource metadata"
        );
        assert!(
            !backlog.has_output(),
            "combined delivery snapshots should consume every prepared backlog family"
        );

        agent.mark_network_backlog_delivery_snapshot_emitted(&snapshot);
        assert!(
            !agent
                .backlog_prepared_delivery(
                    None,
                    None,
                    None,
                    &mut ConnectionNetworkRequestIdAllocator::default()
                )
                .has_output(),
            "combined mark-emitted should advance subresource and WebSocket cursors together"
        );
    }

    #[test]
    fn prepared_backlog_preserves_preferred_request_id_after_bound_websocket_metadata() {
        let mut agent = TargetNetworkAgentState::default();
        agent.enable_primary_events();
        agent.register_synthetic_websocket_request(
            "FETCH-ws".to_owned(),
            "REQ-synthetic-ws".to_owned(),
            7,
        );
        let websocket_record = websocket_record("wss://example.com/socket", 7);
        let api_record = subresource_record("https://example.com/api");
        let events = [websocket_event(7, 12)];
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record)),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(events[0].clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(api_record)),
        ];
        for item in &items {
            agent.ingest_renderer_output_item(item, "LOADER-1");
        }

        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let mut backlog = agent.backlog_prepared_delivery(
            None,
            None,
            Some(contextual_preferred_request_id("REQ-main")),
            &mut request_id_allocator,
        );
        let snapshot = agent
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut backlog)
            .expect("prepared network backlog should build one combined delivery snapshot");
        let output_order = snapshot
            .outputs()
            .into_iter()
            .map(|item| {
                if let Some(output) = item.as_subresource() {
                    return format!("sub:{}", output.request_id());
                }
                let output = item
                    .as_websocket()
                    .expect("combined backlog item should be subresource or WebSocket");
                if let Some(output) = output.as_handshake() {
                    format!("ws-handshake:{}", output.request_id())
                } else if let Some(output) = output.as_frame() {
                    format!("ws-frame:{}", output.request_id())
                } else {
                    unreachable!("test WebSocket item should be handshake or frame")
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(
            output_order,
            vec![
                "sub:REQ-synthetic-ws",
                "ws-handshake:REQ-synthetic-ws",
                "ws-frame:REQ-synthetic-ws",
                "sub:REQ-main",
            ],
            "resolving already-bound WebSocket metadata must not consume the preferred id needed by the next ordinary subresource"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            0,
            "synthetic WebSocket binding and preferred subresource id should not allocate fallback request ids"
        );
    }

    #[test]
    fn merged_prepared_backlog_preserves_preferred_request_id_after_bound_websocket_metadata() {
        let mut agent = TargetNetworkAgentState::default();
        agent.enable_primary_events();
        agent.register_synthetic_websocket_request(
            "FETCH-ws".to_owned(),
            "REQ-synthetic-ws".to_owned(),
            7,
        );
        let websocket_record = websocket_record("wss://example.com/socket", 7);
        let api_record = subresource_record("https://example.com/api");
        let events = [websocket_event(7, 12)];
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record)),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(events[0].clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(api_record)),
        ];
        for item in &items {
            agent.ingest_renderer_output_item(item, "LOADER-1");
        }

        let subresource_activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ])
        .expect("test subresource activity should be visible");
        let websocket_activity = PendingWebSocketNetworkActivity::from_sessions(vec![
            PendingWebSocketNetworkActivitySession::new(None, 0, 0),
        ])
        .expect("test websocket activity should be visible");
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let mut request_ids = NetworkBacklogRequestIdPlan::new(
            &mut agent.artifacts.subresource_network_artifacts,
            &mut agent.artifacts.websocket_network_artifacts,
            Some(contextual_preferred_request_id("REQ-main")),
            &mut request_id_allocator,
        );
        let mut subresource_backlog = agent.output_queue.backlog_prepared_delivery_for_activity(
            Some(subresource_activity),
            None,
            &mut request_ids,
        );
        let websocket_backlog = agent.output_queue.backlog_prepared_delivery_for_activity(
            None,
            Some(websocket_activity),
            &mut request_ids,
        );
        subresource_backlog.extend(websocket_backlog);

        let snapshot = agent
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut subresource_backlog)
            .expect("merged prepared backlog should build one combined delivery snapshot");
        let output_order = snapshot
            .outputs()
            .into_iter()
            .map(|item| {
                if let Some(output) = item.as_subresource() {
                    return format!("sub:{}", output.request_id());
                }
                let output = item
                    .as_websocket()
                    .expect("combined backlog item should be subresource or WebSocket");
                if let Some(output) = output.as_handshake() {
                    format!("ws-handshake:{}", output.request_id())
                } else if let Some(output) = output.as_frame() {
                    format!("ws-frame:{}", output.request_id())
                } else {
                    unreachable!("test WebSocket item should be handshake or frame")
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(
            output_order,
            vec![
                "sub:REQ-synthetic-ws",
                "ws-handshake:REQ-synthetic-ws",
                "ws-frame:REQ-synthetic-ws",
                "sub:REQ-main",
            ],
            "merged prepared tokens must preserve preferred-id availability across already-bound WebSocket metadata"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            0,
            "merged synthetic WebSocket binding and preferred subresource id should not allocate fallback request ids"
        );
    }

    #[test]
    fn websocket_handshake_backlog_survives_caught_up_subresource_cursor() {
        let mut agent = TargetNetworkAgentState::default();
        agent.enable_primary_events();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        ingest_network_output(&mut agent, &records, &[]);
        let snapshot = pending_combined_snapshot(&mut agent, Some("REQ-bound"));
        assert_eq!(subresource_request_ids(&snapshot), vec!["REQ-bound"]);
        agent.mark_subresource_records_emitted(None, 0, 1);

        assert!(
            agent
                .backlog_prepared_delivery(
                    None,
                    None,
                    None,
                    &mut ConnectionNetworkRequestIdAllocator::default()
                )
                .has_output(),
            "subresource and websocket handshake cursors must remain independent"
        );
        let handshake_snapshot = pending_combined_snapshot(&mut agent, None);
        assert_eq!(
            websocket_handshake_request_ids(&handshake_snapshot),
            vec!["REQ-bound"]
        );
        agent.mark_network_backlog_delivery_snapshot_emitted(&handshake_snapshot);
        assert!(
            !agent
                .backlog_prepared_delivery(
                    None,
                    None,
                    None,
                    &mut ConnectionNetworkRequestIdAllocator::default()
                )
                .has_output()
        );
    }

    #[test]
    fn websocket_frame_backlog_survives_caught_up_handshake_cursor() {
        let mut agent = TargetNetworkAgentState::default();
        agent.enable_primary_events();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        let events = [websocket_event(7, 12)];
        ingest_network_output(&mut agent, &records, &events);
        let handshake_snapshot = pending_combined_snapshot(&mut agent, None);
        assert_eq!(
            websocket_handshake_request_ids(&handshake_snapshot),
            vec!["REQ-1"]
        );
        agent.mark_websocket_activity_emitted(None, 0, 1, 0, 0);

        assert!(
            agent
                .backlog_prepared_delivery(
                    None,
                    None,
                    None,
                    &mut ConnectionNetworkRequestIdAllocator::default()
                )
                .has_output(),
            "handshake cursor must not consume subresource or frame backlog"
        );
        let frame_snapshot = pending_combined_snapshot(&mut agent, None);
        assert!(
            websocket_handshake_request_ids(&frame_snapshot).is_empty(),
            "handshake cursor should hide handshake output from the next combined snapshot"
        );
        assert_eq!(
            websocket_frame_outputs(&frame_snapshot),
            vec![("REQ-1".to_owned(), 12)]
        );
        agent.mark_network_backlog_delivery_snapshot_emitted(&frame_snapshot);
        assert!(
            !agent
                .backlog_prepared_delivery(
                    None,
                    None,
                    None,
                    &mut ConnectionNetworkRequestIdAllocator::default()
                )
                .has_output(),
            "combined mark should consume the remaining subresource and frame outputs together"
        );
    }

    #[test]
    fn websocket_subresource_delivery_reuses_existing_socket_request_id() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        agent.enable_primary_events();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        let events = [websocket_event(7, 12)];
        ingest_network_output(&mut agent, &records, &events);
        let handshake_snapshot =
            pending_combined_snapshot_with_allocator(&mut agent, None, &mut request_id_allocator);
        assert_eq!(
            websocket_handshake_request_ids(&handshake_snapshot),
            vec!["REQ-1"]
        );

        assert_eq!(
            subresource_request_ids(&handshake_snapshot),
            vec!["REQ-1"],
            "websocket subresource delivery must reuse the socket request id instead of inventing a second id"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            1,
            "reusing the existing socket request id should not allocate a second request id"
        );
        assert_eq!(
            websocket_frame_outputs(&handshake_snapshot),
            vec![("REQ-1".to_owned(), 12)]
        );
    }

    #[test]
    fn combined_frame_delivery_prefers_visible_subresource_binding() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        agent.enable_primary_events();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        let events = [websocket_event(7, 12)];
        ingest_network_output(&mut agent, &records, &events);

        agent.mark_websocket_activity_emitted(None, 0, 1, 0, 0);

        let frame_snapshot = pending_combined_snapshot_with_allocator(
            &mut agent,
            Some("REQ-preferred"),
            &mut request_id_allocator,
        );
        assert!(
            websocket_handshake_request_ids(&frame_snapshot).is_empty(),
            "record cursor should hide handshake output from the combined snapshot"
        );
        assert_eq!(
            websocket_frame_outputs(&frame_snapshot),
            vec![("REQ-preferred".to_owned(), 12)],
            "combined delivery should let the still-visible subresource metadata bind the socket before the frame is materialized"
        );
        assert_eq!(
            subresource_request_ids(&frame_snapshot),
            vec!["REQ-preferred"],
            "visible subresource metadata owns the request id binding in the combined production path"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            0,
            "preferred subresource binding should not allocate a synthetic WebSocket request id"
        );
    }

    #[test]
    fn frame_only_delivery_does_not_consume_preferred_subresource_request_id() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        agent.enable_primary_events();
        let websocket_record = websocket_record("wss://example.com/socket", 7);
        let api_record = subresource_record("https://example.com/api");
        let events = [websocket_event(7, 12)];
        let items = vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(websocket_record)),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(events[0].clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(api_record)),
        ];
        for item in &items {
            agent.ingest_renderer_output_item(item, "LOADER-1");
        }

        agent.mark_subresource_records_emitted(None, 0, 1);
        agent.mark_websocket_activity_emitted(None, 0, 1, 0, 0);

        let frame_snapshot = pending_combined_snapshot_with_allocator(
            &mut agent,
            Some("REQ-preferred"),
            &mut request_id_allocator,
        );
        assert!(
            websocket_handshake_request_ids(&frame_snapshot).is_empty(),
            "handshake cursor should hide the WebSocket handshake output"
        );
        assert_eq!(
            websocket_frame_outputs(&frame_snapshot),
            vec![("REQ-1".to_owned(), 12)],
            "frame-only delivery should allocate a socket request id instead of consuming the preferred subresource id"
        );
        assert_eq!(
            subresource_request_ids(&frame_snapshot),
            vec!["REQ-preferred"],
            "the later ordinary subresource should still consume the preferred request id"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            1,
            "only the frame-only WebSocket path should allocate a fallback request id"
        );
    }

    #[test]
    fn websocket_delivery_snapshots_resolve_request_ids_in_network_agent() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        agent.enable_primary_events();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        let events = [websocket_event(7, 12)];
        ingest_network_output(&mut agent, &records, &events);
        assert_eq!(
            subresource_request_ids(&pending_combined_snapshot_with_allocator(
                &mut agent,
                Some("REQ-bound"),
                &mut request_id_allocator
            )),
            vec!["REQ-bound"],
            "subresource delivery should bind the preferred request id"
        );
        let snapshot =
            pending_combined_snapshot_with_allocator(&mut agent, None, &mut request_id_allocator);
        assert_eq!(
            websocket_handshake_request_ids(&snapshot),
            vec!["REQ-bound"],
            "handshake delivery should reuse the subresource request id"
        );
        assert_eq!(
            websocket_frame_outputs(&snapshot),
            vec![("REQ-bound".to_owned(), 12)],
            "frame delivery should reuse the same owner-resolved request id"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            0,
            "delivery snapshots should not allocate when subresource binding exists"
        );
    }

    #[test]
    fn websocket_delivery_snapshots_allocate_unbound_socket_request_id_once() {
        let mut agent = TargetNetworkAgentState::default();
        let mut request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        agent.enable_primary_events();
        let records = vec![websocket_record("wss://example.com/socket", 7)];
        let events = [websocket_event(7, 12)];
        ingest_network_output(&mut agent, &records, &events);

        assert_eq!(
            websocket_handshake_request_ids(&pending_combined_snapshot_with_allocator(
                &mut agent,
                None,
                &mut request_id_allocator
            )),
            vec!["REQ-1"],
            "unbound handshake delivery should allocate a request id"
        );
        assert_eq!(request_id_allocator.next_sequence_for_test(), 1);

        assert_eq!(
            websocket_frame_outputs(&pending_combined_snapshot_with_allocator(
                &mut agent,
                None,
                &mut request_id_allocator
            )),
            vec![("REQ-1".to_owned(), 12)],
            "frame delivery should reuse the request id allocated for the same socket"
        );
        assert_eq!(
            request_id_allocator.next_sequence_for_test(),
            1,
            "frame delivery should not allocate another request id for the same socket"
        );
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WebSocketNetworkArtifacts {
    observation: WebSocketNetworkObservationArtifacts,
    request_ids: WebSocketRequestIdState,
}

impl WebSocketNetworkArtifacts {
    pub(crate) fn emitted_record_count_for_session(&self, session_id: Option<&str>) -> usize {
        self.observation
            .emitted_record_count_for_session(session_id)
    }

    pub(crate) fn emitted_event_count_for_session(&self, session_id: Option<&str>) -> usize {
        self.observation.emitted_event_count_for_session(session_id)
    }

    pub(crate) fn mark_emitted(
        &mut self,
        session_id: Option<&str>,
        record_start_index: usize,
        record_count: usize,
        event_start_index: usize,
        event_count: usize,
    ) {
        self.observation.mark_emitted(
            session_id,
            record_start_index,
            record_count,
            event_start_index,
            event_count,
        );
    }

    pub(crate) fn reset_cursors(&mut self) {
        self.observation.reset_cursors();
    }

    pub(crate) fn set_session_cursors(
        &mut self,
        session_id: Option<&str>,
        record_count: usize,
        event_count: usize,
    ) {
        self.observation
            .set_session_cursors(session_id, record_count, event_count);
    }

    pub(crate) fn remove_session_cursors(&mut self, session_id: Option<&str>) {
        self.observation.remove_session_cursors(session_id);
    }

    pub(crate) fn clear_request_ids(&mut self) {
        self.request_ids.clear();
    }

    pub(crate) fn clear_all(&mut self) {
        self.reset_cursors();
        self.clear_request_ids();
    }

    pub(crate) fn register_synthetic_request(
        &mut self,
        request_id: String,
        network_request_id: String,
        socket_id: u64,
    ) {
        self.request_ids
            .register_synthetic_request(request_id, network_request_id, socket_id);
    }

    pub(crate) fn synthetic_socket_id_for_request(&self, request_id: &str) -> Option<u64> {
        self.request_ids.synthetic_socket_id_for_request(request_id)
    }

    pub(crate) fn request_id_for_socket(&self, socket_id: u64) -> Option<&str> {
        self.request_ids.request_id_for_socket(socket_id)
    }

    pub(crate) fn set_request_id_for_socket_if_absent(
        &mut self,
        socket_id: u64,
        request_id: String,
    ) {
        self.request_ids
            .set_request_id_for_socket_if_absent(socket_id, request_id);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WebSocketNetworkObservationArtifacts {
    emitted_record_counts_by_session: HashMap<Option<String>, usize>,
    emitted_event_counts_by_session: HashMap<Option<String>, usize>,
}

impl WebSocketNetworkObservationArtifacts {
    pub(crate) fn emitted_record_count_for_session(&self, session_id: Option<&str>) -> usize {
        self.emitted_record_counts_by_session
            .get(&session_id.map(str::to_owned))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn emitted_event_count_for_session(&self, session_id: Option<&str>) -> usize {
        self.emitted_event_counts_by_session
            .get(&session_id.map(str::to_owned))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn mark_emitted(
        &mut self,
        session_id: Option<&str>,
        record_start_index: usize,
        record_count: usize,
        event_start_index: usize,
        event_count: usize,
    ) {
        let record_cursor = record_start_index
            .checked_add(record_count)
            .expect("websocket emitted record cursor exhausted");
        let event_cursor = event_start_index
            .checked_add(event_count)
            .expect("websocket emitted event cursor exhausted");
        self.set_session_cursors(session_id, record_cursor, event_cursor);
    }

    pub(crate) fn reset_cursors(&mut self) {
        self.emitted_record_counts_by_session.clear();
        self.emitted_event_counts_by_session.clear();
    }

    pub(crate) fn set_session_cursors(
        &mut self,
        session_id: Option<&str>,
        record_count: usize,
        event_count: usize,
    ) {
        let session_id = session_id.map(str::to_owned);
        self.emitted_record_counts_by_session
            .insert(session_id.clone(), record_count);
        self.emitted_event_counts_by_session
            .insert(session_id, event_count);
    }

    pub(crate) fn remove_session_cursors(&mut self, session_id: Option<&str>) {
        let session_id = session_id.map(str::to_owned);
        self.emitted_record_counts_by_session.remove(&session_id);
        self.emitted_event_counts_by_session.remove(&session_id);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WebSocketRequestIdState {
    request_ids_by_socket_id: HashMap<u64, String>,
    socket_ids_by_request_id: HashMap<String, u64>,
}

impl WebSocketRequestIdState {
    pub(crate) fn clear(&mut self) {
        self.request_ids_by_socket_id.clear();
        self.socket_ids_by_request_id.clear();
    }

    pub(crate) fn register_synthetic_request(
        &mut self,
        request_id: String,
        network_request_id: String,
        socket_id: u64,
    ) {
        self.socket_ids_by_request_id.insert(request_id, socket_id);
        self.socket_ids_by_request_id
            .insert(network_request_id.clone(), socket_id);
        self.request_ids_by_socket_id
            .insert(socket_id, network_request_id);
    }

    pub(crate) fn synthetic_socket_id_for_request(&self, request_id: &str) -> Option<u64> {
        self.socket_ids_by_request_id.get(request_id).copied()
    }

    pub(crate) fn request_id_for_socket(&self, socket_id: u64) -> Option<&str> {
        self.request_ids_by_socket_id
            .get(&socket_id)
            .map(String::as_str)
    }

    pub(crate) fn set_request_id_for_socket_if_absent(
        &mut self,
        socket_id: u64,
        request_id: String,
    ) {
        self.request_ids_by_socket_id
            .entry(socket_id)
            .or_insert(request_id);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoStreamState {
    body: CapturedBody,
    pub offset: usize,
}

impl IoStreamState {
    pub(crate) fn from_bytes(bytes: Vec<u8>, offset: usize) -> Self {
        Self {
            body: CapturedBody::from_bytes_spooled(bytes),
            offset,
        }
    }

    pub(crate) fn from_body_source(body: CapturedBody, offset: usize) -> Self {
        Self { body, offset }
    }

    pub(crate) fn len(&self) -> usize {
        self.body.len()
    }

    pub(crate) fn read_range(&self, offset: usize, len: usize) -> Vec<u8> {
        self.body.read_range(offset, len).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetIoStreamRead {
    pub(crate) bytes: Vec<u8>,
    pub(crate) eof: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TargetIoStreamArtifacts {
    next_stream_id: u64,
    streams: HashMap<String, IoStreamState>,
}

impl TargetIoStreamArtifacts {
    pub(crate) fn allocate_handle(&mut self) -> String {
        self.next_stream_id = self
            .next_stream_id
            .checked_add(1)
            .expect("IO stream id sequence exhausted");
        format!("STREAM-{}", self.next_stream_id)
    }

    pub(crate) fn insert_stream(&mut self, handle: String, bytes: Vec<u8>, offset: usize) {
        self.streams
            .insert(handle, IoStreamState::from_bytes(bytes, offset));
    }

    pub(crate) fn insert_stream_body_source(
        &mut self,
        handle: String,
        body: CapturedBody,
        offset: usize,
    ) {
        self.streams
            .insert(handle, IoStreamState::from_body_source(body, offset));
    }

    pub(crate) fn remove(&mut self, handle: &str) -> Option<IoStreamState> {
        self.streams.remove(handle)
    }

    pub(crate) fn read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> Option<TargetIoStreamRead> {
        let stream = self.streams.get_mut(handle)?;
        let stream_len = stream.len();
        let start = offset.unwrap_or(stream.offset).min(stream_len);
        stream.offset = start;
        let requested_len = size.unwrap_or_else(|| stream_len.saturating_sub(start));
        let bytes = stream.read_range(start, requested_len);
        let end = start.saturating_add(bytes.len()).min(stream_len);
        stream.offset = end;
        Some(TargetIoStreamRead {
            bytes,
            eof: end >= stream_len,
        })
    }

    pub(crate) fn clear_streams(&mut self) {
        self.streams.clear();
    }

    pub(crate) fn reset_all(&mut self) {
        self.next_stream_id = 0;
        self.streams.clear();
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_next_stream_id_for_test(&mut self, next_stream_id: u64) {
        self.next_stream_id = next_stream_id;
    }

    #[cfg(test)]
    pub(crate) fn next_stream_id_for_test(&self) -> u64 {
        self.next_stream_id
    }
}
