use crate::conn::{
    CdpConnection, CommandOwnerScope, PendingFetchAuthNavigation, PendingFetchNavigation,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest,
};
use crate::devtools_runtime::{DevToolsProtocol, DevToolsSessionId};
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
            && !conn.register_in_flight_subresource_fetch_request_for_route(
                owner.session_id(),
                owner.session_owner_route(),
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
        })
    }

    pub(super) fn rollback(self, conn: &mut CdpConnection) {
        if self.registered {
            conn.take_in_flight_subresource_fetch_request_for_route(
                self.owner_scope.session_id(),
                self.owner_scope.session_owner_route(),
                self.internal_id,
            );
        }
    }

    pub(super) fn prepare_deferred_response_stage(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        request_id: &str,
        pending: &PendingSubresourceFetchRequest,
    ) -> Option<Self> {
        if !conn.register_in_flight_deferred_response_stage_subresource_fetch_request_for_route(
            owner.session_id(),
            owner.session_owner_route(),
            Some(request_id.to_owned()),
            pending.clone(),
        ) {
            return None;
        }
        Some(Self {
            internal_id: pending.internal_id,
            registered: true,
            owner_scope: owner.clone(),
        })
    }
}

pub(crate) fn action_session_id_for_devtools_context<'a>(
    command_session_id: Option<&'a str>,
    protocol: DevToolsProtocol,
    context_session_id: Option<&'a DevToolsSessionId>,
) -> Option<&'a str> {
    if protocol == DevToolsProtocol::Cdp {
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
    let Some(result) = conn.consume_pending_request_action_for_route(
        owner.session_id(),
        owner.session_owner_route(),
        request_id,
    ) else {
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
    conn.take_pending_fetch_navigation_for_route(
        owner.session_id(),
        owner.session_owner_route(),
        action_session_id,
        request_id,
    )
}

pub(crate) fn take_pending_auth_navigation_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingFetchAuthNavigation> {
    conn.take_pending_fetch_auth_navigation_for_route(
        owner.session_id(),
        owner.session_owner_route(),
        action_session_id,
        request_id,
    )
}

pub(crate) fn take_pending_subresource_fetch_request_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingSubresourceFetchRequest> {
    conn.take_pending_subresource_fetch_request_for_route(
        owner.session_id(),
        owner.session_owner_route(),
        action_session_id,
        request_id,
    )
}

pub(crate) fn take_pending_subresource_auth_request_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingSubresourceFetchAuthRequest> {
    conn.take_pending_subresource_fetch_auth_request_for_route(
        owner.session_id(),
        owner.session_owner_route(),
        action_session_id,
        request_id,
    )
}

pub(crate) fn take_pending_subresource_response_request_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> Option<PendingSubresourceFetchResponseRequest> {
    conn.take_pending_subresource_fetch_response_request_for_route(
        owner.session_id(),
        owner.session_owner_route(),
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
    conn.pending_subresource_fetch_response_request_for_route(
        owner.session_id(),
        owner.session_owner_route(),
        action_session_id,
        request_id,
    )
}

pub(crate) fn mark_pending_subresource_response_body_taken_as_stream_for_action_session(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action_session_id: Option<&str>,
    request_id: &str,
) -> bool {
    conn.mark_pending_subresource_fetch_response_body_taken_as_stream_for_route(
        owner.session_id(),
        owner.session_owner_route(),
        action_session_id,
        request_id,
    )
}
