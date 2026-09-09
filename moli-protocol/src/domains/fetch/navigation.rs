use crate::conn::{
    CdpConnection, CommandOwnerScope, FetchAuthChallenge, PendingFetchNavigation,
    PendingSubresourceFetchAuthStage, PendingSubresourceFetchAuthStageChain,
    PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use moli_cookie_jar::StoredCookieQueryReport;
use moli_core::page::SubresourceResourceType;
use url::Url;

use super::helpers::pending_fetch_auth_navigation_required_event;

pub(crate) fn prepare_navigation_response_stage(
    conn: &CdpConnection,
    pending: &mut PendingFetchNavigation,
    final_url: &Url,
) -> bool {
    if !pending.intercept_response {
        return false;
    }
    if !pending
        .response_stage_url_match_policy
        .requires_final_url_match()
    {
        return true;
    }
    let Some(response_stage) = conn
        .target_fetch_subresource_interception_snapshot_for_owner(&pending.navigation.owner)
        .and_then(|snapshot| {
            snapshot
                .matching_response_stage_pause_sessions(
                    pending.navigation.owner.session_id(),
                    DevToolsNetworkResourceType::Document,
                    final_url,
                )
                .into_iter()
                .next()
        })
    else {
        return false;
    };
    pending.interception_session_id = response_stage.session_id;
    true
}

pub(super) fn continue_navigation_request(
    conn: &mut CdpConnection,
    claimed: crate::conn::ClaimedFetchNavigation,
) {
    let (pending, request) = claimed.into_parts();
    if let Some(request) = request {
        conn.update_native_navigation_dispatch(&pending);
        conn.resolve_native_navigation_decision(
            pending.navigation.web_contents,
            pending.navigation_permit,
            request.into_navigation_decision(),
        );
    }
}

pub(super) fn continue_navigation_response_neutrally(
    conn: &mut CdpConnection,
    pending: crate::conn::PendingFetchResponseNavigation,
    transfer: Option<crate::conn::PausedDocumentTransfer>,
) {
    if let Some(transfer) = transfer {
        conn.resolve_native_navigation_decision(
            pending.navigation.web_contents,
            pending.permit,
            moli_core::browser::NavigationDecision::Response {
                transfer: Box::new(transfer),
                status: None,
                headers: Vec::new(),
            },
        );
    }
}

pub(crate) async fn continue_subresource_without_fetch_pause_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: Option<String>,
    page_owner: crate::conn::TargetPageResidenceIdentity,
    internal_id: u64,
    network_request_id: String,
    network_request_handle: Option<moli_core::page::SubresourceNetworkRequestHandle>,
    frame_id: String,
    document_url: Url,
    resource_type: SubresourceResourceType,
    handle_auth_requests: bool,
    owner_kind: PendingSubresourceFetchOwnerKind,
) {
    let pending = PendingSubresourceFetchRequest {
        residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner),
        owner_session_id: None,
        action_session_id: None,
        owner_kind,
        internal_id,
        network_request_id,
        network_request_handle,
        frame_id,
        document_url,
        resource_type,
        websocket_socket_id: None,
        request_stage_chain: None,
    };
    if conn
        .continue_pending_subresource_fetch_for_owner_async(
            owner,
            internal_id,
            None,
            None,
            None,
            None,
            false,
            handle_auth_requests,
        )
        .await
        .is_ok()
    {
        conn.register_in_flight_subresource_fetch_request_for_owner(owner, request_id, pending);
    }
}

