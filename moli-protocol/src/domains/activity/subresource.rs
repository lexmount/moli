use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, ClaimedSubresourceContinueRequest, CommandOwnerScope,
    FetchAuthChallenge, FetchRequestStage, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchAuthStage, PendingSubresourceFetchAuthStageChain,
    PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest, PendingSubresourceFetchResponseStage,
    PendingSubresourceFetchResponseStageChain, TargetPageResidenceIdentity,
};
use crate::devtools_runtime::DevToolsNetworkInterceptId;
use crate::domains::fetch;
use moli_core::page::{
    PendingSubresourceAuthInfo, PendingSubresourceContinueEvent, PendingSubresourceResponseInfo,
    SubresourceNetworkRequestHandle,
};

use super::contextual_projection::MainDocumentBodyCompleteProjection;
use crate::domains::network::{
    MainDocumentProgressGate, NetworkBacklogPreferredRequestId, NetworkBacklogProjectionContext,
    emit_pending_network_backlog_activity_background_events,
    network_backlog_prepared_outputs_for_owner,
};

#[derive(Debug)]
pub(crate) struct PreparedSubresourceContinueAction {
    owner: TargetPageResidenceIdentity,
    event: PendingSubresourceContinueEvent,
    request: Option<ClaimedSubresourceContinueRequest>,
}

impl PreparedSubresourceContinueAction {
    fn internal_id(event: &PendingSubresourceContinueEvent) -> u64 {
        match event {
            PendingSubresourceContinueEvent::Completed { internal_id } => *internal_id,
            PendingSubresourceContinueEvent::ResponsePaused(info) => info.internal_id,
            PendingSubresourceContinueEvent::AuthRequired(info) => info.internal_id,
        }
    }

    fn capture(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        owner: TargetPageResidenceIdentity,
        event: PendingSubresourceContinueEvent,
    ) -> Self {
        let internal_id = Self::internal_id(&event);
        let request = conn.claim_subresource_continue_request_for_session_owner(
            session_id,
            &owner,
            internal_id,
            matches!(&event, PendingSubresourceContinueEvent::Completed { .. }),
        );
        Self {
            owner,
            event,
            request,
        }
    }

    #[cfg(test)]
    pub(in crate::domains) fn capture_for_test(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        event: PendingSubresourceContinueEvent,
    ) -> Option<Self> {
        let owner = conn.target_page_residence_identity_for_session(session_id)?;
        Some(Self::capture(conn, session_id, owner, event))
    }
}

pub(in crate::domains) fn prepare_subresource_continue_action_for_renderer_record(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    source_document: moli_core::RendererDocumentLifecycleIdentity,
    event: PendingSubresourceContinueEvent,
) -> Option<PreparedSubresourceContinueAction> {
    if conn.target_root_document_lifecycle_identity_for_session(session_id) != Some(source_document)
    {
        return None;
    }
    let owner = conn.target_page_residence_identity_for_session(session_id)?;
    Some(PreparedSubresourceContinueAction::capture(
        conn, session_id, owner, event,
    ))
}

pub(in crate::domains) async fn flush_prepared_subresource_continue_actions_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    actions: Vec<PreparedSubresourceContinueAction>,
) {
    for action in actions {
        if conn.target_page_residence_identity_is_current_for_session(session_id, &action.owner) {
            flush_prepared_subresource_continue_action_background_events_async(
                conn, out, session_id, action,
            )
            .await;
        }
    }
}

