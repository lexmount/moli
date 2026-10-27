use crate::automation::{DevToolsSessionId, FrontendProtocol};
use crate::conn::{
    CdpConnection, CommandOwnerScope, PendingFetchAuthNavigation, PendingFetchNavigation,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest,
};
use crate::domains::command_output::CommandOutputPlan;

use super::patterns::validate_request_id;

/// Correlation state installed before renderer work can produce an observable
/// subresource completion. The command keeps this token only so a failed
/// renderer dispatch can remove the prepared entry again.
#[derive(Debug, Clone)]
pub(super) struct PreparedSubresourceCorrelation {
    internal_id: u64,
    registered: bool,
    owner_scope: CommandOwnerScope,
    network_request_id: String,
}

impl PreparedSubresourceCorrelation {
    pub(super) fn prepare(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        request_id: &str,
        pending: &PendingSubresourceFetchRequest,
        should_register: bool,
    ) -> Option<Self> {
        if should_register
            && !conn.register_in_flight_subresource_fetch_request_for_owner(
                owner,
                Some(request_id.to_owned()),
                pending.clone(),
            )
        {
            return None;
        }
        Some(Self {
            internal_id: pending.internal_id,
            registered: should_register,
            owner_scope: owner.clone(),
            network_request_id: pending.network_request_id.clone(),
        })
    }

    pub(super) fn rollback(self, conn: &mut CdpConnection) {
        if self.registered {
            conn.rollback_in_flight_subresource_fetch_request_for_owner(
                &self.owner_scope,
                self.internal_id,
                &self.network_request_id,
            );
        }
    }

    pub(super) fn prepare_deferred_response_stage(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        request_id: &str,
        pending: &PendingSubresourceFetchRequest,
    ) -> Option<Self> {
        if !conn.register_in_flight_deferred_response_stage_subresource_fetch_request_for_owner(
            owner,
            Some(request_id.to_owned()),
            pending.clone(),
        ) {
            return None;
        }
        Some(Self {
            internal_id: pending.internal_id,
            registered: true,
            owner_scope: owner.clone(),
            network_request_id: pending.network_request_id.clone(),
        })
    }

    pub(super) fn prepare_automatic(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        request_id: Option<String>,
        pending: PendingSubresourceFetchRequest,
        response: super::subresource_continuation::SubresourceResponseInterception,
    ) -> Option<Self> {
        use super::subresource_continuation::SubresourceResponseInterception;
        let correlation = Self {
            internal_id: pending.internal_id,
            registered: true,
            owner_scope: owner.clone(),
            network_request_id: pending.network_request_id.clone(),
        };
        let registered = match response {
            SubresourceResponseInterception::None => conn
                .register_in_flight_subresource_fetch_request_for_owner(owner, request_id, pending),
            SubresourceResponseInterception::Matched(intercepts) => conn
                .register_in_flight_response_stage_subresource_fetch_request_for_owner(
                    owner, request_id, pending, intercepts,
                ),
            SubresourceResponseInterception::Candidate => conn
                .register_in_flight_deferred_response_stage_subresource_fetch_request_for_owner(
                    owner, request_id, pending,
                ),
        };
        registered.then_some(correlation)
    }
}

pub(crate) fn action_session_id_for_devtools_context<'a>(
    command_session_id: Option<&'a str>,
    protocol: FrontendProtocol,
    context_session_id: Option<&'a DevToolsSessionId>,
) -> Option<&'a str> {
    if protocol == FrontendProtocol::Cdp {
        command_session_id
    } else {
        context_session_id.map(|session_id| session_id.as_str())
    }
}

pub(crate) fn pending_request_action_output_plan(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
) -> CommandOutputPlan {
    pending_request_action_result(conn, owner, request_id)
        .map_or_else(|plan| plan, |()| CommandOutputPlan::success())
}

pub(crate) fn pending_request_action_result(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
) -> Result<(), CommandOutputPlan> {
    pending_request_action_result_with_id_validation(conn, owner, request_id, true)
}

pub(crate) fn pending_request_action_output_plan_with_id_validation(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
    validate_id: bool,
) -> CommandOutputPlan {
    pending_request_action_result_with_id_validation(conn, owner, request_id, validate_id)
        .map_or_else(|plan| plan, |()| CommandOutputPlan::success())
}