fn navigation_auth_required_blocked_intercepts(
    conn: &CdpConnection,
    pending: &PendingFetchNavigation,
) -> Vec<DevToolsNetworkInterceptId> {
    if !pending.auth_required_blocked_intercepts.is_empty() {
        return pending.auth_required_blocked_intercepts.clone();
    }
    let intercepts = conn.target_fetch_matching_auth_required_network_intercepts_for_owner(
        &pending.navigation.owner,
        &pending.navigation.requested_url,
    );
    if !intercepts.is_empty() {
        return intercepts;
    }
    conn.target_fetch_matching_auth_required_network_intercepts_for_target(
        pending.navigation.frame_id.as_str(),
        &pending.navigation.requested_url,
    )
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

fn routable_stage_owner_session_id(
    conn: &CdpConnection,
    fallback_session_id: Option<&str>,
    stage_session_id: Option<&str>,
) -> Option<String> {
    if let Some(stage_session_id) = stage_session_id
        && conn.session_route(Some(stage_session_id)).is_some()
    {
        return Some(stage_session_id.to_owned());
    }
    fallback_session_id.map(str::to_owned)
}

pub(crate) fn register_navigation_auth_required_event_for_permit(
    conn: &mut CdpConnection,
    pending: &PendingFetchNavigation,
    challenge: FetchAuthChallenge,
    request_cookie_report: Option<StoredCookieQueryReport>,
    auth_permit: moli_core::browser::web_contents::NavigationInterceptionPermit,
) -> Result<crate::conn::BackgroundProtocolEvent, String> {
    let blocked_intercepts = navigation_auth_required_blocked_intercepts(conn, pending);
    let mut pending_auth = crate::conn::PendingFetchAuthNavigation {
        owner_session_id: pending.navigation.owner.session_id().map(str::to_owned),
        action_session_id: pending.interception_session_id.clone(),
        interception_session_id: pending.interception_session_id.clone(),
        owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
        fetch_request_id: pending.fetch_request_id.clone(),
        response_stage_request_id: pending.fetch_request_id.clone(),
        navigation: pending.navigation.clone(),
        request_cookie_report,
        auth_permit,
        challenge,
        intercept_response: pending.intercept_response,
        response_stage_url_match_policy: pending.response_stage_url_match_policy,
        auth_stage_chain: None,
    };
    let mut auth_event_session_id = pending.interception_session_id.clone();
    let mut auth_event_blocked_intercepts = blocked_intercepts.clone();
    let auth_sessions = conn
        .target_fetch_subresource_interception_snapshot_for_target(&pending.navigation.frame_id)
        .or_else(|| {
            conn.target_fetch_subresource_interception_snapshot_for_owner(&pending.navigation.owner)
        })
        .map(|snapshot| {
            snapshot.matching_auth_required_pause_sessions(
                pending.navigation.owner.session_id(),
                &pending.navigation.requested_url,
            )
        })
        .unwrap_or_default();
    if let Some(first_pause_session) = auth_sessions.first().cloned() {
        pending_auth.owner_session_id = routable_stage_owner_session_id(
            conn,
            pending.navigation.owner.session_id(),
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
            let Ok(next_request_id) =
                conn.allocate_fetch_navigation_request_id_for_owner(&pending.navigation.owner)
            else {
                break;
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
            pending_auth.auth_stage_chain = Some(Box::new(PendingSubresourceFetchAuthStageChain {
                remaining_sessions,
            }));
        }
    }
    pending_auth.action_session_id = auth_event_session_id.clone();
    let pending_owner_session_id = pending_auth
        .owner_session_id
        .as_deref()
        .or(pending.navigation.owner.session_id());
    let pending_owner = pending_owner_session_id
        .map(CommandOwnerScope::for_session)
        .unwrap_or_else(|| pending.navigation.owner.clone());
    if !conn.register_pending_fetch_auth_navigation_for_owner(
        &pending_owner,
        pending.fetch_request_id.clone(),
        pending_auth.clone(),
    ) {
        return Err("navigation auth projection unavailable".to_owned());
    }
    Ok(pending_fetch_auth_navigation_required_event(
        auth_event_session_id.as_deref(),
        &pending_auth,
        &auth_event_blocked_intercepts,
    ))
}

pub(crate) async fn continue_subresource_for_response_stage_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    session_id: Option<&str>,
    request_id: String,
    pending: PendingSubresourceFetchRequest,
    handle_auth_requests: bool,
    response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
) {
    let action_owner = session_id
        .map(CommandOwnerScope::for_session)
        .unwrap_or_else(|| owner.clone());
    if conn
        .continue_pending_subresource_fetch_for_owner_async(
            &action_owner,
            pending.internal_id,
            None,
            None,
            None,
            None,
            true,
            handle_auth_requests,
        )
        .await
        .is_ok()
    {
        conn.register_in_flight_response_stage_subresource_fetch_request_for_owner(
            &action_owner,
            Some(request_id),
            pending,
            response_stage_blocked_intercepts,
        );
    }
}

pub(crate) async fn continue_subresource_for_deferred_response_stage_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    session_id: Option<&str>,
    request_id: String,
    pending: PendingSubresourceFetchRequest,
    handle_auth_requests: bool,
) {
    let action_owner = session_id
        .map(CommandOwnerScope::for_session)
        .unwrap_or_else(|| owner.clone());
    if conn
        .continue_pending_subresource_fetch_for_owner_async(
            &action_owner,
            pending.internal_id,
            None,
            None,
            None,
            None,
            true,
            handle_auth_requests,
        )
        .await
        .is_ok()
    {
        conn.register_in_flight_deferred_response_stage_subresource_fetch_request_for_owner(
            &action_owner,
            Some(request_id),
            pending,
        );
    }
}

#[cfg(test)]
mod tests {
    use moli_web_mime::response_headers_indicate_attachment_download;

    #[test]
    fn response_stage_navigation_download_detection_uses_web_mime_attachment_helper() {
        assert!(response_headers_indicate_attachment_download(&[(
            "Content-Disposition".to_owned(),
            "attachment; filename=report.html".to_owned(),
        )]));

        assert!(!response_headers_indicate_attachment_download(&[(
            "Content-Disposition".to_owned(),
            "inline; filename=attachment.html".to_owned(),
        )]));
    }
}
