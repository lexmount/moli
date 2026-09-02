use moli_core::page::{SubresourceBodyFinishedResult, SubresourceResponseBody};

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, FetchRequestStage, monotonic_timestamp_seconds,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
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
    pub(in crate::domains::network) session_id: Option<&'a str>,
    pub(in crate::domains::network) frame_id: Option<&'a str>,
    pub(in crate::domains::network) base_timestamp: Option<f64>,
    pub(in crate::domains::network) preferred_request_id:
        Option<NetworkBacklogPreferredRequestId<'a>>,
    pub(in crate::domains::network) prepared_outputs: Option<&'a mut NetworkPreparedOutputs>,
}

impl<'a> NetworkBacklogProjectionContext<'a> {
    pub(crate) fn new(session_id: Option<&'a str>) -> Self {
        Self {
            session_id,
            frame_id: None,
            base_timestamp: None,
            preferred_request_id: None,
            prepared_outputs: None,
        }
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

fn emit_subresource_network_delivery_record(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    session_id: Option<&str>,
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
                session_id,
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
                session_id,
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
                session_id,
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
                session_id,
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
    session_id: Option<&str>,
    frame_id: &str,
    base_timestamp: f64,
) -> bool {
    let initial_output_len = out.len();
    let request_id = delivery_output.request_id();
    let output = delivery_output.metadata();
    if event_session_ids.is_empty() {
        return false;
    }
    let record_frame_id = output.frame_id().unwrap_or(frame_id);
    let loader_id = output.loader_id();
    let record_document_url = output.document_url();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    let resource_type = output.resource_type().into();
    let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(session_id) else {
        return false;
    };
    let request_was_announced_by_fetch_pause =
        runtime_slot.take_fetch_pause_announced_request_id(request_id);
    if !runtime_slot.claim_completed_subresource_request_id(request_id) {
        return false;
    }
    if output.resource_type() != moli_core::page::SubresourceResourceType::WebSocket {
        record_subresource_pending_response_body(conn, session_id, request_id, event_session_ids);
        record_subresource_request_body(
            conn,
            session_id,
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
            for redirect in redirect_chain {
                for event_session_id in event_session_ids {
                    emit_request_will_be_sent(
                        out,
                        event_session_id.as_deref(),
                        request_id,
                        record_frame_id,
                        loader_id,
                        timestamp,
                        record_document_url,
                        &redirect.to_url,
                        output.method(),
                        output.request_body(),
                        output.request_headers(),
                        resource_type,
                        output.request_initiator_type(),
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
            for event_session_id in event_session_ids {
                let blocked_intercepts = matching_subresource_network_intercepts(
                    conn,
                    session_id,
                    FetchRequestStage::Response,
                    resource_type,
                    final_url,
                );
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
                    &blocked_intercepts,
                    None,
                );
            }
            if let Some(response_body) = output.response_body() {
                record_subresource_response_body_source(
                    conn,
                    session_id,
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
            if output.resource_type() != moli_core::page::SubresourceResourceType::WebSocket {
                record_subresource_failed_response_body(
                    conn,
                    session_id,
                    error_text.clone(),
                    request_id,
                    event_session_ids,
                );
            }
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
    session_id: Option<&str>,
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
    let request_was_announced_by_fetch_pause = conn
        .runtime_session_owner_slot_mut(session_id)
        .is_ok_and(|runtime_slot| runtime_slot.take_fetch_pause_announced_request_id(request_id));
    if output.resource_type() != moli_core::page::SubresourceResourceType::WebSocket {
        record_subresource_pending_response_body(conn, session_id, request_id, event_session_ids);
        record_subresource_request_body(
            conn,
            session_id,
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
    session_id: Option<&str>,
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
    let record_frame_id = request.frame_id().unwrap_or(frame_id);
    let loader_id = request.loader_id();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    let resource_type = request.resource_type().into();
    for redirect in output.redirect_chain() {
        for event_session_id in event_session_ids {
            emit_request_will_be_sent(
                out,
                event_session_id.as_deref(),
                request_id,
                record_frame_id,
                loader_id,
                timestamp,
                request.document_url(),
                &redirect.to_url,
                request.method(),
                request.request_body(),
                request.request_headers(),
                resource_type,
                request.request_initiator_type(),
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
    for event_session_id in event_session_ids {
        let blocked_intercepts = matching_subresource_network_intercepts(
            conn,
            session_id,
            FetchRequestStage::Response,
            resource_type,
            output.final_url(),
        );
        let fetch_request_id = (!blocked_intercepts.is_empty())
            .then(|| {
                conn.in_flight_subresource_fetch_request_id_for_session_owner(
                    session_id,
                    output.handle().get(),
                )
            })
            .flatten();
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
    session_id: Option<&str>,
    request_stage: FetchRequestStage,
    resource_type: DevToolsNetworkResourceType,
    url: &url::Url,
) -> Vec<DevToolsNetworkInterceptId> {
    conn.target_fetch_subresource_interception_snapshot_for_session_owner(session_id)
        .map(|snapshot| snapshot.matching_network_intercepts(request_stage, resource_type, url))
        .unwrap_or_default()
}

fn emit_staged_subresource_body_finished(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    delivery_output: &TargetSubresourceBodyNetworkDeliveryOutput,
    event_session_ids: &[Option<String>],
    session_id: Option<&str>,
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
    let record_frame_id = request.frame_id().unwrap_or(frame_id);
    let loader_id = request.loader_id();
    let timestamp = base_timestamp + ((output.index() + 1) as f64 * 0.000_001);
    let resource_type = request.resource_type().into();
    match output.result() {
        SubresourceBodyFinishedResult::Ready(response_body) => {
            let response_body_len = response_body.len();
            record_subresource_response_body_source(
                conn,
                session_id,
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
                session_id,
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
                session_id,
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
        session_id,
        frame_id,
        base_timestamp,
        preferred_request_id,
        prepared_outputs,
    } = context;
    let primary_session_id = conn.runtime_session_owner_primary_session_id(session_id);
    let Some((frame_id, snapshot)) = (|| {
        let frame_id = frame_id
            .map(str::to_owned)
            .or_else(|| conn.target_owner_identity_for_session(session_id)?.1)?;
        let snapshot = pending_network_backlog_delivery_snapshot(
            conn,
            session_id,
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
        session_id,
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
    session_id: Option<&str>,
    prepared: &mut TargetNetworkBacklogPreparedDelivery,
) {
    let Some((frame_id, snapshot)) = (|| {
        let frame_id = conn.target_owner_identity_for_session(session_id)?.1?;
        let snapshot = conn
            .runtime_session_owner_slot_mut(session_id)
            .ok()?
            .pending_network_backlog_delivery_snapshot_from_backlog(prepared)?;
        Some((frame_id, snapshot))
    })() else {
        return;
    };
    emit_network_delivery_snapshot(
        conn,
        out,
        session_id,
        &frame_id,
        monotonic_timestamp_seconds(),
        snapshot,
    );
}

fn emit_network_delivery_snapshot(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
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
                    session_id,
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
    mark_network_backlog_delivery_snapshot_emitted(conn, session_id, &snapshot);
}

fn pending_network_backlog_delivery_snapshot(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    primary_session_id: Option<&str>,
    preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
    prepared_network_outputs: Option<&mut NetworkPreparedOutputs>,
) -> Option<PendingNetworkBacklogDeliverySnapshot> {
    if let Some(prepared_network_outputs) = prepared_network_outputs {
        let runtime_slot = conn.runtime_session_owner_slot_mut(session_id).ok()?;
        return runtime_slot.pending_network_backlog_delivery_snapshot_from_backlog(
            prepared_network_outputs.backlog_mut(),
        );
    }
    conn.pending_network_backlog_delivery_snapshot_for_session_owner(
        session_id,
        session_id,
        primary_session_id,
        preferred_request_id,
    )
}

fn mark_network_backlog_delivery_snapshot_emitted(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    snapshot: &PendingNetworkBacklogDeliverySnapshot,
) {
    let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(session_id) else {
        return;
    };
    runtime_slot.mark_network_backlog_delivery_snapshot_emitted(snapshot);
}

fn record_subresource_response_body_source(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    response_body: &SubresourceResponseBody,
    request_id: &str,
    session_ids: &[Option<String>],
) {
    let captured_body = crate::conn::CapturedBody::from_subresource_response_body(response_body);
    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Response;
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        session_id,
        data_type,
        captured_body.len(),
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(data_type);
    conn.record_collected_network_data_body(
        request_id.to_owned(),
        data_type,
        captured_body.clone(),
        collector_ids.iter().cloned(),
        collection_was_gated,
    );
    let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(session_id) else {
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

fn record_subresource_request_body(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    request_id: &str,
    request_body: Option<&[u8]>,
    session_ids: &[Option<String>],
) {
    let Some(request_body) = request_body else {
        return;
    };
    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Request;
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        session_id,
        data_type,
        request_body.len(),
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(data_type);
    conn.record_collected_network_data_body(
        request_id.to_owned(),
        data_type,
        crate::conn::CapturedBody::from_bytes(request_body.to_vec()),
        collector_ids.iter().cloned(),
        collection_was_gated,
    );
    let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(session_id) else {
        return;
    };
    runtime_slot.record_captured_request_body_with_collector_scope(
        request_id.to_owned(),
        request_body.to_vec(),
        session_ids.to_vec(),
        collector_ids,
        collection_was_gated,
    );
}

fn record_subresource_pending_response_body(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    request_id: &str,
    session_ids: &[Option<String>],
) {
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        session_id,
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        0,
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
    );
    let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(session_id) else {
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
    session_id: Option<&str>,
    error_text: String,
    request_id: &str,
    session_ids: &[Option<String>],
) {
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        session_id,
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        0,
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
    );
    let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(session_id) else {
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
        ScriptNetworkOutputItem, SubresourceBodyFinished, SubresourceNetworkRecord,
        SubresourceNetworkRequestHandle, SubresourceRequestInitiatorType,
        SubresourceRequestStarted, SubresourceResourceType, SubresourceResponseBody,
        SubresourceResponseStarted,
    };
    use url::Url;

    use crate::{
        conn::CdpConnection,
        devtools_runtime::{AutomationEvent, DevToolsNetworkResourceType},
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

    fn emitted_terminal_for(items: Vec<ScriptNetworkOutputItem>) -> crate::BackgroundProtocolEvent {
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
        emit_network_delivery_snapshot(&mut conn, &mut events, None, "FRAME-1", 1.0, snapshot);
        events
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
    fn backlog_projection_context_wraps_contextual_subresource_preferred_request_id() {
        let context = NetworkBacklogProjectionContext::new(Some("SID-1"))
            .with_frame_id(Some("FRAME-1"))
            .with_base_timestamp(Some(12.5))
            .with_contextual_subresource_request_id(Some("REQ-prepared"));

        assert_eq!(context.session_id, Some("SID-1"));
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
        let context = NetworkBacklogProjectionContext::new(Some("SID-1"))
            .with_prepared_outputs(Some(&mut prepared_outputs));

        assert!(
            context.prepared_outputs.is_some(),
            "Network backlog context should carry the prepared output slot instead of exposing it as a separate emitter parameter"
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
            Vec::new(),
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
            Vec::new(),
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
