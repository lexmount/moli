use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, PendingFetchAuthNavigation,
    PendingFetchNavigation, PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
};
use crate::devtools_runtime::{
    DevToolsAuthChallengeAction, DevToolsCommand, DevToolsContinueWithAuthCommand,
    DevToolsProtocol, DevToolsRequestId,
};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::{activity, network};
use moli_core::page::{CompletedPageCommand, SubresourceAuthCredentials};

use super::PendingFetchCommandOperation;
use super::helpers::{
    pending_fetch_auth_navigation_required_event, pending_subresource_auth_required_event,
    request_auth_for_challenge,
};
use super::navigation::{
    complete_tokened_materialized_navigation_as_background_events_async,
    load_or_pause_navigation_for_auth_as_background_events_async,
};
use super::params::{AuthChallengeResponseResponse, ContinueWithAuthParams};
use super::state::{
    PreparedSubresourceCorrelation, action_session_id_for_devtools_context,
    pending_request_action_output_plan_with_id_validation,
    take_pending_auth_navigation_for_action_session,
    take_pending_subresource_auth_request_for_action_session,
};
use super::{
    FetchCommandOutput, FetchCommandTaskStep, PendingFetchCommandDispatch, PendingFetchCommandKind,
};

pub(super) enum PendingContinueWithAuthState {
    SubresourceAuthCancel {
        pending: Box<crate::conn::PendingSubresourceFetchAuthRequest>,
        correlation: Option<PreparedSubresourceCorrelation>,
    },
    SubresourceAuthFail {
        pending: Box<crate::conn::PendingSubresourceFetchAuthRequest>,
    },
    SubresourceAuthContinue {
        correlation: PreparedSubresourceCorrelation,
    },
    NavigationCancel {
        pending: Box<crate::conn::PendingFetchAuthNavigation>,
    },
    NavigationFail {
        pending: Box<crate::conn::PendingFetchAuthNavigation>,
    },
    NavigationContinue {
        pending: Box<crate::conn::PendingFetchAuthNavigation>,
        auth: SubresourceAuthCredentials,
    },
}

pub(super) fn start_continue_with_auth_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> FetchCommandTaskStep {
    let params: ContinueWithAuthParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let command = build_cdp_continue_with_auth_command(conn, cmd, params);
    super::commands::start_devtools_fetch_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::ContinueWithAuth(command),
    )
}

fn build_cdp_continue_with_auth_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    params: ContinueWithAuthParams,
) -> DevToolsContinueWithAuthCommand {
    let (browser_context_id, target_id) =
        super::commands::devtools_fetch_owner_identity_for_session(conn, cmd.session_id);
    DevToolsContinueWithAuthCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        request_id: DevToolsRequestId::from(params.request_id.as_ref().to_owned()),
        action: devtools_auth_action_from_cdp(params.auth_challenge_response.response),
        username: params.auth_challenge_response.username,
        password: params.auth_challenge_response.password,
    }
}

fn devtools_auth_action_from_cdp(
    action: AuthChallengeResponseResponse,
) -> DevToolsAuthChallengeAction {
    match action {
        AuthChallengeResponseResponse::Default => DevToolsAuthChallengeAction::Default,
        AuthChallengeResponseResponse::CancelAuth => DevToolsAuthChallengeAction::Cancel,
        AuthChallengeResponseResponse::ProvideCredentials => {
            DevToolsAuthChallengeAction::ProvideCredentials
        }
    }
}

pub(super) fn start_devtools_continue_with_auth_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: &DevToolsContinueWithAuthCommand,
) -> FetchCommandTaskStep {
    if let Some(step) =
        start_devtools_continue_with_auth_command_for_pending(conn, command_id, owner, command)
    {
        return step;
    }

    FetchCommandTaskStep::Complete(pending_request_action_output_plan_with_id_validation(
        conn,
        owner,
        command.request_id.as_str(),
        command.context.protocol == DevToolsProtocol::Cdp,
    ))
}

