use moli_core::page::{
    NavigationRedirect, SubresourceBodyFinishedResult, SubresourceRequestInitiatorType,
    SubresourceResponseBody,
};

use crate::automation::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, FetchRequestStage,
    monotonic_timestamp_seconds,
};
use crate::domains::network::{NetworkBacklogPreferredRequestId, NetworkPreparedOutputs};

use super::events::{
    emit_body_finished, emit_data_received, emit_event_source_message_received,
    emit_loading_failed, emit_loading_finished, emit_redirect_response_received_extra_info,
    emit_request_will_be_sent, emit_request_will_be_sent_extra_info, emit_response_received,
    emit_websocket_closed, emit_websocket_created, emit_websocket_frame,
    emit_websocket_frame_error, emit_websocket_handshake_response_received,
    emit_websocket_will_send_handshake_request,
};
use super::output_queue::{
    PendingNetworkBacklogDeliveryItem, PendingNetworkBacklogDeliverySnapshot,
    TargetNetworkBacklogPreparedDelivery, TargetSubresourceBodyNetworkDeliveryOutput,
    TargetSubresourceCompleteNetworkDeliveryOutput, TargetSubresourceDataNetworkDeliveryOutput,
    TargetSubresourceEventSourceMessageNetworkDeliveryOutput, TargetSubresourceMetadataOutcome,
    TargetSubresourceNetworkDeliveryOutput, TargetSubresourceRequestExtraInfoNetworkDeliveryOutput,
    TargetSubresourceRequestNetworkDeliveryOutput, TargetSubresourceResponseNetworkDeliveryOutput,
    TargetWebSocketDeliveryRecord, TargetWebSocketLifecycleDeliveryKind,
};

pub(crate) struct NetworkBacklogProjectionContext<'a> {
    owner: CommandOwnerScope,
    pub(in crate::domains::network) frame_id: Option<&'a str>,
    pub(in crate::domains::network) base_timestamp: Option<f64>,
    pub(in crate::domains::network) preferred_request_id:
        Option<NetworkBacklogPreferredRequestId<'a>>,
    pub(in crate::domains::network) prepared_outputs: Option<&'a mut NetworkPreparedOutputs>,
}

impl<'a> NetworkBacklogProjectionContext<'a> {
    pub(crate) fn for_owner(owner: &CommandOwnerScope) -> Self {
        Self {
            owner: owner.clone(),
            frame_id: None,
            base_timestamp: None,
            preferred_request_id: None,
            prepared_outputs: None,
        }
    }

    #[cfg(test)]
    pub(in crate::domains::network) fn session_id(&self) -> Option<&str> {
        self.owner.session_id()
    }

    pub(crate) fn with_frame_id(mut self, frame_id: Option<&'a str>) -> Self {
        self.frame_id = frame_id;
        self
    }

    pub(crate) fn with_base_timestamp(mut self, base_timestamp: Option<f64>) -> Self {
        self.base_timestamp = base_timestamp;
        self
    }

    pub(crate) fn with_contextual_subresource_request_id(
        mut self,
        request_id: Option<&'a str>,
    ) -> Self {
        self.preferred_request_id =
            request_id.map(NetworkBacklogPreferredRequestId::contextual_subresource);
        self
    }

    pub(crate) fn with_prepared_outputs(
        mut self,
        prepared_outputs: Option<&'a mut NetworkPreparedOutputs>,
    ) -> Self {
        self.prepared_outputs = prepared_outputs;
        self
    }
}

fn emit_subresource_redirect_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    event_session_ids: &[Option<String>],
    request_id: &str,
    frame_id: &str,
    loader_id: &str,
    timestamp: f64,
    document_url: &url::Url,
    resource_type: DevToolsNetworkResourceType,
    initiator: SubresourceRequestInitiatorType,
    redirects: &[NavigationRedirect],
) {
    for redirect in redirects {
        // An unissued hop has no dispatched request to report. ExtraInfo still
        // belongs to the preceding response, independently of its destination.
        let request = redirect.dispatched_request.as_deref();
        let body = request.and_then(moli_fetch::DispatchedRequest::body_text);
        for event_session_id in event_session_ids {
            if let Some(request) = request {
                emit_request_will_be_sent(
                    out,
                    event_session_id.as_deref(),
                    request_id,
                    frame_id,
                    loader_id,
                    timestamp,
                    document_url,
                    &redirect.to_url,
                    &request.method,
                    body.as_deref(),
                    &request.headers,
                    resource_type,
                    initiator,
                    Some((
                        &redirect.from_url,
                        redirect.status,
                        &redirect.headers,
                        redirect.from_cache,
                        redirect.negotiated_http_version,
                    )),
                    !redirect.cookie_set_reports.is_empty(),
                    redirect.request_cookie_report.as_ref(),
                    &[],
                );
            }
            emit_redirect_response_received_extra_info(
                out,
                event_session_id.as_deref(),
                request_id,
                &redirect.headers,
                redirect.status,
                &redirect.cookie_set_reports,
            );
        }
    }
}

