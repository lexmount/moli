use anyhow::Result;

mod auth;
mod body_stream;
mod cleanup;
mod commands;
pub(crate) use cleanup::CompletedSubresourceFailure;
pub(crate) use cleanup::PreparedFetchCleanup;
mod document_navigation;
mod document_request;
mod helpers;
mod navigation;
mod params;
mod patterns;
mod state;
mod subresource;
mod subresource_continuation;
pub(crate) use subresource_continuation::{
    PreparedSubresourceAutoContinue, SubresourceAutoContinueCompletion,
};

use crate::automation::{
    AutomationCommand, AutomationResult, DevToolsAddNetworkInterceptCommand,
    DevToolsAddNetworkInterceptResult, DevToolsError, DevToolsErrorKind,
    DevToolsNetworkInterceptPhase, FrontendProtocol,
};
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, FetchInterceptionPattern,
    FetchRequestStage as ConnFetchRequestStage,
};
use crate::domains::actions::FetchAction;
use crate::domains::command_output::{CommandOutputPlan, devtools_error_from_cdp_error_parts};
use serde_json::json;

#[cfg(test)]
pub(crate) use crate::conn::FetchAuthChallenge;
#[cfg(test)]
pub(crate) use crate::conn::FetchRequestStage;
#[cfg(test)]
pub(crate) use crate::conn::FetchResourceTypeFilter;
#[cfg(test)]
pub(crate) use crate::conn::PendingFetchAuthNavigation;
#[cfg(test)]
pub(crate) use crate::conn::PendingFetchNavigation;
pub(crate) use document_request::{
    FetchNavigationResponse, PreparedFetchNavigationResponse, accept_navigation_response,
};
#[cfg(test)]
pub(crate) use helpers::encode_basic_auth;
pub(crate) use helpers::request_paused_background_event;
#[cfg(test)]
use helpers::response_headers_from_params;
#[cfg(test)]
pub(crate) use helpers::{emit_auth_required, extract_auth_challenge, request_auth_for_challenge};
pub(crate) use helpers::{
    pending_subresource_auth_required_event,
    pending_subresource_response_stage_request_paused_event, populate_auth_challenge_origin,
};
#[cfg(test)]
pub(crate) use moli_fetch::url_pattern_matches;
use params::EnableParams;
use patterns::supported_pattern_config;
pub(crate) use subresource::{
    detached_parser_script_fetch_pause_prepared_outputs_for_renderer_record,
    emit_subresource_fetch_pause_outputs,
    subresource_fetch_pause_prepared_outputs_for_renderer_record,
};

pub(in crate::domains) fn prepare_owner_disposal(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Option<PreparedFetchCleanup> {
    let (pending, policy) = conn.start_disable_fetch_for_session_owner(session_id)?;
    let owner = CommandOwnerScope::capture(conn, session_id);
    Some(PreparedFetchCleanup::new(
        conn,
        &owner,
        pending,
        policy,
        cleanup::FetchCleanupReason::Detached,
        true,
    ))
}

pub(in crate::domains) fn prepare_target_close(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> Option<PreparedFetchCleanup> {
    let (pending, policy) = conn.start_disable_target_fetch_for_owner(owner)?;
    Some(PreparedFetchCleanup::new(
        conn,
        owner,
        pending,
        policy,
        cleanup::FetchCleanupReason::TargetClosed,
        false,
    ))
}

pub(in crate::domains) fn prepare_page_close(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    pending: FetchDisablePendingState,
) -> PreparedFetchCleanup {
    PreparedFetchCleanup::new(
        conn,
        owner,
        pending,
        None,
        cleanup::FetchCleanupReason::PageClosed,
        true,
    )
}

pub(in crate::domains) fn prepare_browser_context_disposal(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    mut pending: FetchDisablePendingState,
) -> PreparedFetchCleanup {
    // The renderer lifetime retires subresource promises without synthetic
    // loadingFailed events; only main-document commands still owe terminals.
    pending.3.clear();
    pending.4.clear();
    pending.5.clear();
    PreparedFetchCleanup::new(
        conn,
        owner,
        pending,
        None,
        cleanup::FetchCleanupReason::BrowserContextDisposed,
        false,
    )
}

pub(in crate::domains) fn prepare_page_stop_loading(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    pending: FetchDisablePendingState,
) -> PreparedFetchCleanup {
    PreparedFetchCleanup::new(
        conn,
        owner,
        pending,
        None,
        cleanup::FetchCleanupReason::NavigationStopped,
        true,
    )
}

pub(in crate::domains) fn prepare_page_crash(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    pending: FetchDisablePendingState,
) -> PreparedFetchCleanup {
    PreparedFetchCleanup::new(
        conn,
        owner,
        pending,
        None,
        cleanup::FetchCleanupReason::PageCrashed,
        false,
    )
}

pub(crate) struct PendingFetchCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingFetchCommandKind,
    pending: PendingFetchCommandOperation,
    navigation_termination: Option<navigation::NavigationTermination>,
}

pub(crate) struct CompletedFetchCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingFetchCommandKind,
    completed: CompletedFetchCommandOperation,
    navigation_termination: Option<navigation::NavigationTermination>,
}