pub(super) fn start_devtools_continue_with_auth_command_for_pending(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: &DevToolsContinueWithAuthCommand,
) -> Option<FetchCommandTaskStep> {
    let command_session_id = owner.session_id();
    let request_id = command.request_id.as_str().to_owned();
    let action_session_id = action_session_id_for_devtools_context(
        command_session_id,
        command.context.protocol,
        command.context.session_id.as_ref(),
    );
    if let Some(pending) = take_pending_subresource_auth_request_for_action_session(
        conn,
        owner,
        action_session_id,
        &request_id,
    ) {
        if matches!(command.action, DevToolsAuthChallengeAction::Default)
            && pending
                .auth_stage_pause_state()
                .is_some_and(|chain| !chain.remaining_sessions.is_empty())
        {
            return Some(FetchCommandTaskStep::Complete(
                chained_subresource_auth_required_output_plan(conn, command_session_id, pending)
                    .unwrap_or_else(|| CommandOutputPlan::error(-32000, "RequestNotFound")),
            ));
        }
        match command.action {
            DevToolsAuthChallengeAction::Default | DevToolsAuthChallengeAction::Cancel => {
                let cancel_correlation =
                    if matches!(command.action, DevToolsAuthChallengeAction::Cancel)
                        && pending.intercept_response
                    {
                        let continued = continued_subresource_request(&pending);
                        match PreparedSubresourceCorrelation::prepare(
                            conn,
                            owner,
                            &request_id,
                            &continued,
                            true,
                        ) {
                            Some(correlation) => Some(correlation),
                            None => {
                                conn.register_pending_subresource_fetch_auth_request_for_route(
                                    command_session_id,
                                    owner.session_owner_route(),
                                    request_id,
                                    pending,
                                );
                                return Some(FetchCommandTaskStep::Complete(
                                    CommandOutputPlan::error(-32000, "RequestNotFound"),
                                ));
                            }
                        }
                    } else {
                        None
                    };
                let pending_page = match conn.loaded_page_mut_for_protocol_access_for_route(
                    command_session_id,
                    owner.session_owner_route(),
                ) {
                    Ok(page) => (match command.action {
                        DevToolsAuthChallengeAction::Default => page
                            .start_fail_pending_subresource_auth(
                                pending.internal_id,
                                "Fetch auth challenge aborted".to_owned(),
                            ),
                        DevToolsAuthChallengeAction::Cancel => {
                            page.start_cancel_pending_subresource_auth(pending.internal_id)
                        }
                        DevToolsAuthChallengeAction::ProvideCredentials => unreachable!(),
                    })
                    .map_err(|error| error.to_string()),
                    Err(message) => Err(message.to_owned()),
                };
                let pending_page = match pending_page {
                    Ok(pending_page) => pending_page,
                    Err(error) => {
                        if let Some(correlation) = cancel_correlation {
                            correlation.rollback(conn);
                        }
                        conn.register_pending_subresource_fetch_auth_request_for_route(
                            command_session_id,
                            owner.session_owner_route(),
                            request_id,
                            pending,
                        );
                        return Some(FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                            -32000,
                            format!("subresource auth terminal action failed: {error}"),
                        )));
                    }
                };
                let state = match command.action {
                    DevToolsAuthChallengeAction::Default => {
                        PendingContinueWithAuthState::SubresourceAuthFail {
                            pending: Box::new(pending),
                        }
                    }
                    DevToolsAuthChallengeAction::Cancel => {
                        PendingContinueWithAuthState::SubresourceAuthCancel {
                            pending: Box::new(pending),
                            correlation: cancel_correlation,
                        }
                    }
                    DevToolsAuthChallengeAction::ProvideCredentials => unreachable!(),
                };
                return Some(FetchCommandTaskStep::Pending(
                    PendingFetchCommandDispatch::new_for_owner(
                        command_id,
                        owner.clone(),
                        PendingFetchCommandKind::ContinueWithAuth {
                            state: Box::new(state),
                        },
                        PendingFetchCommandOperation::Page(pending_page),
                    ),
                ));
            }
            DevToolsAuthChallengeAction::ProvideCredentials => {
                let Some(auth) = request_auth_for_challenge(
                    &pending.challenge,
                    command.username.as_deref().unwrap_or_default(),
                    command.password.as_deref().unwrap_or_default(),
                ) else {
                    conn.register_pending_subresource_fetch_auth_request_for_route(
                        command_session_id,
                        owner.session_owner_route(),
                        request_id.clone(),
                        pending,
                    );
                    return Some(FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "NotImplemented",
                    )));
                };
                let continued = continued_subresource_request(&pending);
                let correlation = match PreparedSubresourceCorrelation::prepare(
                    conn,
                    owner,
                    &request_id,
                    &continued,
                    true,
                ) {
                    Some(correlation) => correlation,
                    None => {
                        conn.register_pending_subresource_fetch_auth_request_for_route(
                            command_session_id,
                            owner.session_owner_route(),
                            request_id,
                            pending,
                        );
                        return Some(FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                            -32000,
                            "RequestNotFound",
                        )));
                    }
                };
                let pending_page = match conn.loaded_page_mut_for_protocol_access_for_route(
                    command_session_id,
                    owner.session_owner_route(),
                ) {
                    Ok(page) => page
                        .start_continue_pending_subresource_auth(pending.internal_id, auth)
                        .map_err(|error| format!("subresource auth continue failed: {error}")),
                    Err(message) => Err(message.to_owned()),
                };
                let pending_page = match pending_page {
                    Ok(pending_page) => pending_page,
                    Err(message) => {
                        correlation.rollback(conn);
                        conn.register_pending_subresource_fetch_auth_request_for_route(
                            command_session_id,
                            owner.session_owner_route(),
                            request_id,
                            pending,
                        );
                        return Some(FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                            -32000, message,
                        )));
                    }
                };
                return Some(FetchCommandTaskStep::Pending(
                    PendingFetchCommandDispatch::new_for_owner(
                        command_id,
                        owner.clone(),
                        PendingFetchCommandKind::ContinueWithAuth {
                            state: Box::new(
                                PendingContinueWithAuthState::SubresourceAuthContinue {
                                    correlation,
                                },
                            ),
                        },
                        PendingFetchCommandOperation::Page(pending_page),
                    ),
                ));
            }
        }
    }
    let pending = take_pending_auth_navigation_for_action_session(
        conn,
        owner,
        action_session_id,
        &request_id,
    )?;

    Some(match command.action {
        DevToolsAuthChallengeAction::Default
            if pending
                .auth_stage_pause_state()
                .is_some_and(|chain| !chain.remaining_sessions.is_empty()) =>
        {
            FetchCommandTaskStep::Complete(
                chained_navigation_auth_required_output_plan(conn, command_session_id, pending)
                    .unwrap_or_else(|| CommandOutputPlan::error(-32000, "RequestNotFound")),
            )
        }
        DevToolsAuthChallengeAction::Default => {
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
                command_id,
                owner.clone(),
                PendingFetchCommandKind::ContinueWithAuth {
                    state: Box::new(PendingContinueWithAuthState::NavigationFail {
                        pending: Box::new(pending),
                    }),
                },
                PendingFetchCommandOperation::Ready,
            ))
        }
        DevToolsAuthChallengeAction::Cancel => {
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
                command_id,
                owner.clone(),
                PendingFetchCommandKind::ContinueWithAuth {
                    state: Box::new(PendingContinueWithAuthState::NavigationCancel {
                        pending: Box::new(pending),
                    }),
                },
                PendingFetchCommandOperation::Ready,
            ))
        }
        DevToolsAuthChallengeAction::ProvideCredentials => {
            let Some(auth) = request_auth_for_challenge(
                &pending.challenge,
                command.username.as_deref().unwrap_or_default(),
                command.password.as_deref().unwrap_or_default(),
            ) else {
                conn.register_pending_fetch_auth_navigation_for_route(
                    command_session_id,
                    owner.session_owner_route(),
                    request_id.clone(),
                    pending,
                );
                return Some(FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "NotImplemented",
                )));
            };
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
                command_id,
                owner.clone(),
                PendingFetchCommandKind::ContinueWithAuth {
                    state: Box::new(PendingContinueWithAuthState::NavigationContinue {
                        pending: Box::new(pending),
                        auth,
                    }),
                },
                PendingFetchCommandOperation::Ready,
            ))
        }
    })
}