fn emit_subresource_network_delivery_record(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    owner: &CommandOwnerScope,
    frame_id: &str,
    base_timestamp: f64,
) -> bool {
    match delivery_output {
        TargetSubresourceNetworkDeliveryOutput::Complete(output) => {
            emit_complete_subresource_network_delivery_record(
                conn,
                out,
                output,
                event_session_ids,
                owner,
                frame_id,
                base_timestamp,
            )
        }
        TargetSubresourceNetworkDeliveryOutput::RequestStarted(output) => {
            emit_staged_subresource_request_started(
                conn,
                out,
                output,
                event_session_ids,
                owner,
                frame_id,
                base_timestamp,
            )
        }
        TargetSubresourceNetworkDeliveryOutput::RequestExtraInfo(output) => {
            emit_staged_subresource_request_extra_info(
                out,
                output,
                event_session_ids,
                base_timestamp,
            )
        }
        TargetSubresourceNetworkDeliveryOutput::ResponseStarted(output) => {
            emit_staged_subresource_response_started(
                conn,
                out,
                output,
                event_session_ids,
                owner,
                frame_id,
                base_timestamp,
            )
        }
        TargetSubresourceNetworkDeliveryOutput::DataReceived(output) => {
            emit_staged_subresource_data_received(out, output, event_session_ids, base_timestamp)
        }
        TargetSubresourceNetworkDeliveryOutput::EventSourceMessageReceived(output) => {
            emit_staged_subresource_event_source_message_received(
                out,
                output,
                event_session_ids,
                base_timestamp,
            )
        }
        TargetSubresourceNetworkDeliveryOutput::BodyFinished(output) => {
            emit_staged_subresource_body_finished(
                conn,
                out,
                output,
                event_session_ids,
                owner,
                frame_id,
                base_timestamp,
            )
        }
    }
}

fn emit_staged_subresource_data_received(
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceDataNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    let output = delivery_output.output();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    for event_session_id in event_session_ids {
        emit_data_received(
            out,
            event_session_id.as_deref(),
            delivery_output.request_id(),
            timestamp,
            output.data_length(),
            output.encoded_data_length(),
        );
    }
    out.len() > initial_output_len
}

fn emit_staged_subresource_event_source_message_received(
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceEventSourceMessageNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    let output = delivery_output.output();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    for event_session_id in event_session_ids {
        emit_event_source_message_received(
            out,
            event_session_id.as_deref(),
            delivery_output.request_id(),
            timestamp,
            output.event_name(),
            output.event_id(),
            output.data(),
        );
    }
    out.len() > initial_output_len
}