pub(crate) enum FetchCommandTaskStep {
    Pending(PendingFetchCommandDispatch),
    Complete(CommandOutputPlan),
}

pub(crate) enum FetchCommandCompletion {
    Complete(CommandOutputPlan),
    Workflow(crate::conn::PreparedCommandWorkflow),
}

enum PendingFetchCommandKind {
    DocumentNavigation,
    Enable,
    AddNetworkIntercept {
        intercept_id: String,
    },
    RemoveNetworkIntercept,
    Cleanup,
    ContinueRequest {
        state: Box<commands::PendingContinueRequestState>,
    },
    ContinueWithAuth {
        state: Box<auth::PendingContinueWithAuthState>,
    },
    FailRequest {
        state: Box<commands::PendingFailRequestState>,
    },
    FulfillRequest {
        state: Box<commands::PendingFulfillRequestState>,
    },
    DispatchWebSocketMessage {
        operation: commands::PendingWebSocketCommandOperation,
    },
    CloseWebSocket,
    ContinueResponse {
        state: Box<commands::PendingContinueResponseState>,
    },
    GetResponseBody,
}

enum PendingFetchCommandOperation {
    Cleanup(Box<cleanup::PreparedFetchCleanup>),
    DocumentNavigation(Box<document_navigation::PendingDocumentNavigation>),
    Page(moli_core::page::PendingPageCommand),
    MaterializeResponseBody(Box<body_stream::PreparedFetchBodyRead>),
}

enum CompletedFetchCommandOperation {
    Cleanup(Box<cleanup::CompletedFetchCleanup>),
    DocumentNavigation(Box<document_navigation::CompletedDocumentNavigation>),
    Page(Box<Result<moli_core::page::CompletedPageCommand>>),
    MaterializeResponseBody(Box<body_stream::CompletedFetchBodyRead>),
}

type FetchDisablePendingState = (
    Vec<crate::conn::PendingFetchNavigation>,
    Vec<crate::conn::PendingFetchAuthNavigation>,
    Vec<crate::conn::PausedDocumentTransfer>,
    Vec<(String, crate::conn::PendingSubresourceFetchRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchAuthRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchResponseRequest)>,
);

#[derive(Default)]
pub(super) struct FetchCommandOutput {
    plan: CommandOutputPlan,
    command_status: Option<Result<(), DevToolsError>>,
}

impl FetchCommandOutput {
    fn push_success(&mut self) {
        self.record_command_status(Ok(()));
        self.plan.push_result(json!({}));
    }

    fn push_error(&mut self, code: i32, message: impl std::fmt::Display) {
        let message = format!("{message:#}");
        self.record_command_status(Err(devtools_error_from_cdp_error_parts(
            Some(i64::from(code)),
            &message,
        )));
        self.plan.push_error(code, message);
    }