async fn flush_prepared_subresource_continue_action_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    action: PreparedSubresourceContinueAction,
) {
    let PreparedSubresourceContinueAction { event, request, .. } = action;
    let fetch_session_id = conn.target_fetch_event_session_id_for_session_owner(session_id);
    let fetch_event_session_id = fetch_session_id.as_deref().or(session_id);

    match event {
        PendingSubresourceContinueEvent::Completed { .. } => {
            let pending = match request {
                Some(ClaimedSubresourceContinueRequest::InFlight(in_flight)) => {
                    Some(in_flight.pending)
                }
                Some(ClaimedSubresourceContinueRequest::PendingCompletion(pending)) => {
                    Some(pending)
                }
                None => None,
            };
            if let Some(pending) = pending {
                Box::pin(
                    flush_post_subresource_fetch_request_activity_background_events_async(
                        conn, out, session_id, &pending,
                    ),
                )
                .await;
            }
        }
        PendingSubresourceContinueEvent::ResponsePaused(response_info) => {
            let Some(ClaimedSubresourceContinueRequest::InFlight(in_flight)) = request else {
                return;
            };
            let Some(request_id) = in_flight.request_id else {
                let _ = conn
                    .continue_pending_subresource_response_for_session_owner_async(
                        session_id,
                        response_info.internal_id,
                        None,
                        None,
                    )
                    .await;
                Box::pin(
                    flush_post_subresource_fetch_request_activity_background_events_async(
                        conn,
                        out,
                        session_id,
                        &in_flight.pending,
                    ),
                )
                .await;
                return;
            };
            if in_flight
                .response_stage_url_match_policy
                .requires_final_url_match()
                && !conn
                    .target_fetch_subresource_interception_snapshot_for_session_owner(session_id)
                    .is_some_and(|snapshot| {
                        snapshot.matches_response_stage(
                            response_info.resource_type.into(),
                            &response_info.final_url,
                        )
                    })
            {
                let _ = conn
                    .continue_pending_subresource_response_for_session_owner_async(
                        session_id,
                        response_info.internal_id,
                        None,
                        None,
                    )
                    .await;
                Box::pin(
                    flush_post_subresource_fetch_request_activity_background_events_async(
                        conn,
                        out,
                        session_id,
                        &in_flight.pending,
                    ),
                )
                .await;
                return;
            }
            let blocked_intercepts = if in_flight.response_stage_blocked_intercepts.is_empty() {
                conn.target_fetch_subresource_interception_snapshot_for_session_owner(session_id)
                    .map(|snapshot| {
                        snapshot.matching_network_intercepts(
                            FetchRequestStage::Response,
                            response_info.resource_type.into(),
                            &response_info.final_url,
                        )
                    })
                    .unwrap_or_default()
            } else {
                in_flight.response_stage_blocked_intercepts.clone()
            };
            let blocked_intercepts = if blocked_intercepts.is_empty() {
                conn.target_fetch_matching_network_intercepts_for_target(
                    &in_flight.pending.frame_id,
                    FetchRequestStage::Response,
                    response_info.resource_type.into(),
                    &response_info.final_url,
                )
            } else {
                blocked_intercepts
            };
            let mut pending_response = pending_response_request(&in_flight.pending, &response_info);
            let mut response_event_session_id = fetch_event_session_id.map(str::to_owned);
            let mut response_event_blocked_intercepts = blocked_intercepts.clone();
            let response_stage_sessions = conn
                .target_fetch_subresource_interception_snapshot_for_target(
                    &in_flight.pending.frame_id,
                )
                .or_else(|| {
                    conn.target_fetch_subresource_interception_snapshot_for_session_owner(
                        session_id,
                    )
                })
                .map(|snapshot| {
                    snapshot.matching_response_stage_pause_sessions(
                        session_id,
                        response_info.resource_type.into(),
                        &response_info.final_url,
                    )
                })
                .unwrap_or_default();
            if let Some(first_pause_session) = response_stage_sessions.first().cloned() {
                pending_response.owner_session_id = routable_stage_owner_session_id(
                    conn,
                    session_id,
                    first_pause_session.session_id.as_deref(),
                );
                pending_response.owner_kind = first_pause_session.owner_kind;
                response_event_session_id = first_pause_session.session_id.clone();
                response_event_blocked_intercepts = stage_blocked_intercepts(
                    first_pause_session.owner_kind,
                    first_pause_session.blocked_intercepts,
                    &blocked_intercepts,
                );
                let mut remaining_sessions = Vec::new();
                for session in response_stage_sessions.into_iter().skip(1) {
                    let Ok((next_request_id, _)) = conn
                        .allocate_pending_subresource_fetch_request_ids_for_session_owner(
                            session_id,
                        )
                    else {
                        return;
                    };
                    remaining_sessions.push(PendingSubresourceFetchResponseStage {
                        session_id: session.session_id,
                        owner_kind: session.owner_kind,
                        request_id: next_request_id,
                        blocked_intercepts: stage_blocked_intercepts(
                            session.owner_kind,
                            session.blocked_intercepts,
                            &blocked_intercepts,
                        ),
                    });
                }
                if !remaining_sessions.is_empty() {
                    pending_response.response_stage_chain =
                        Some(Box::new(PendingSubresourceFetchResponseStageChain {
                            remaining_sessions,
                        }));
                }
            }
            pending_response.action_session_id = response_event_session_id.clone();
            let pending_owner_session_id = pending_response.owner_session_id.clone();
            let pending_owner_session_id = pending_owner_session_id.as_deref().or(session_id);
            if !conn.register_pending_subresource_fetch_response_request_for_session_owner(
                pending_owner_session_id,
                request_id.clone(),
                pending_response.clone(),
            ) {
                return;
            }
            out.push(
                fetch::pending_subresource_response_stage_request_paused_event(
                    response_event_session_id.as_deref(),
                    &request_id,
                    &pending_response,
                    &response_event_blocked_intercepts,
                ),
            );
        }
        PendingSubresourceContinueEvent::AuthRequired(auth_info) => {
            let Some(ClaimedSubresourceContinueRequest::InFlight(in_flight)) = request else {
                return;
            };
            let Some(request_id) = in_flight.request_id else {
                let _ = conn
                    .fail_pending_subresource_auth_for_session_owner_async(
                        session_id,
                        auth_info.internal_id,
                        "Fetch auth challenge has no DevTools request".to_owned(),
                    )
                    .await;
                Box::pin(
                    flush_post_subresource_fetch_request_activity_background_events_async(
                        conn,
                        out,
                        session_id,
                        &in_flight.pending,
                    ),
                )
                .await;
                return;
            };
            let blocked_intercepts = conn
                .target_fetch_subresource_interception_snapshot_for_session_owner(session_id)
                .map(|snapshot| snapshot.matching_auth_required_network_intercepts(&auth_info.url))
                .unwrap_or_default();
            let blocked_intercepts = if blocked_intercepts.is_empty() {
                conn.target_fetch_matching_auth_required_network_intercepts_for_target(
                    &in_flight.pending.frame_id,
                    &auth_info.url,
                )
            } else {
                blocked_intercepts
            };
            let mut pending_auth = pending_auth_request(&in_flight.pending, &auth_info);
            fetch::populate_auth_challenge_origin(
                conn,
                session_id,
                &pending_auth.url,
                &mut pending_auth.challenge,
            );
            let mut auth_event_session_id = fetch_event_session_id.map(str::to_owned);
            let mut auth_event_blocked_intercepts = blocked_intercepts.clone();
            let auth_sessions = conn
                .target_fetch_subresource_interception_snapshot_for_target(
                    &in_flight.pending.frame_id,
                )
                .or_else(|| {
                    conn.target_fetch_subresource_interception_snapshot_for_session_owner(
                        session_id,
                    )
                })
                .map(|snapshot| {
                    snapshot.matching_auth_required_pause_sessions(session_id, &auth_info.url)
                })
                .unwrap_or_default();
            if let Some(first_pause_session) = auth_sessions.first().cloned() {
                pending_auth.owner_session_id = routable_stage_owner_session_id(
                    conn,
                    session_id,
                    first_pause_session.session_id.as_deref(),
                );
                pending_auth.owner_kind = first_pause_session.owner_kind;
                auth_event_session_id = first_pause_session.session_id.clone();
                auth_event_blocked_intercepts = stage_blocked_intercepts(
                    first_pause_session.owner_kind,
                    first_pause_session.blocked_intercepts,
                    &blocked_intercepts,
                );
                let mut remaining_sessions = Vec::new();
                for session in auth_sessions.into_iter().skip(1) {
                    let Ok((next_request_id, _)) = conn
                        .allocate_pending_subresource_fetch_request_ids_for_session_owner(
                            session_id,
                        )
                    else {
                        return;
                    };
                    remaining_sessions.push(PendingSubresourceFetchAuthStage {
                        session_id: session.session_id,
                        owner_kind: session.owner_kind,
                        request_id: next_request_id,
                        blocked_intercepts: stage_blocked_intercepts(
                            session.owner_kind,
                            session.blocked_intercepts,
                            &blocked_intercepts,
                        ),
                    });
                }
                if !remaining_sessions.is_empty() {
                    pending_auth.auth_stage_chain =
                        Some(Box::new(PendingSubresourceFetchAuthStageChain {
                            remaining_sessions,
                        }));
                }
            }
            pending_auth.action_session_id = auth_event_session_id.clone();
            let pending_owner_session_id = pending_auth.owner_session_id.clone();
            let pending_owner_session_id = pending_owner_session_id.as_deref().or(session_id);
            if !conn.register_pending_subresource_fetch_auth_request_for_session_owner(
                pending_owner_session_id,
                request_id.clone(),
                pending_auth.clone(),
            ) {
                return;
            }
            out.push(fetch::pending_subresource_auth_required_event(
                auth_event_session_id.as_deref(),
                &request_id,
                &pending_auth,
                &auth_event_blocked_intercepts,
            ));
        }
    }
}