fn emit_complete_subresource_network_delivery_record(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceCompleteNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    owner: &CommandOwnerScope,
    frame_id: &str,
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    let request_id = delivery_output.request_id();
    let output = delivery_output.metadata();
    // The same native handshake record also feeds the dedicated WebSocket
    // delivery below. Publishing it as an ordinary HTTP request would give
    // clients an extra inflight request for a long-lived socket.
    if output.resource_type() == moli_core::page::SubresourceResourceType::WebSocket {
        return false;
    }
    let Some(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(owner) else {
        return false;
    };
    let request_was_announced_by_fetch_pause =
        runtime_slot.request_was_announced_by_fetch_pause(request_id);
    let announced_redirects = runtime_slot.fetch_pause_announced_redirects(request_id);
    runtime_slot.clear_fetch_pause_announced_request_id(request_id);
    if event_session_ids.is_empty() {
        return false;
    }
    let record_frame_id = output.frame_id().unwrap_or(frame_id);
    let loader_id = output.loader_id();
    let record_document_url = output.document_url();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    let resource_type = output.resource_type().into();
    if !runtime_slot.claim_completed_subresource_request_id(request_id) {
        return false;
    }
    record_subresource_pending_response_body(conn, owner, request_id, event_session_ids);
    record_subresource_request_body(
        conn,
        owner,
        request_id,
        output.request_body_bytes(),
        event_session_ids,
    );
    if !request_was_announced_by_fetch_pause {
        for event_session_id in event_session_ids {
            emit_request_will_be_sent(
                out,
                event_session_id.as_deref(),
                request_id,
                record_frame_id,
                loader_id,
                timestamp,
                record_document_url,
                output.url(),
                output.method(),
                output.request_body(),
                output.request_headers(),
                resource_type,
                output.request_initiator_type(),
                None,
                false,
                output
                    .network_request_headers()
                    .is_none()
                    .then(|| output.request_cookie_report())
                    .flatten(),
                &[],
            );
        }
    }
    if let Some(network_request_headers) = output.network_request_headers() {
        let request_cookie_report = output.request_cookie_report().cloned().unwrap_or_default();
        for event_session_id in event_session_ids {
            emit_request_will_be_sent_extra_info(
                out,
                event_session_id.as_deref(),
                request_id,
                network_request_headers,
                &request_cookie_report,
                timestamp,
            );
        }
    }
    match output.outcome() {
        TargetSubresourceMetadataOutcome::Success {
            redirect_chain,
            final_url,
            status,
            status_text,
            response_headers,
            response_body_len,
        } => {
            emit_subresource_redirect_events(
                out,
                event_session_ids,
                request_id,
                record_frame_id,
                loader_id,
                timestamp,
                record_document_url,
                resource_type,
                output.request_initiator_type(),
                &redirect_chain[announced_redirects.min(redirect_chain.len())..],
            );
            for event_session_id in event_session_ids {
                // Complete records have already delivered their response and
                // body. A matching rule cannot make them resumably paused.
                emit_response_received(
                    out,
                    event_session_id.as_deref(),
                    request_id,
                    record_frame_id,
                    loader_id,
                    timestamp,
                    final_url,
                    *status,
                    status_text.as_deref(),
                    response_headers,
                    output.cookie_set_reports(),
                    *response_body_len,
                    output.is_from_cache(),
                    output.negotiated_http_version(),
                    output.network_request_headers().is_some(),
                    resource_type,
                    &[],
                    None,
                );
            }
            if let Some(response_body) = output.response_body() {
                record_subresource_response_body_source(
                    conn,
                    owner,
                    response_body,
                    request_id,
                    event_session_ids,
                );
            }
            for event_session_id in event_session_ids {
                emit_body_finished(
                    out,
                    event_session_id.as_deref(),
                    request_id,
                    record_frame_id,
                    loader_id,
                    timestamp,
                    *response_body_len,
                    resource_type,
                );
            }
        }
        TargetSubresourceMetadataOutcome::Failure { error_text } => {
            record_subresource_failed_response_body(
                conn,
                owner,
                error_text.clone(),
                request_id,
                event_session_ids,
            );
            for event_session_id in event_session_ids {
                emit_loading_failed(
                    out,
                    event_session_id.as_deref(),
                    request_id,
                    record_frame_id,
                    loader_id,
                    timestamp,
                    error_text,
                    resource_type,
                );
            }
        }
    }
    out.len() > initial_output_len
}

fn emit_staged_subresource_request_started(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceRequestNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    owner: &CommandOwnerScope,
    frame_id: &str,
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    if event_session_ids.is_empty() {
        return false;
    }
    let request_id = delivery_output.request_id();
    let output = delivery_output.output();
    let record_frame_id = output.frame_id().unwrap_or(frame_id);
    let loader_id = output.loader_id();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    let resource_type = output.resource_type().into();
    // The staged start and final delivery share the same announcement. Only
    // final delivery retires it, including when Network has no subscribers.
    let request_was_announced_by_fetch_pause = conn
        .runtime_session_owner_slot_for_owner(owner)
        .is_some_and(|runtime_slot| runtime_slot.request_was_announced_by_fetch_pause(request_id));
    if output.resource_type() != moli_core::page::SubresourceResourceType::WebSocket {
        record_subresource_pending_response_body(conn, owner, request_id, event_session_ids);
        record_subresource_request_body(
            conn,
            owner,
            request_id,
            output.request_body_bytes(),
            event_session_ids,
        );
    }
    if !request_was_announced_by_fetch_pause {
        for event_session_id in event_session_ids {
            emit_request_will_be_sent(
                out,
                event_session_id.as_deref(),
                request_id,
                record_frame_id,
                loader_id,
                timestamp,
                output.document_url(),
                output.url(),
                output.method(),
                output.request_body(),
                output.request_headers(),
                resource_type,
                output.request_initiator_type(),
                None,
                false,
                output.request_cookie_report(),
                &[],
            );
        }
    }
    out.len() > initial_output_len
}

fn emit_staged_subresource_request_extra_info(
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceRequestExtraInfoNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    let request_id = delivery_output.request_id();
    let output = delivery_output.output();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    for event_session_id in event_session_ids {
        emit_request_will_be_sent_extra_info(
            out,
            event_session_id.as_deref(),
            request_id,
            output.request_headers(),
            output.request_cookie_report(),
            timestamp,
        );
    }
    out.len() > initial_output_len
}

fn emit_staged_subresource_response_started(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceResponseNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    owner: &CommandOwnerScope,
    frame_id: &str,
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    if event_session_ids.is_empty() {
        return false;
    }
    let request_id = delivery_output.request_id();
    let output = delivery_output.output();
    let request = output.request();
    let announced_redirects = conn
        .runtime_session_owner_slot_for_owner(owner)
        .map_or(0, |slot| slot.fetch_pause_announced_redirects(request_id));
    let record_frame_id = request.frame_id().unwrap_or(frame_id);
    let loader_id = request.loader_id();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    let resource_type = request.resource_type().into();
    emit_subresource_redirect_events(
        out,
        event_session_ids,
        request_id,
        record_frame_id,
        loader_id,
        timestamp,
        request.document_url(),
        resource_type,
        request.request_initiator_type(),
        &output.redirect_chain()[announced_redirects.min(output.redirect_chain().len())..],
    );
    for event_session_id in event_session_ids {
        let fetch_request_id = conn
            .paused_subresource_fetch_response_request_id_for_owner(owner, output.handle().get());
        let blocked_intercepts = if fetch_request_id.is_some() {
            matching_subresource_network_intercepts(
                conn,
                owner,
                FetchRequestStage::Response,
                resource_type,
                output.final_url(),
            )
        } else {
            Vec::new()
        };
        emit_response_received(
            out,
            event_session_id.as_deref(),
            request_id,
            record_frame_id,
            loader_id,
            timestamp,
            output.final_url(),
            output.status(),
            output.status_text(),
            output.response_headers(),
            output.cookie_set_reports(),
            0,
            output.is_from_cache(),
            output.negotiated_http_version(),
            output.network_request_headers().is_some(),
            resource_type,
            &blocked_intercepts,
            fetch_request_id.as_deref(),
        );
    }
    out.len() > initial_output_len
}

fn matching_subresource_network_intercepts(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    request_stage: FetchRequestStage,
    resource_type: DevToolsNetworkResourceType,
    url: &url::Url,
) -> Vec<DevToolsNetworkInterceptId> {
    conn.target_fetch_subresource_interception_snapshot_for_owner(owner)
        .map(|snapshot| snapshot.matching_network_intercepts(request_stage, resource_type, url))
        .unwrap_or_default()
}

fn emit_staged_subresource_body_finished(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceBodyNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    owner: &CommandOwnerScope,
    frame_id: &str,
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    let request_id = delivery_output.request_id();
    if let Some(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(owner) {
        runtime_slot.clear_fetch_pause_announced_request_id(request_id);
    }
    if event_session_ids.is_empty() {
        return false;
    }
    let output = delivery_output.output();
    let request = output.request();
    let record_frame_id = request.frame_id().unwrap_or(frame_id);
    let loader_id = request.loader_id();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    let resource_type = request.resource_type().into();
    match output.result() {
        SubresourceBodyFinishedResult::Ready(response_body) => {
            let response_body_len = response_body.len();
            record_subresource_response_body_source(
                conn,
                owner,
                response_body,
                request_id,
                event_session_ids,
            );
            for event_session_id in event_session_ids {
                if output.data_was_streamed() {
                    emit_loading_finished(
                        out,
                        event_session_id.as_deref(),
                        request_id,
                        record_frame_id,
                        loader_id,
                        timestamp,
                        response_body_len,
                        resource_type,
                    );
                } else {
                    emit_body_finished(
                        out,
                        event_session_id.as_deref(),
                        request_id,
                        record_frame_id,
                        loader_id,
                        timestamp,
                        response_body_len,
                        resource_type,
                    );
                }
            }
        }
        SubresourceBodyFinishedResult::Failed(error_text) => {
            record_subresource_failed_response_body(
                conn,
                owner,
                error_text.clone(),
                request_id,
                event_session_ids,
            );
            for event_session_id in event_session_ids {
                emit_loading_failed(
                    out,
                    event_session_id.as_deref(),
                    request_id,
                    record_frame_id,
                    loader_id,
                    timestamp,
                    error_text,
                    resource_type,
                );
            }
        }
        SubresourceBodyFinishedResult::FailedWithPartialBody {
            error_text,
            partial_body,
        } => {
            record_subresource_response_body_source(
                conn,
                owner,
                partial_body,
                request_id,
                event_session_ids,
            );
            for event_session_id in event_session_ids {
                emit_loading_failed(
                    out,
                    event_session_id.as_deref(),
                    request_id,
                    record_frame_id,
                    loader_id,
                    timestamp,
                    error_text,
                    resource_type,
                );
            }
        }
    }
    out.len() > initial_output_len
}

pub(crate) fn emit_pending_network_backlog_activity_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    context: NetworkBacklogProjectionContext<'_>,
) {
    let NetworkBacklogProjectionContext {
        owner,
        frame_id,
        base_timestamp,
        preferred_request_id,
        prepared_outputs,
    } = context;
    let primary_session_id = conn.runtime_session_owner_primary_session_id_for_owner(&owner);
    let Some((frame_id, snapshot)) = (|| {
        let frame_id = frame_id
            .map(str::to_owned)
            .or_else(|| conn.target_owner_identity_for_owner(&owner)?.1)?;
        let snapshot = pending_network_backlog_delivery_snapshot(
            conn,
            &owner,
            primary_session_id.as_deref(),
            preferred_request_id,
            prepared_outputs,
        )?;
        Some((frame_id, snapshot))
    })() else {
        return;
    };
    emit_network_delivery_snapshot(
        conn,
        out,
        &owner,
        &frame_id,
        base_timestamp.unwrap_or_else(monotonic_timestamp_seconds),
        snapshot,
    );
}

/// Projects the exact delivery token frozen synchronously after one renderer
/// Network fact was ingested.
///
/// This path intentionally cannot discover records from the target backlog:
/// the token already owns its concrete entries and cursor advances. That keeps
/// live CDP delivery separate from `Network.enable` (which starts at the
/// current tail) and from durable `Log.enable` replay.
pub(crate) fn emit_prepared_renderer_network_live_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    prepared: &mut TargetNetworkBacklogPreparedDelivery,
) {
    let Some((frame_id, snapshot)) = (|| {
        let frame_id = conn.target_owner_identity_for_owner(owner)?.1?;
        let snapshot = conn
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .pending_network_backlog_delivery_snapshot_from_backlog(prepared)?;
        Some((frame_id, snapshot))
    })() else {
        return;
    };
    emit_network_delivery_snapshot(
        conn,
        out,
        owner,
        &frame_id,
        monotonic_timestamp_seconds(),
        snapshot,
    );
}