fn continued_subresource_request(
    pending: &PendingSubresourceFetchAuthRequest,
) -> PendingSubresourceFetchRequest {
    PendingSubresourceFetchRequest {
        residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(
            pending.page_owner.clone(),
        ),
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
        request_stage_chain: None,
    }
}

fn chained_subresource_auth_required_output_plan(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    pending: PendingSubresourceFetchAuthRequest,
) -> Option<CommandOutputPlan> {
    let event = next_chained_subresource_auth_required_event(conn, command_session_id, pending)?;
    let mut plan = CommandOutputPlan::default();
    plan.push_success();
    plan.push_background_event(event);
    Some(plan)
}

fn chained_navigation_auth_required_output_plan(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    pending: PendingFetchAuthNavigation,
) -> Option<CommandOutputPlan> {
    let event = next_chained_navigation_auth_required_event(conn, command_session_id, pending)?;
    let mut plan = CommandOutputPlan::default();
    plan.push_success();
    plan.push_background_event(event);
    Some(plan)
}

fn next_chained_navigation_auth_required_event(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    mut pending: PendingFetchAuthNavigation,
) -> Option<BackgroundProtocolEvent> {
    let next_pause = pending.pop_next_auth_required_pause()?;
    let previous_owner_session_id = pending.owner_session_id.clone();
    let next_session_has_route = next_pause
        .session_id
        .as_deref()
        .is_some_and(|session_id| conn.session_route(Some(session_id)).is_some());
    pending.owner_session_id = if next_session_has_route {
        next_pause.session_id.clone()
    } else {
        previous_owner_session_id.clone()
    };
    pending.action_session_id = next_pause.session_id.clone();
    pending.owner_kind = next_pause.owner_kind;
    let owner_session_id = next_pause
        .session_id
        .as_deref()
        .filter(|_| next_session_has_route)
        .or(previous_owner_session_id.as_deref())
        .or(command_session_id);
    pending.fetch_request_id = next_pause.request_id.clone();
    if !conn.register_pending_fetch_auth_navigation_for_session_owner(
        owner_session_id,
        next_pause.request_id.clone(),
        pending.clone(),
    ) {
        return None;
    }
    Some(pending_fetch_auth_navigation_required_event(
        next_pause.session_id.as_deref(),
        &pending,
        &next_pause.blocked_intercepts,
    ))
}