fn routable_stage_owner_session_id(
    conn: &CdpConnection,
    current_owner_session_id: Option<&str>,
    stage_session_id: Option<&str>,
) -> Option<String> {
    if stage_session_id.is_some_and(|session_id| conn.session_route(Some(session_id)).is_some()) {
        return stage_session_id.map(str::to_owned);
    }
    current_owner_session_id.map(str::to_owned)
}

fn stage_blocked_intercepts(
    owner_kind: PendingSubresourceFetchOwnerKind,
    stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    fallback_blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> Vec<DevToolsNetworkInterceptId> {
    if !stage_blocked_intercepts.is_empty() {
        return stage_blocked_intercepts;
    }
    match owner_kind {
        PendingSubresourceFetchOwnerKind::Fetch => Vec::new(),
        PendingSubresourceFetchOwnerKind::NetworkOrBidi => fallback_blocked_intercepts.to_vec(),
    }
}

fn pending_auth_request(
    pending: &PendingSubresourceFetchRequest,
    auth_info: &PendingSubresourceAuthInfo,
) -> PendingSubresourceFetchAuthRequest {
    PendingSubresourceFetchAuthRequest {
        page_owner: pending
            .installed_page_owner()
            .expect("an in-flight auth continuation must belong to an installed Page")
            .clone(),
        owner_session_id: pending.owner_session_id.clone(),
        action_session_id: pending.action_session_id.clone(),
        owner_kind: pending.owner_kind,
        internal_id: pending.internal_id,
        network_request_id: pending.network_request_id.clone(),
        network_request_handle: pending.network_request_handle,
        frame_id: pending.frame_id.clone(),
        document_url: pending.document_url.clone(),
        resource_type: pending.resource_type,
        websocket_socket_id: pending.websocket_socket_id,
        url: auth_info.url.clone(),
        method: auth_info.method.clone(),
        request_headers: auth_info.request_headers.clone(),
        request_body: auth_info.request_body.clone(),
        request_cookie_report: auth_info.request_cookie_report.clone(),
        challenge: FetchAuthChallenge {
            origin: String::new(),
            source: auth_info.challenge.source.clone(),
            scheme: auth_info.challenge.scheme.clone(),
            realm: auth_info.challenge.realm.clone(),
        },
        intercept_response: auth_info.intercept_response,
        auth_stage_chain: None,
    }
}

fn pending_response_request(
    pending: &PendingSubresourceFetchRequest,
    response_info: &PendingSubresourceResponseInfo,
) -> PendingSubresourceFetchResponseRequest {
    PendingSubresourceFetchResponseRequest {
        page_owner: pending
            .installed_page_owner()
            .expect("an in-flight response continuation must belong to an installed Page")
            .clone(),
        owner_session_id: pending.owner_session_id.clone(),
        action_session_id: pending.action_session_id.clone(),
        owner_kind: pending.owner_kind,
        internal_id: pending.internal_id,
        network_request_id: pending.network_request_id.clone(),
        network_request_handle: pending.network_request_handle,
        frame_id: pending.frame_id.clone(),
        document_url: pending.document_url.clone(),
        resource_type: pending.resource_type,
        websocket_socket_id: pending.websocket_socket_id,
        url: response_info.final_url.clone(),
        method: response_info.method.clone(),
        request_headers: response_info.request_headers.clone(),
        request_body: response_info.request_body.clone(),
        request_cookie_report: response_info.request_cookie_report.clone(),
        response_status: response_info.response_status,
        response_headers: response_info.response_headers.clone(),
        response_head_overridden: false,
        response_body_taken_as_stream: false,
        response_body: crate::conn::CapturedBody::from_subresource_response_body(
            &response_info.response_body,
        ),
        response_stage_chain: None,
    }
}

pub(crate) async fn flush_post_subresource_fetch_request_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    pending: &PendingSubresourceFetchRequest,
) {
    let owner = CommandOwnerScope::capture(conn, session_id);
    flush_post_subresource_fetch_request_activity_for_owner_background_events_async(
        conn, out, &owner, pending,
    )
    .await;
}