    fn extend_plan_as_command_response(&mut self, plan: CommandOutputPlan) {
        if let Some(status) = plan.command_status() {
            self.record_command_status(status);
        }
        self.plan.extend(plan);
    }

    fn extend_plan_as_background_events(
        &mut self,
        plan: CommandOutputPlan,
        command_id: Option<u64>,
        session_id: Option<&str>,
    ) {
        self.plan
            .extend(plan.into_background_event_plan(command_id, session_id));
    }

    fn set_renderer_output_predecessor(&mut self, predecessor: moli_core::RendererOutputFence) {
        self.plan.set_renderer_output_predecessor(predecessor);
    }

    fn extend_background_events(
        &mut self,
        events: impl IntoIterator<Item = BackgroundProtocolEvent>,
    ) {
        self.plan.extend_background_events(events);
    }

    fn into_output_plan(self) -> CommandOutputPlan {
        self.plan
    }

    fn record_command_status(&mut self, status: Result<(), DevToolsError>) {
        if self.command_status.is_none() {
            self.command_status = Some(status);
        } else {
            tracing::warn!("fetch command produced multiple command responses");
        }
    }
}

impl PendingFetchCommandDispatch {
    pub(crate) fn wait_lifetime(&self) -> Option<crate::conn::CommandWaitLifetime> {
        self.navigation_termination
            .as_ref()
            .and_then(|termination| termination.wait_lifetime.clone())
    }

    fn new(
        conn: &CdpConnection,
        command_id: Option<u64>,
        session_id: Option<&str>,
        kind: PendingFetchCommandKind,
        pending: PendingFetchCommandOperation,
    ) -> Self {
        let owner = CommandOwnerScope::capture(conn, session_id);
        Self::new_for_owner(command_id, owner, kind, pending)
    }

    fn new_for_owner(
        command_id: Option<u64>,
        owner_scope: CommandOwnerScope,
        kind: PendingFetchCommandKind,
        pending: PendingFetchCommandOperation,
    ) -> Self {
        Self {
            command_id,
            owner_scope,
            kind,
            pending,
            navigation_termination: None,
        }
    }

    fn with_navigation_termination(
        mut self,
        termination: Option<navigation::NavigationTermination>,
    ) -> Self {
        self.navigation_termination = termination;
        self
    }

    pub(crate) async fn wait(self) -> CompletedFetchCommandDispatch {
        let completed = match self.pending {
            PendingFetchCommandOperation::Cleanup(pending) => {
                let completed = Box::pin(pending.wait()).await;
                CompletedFetchCommandOperation::Cleanup(Box::new(completed))
            }
            PendingFetchCommandOperation::DocumentNavigation(pending) => {
                let completed = Box::pin(pending.wait()).await;
                CompletedFetchCommandOperation::DocumentNavigation(Box::new(completed))
            }
            PendingFetchCommandOperation::Page(pending) => {
                CompletedFetchCommandOperation::Page(Box::new(pending.wait().await))
            }
            PendingFetchCommandOperation::MaterializeResponseBody(read) => {
                CompletedFetchCommandOperation::MaterializeResponseBody(Box::new(
                    Box::pin(read.run()).await,
                ))
            }
        };
        CompletedFetchCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            kind: self.kind,
            completed,
            navigation_termination: self.navigation_termination,
        }
    }
}

impl CompletedFetchCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

impl CompletedFetchCommandOperation {
    fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        match self {
            Self::Page(completed) => completed
                .as_ref()
                .as_ref()
                .ok()
                .and_then(moli_core::page::CompletedPageCommand::renderer_output_predecessor),
            Self::MaterializeResponseBody(_) | Self::DocumentNavigation(_) | Self::Cleanup(_) => {
                None
            }
        }
    }

    fn into_page_completion(self) -> Option<Result<moli_core::page::CompletedPageCommand>> {
        match self {
            Self::Page(completed) => Some(*completed),
            Self::MaterializeResponseBody(_) | Self::DocumentNavigation(_) | Self::Cleanup(_) => {
                None
            }
        }
    }
}