fn next_chained_subresource_auth_required_event(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    mut pending: PendingSubresourceFetchAuthRequest,
) -> Option<BackgroundProtocolEvent> {
    let next_pause = pending.pop_next_auth_required_pause()?;
    let previous_owner_session_id = pending.owner_session_id.clone();
    let next_session_has_route = next_pause
        .session_id
        .as_deref()
        .is_some_and(|session_id| conn.session_route(Some(session_id)).is_some());
    pending.owner_session_id = if next_session_has_route {
        next_pause.session_id.clone()
    } else {
        previous_owner_session_id.clone()
    };
    pending.action_session_id = next_pause.session_id.clone();
    pending.owner_kind = next_pause.owner_kind;
    let owner_session_id = next_pause
        .session_id
        .as_deref()
        .filter(|_| next_session_has_route)
        .or(previous_owner_session_id.as_deref())
        .or(command_session_id);
    if !conn.register_pending_subresource_fetch_auth_request_for_session_owner(
        owner_session_id,
        next_pause.request_id.clone(),
        pending.clone(),
    ) {
        return None;
    }
    Some(pending_subresource_auth_required_event(
        next_pause.session_id.as_deref(),
        &next_pause.request_id,
        &pending,
        &next_pause.blocked_intercepts,
    ))
}