pub(crate) fn pending_request_action_result_with_id_validation(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    request_id: &str,
    validate_id: bool,
) -> Result<(), CommandOutputPlan> {
    if validate_id && validate_request_id(request_id).is_err() {
        return Err(CommandOutputPlan::error(-32602, "InvalidParams"));
    }
    let Some(result) = conn.consume_pending_request_action_for_owner(owner, request_id) else {
        return Err(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
    };
    if result.is_err() {
        return Err(CommandOutputPlan::error(-32000, "RequestNotFound"));
    }
    Ok(())
}

pub(crate) fn take_pending_navigation(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingFetchNavigation> {
    conn.take_pending_fetch_navigation_for_owner(owner, action_session_id, request_id)
}

pub(crate) fn take_pending_auth_navigation_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingFetchAuthNavigation> {
    conn.take_pending_fetch_auth_navigation_for_owner(owner, action_session_id, request_id)
}

pub(crate) fn take_pending_subresource_fetch_request_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingSubresourceFetchRequest> {
    conn.take_pending_subresource_fetch_request_for_owner(owner, action_session_id, request_id)
}

pub(crate) fn take_pending_subresource_auth_request_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingSubresourceFetchAuthRequest> {
    conn.take_pending_subresource_fetch_auth_request_for_owner(owner, action_session_id, request_id)
}

pub(crate) fn take_pending_subresource_response_request_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingSubresourceFetchResponseRequest> {
    conn.take_pending_subresource_fetch_response_request_for_owner(
        owner,
        action_session_id,
        request_id,
    )
}

pub(crate) fn pending_subresource_response_request_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingSubresourceFetchResponseRequest> {
    conn.pending_subresource_fetch_response_request_for_owner(owner, action_session_id, request_id)
}

pub(crate) fn mark_pending_subresource_response_body_taken_as_stream_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> bool {
    conn.mark_pending_subresource_fetch_response_body_taken_as_stream_for_owner(
        owner,
        action_session_id,
        request_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, PendingSubresourceFetchOwnerKind, PendingSubresourceFetchResidence,
    };
    use moli_core::page::SubresourceResourceType;
    use url::Url;

    fn request(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        network_id: &str,
    ) -> PendingSubresourceFetchRequest {
        PendingSubresourceFetchRequest {
            residence: PendingSubresourceFetchResidence::InstalledPage(
                conn.target_page_residence_identity_for_owner(owner)
                    .unwrap(),
            ),
            owner_session_id: None,
            action_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            internal_id: 7,
            network_request_id: network_id.to_owned(),
            network_request_handle: None,
            frame_id: "TID-correlation".to_owned(),
            document_url: Url::parse("https://example.test/page").unwrap(),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            request_stage_chain: None,
        }
    }

    #[test]
    fn delayed_continuation_failure_keeps_new_request_with_reused_native_id() {
        for attachment in [1, 2] {
            let mut conn = CdpConnection::default();
            let mut browser = BrowserContext::new("BID-correlation".into());
            browser.set_active_target_id("TID-correlation");
            browser.attach_active_session("SID-correlation");
            browser
                .active_page_target_mut()
                .runtime_slot
                .set_page_attachment_id_for_test(1);
            conn.install_browser_context_fixture_for_test(browser);
            let owner = CommandOwnerScope::for_session("SID-correlation");
            let original_request = request(&conn, &owner, "NETWORK-original");
            let original = PreparedSubresourceCorrelation::prepare_automatic(
                &mut conn,
                &owner,
                Some("FETCH-original".into()),
                original_request,
                super::super::subresource_continuation::SubresourceResponseInterception::Candidate,
            )
            .unwrap();
            conn.browser_context_by_id_mut("BID-correlation")
                .unwrap()
                .active_page_target_mut()
                .runtime_slot
                .set_page_attachment_id_for_test(attachment);
            let replacement = request(&conn, &owner, "NETWORK-replacement");
            assert!(conn.register_in_flight_subresource_fetch_request_for_owner(
                &owner,
                Some("FETCH-replacement".into()),
                replacement,
            ));

            original.rollback(&mut conn);

            assert_eq!(
                conn.in_flight_subresource_fetch_request_id_for_owner(&owner, 7)
                    .as_deref(),
                Some("FETCH-replacement")
            );
        }
    }
}