pub(crate) fn try_start_fetch_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<FetchCommandTaskStep> {
    match cmd.parse_action::<FetchAction>() {
        Some(FetchAction::Enable) => Some(start_enable_command(conn, cmd)),
        Some(FetchAction::Disable) => Some(start_disable_command(conn, cmd)),
        Some(FetchAction::ContinueRequest) => {
            Some(commands::start_continue_request_command(conn, cmd))
        }
        Some(FetchAction::ContinueWithAuth) => {
            Some(auth::start_continue_with_auth_command(conn, cmd))
        }
        Some(FetchAction::FailRequest) => Some(commands::start_fail_request_command(conn, cmd)),
        Some(FetchAction::FulfillRequest) => {
            Some(commands::start_fulfill_request_command(conn, cmd))
        }
        Some(FetchAction::ContinueResponse) => {
            Some(commands::start_continue_response_command(conn, cmd))
        }
        Some(FetchAction::DispatchWebSocketMessage) => Some(
            commands::start_dispatch_websocket_message_command(conn, cmd),
        ),
        Some(FetchAction::CloseWebSocket) => {
            Some(commands::start_close_websocket_command(conn, cmd))
        }
        Some(FetchAction::GetResponseBody) => {
            Some(body_stream::start_get_response_body_command(conn, cmd))
        }
        Some(FetchAction::TakeResponseBodyAsStream) => Some(FetchCommandTaskStep::Complete(
            body_stream::take_response_body_as_stream_command(conn, cmd),
        )),
        None => Some(FetchCommandTaskStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        ))),
    }
}

pub(crate) fn devtools_fetch_success_result(command: &AutomationCommand) -> AutomationResult {
    match command {
        AutomationCommand::AddNetworkIntercept(command) => {
            AutomationResult::AddNetworkIntercept(DevToolsAddNetworkInterceptResult {
                intercept_id: command.intercept_id.clone(),
            })
        }
        _ => AutomationResult::Empty,
    }
}

pub(crate) fn start_devtools_fetch_command_for_owner(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: AutomationCommand,
) -> FetchCommandTaskStep {
    match &command {
        AutomationCommand::AddNetworkIntercept(command) => {
            start_devtools_add_network_intercept_command(conn, command_id, owner, command)
        }
        AutomationCommand::RemoveNetworkIntercept(command) => {
            start_devtools_remove_network_intercept_command(
                conn,
                command_id,
                owner,
                command.intercept_id.as_str(),
                command.context.protocol != FrontendProtocol::Cdp
                    && command.context.target_id.is_none(),
            )
        }
        _ => commands::start_devtools_fetch_command_for_owner(conn, command_id, owner, command),
    }
}

pub(crate) fn fetch_automation_command_owner(
    conn: &CdpConnection,
    command: &AutomationCommand,
) -> Result<CommandOwnerScope, DevToolsError> {
    let (context, request_id) = match command {
        AutomationCommand::AddNetworkIntercept(command) => {
            return fetch_config_automation_command_owner(conn, &command.context);
        }
        AutomationCommand::RemoveNetworkIntercept(command) => {
            return fetch_config_automation_command_owner(conn, &command.context);
        }
        AutomationCommand::ContinueInterceptedRequest(command) => {
            (&command.context, command.request_id.as_str())
        }
        AutomationCommand::ContinueInterceptedResponse(command) => {
            (&command.context, command.request_id.as_str())
        }
        AutomationCommand::ContinueWithAuth(command) => {
            (&command.context, command.request_id.as_str())
        }
        AutomationCommand::FailInterceptedRequest(command) => {
            (&command.context, command.request_id.as_str())
        }
        AutomationCommand::FulfillInterceptedRequest(command) => {
            (&command.context, command.request_id.as_str())
        }
        _ => return Ok(CommandOwnerScope::capture(conn, None)),
    };
    if context.protocol == FrontendProtocol::Cdp {
        return Ok(CommandOwnerScope::capture(
            conn,
            context.session_id.as_ref().map(|session| session.as_str()),
        ));
    }
    Ok(conn
        .pending_fetch_request_session_route(request_id)
        .map(CommandOwnerScope::for_route)
        .unwrap_or_else(|| CommandOwnerScope::capture(conn, None)))
}