pub(super) async fn complete_continue_with_auth_command_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completed: Option<Result<CompletedPageCommand, String>>,
    state: PendingContinueWithAuthState,
    out: &mut FetchCommandOutput,
) {
    match state {
        PendingContinueWithAuthState::SubresourceAuthCancel {
            pending,
            correlation,
        } => {
            complete_subresource_auth_terminal_async(
                conn,
                owner,
                completed,
                *pending,
                true,
                correlation,
                out,
            )
            .await;
        }
        PendingContinueWithAuthState::SubresourceAuthFail { pending } => {
            complete_subresource_auth_terminal_async(
                conn, owner, completed, *pending, false, None, out,
            )
            .await;
        }
        PendingContinueWithAuthState::SubresourceAuthContinue { correlation } => {
            if let Err(error) = finish_continue_subresource_auth(conn, owner, completed) {
                correlation.rollback(conn);
                out.push_error(-32000, error);
                return;
            }
            out.push_success();
        }
        PendingContinueWithAuthState::NavigationCancel { pending } => {
            out.push_success();
            super::navigation::cancel_navigation_auth_as_background_events_async(
                conn, out, *pending,
            )
            .await;
        }
        PendingContinueWithAuthState::NavigationFail { pending } => {
            out.push_success();
            let token = pending.document_navigation_token;
            let navigation_state = pending.navigation;
            let navigation = network::materialize_navigation_load_result(
                conn,
                &navigation_state,
                Err("Fetch auth challenge aborted".to_owned()),
            );
            complete_tokened_materialized_navigation_as_background_events_async(
                conn,
                out,
                token,
                navigation_state,
                navigation,
            )
            .await;
        }
        PendingContinueWithAuthState::NavigationContinue { pending, auth } => {
            out.push_success();
            let prior_network_observation_journal =
                pending.auth_response.observation_journal().clone();
            load_or_pause_navigation_for_auth_as_background_events_async(
                conn,
                out,
                PendingFetchNavigation {
                    fetch_request_id: pending.response_stage_request_id,
                    interception_session_id: pending.interception_session_id.clone(),
                    document_navigation_token: pending.document_navigation_token,
                    navigation: pending.navigation,
                    request_cookie_report: None,
                    intercept_response: pending.intercept_response,
                    response_stage_url_match_policy: pending.response_stage_url_match_policy,
                    auth_required_blocked_intercepts: Vec::new(),
                },
                Some(auth),
                Some(prior_network_observation_journal),
            )
            .await;
        }
    }
}

async fn complete_subresource_auth_terminal_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completed: Option<Result<CompletedPageCommand, String>>,
    pending: crate::conn::PendingSubresourceFetchAuthRequest,
    expose_challenged_response: bool,
    correlation: Option<PreparedSubresourceCorrelation>,
    out: &mut FetchCommandOutput,
) {
    let activity_session_id = pending.owner_session_id.as_deref().or(owner.session_id());
    let activity_owner_route = activity_session_id
        .is_none()
        .then(|| owner.session_owner_route())
        .flatten();
    let Some(completed) = completed else {
        if let Some(correlation) = correlation {
            correlation.rollback(conn);
        }
        out.push_error(-32000, "Missing renderer completion");
        return;
    };
    let completion = match completed {
        Ok(completion) => completion,
        Err(error) => {
            if let Some(correlation) = correlation {
                correlation.rollback(conn);
            }
            out.push_error(-32000, error);
            return;
        }
    };
    let result = match conn
        .loaded_page_mut_for_protocol_access_for_route(activity_session_id, activity_owner_route)
    {
        Ok(page) if expose_challenged_response => page
            .finish_cancel_pending_subresource_auth(completion)
            .map(|_| ()),
        Ok(page) => page
            .finish_fail_pending_subresource_auth(completion)
            .map(|_| ()),
        Err(message) => {
            if let Some(correlation) = correlation {
                correlation.rollback(conn);
            }
            out.push_error(-32000, message);
            return;
        }
    };
    if let Err(error) = result {
        if let Some(correlation) = correlation {
            correlation.rollback(conn);
        }
        out.push_error(
            -32000,
            format!("subresource auth terminal action failed: {error}"),
        );
        return;
    }
    out.push_success();
    let mut events = Vec::new();
    activity::flush_post_subresource_auth_activity_background_events_async(
        conn,
        &mut events,
        activity_session_id,
        &pending,
    )
    .await;
    out.extend_background_events(events);
}

fn finish_continue_subresource_auth(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completed: Option<Result<CompletedPageCommand, String>>,
) -> Result<(), String> {
    let completion = completed.ok_or_else(|| "Missing renderer completion".to_owned())??;
    let page = conn.loaded_page_mut_for_protocol_access_for_route(
        owner.session_id(),
        owner.session_owner_route(),
    )?;
    page.finish_continue_pending_subresource_auth(completion)
        .map(|_| ())
        .map_err(|error| format!("subresource auth continue failed: {error}"))
}