pub(crate) async fn flush_post_subresource_fetch_request_activity_for_owner_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    pending: &PendingSubresourceFetchRequest,
) {
    record_subresource_network_request_handle(
        conn,
        owner,
        &pending.network_request_id,
        pending.network_request_handle,
    );
    project_network_backlog_for_owner(
        conn,
        out,
        owner,
        &pending.frame_id,
        Some(&pending.network_request_id),
    );
    clear_fetch_pause_network_announcement(conn, owner, &pending.network_request_id);
}

pub(crate) async fn flush_post_subresource_response_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    pending: &PendingSubresourceFetchResponseRequest,
) {
    let owner = CommandOwnerScope::capture(conn, session_id);
    flush_post_subresource_response_activity_for_owner_background_events_async(
        conn, out, &owner, pending,
    )
    .await;
}

pub(crate) async fn flush_post_subresource_response_activity_for_owner_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    pending: &PendingSubresourceFetchResponseRequest,
) {
    record_subresource_network_request_handle(
        conn,
        owner,
        &pending.network_request_id,
        pending.network_request_handle,
    );
    flush_post_subresource_network_activity_background_events_async(
        conn,
        out,
        owner,
        &pending.frame_id,
        &pending.network_request_id,
    )
    .await;
}

pub(crate) async fn flush_post_subresource_auth_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    pending: &PendingSubresourceFetchAuthRequest,
) {
    let owner = CommandOwnerScope::capture(conn, session_id);
    flush_post_subresource_auth_activity_for_owner_background_events_async(
        conn, out, &owner, pending,
    )
    .await;
}