fn fetch_config_automation_command_owner(
    conn: &CdpConnection,
    context: &crate::automation::AutomationContext,
) -> Result<CommandOwnerScope, DevToolsError> {
    if context.protocol == FrontendProtocol::Cdp {
        return Ok(CommandOwnerScope::capture(
            conn,
            context.session_id.as_ref().map(|session| session.as_str()),
        ));
    }
    if let Some(target_id) = context.target_id.as_ref() {
        let route = conn
            .target_session_route_for_target_id(target_id.as_str())
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        Ok(CommandOwnerScope::for_route(route))
    } else {
        Ok(CommandOwnerScope::capture(conn, None))
    }
}

fn start_enable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> FetchCommandTaskStep {
    let params: EnableParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => EnableParams::default(),
        Err(_) => {
            return FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };

    let patterns = match supported_pattern_config(&params.patterns) {
        Ok(patterns) => patterns,
        Err(()) => {
            return FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };

    match conn.start_enable_fetch_for_session_owner(
        cmd.session_id,
        params.handle_auth_requests,
        patterns,
    ) {
        Ok(Some(pending)) => FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new(
            conn,
            cmd.id,
            cmd.session_id,
            PendingFetchCommandKind::Enable,
            PendingFetchCommandOperation::Page(pending),
        )),
        Ok(None) => FetchCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(message) if message.is::<crate::conn::BrowserContextNotLoaded>() => {
            FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                "BrowserContextNotLoaded",
            ))
        }
        Err(message) => {
            FetchCommandTaskStep::Complete(CommandOutputPlan::error(-32000, format!("{message:#}")))
        }
    }
}

fn start_devtools_add_network_intercept_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: &DevToolsAddNetworkInterceptCommand,
) -> FetchCommandTaskStep {
    let (handle_auth_requests, auth_url_patterns, patterns) =
        network_intercept_fetch_config(command);
    let intercept_session_id = if command.context.protocol == FrontendProtocol::Cdp {
        owner.session_id().map(str::to_owned)
    } else {
        command
            .context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str().to_owned())
    };
    match conn.start_add_network_intercept_for_owner(
        owner,
        intercept_session_id,
        command.intercept_id.as_str().to_owned(),
        handle_auth_requests,
        auth_url_patterns,
        patterns,
    ) {
        Ok(Some(pending)) => {
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
                command_id,
                owner.clone(),
                PendingFetchCommandKind::AddNetworkIntercept {
                    intercept_id: command.intercept_id.as_str().to_owned(),
                },
                PendingFetchCommandOperation::Page(pending),
            ))
        }
        Ok(None) => FetchCommandTaskStep::Complete(CommandOutputPlan::from_devtools_result(
            AutomationResult::AddNetworkIntercept(DevToolsAddNetworkInterceptResult {
                intercept_id: command.intercept_id.clone(),
            }),
        )),
        Err(message) if message.is::<crate::conn::BrowserContextNotLoaded>() => {
            FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                "BrowserContextNotLoaded",
            ))
        }
        Err(message) => {
            FetchCommandTaskStep::Complete(CommandOutputPlan::error(-32000, format!("{message:#}")))
        }
    }
}