fn emit_network_delivery_snapshot(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    frame_id: &str,
    base_timestamp: f64,
    snapshot: PendingNetworkBacklogDeliverySnapshot,
) {
    for (item, event_session_ids) in snapshot.delivery_entries() {
        match item {
            PendingNetworkBacklogDeliveryItem::Subresource(output) => {
                emit_subresource_network_delivery_record(
                    conn,
                    out,
                    output,
                    event_session_ids,
                    owner,
                    frame_id,
                    base_timestamp,
                );
            }
            PendingNetworkBacklogDeliveryItem::WebSocket(record) => {
                emit_websocket_network_delivery_record(
                    out,
                    record,
                    event_session_ids,
                    base_timestamp,
                );
            }
        }
    }
    mark_network_backlog_delivery_snapshot_emitted(conn, owner, &snapshot);
}

fn pending_network_backlog_delivery_snapshot(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    primary_session_id: Option<&str>,
    preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
    prepared_network_outputs: Option<&mut NetworkPreparedOutputs>,
) -> Option<PendingNetworkBacklogDeliverySnapshot> {
    if let Some(prepared_network_outputs) = prepared_network_outputs {
        let runtime_slot = conn.runtime_session_owner_slot_mut_for_owner(owner)?;
        return runtime_slot.pending_network_backlog_delivery_snapshot_from_backlog(
            prepared_network_outputs.backlog_mut(),
        );
    }
    conn.pending_network_backlog_delivery_snapshot_for_owner(
        owner,
        owner.session_id(),
        primary_session_id,
        preferred_request_id,
    )
}