pub(crate) async fn flush_post_subresource_auth_activity_for_owner_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    pending: &PendingSubresourceFetchAuthRequest,
) {
    record_subresource_network_request_handle(
        conn,
        owner,
        &pending.network_request_id,
        pending.network_request_handle,
    );
    flush_post_subresource_network_activity_background_events_async(
        conn,
        out,
        owner,
        &pending.frame_id,
        &pending.network_request_id,
    )
    .await;
}

async fn flush_post_subresource_network_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    frame_id: &str,
    network_request_id: &str,
) {
    conn.ingest_runtime_session_owner_output_updates_for_route(
        owner.session_id(),
        owner.session_owner_route(),
    );
    project_network_backlog_for_owner(conn, out, owner, frame_id, Some(network_request_id));
    clear_fetch_pause_network_announcement(conn, owner, network_request_id);
}

fn project_network_backlog_for_owner(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    frame_id: &str,
    network_request_id: Option<&str>,
) {
    let preferred_request_id =
        network_request_id.map(NetworkBacklogPreferredRequestId::contextual_subresource);
    let mut prepared =
        network_backlog_prepared_outputs_for_owner(conn, owner, preferred_request_id);
    emit_pending_network_backlog_activity_background_events(
        conn,
        out,
        NetworkBacklogProjectionContext::for_owner(owner)
            .with_frame_id(Some(frame_id))
            .with_contextual_subresource_request_id(network_request_id)
            .with_prepared_outputs(Some(&mut prepared)),
    );
}

fn clear_fetch_pause_network_announcement(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
) {
    if let Ok(runtime_slot) = conn
        .runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
    {
        runtime_slot.clear_fetch_pause_announced_request_id(request_id);
    }
}

fn record_subresource_network_request_handle(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
    request_handle: Option<SubresourceNetworkRequestHandle>,
) {
    let Some(request_handle) = request_handle else {
        return;
    };
    if let Ok(runtime_slot) = conn
        .runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
    {
        runtime_slot.record_subresource_request_id_for_handle_if_absent(
            request_handle,
            request_id.to_owned(),
        );
    }
}

pub(super) fn flush_main_document_body_complete_activity_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    progress_gate: &mut MainDocumentProgressGate,
) {
    MainDocumentBodyCompleteProjection::new(progress_gate).project_background_events(out);
}