fn start_devtools_remove_network_intercept_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    intercept_id: &str,
    allow_global_lookup: bool,
) -> FetchCommandTaskStep {
    match conn.start_remove_network_intercept_for_owner(owner, intercept_id, allow_global_lookup) {
        Ok(Some(pending)) => {
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
                command_id,
                owner.clone(),
                PendingFetchCommandKind::RemoveNetworkIntercept,
                PendingFetchCommandOperation::Page(pending),
            ))
        }
        Ok(None) => FetchCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(message) if message.is::<crate::conn::NetworkInterceptNotFound>() => {
            FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "NetworkInterceptNotFound",
            ))
        }
        Err(message) if message.is::<crate::conn::BrowserContextNotLoaded>() => {
            FetchCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                "BrowserContextNotLoaded",
            ))
        }
        Err(message) => {
            FetchCommandTaskStep::Complete(CommandOutputPlan::error(-32000, format!("{message:#}")))
        }
    }
}

fn network_intercept_fetch_config(
    command: &DevToolsAddNetworkInterceptCommand,
) -> (bool, Vec<String>, Vec<FetchInterceptionPattern>) {
    let handle_auth_requests = command
        .phases
        .contains(&DevToolsNetworkInterceptPhase::AuthRequired);
    let auth_url_patterns = if handle_auth_requests {
        if command.url_patterns.is_empty() {
            vec!["*".to_owned()]
        } else {
            command
                .url_patterns
                .iter()
                .map(|pattern| pattern.url_pattern.clone())
                .collect()
        }
    } else {
        Vec::new()
    };
    let mut patterns = Vec::new();
    for request_stage in [
        ConnFetchRequestStage::Request,
        ConnFetchRequestStage::Response,
    ] {
        let phase = match request_stage {
            ConnFetchRequestStage::Request => DevToolsNetworkInterceptPhase::BeforeRequestSent,
            ConnFetchRequestStage::Response => DevToolsNetworkInterceptPhase::ResponseStarted,
        };
        if !command.phases.contains(&phase) {
            continue;
        }
        if command.url_patterns.is_empty() {
            patterns.push(FetchInterceptionPattern {
                url_pattern: "*".to_owned(),
                resource_type_filter: None,
                request_stage,
            });
            continue;
        }
        patterns.extend(
            command
                .url_patterns
                .iter()
                .map(|pattern| FetchInterceptionPattern {
                    url_pattern: pattern.url_pattern.clone(),
                    resource_type_filter: None,
                    request_stage,
                }),
        );
    }
    (handle_auth_requests, auth_url_patterns, patterns)
}