fn mark_network_backlog_delivery_snapshot_emitted(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    snapshot: &PendingNetworkBacklogDeliverySnapshot,
) {
    let Some(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(owner) else {
        return;
    };
    runtime_slot.mark_network_backlog_delivery_snapshot_emitted(snapshot);
}

fn record_subresource_response_body_source(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    response_body: &SubresourceResponseBody,
    request_id: &str,
    session_ids: &[Option<String>],
) {
    let captured_body = crate::conn::CapturedBody::from_subresource_response_body(response_body);
    let data_type = crate::automation::DevToolsNetworkDataType::Response;
    let collector_ids =
        conn.network_data_collector_ids_for_owner_body(owner, data_type, captured_body.len());
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(data_type);
    conn.record_collected_network_data_body(
        request_id.to_owned(),
        data_type,
        captured_body.clone(),
        collector_ids.iter().cloned(),
        collection_was_gated,
    );
    let Some(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(owner) else {
        return;
    };
    runtime_slot.record_captured_response_body_source_with_collector_scope(
        request_id.to_owned(),
        captured_body,
        session_ids.to_vec(),
        collector_ids,
        collection_was_gated,
    );
}

pub(crate) fn record_subresource_request_body(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
    request_body: Option<&bytes::Bytes>,
    session_ids: &[Option<String>],
) {
    let Some(request_body) = request_body else {
        return;
    };
    let data_type = crate::automation::DevToolsNetworkDataType::Request;
    let collector_ids =
        conn.network_data_collector_ids_for_owner_body(owner, data_type, request_body.len());
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(data_type);
    conn.record_collected_network_data_body(
        request_id.to_owned(),
        data_type,
        crate::conn::CapturedBody::from_bytes(request_body.clone()),
        collector_ids.iter().cloned(),
        collection_was_gated,
    );
    let Some(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(owner) else {
        return;
    };
    runtime_slot.record_captured_request_body_with_collector_scope(
        request_id.to_owned(),
        request_body.clone(),
        session_ids.to_vec(),
        collector_ids,
        collection_was_gated,
    );
}

fn record_subresource_pending_response_body(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
    session_ids: &[Option<String>],
) {
    let collector_ids = conn.network_data_collector_ids_for_owner_body(
        owner,
        crate::automation::DevToolsNetworkDataType::Response,
        0,
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(
        crate::automation::DevToolsNetworkDataType::Response,
    );
    let Some(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(owner) else {
        return;
    };
    runtime_slot.record_pending_response_body_with_collector_scope(
        request_id.to_owned(),
        session_ids.to_vec(),
        collector_ids,
        collection_was_gated,
    );
}

fn record_subresource_failed_response_body(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    error_text: String,
    request_id: &str,
    session_ids: &[Option<String>],
) {
    let collector_ids = conn.network_data_collector_ids_for_owner_body(
        owner,
        crate::automation::DevToolsNetworkDataType::Response,
        0,
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(
        crate::automation::DevToolsNetworkDataType::Response,
    );
    let Some(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(owner) else {
        return;
    };
    runtime_slot.record_failed_response_body_with_collector_scope(
        request_id.to_owned(),
        error_text,
        session_ids.to_vec(),
        collector_ids,
        collection_was_gated,
    );
}

fn emit_websocket_network_delivery_record(
    out: &mut Vec<BackgroundProtocolEvent>,
    record: &TargetWebSocketDeliveryRecord,
    event_session_ids: &[Option<String>],
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    match record {
        TargetWebSocketDeliveryRecord::Handshake(output) => {
            if event_session_ids.is_empty() {
                return false;
            }
            let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
            for event_session_id in event_session_ids {
                emit_websocket_created(
                    out,
                    event_session_id.as_deref(),
                    output.request_id(),
                    output.url(),
                );
                emit_websocket_will_send_handshake_request(
                    out,
                    event_session_id.as_deref(),
                    output.request_id(),
                    timestamp,
                    output.request_headers(),
                );
            }
            if let Some(response) = output.response() {
                for event_session_id in event_session_ids {
                    emit_websocket_handshake_response_received(
                        out,
                        event_session_id.as_deref(),
                        output.request_id(),
                        timestamp,
                        response.status(),
                        response.response_headers(),
                    );
                }
            }
        }
        TargetWebSocketDeliveryRecord::Frame(event) => {
            if event_session_ids.is_empty() {
                return false;
            }
            let timestamp = base_timestamp + (event.timestamp_order_index() as f64 * 0.000_001);
            for event_session_id in event_session_ids {
                emit_websocket_frame(
                    out,
                    event_session_id.as_deref(),
                    event.request_id(),
                    timestamp,
                    event.direction(),
                    event.opcode(),
                    event.payload_length(),
                );
            }
        }
        TargetWebSocketDeliveryRecord::Lifecycle(event) => {
            if event_session_ids.is_empty() {
                return false;
            }
            let timestamp = base_timestamp + (event.timestamp_order_index() as f64 * 0.000_001);
            for event_session_id in event_session_ids {
                match event.kind() {
                    TargetWebSocketLifecycleDeliveryKind::FrameError { error_text } => {
                        emit_websocket_frame_error(
                            out,
                            event_session_id.as_deref(),
                            event.request_id(),
                            timestamp,
                            error_text,
                        );
                    }
                    TargetWebSocketLifecycleDeliveryKind::Closed => emit_websocket_closed(
                        out,
                        event_session_id.as_deref(),
                        event.request_id(),
                        timestamp,
                    ),
                }
            }
        }
    }
    out.len() > initial_output_len
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        NavigationRedirect, ScriptNetworkOutputItem, SubresourceBodyFinished,
        SubresourceNetworkRecord, SubresourceNetworkRequestHandle, SubresourceRequestInitiatorType,
        SubresourceRequestStarted, SubresourceResourceType, SubresourceResponseBody,
        SubresourceResponseStarted,
    };
    use url::Url;

    use crate::{
        automation::{AutomationEvent, DevToolsNetworkResourceType},
        conn::{CdpConnection, CommandOwnerScope},
        domains::network::{
            NetworkPreparedOutputs, PendingSubresourceNetworkActivity,
            PendingSubresourceNetworkActivitySession, TargetNetworkBacklogRequestIdResolver,
            TargetNetworkOutputQueue, TargetSubresourcePlanOutput,
        },
    };

    use super::{
        NetworkBacklogPreferredRequestId, NetworkBacklogProjectionContext,
        emit_network_delivery_snapshot,
    };

    struct FixedRequestId;

    impl TargetNetworkBacklogRequestIdResolver for FixedRequestId {
        fn request_id_for_subresource_output(
            &mut self,
            _output: &TargetSubresourcePlanOutput,
        ) -> String {
            "REQ-XHR".to_owned()
        }

        fn request_id_for_websocket_socket(&mut self, _socket_id: u64) -> String {
            "REQ-WS".to_owned()
        }
    }

    fn emitted_events_for(
        items: Vec<ScriptNetworkOutputItem>,
    ) -> Vec<crate::BackgroundProtocolEvent> {
        let mut queue = TargetNetworkOutputQueue::default();
        for item in items {
            queue.append_renderer_output_item_for_loader(&item, "LOADER-1");
        }
        let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
            PendingSubresourceNetworkActivitySession::new(None, 0),
        ]);
        let mut request_ids = FixedRequestId;
        let mut prepared =
            queue.backlog_prepared_delivery_for_activity(activity, None, &mut request_ids);
        let snapshot = queue
            .pending_network_backlog_delivery_snapshot_from_backlog(&mut prepared)
            .expect("XHR output should produce a delivery snapshot");
        let mut events = Vec::new();
        let mut conn = CdpConnection::new();
        conn.install_default_browser_target();
        let owner = crate::conn::CommandOwnerScope::capture(&conn, None);
        emit_network_delivery_snapshot(&mut conn, &mut events, &owner, "FRAME-1", 1.0, snapshot);
        events
    }

    fn emitted_terminal_for(items: Vec<ScriptNetworkOutputItem>) -> crate::BackgroundProtocolEvent {
        emitted_events_for(items)
            .into_iter()
            .find(|event| event.protocol_method() == Some("Network.loadingFinished"))
            .expect("successful XHR should emit loadingFinished")
    }

    fn assert_xhr_terminal_retains_internal_resource_type(items: Vec<ScriptNetworkOutputItem>) {
        let terminal = emitted_terminal_for(items);
        assert_eq!(
            terminal
                .trace_network_summary()
                .and_then(|summary| summary.1),
            Some("XHR")
        );
        assert!(
            terminal.should_wait_for_background_navigation_completion(),
            "a successful XHR terminal must stay behind the same navigation gate as its start and response"
        );
        let (message, automation_event) = terminal.into_parts();
        assert_eq!(message["method"], "Network.loadingFinished");
        assert!(message["params"].get("type").is_none());
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::NetworkResponseCompleted(event))
                if event.resource_type == Some(DevToolsNetworkResourceType::Xhr)
        ));
    }

    #[test]
    fn complete_and_staged_redirects_report_effective_method_and_body_per_hop() {
        let cases: &[(&str, &[u16], &[&str])] = &[
            ("POST", &[301], &["POST", "GET"]),
            ("POST", &[302], &["POST", "GET"]),
            ("PATCH", &[303], &["PATCH", "GET"]),
            ("HEAD", &[303], &["HEAD", "HEAD"]),
            ("GET", &[302, 303], &["GET", "GET", "GET"]),
            ("POST", &[307, 308], &["POST", "POST", "POST"]),
            ("POST", &[307, 302, 308], &["POST", "POST", "GET", "GET"]),
        ];
        for &(method, statuses, expected) in cases {
            let document_url = Url::parse("https://example.test/").unwrap();
            let request_url = document_url.join("api-0").unwrap();
            let final_url = document_url
                .join(&format!("api-{}", statuses.len()))
                .unwrap();
            let body = (!matches!(method, "GET" | "HEAD")).then(|| "payload".to_owned());
            let headers: Vec<(String, String)> = [
                ("Content-Encoding", "gzip"),
                ("content-language", "de"),
                ("CONTENT-LENGTH", "7"),
                ("Content-Location", "/payload"),
                ("content-type", "text/plain"),
                ("X-Keep", "value"),
            ]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
            let mut dispatched = moli_fetch::Request::new(
                method,
                request_url.as_str(),
                body.clone(),
                headers.clone(),
            )
            .unwrap();
            let redirects = statuses
                .iter()
                .enumerate()
                .map(|(index, &status)| {
                    dispatched
                        .follow_redirect(moli_fetch::RedirectInfo {
                            source: moli_fetch::RedirectSource::Network,
                            from_url: document_url.join(&format!("api-{index}")).unwrap(),
                            to_url: document_url.join(&format!("api-{}", index + 1)).unwrap(),
                            status,
                            headers: Vec::new(),
                            network_extra_info_available: false,
                            request_extra_info: None,
                            dispatched_request: None,
                            response_extra_info: None,
                            redirect_has_extra_info: false,
                            request_cookie_report: None,
                            cookie_set_reports: Vec::new(),
                            from_cache: false,
                            negotiated_http_version: None,
                        })
                        .unwrap();
                    dispatched.redirect_chain().last().unwrap().clone().into()
                })
                .collect::<Vec<NavigationRedirect>>();
            let complete = SubresourceNetworkRecord::success_with_body(
                Some("FRAME-1".to_owned()),
                document_url.clone(),
                request_url.clone(),
                method.to_owned(),
                headers.clone().into(),
                body.clone(),
                SubresourceResourceType::Xhr,
                None,
                redirects.clone(),
                final_url.clone(),
                200,
                Vec::new(),
                SubresourceResponseBody::from_bytes(b"complete".to_vec()),
                Vec::new(),
            );
            let handle = SubresourceNetworkRequestHandle::new(1);
            let request = SubresourceRequestStarted::new(
                handle,
                Some("FRAME-1".to_owned()),
                document_url,
                request_url,
                method.to_owned(),
                headers.clone().into(),
                body.clone(),
                SubresourceResourceType::Xhr,
                SubresourceRequestInitiatorType::Script,
                None,
            );
            let response = SubresourceResponseStarted::new(
                handle,
                redirects,
                final_url,
                200,
                Vec::new(),
                Vec::new(),
            );
            let finished = SubresourceBodyFinished::ready(
                handle,
                SubresourceResponseBody::from_bytes(b"staged".to_vec()),
            );
            let variants = [
                vec![ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
                    complete,
                ))],
                vec![
                    ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
                    ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
                    ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(finished)),
                ],
            ];
            for items in variants {
                let requests = emitted_events_for(items)
                    .into_iter()
                    .filter_map(|event| {
                        let (message, _) = event.into_parts();
                        (message["method"] == "Network.requestWillBeSent").then_some(message)
                    })
                    .collect::<Vec<_>>();
                assert_eq!(requests.len(), expected.len());
                for (message, &expected_method) in requests.iter().zip(expected) {
                    let request = &message["params"]["request"];
                    assert_eq!(
                        request["method"].as_str(),
                        Some(expected_method),
                        "{method} {statuses:?}"
                    );
                    let expected_body = body
                        .as_deref()
                        .filter(|_| !matches!(expected_method, "GET" | "HEAD"));
                    assert_eq!(
                        request["hasPostData"].as_bool(),
                        Some(expected_body.is_some())
                    );
                    assert_eq!(
                        request.get("postData").and_then(|value| value.as_str()),
                        expected_body
                    );
                    let emitted_headers = request["headers"].as_object().expect("request headers");
                    for (name, value) in &headers {
                        let actual = emitted_headers
                            .iter()
                            .find(|(key, _)| key.eq_ignore_ascii_case(name))
                            .and_then(|(_, value)| value.as_str());
                        let expected = (name == "X-Keep" || method == expected_method)
                            .then_some(value.as_str());
                        assert_eq!(actual, expected, "{method} {statuses:?} header {name}");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn redirects_project_dispatched_interception_metadata_in_complete_and_staged_outputs() {
        use axum::{Router, body::Bytes, http::StatusCode};
        use moli_cookie_jar::new_shared_browser_cookie_store;
        use moli_fetch::{FetchClient, FetchConfig, Request};
        use parking_lot::Mutex;
        use std::sync::Arc;

        let received = Arc::new(Mutex::new(Vec::new()));
        let observations = received.clone();
        let app = Router::new().fallback(move |request: axum::extract::Request| {
            let observations = observations.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = axum::body::to_bytes(body, 1024).await.unwrap();
                observations
                    .lock()
                    .push((parts.method, parts.headers, body));
                match parts.uri.path() {
                    "/api-0" => (StatusCode::TEMPORARY_REDIRECT, [("location", "/api-1")], ""),
                    "/api-1" => (StatusCode::FOUND, [("location", "/api-2")], ""),
                    "/api-2" => (StatusCode::PERMANENT_REDIRECT, [("location", "/api-3")], ""),
                    _ => (StatusCode::OK, [("content-type", "text/plain")], "done"),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let document_url =
            Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let request_url = document_url.join("api-0").unwrap();
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let response = client
            .fetch(
                Request::new(
                    "POST",
                    request_url.as_str(),
                    Some("overridden".into()),
                    vec![
                        ("X-Once".into(), "temporary".into()),
                        ("Content-Type".into(), "application/json".into()),
                    ],
                )
                .unwrap()
                .with_redirect_headers(Some(
                    vec![
                        ("X-Persistent".into(), "restored".into()),
                        ("Content-Type".into(), "text/plain".into()),
                    ]
                    .into(),
                )),
            )
            .await
            .unwrap();
        assert!(client.shutdown().is_clean());
        server.abort();
        let requests = received.lock();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests
                .iter()
                .map(|(method, _, _)| method.as_str())
                .collect::<Vec<_>>(),
            ["POST", "POST", "GET", "GET"]
        );
        assert_eq!(requests[1].2, Bytes::from_static(b"overridden"));
        assert!(requests[2].2.is_empty());
        assert!(!requests[1].1.contains_key("x-once"));
        assert_eq!(requests[1].1["x-persistent"], "restored");
        assert_eq!(requests[1].1["content-type"], "text/plain");
        assert!(!requests[2].1.contains_key("content-type"));
        let response: moli_core::page::NavigationResponse = response.into();
        let handle = SubresourceNetworkRequestHandle::new(1);
        let complete = SubresourceNetworkRecord::success_with_body(
            None,
            document_url.clone(),
            request_url.clone(),
            "PATCH".into(),
            vec![("X-Once".into(), "temporary".into())].into(),
            Some("original".into()),
            SubresourceResourceType::Xhr,
            None,
            response.redirect_chain.clone(),
            response.final_url.clone(),
            response.status,
            response.headers.clone(),
            SubresourceResponseBody::from_bytes(b"done".to_vec()),
            Vec::new(),
        );
        let started = SubresourceRequestStarted::new(
            handle,
            None,
            document_url,
            request_url,
            "PATCH".into(),
            vec![("X-Once".into(), "temporary".into())].into(),
            Some("original".into()),
            SubresourceResourceType::Xhr,
            SubresourceRequestInitiatorType::Script,
            None,
        );
        let response = SubresourceResponseStarted::new(
            handle,
            response.redirect_chain,
            response.final_url,
            response.status,
            response.headers,
            Vec::new(),
        );
        for items in [
            vec![ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
                complete,
            ))],
            vec![
                ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(started)),
                ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
                ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(
                    SubresourceBodyFinished::ready(
                        handle,
                        SubresourceResponseBody::from_bytes(b"done".to_vec()),
                    ),
                )),
            ],
        ] {
            let events = emitted_events_for(items)
                .into_iter()
                .filter_map(|event| {
                    let message = event.into_parts().0;
                    (message["method"] == "Network.requestWillBeSent").then_some(message)
                })
                .collect::<Vec<_>>();
            assert_eq!(events.len(), 4);
            assert_eq!(
                events[0]["params"]["request"]["method"], "PATCH",
                "Started remains the pre-interception request"
            );
            for (event, (method, headers, body)) in events[1..].iter().zip(&requests[1..]) {
                let request = &event["params"]["request"];
                assert_eq!(request["method"], method.as_str());
                assert_eq!(
                    request.get("postData").and_then(|value| value.as_str()),
                    (!body.is_empty()).then(|| std::str::from_utf8(body).unwrap())
                );
                let projected = request["headers"].as_object().unwrap();
                for name in ["x-once", "x-persistent", "content-type"] {
                    let value = projected
                        .iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case(name))
                        .and_then(|(_, value)| value.as_str());
                    assert_eq!(
                        value,
                        headers.get(name).map(|value| value.to_str().unwrap()),
                        "header {name}"
                    );
                }
            }
        }
    }

    #[test]
    fn backlog_projection_context_wraps_contextual_subresource_preferred_request_id() {
        let owner = CommandOwnerScope::for_session("SID-1");
        let context = NetworkBacklogProjectionContext::for_owner(&owner)
            .with_frame_id(Some("FRAME-1"))
            .with_base_timestamp(Some(12.5))
            .with_contextual_subresource_request_id(Some("REQ-prepared"));

        assert_eq!(context.session_id(), Some("SID-1"));
        assert_eq!(context.frame_id, Some("FRAME-1"));
        assert_eq!(context.base_timestamp, Some(12.5));
        assert_eq!(
            context.preferred_request_id,
            Some(NetworkBacklogPreferredRequestId::contextual_subresource(
                "REQ-prepared"
            )),
            "protocol output projection should not construct preferred request id variants directly"
        );
    }

    #[test]
    fn backlog_projection_context_carries_captured_outputs_slot() {
        let mut prepared_outputs = NetworkPreparedOutputs::default();
        let owner = CommandOwnerScope::for_session("SID-1");
        let context = NetworkBacklogProjectionContext::for_owner(&owner)
            .with_prepared_outputs(Some(&mut prepared_outputs));

        assert!(
            context.prepared_outputs.is_some(),
            "Network backlog context should carry the prepared output slot instead of exposing it as a separate emitter parameter"
        );
    }

    fn emitted_request_methods_for_redirects(
        initial_method: &str,
        redirect_statuses: &[u16],
    ) -> Vec<String> {
        let document_url = Url::parse("https://example.test/").unwrap();
        let start_url = Url::parse("https://example.test/start").unwrap();
        let mut from_url = start_url.clone();
        let mut dispatched =
            moli_fetch::Request::new(initial_method, start_url.as_str(), None, Vec::new()).unwrap();
        let redirects = redirect_statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let to_url = Url::parse(&format!("https://example.test/redirect-{index}")).unwrap();
                let redirect = moli_fetch::RedirectInfo {
                    source: moli_fetch::RedirectSource::Network,
                    from_url: from_url.clone(),
                    to_url: to_url.clone(),
                    status: *status,
                    headers: Vec::new(),
                    network_extra_info_available: true,
                    request_extra_info: None,
                    dispatched_request: None,
                    response_extra_info: None,
                    redirect_has_extra_info: true,
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                };
                from_url = to_url;
                dispatched.follow_redirect(redirect).unwrap();
                dispatched.redirect_chain().last().unwrap().clone().into()
            })
            .collect();
        let handle = SubresourceNetworkRequestHandle::new(1);
        let request = SubresourceRequestStarted::new(
            handle,
            Some("FRAME-1".to_owned()),
            document_url,
            start_url,
            initial_method.to_owned(),
            Vec::new().into(),
            None,
            SubresourceResourceType::Xhr,
            SubresourceRequestInitiatorType::Script,
            None,
        );
        let response = SubresourceResponseStarted::new(
            handle,
            redirects,
            from_url,
            200,
            Vec::new(),
            Vec::new(),
        );
        let body =
            SubresourceBodyFinished::ready(handle, SubresourceResponseBody::from_bytes(Vec::new()));
        let items = vec![
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
            ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
            ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(body)),
        ];
        emitted_events_for(items)
            .into_iter()
            .filter(|event| event.protocol_method() == Some("Network.requestWillBeSent"))
            .map(|event| {
                event.into_parts().0["params"]["request"]["method"]
                    .as_str()
                    .expect("request method")
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn redirected_request_events_use_the_effective_method_for_each_hop() {
        assert_eq!(
            emitted_request_methods_for_redirects("POST", &[302, 307, 308]),
            ["POST", "GET", "GET", "GET"]
        );
        assert_eq!(
            emitted_request_methods_for_redirects("POST", &[303]),
            ["POST", "GET"]
        );
        assert_eq!(
            emitted_request_methods_for_redirects("HEAD", &[303]),
            ["HEAD", "HEAD"]
        );
        assert_eq!(
            emitted_request_methods_for_redirects("POST", &[307, 308]),
            ["POST", "POST", "POST"]
        );
    }

    #[test]
    fn successful_complete_and_staged_xhr_terminals_keep_their_internal_resource_type() {
        let document_url = Url::parse("https://example.test/").unwrap();
        let request_url = Url::parse("https://example.test/api").unwrap();
        let complete = SubresourceNetworkRecord::success_with_body(
            Some("FRAME-1".to_owned()),
            document_url.clone(),
            request_url.clone(),
            "GET".to_owned(),
            Vec::new().into(),
            None,
            SubresourceResourceType::Xhr,
            None,
            Vec::new(),
            request_url.clone(),
            200,
            Vec::new(),
            SubresourceResponseBody::from_bytes(b"complete".to_vec()),
            Vec::new(),
        );
        assert_xhr_terminal_retains_internal_resource_type(vec![
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(complete)),
        ]);

        let handle = SubresourceNetworkRequestHandle::new(1);
        let request = SubresourceRequestStarted::new(
            handle,
            Some("FRAME-1".to_owned()),
            document_url,
            request_url.clone(),
            "GET".to_owned(),
            Vec::new().into(),
            None,
            SubresourceResourceType::Xhr,
            SubresourceRequestInitiatorType::Script,
            None,
        );
        let response = SubresourceResponseStarted::new(
            handle,
            Vec::new(),
            request_url,
            200,
            Vec::new(),
            Vec::new(),
        );
        let body = SubresourceBodyFinished::ready(
            handle,
            SubresourceResponseBody::from_bytes(b"staged".to_vec()),
        );
        assert_xhr_terminal_retains_internal_resource_type(vec![
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
            ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
            ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(body)),
        ]);
    }
}