pub(crate) fn complete_pending_fetch_command(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> FetchCommandCompletion {
    if let CompletedFetchCommandOperation::DocumentNavigation(document) = completed.completed {
        FetchCommandCompletion::Workflow(document_navigation::complete_owned(
            conn.protocol_state_access(),
            *document,
            completed.navigation_termination,
        ))
    } else {
        FetchCommandCompletion::Complete(complete_finite_fetch_command(conn, completed))
    }
}

fn complete_finite_fetch_command(
    conn: &mut CdpConnection,
    mut completed: CompletedFetchCommandDispatch,
) -> CommandOutputPlan {
    let mut termination = completed.navigation_termination.take();
    let owner = completed.owner_scope.clone();
    let mut output = FetchCommandOutput::default();
    if let Some(predecessor) = completed.completed.renderer_output_predecessor() {
        output.set_renderer_output_predecessor(predecessor);
    }
    match completed.kind {
        PendingFetchCommandKind::Enable => {
            output.extend_plan_as_command_response(complete_enable_command(conn, completed))
        }
        PendingFetchCommandKind::AddNetworkIntercept { ref intercept_id } => {
            let result = AutomationResult::AddNetworkIntercept(DevToolsAddNetworkInterceptResult {
                intercept_id: intercept_id.clone().into(),
            });
            output.extend_plan_as_command_response(complete_fetch_config_update_command(
                conn, completed, result,
            ));
        }
        PendingFetchCommandKind::RemoveNetworkIntercept => output.extend_plan_as_command_response(
            complete_fetch_config_update_command(conn, completed, AutomationResult::Empty),
        ),
        PendingFetchCommandKind::Cleanup => {
            let CompletedFetchCommandOperation::Cleanup(cleanup) = completed.completed else {
                unreachable!("checked cleanup result")
            };
            output.extend_plan_as_command_response(cleanup.into_command_plan());
        }
        PendingFetchCommandKind::ContinueRequest { state } => {
            commands::complete_continue_request_command(
                conn,
                completed.completed.into_page_completion(),
                *state,
                &mut output,
            )
        }
        PendingFetchCommandKind::ContinueWithAuth { state } => {
            auth::complete_continue_with_auth_command(
                conn,
                &owner,
                completed.completed.into_page_completion(),
                *state,
                &mut output,
            )
        }
        PendingFetchCommandKind::FailRequest { state } => commands::complete_fail_request_command(
            conn,
            &owner,
            completed.completed.into_page_completion(),
            *state,
            &mut output,
        ),
        PendingFetchCommandKind::FulfillRequest { state } => {
            commands::complete_fulfill_request_command(
                conn,
                &owner,
                completed.completed.into_page_completion(),
                *state,
                &mut output,
            )
        }
        PendingFetchCommandKind::ContinueResponse { state } => {
            commands::complete_continue_response_command(
                conn,
                &owner,
                completed.completed.into_page_completion(),
                *state,
                &mut output,
            )
        }
        PendingFetchCommandKind::DispatchWebSocketMessage { operation } => {
            commands::complete_websocket_page_command(
                conn,
                &owner,
                completed.completed.into_page_completion(),
                operation,
                &mut output,
            )
        }
        PendingFetchCommandKind::CloseWebSocket => commands::complete_websocket_page_command(
            conn,
            &owner,
            completed.completed.into_page_completion(),
            commands::PendingWebSocketCommandOperation::Close,
            &mut output,
        ),
        PendingFetchCommandKind::GetResponseBody => {
            body_stream::complete_get_response_body_from_transfer(
                conn,
                &owner,
                completed.completed,
                &mut output,
            )
        }
        PendingFetchCommandKind::DocumentNavigation => {
            unreachable!("document completion owns external waits")
        }
    }
    if let Some(termination) = termination.as_mut() {
        termination.disarm();
    }
    output.into_output_plan()
}

fn complete_enable_command(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> CommandOutputPlan {
    complete_fetch_config_update_command(conn, completed, AutomationResult::Empty)
}

fn complete_fetch_config_update_command(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
    result: AutomationResult,
) -> CommandOutputPlan {
    let owner_scope = completed.owner_scope.clone();
    let Some(completed_page_command) = completed.completed.into_page_completion() else {
        return CommandOutputPlan::error(-32000, "Missing renderer completion");
    };
    let completion = match completed_page_command {
        Ok(completion) => completion,
        Err(error) => return CommandOutputPlan::error(-32000, format!("{error:#}")),
    };
    // Fetch policy belongs to the target and may finish updating while a
    // navigation is replacing its document.
    let Some(page) = conn.loaded_page_mut_for_owner(&owner_scope) else {
        return CommandOutputPlan::from_devtools_result(result);
    };
    match page.finish_set_fetch_subresource_interception(completion) {
        Ok(()) => CommandOutputPlan::from_devtools_result(result),
        Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
    }
}

fn start_disable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> FetchCommandTaskStep {
    match conn.start_disable_fetch_for_session_owner(cmd.session_id) {
        Some((pending_fetch_state, pending)) => {
            let owner = CommandOwnerScope::capture(conn, cmd.session_id);
            let cleanup = cleanup::PreparedFetchCleanup::new(
                conn,
                &owner,
                pending_fetch_state,
                pending,
                cleanup::FetchCleanupReason::Disabled,
                false,
            );
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
                cmd.id,
                owner,
                PendingFetchCommandKind::Cleanup,
                PendingFetchCommandOperation::Cleanup(Box::new(cleanup)),
            ))
        }
        None => FetchCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        )),
    }
}

#[cfg(test)]
mod tests;

pub(crate) use document_request::PreparedInterceptedNavigationRequest;
pub(crate) use navigation::NavigationTermination;
