mod auth;
mod body_stream;
mod commands;
mod helpers;
mod navigation;
mod params;
mod patterns;
mod state;
mod subresource;

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, CompletedDocumentFetchCommand,
    DevToolsCommandExecutionOutput, DocumentFetchCommand, DocumentFetchCommandOutcome,
    FetchInterceptionPattern, FetchRequestStage as ConnFetchRequestStage,
    PendingDocumentFetchCommand,
};
use crate::devtools_runtime::{
    DevToolsAddNetworkInterceptCommand, DevToolsAddNetworkInterceptResult, DevToolsCommand,
    DevToolsCommandResult, DevToolsError, DevToolsErrorKind, DevToolsNetworkInterceptPhase,
    DevToolsProtocol,
};
use crate::domains::actions::FetchAction;
use crate::domains::command_output::{CommandOutputPlan, devtools_error_from_cdp_error_parts};
use crate::domains::{activity, page};
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
pub(crate) use navigation::continue_navigation_without_request_pause_into_buffer_async;
use params::EnableParams;
use patterns::supported_pattern_config;
pub(crate) use subresource::{
    detached_parser_script_fetch_pause_prepared_outputs_for_renderer_record_async,
    emit_subresource_fetch_pause_outputs,
    subresource_fetch_pause_prepared_outputs_for_renderer_record_async,
};

/// Disables the Fetch handler owned by one DevTools session and drains every
/// request that was paused by that handler before its binding is removed.
pub(in crate::domains) async fn dispose_session_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
    renderer_policy_reconciled: bool,
) -> anyhow::Result<Option<moli_core::RendererOutputFence>> {
    dispose_owner_async(conn, out, Some(session_id), renderer_policy_reconciled).await
}

pub(in crate::domains) async fn dispose_owner_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    renderer_policy_reconciled: bool,
) -> anyhow::Result<Option<moli_core::RendererOutputFence>> {
    let owner = CommandOwnerScope::capture(conn, session_id);
    let Some((pending_fetch_state, pending_page_command)) =
        conn.start_dispose_fetch_for_session_owner(session_id, renderer_policy_reconciled)
    else {
        return Ok(None);
    };

    let mut renderer_cleanup_error = None;
    let pending_page_command = match pending_page_command {
        Ok(pending) => pending,
        Err(error) => {
            renderer_cleanup_error = Some(anyhow::Error::msg(error));
            None
        }
    };
    if let Some(pending_page_command) = pending_page_command
        && let Err(message) = conn.finish_document_fetch_command(pending_page_command.wait().await)
    {
        renderer_cleanup_error = Some(anyhow::anyhow!(
            "failed to finish Fetch interception disable while disposing session: {message}"
        ));
    }

    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = pending_fetch_state;
    // Session teardown has no command response to fence. The concrete
    // renderer publication remains ordered on its own stream and will reach
    // protocol ingress independently.
    let mut navigation_output = FetchCommandOutput::default();
    release_main_document_interceptions_neutrally_async(
        conn,
        &mut navigation_output,
        owner.session_id(),
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
    )
    .await;
    let mut navigation_plan = navigation_output.into_output_plan();
    let predecessor = navigation_plan.take_renderer_output_predecessor();
    let (_, navigation_events) = navigation_plan.into_command_status_and_background_events();
    out.extend(navigation_events);

    let subresource_predecessor = page::fail_pending_fetch_state_background_events_async(
        conn,
        out,
        session_id,
        "Target detached",
        "Target detached",
        Vec::new(),
        Vec::new(),
        Vec::new(),
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await;
    let predecessor =
        merge_optional_renderer_output_predecessors(predecessor, subresource_predecessor);
    if let Some(error) = renderer_cleanup_error {
        return Err(error);
    }
    Ok(predecessor)
}

pub(crate) struct PendingFetchCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingFetchCommandKind,
    pending: PendingFetchCommandOperation,
}

pub(crate) struct CompletedFetchCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingFetchCommandKind,
    completed: CompletedFetchCommandOperation,
}

pub(crate) enum FetchCommandTaskStep {
    Pending(PendingFetchCommandDispatch),
    Complete(CommandOutputPlan),
}

enum PendingFetchCommandKind {
    Enable,
    AddNetworkIntercept {
        intercept_id: String,
    },
    RemoveNetworkIntercept,
    Disable {
        pending_fetch_state: Box<FetchDisablePendingState>,
    },
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
    DispatchWebSocketMessage,
    CloseWebSocket,
    ContinueResponse {
        state: Box<commands::PendingContinueResponseState>,
    },
    GetResponseBody,
}

enum PendingFetchCommandOperation {
    Ready,
    DocumentFetch(Result<PendingDocumentFetchCommand, String>),
    MaterializeResponseBody {
        request_id: String,
        transfer: Box<crate::conn::PausedDocumentTransfer>,
        limit: usize,
    },
}

enum CompletedFetchCommandOperation {
    Ready,
    DocumentFetch(Box<Result<CompletedDocumentFetchCommand, String>>),
    MaterializeResponseBody {
        request_id: String,
        result: Box<
            Result<
                (Option<Vec<u8>>, crate::conn::PausedDocumentTransfer),
                (String, crate::conn::PausedDocumentTransfer),
            >,
        >,
    },
}

fn start_document_fetch_command_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    command: DocumentFetchCommand,
) -> Result<PendingDocumentFetchCommand, String> {
    let document = conn.resolve_browser_document_for_owner(owner)?;
    conn.start_document_fetch_command(document, command)
}

fn finish_document_fetch_command(
    conn: &mut CdpConnection,
    completed: Option<Result<CompletedDocumentFetchCommand, String>>,
) -> Result<DocumentFetchCommandOutcome, String> {
    let completion = completed.ok_or_else(|| "Missing renderer completion".to_owned())??;
    conn.finish_document_fetch_command(completion)
}

type FetchDisablePendingState = (
    Vec<crate::conn::PendingFetchNavigation>,
    Vec<crate::conn::PendingFetchAuthNavigation>,
    Vec<crate::conn::PendingFetchResponseNavigation>,
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

    fn push_error(&mut self, code: i32, message: impl AsRef<str>) {
        let message = message.as_ref();
        self.record_command_status(Err(devtools_error_from_cdp_error_parts(
            Some(i64::from(code)),
            message,
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

    fn into_devtools_result_and_background_events(
        mut self,
        success_result: DevToolsCommandResult,
    ) -> DevToolsCommandExecutionOutput {
        let status = self.command_status.unwrap_or_else(|| {
            Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "MissingFetchCommandResponse",
            ))
        });
        let renderer_output_predecessor = self.plan.take_renderer_output_predecessor();
        let (_, events) = self.plan.into_command_status_and_background_events();
        DevToolsCommandExecutionOutput::from_parts(
            status.map(|()| success_result),
            events,
            renderer_output_predecessor,
        )
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
        }
    }

    pub(crate) async fn wait(self) -> CompletedFetchCommandDispatch {
        let completed = match self.pending {
            PendingFetchCommandOperation::Ready => CompletedFetchCommandOperation::Ready,
            PendingFetchCommandOperation::DocumentFetch(pending) => {
                CompletedFetchCommandOperation::DocumentFetch(Box::new(match pending {
                    Ok(pending) => Ok(pending.wait().await),
                    Err(error) => Err(error),
                }))
            }
            PendingFetchCommandOperation::MaterializeResponseBody {
                request_id,
                transfer,
                limit,
            } => CompletedFetchCommandOperation::MaterializeResponseBody {
                request_id,
                result: Box::new(transfer.materialize_body_limited_async(limit).await),
            },
        };
        CompletedFetchCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            kind: self.kind,
            completed,
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
            Self::DocumentFetch(completed) => completed
                .as_ref()
                .as_ref()
                .ok()
                .and_then(CompletedDocumentFetchCommand::renderer_output_predecessor),
            Self::Ready | Self::MaterializeResponseBody { .. } => None,
        }
    }

    fn into_document_fetch_completion(
        self,
    ) -> Option<Result<CompletedDocumentFetchCommand, String>> {
        match self {
            Self::DocumentFetch(completed) => Some(*completed),
            Self::Ready | Self::MaterializeResponseBody { .. } => None,
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

pub(crate) async fn execute_devtools_fetch_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> DevToolsCommandExecutionOutput {
    let success_result = devtools_fetch_success_result(&command);
    let owner = match fetch_devtools_command_owner(conn, &command) {
        Ok(owner) => owner,
        Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
    };
    let step = start_devtools_fetch_command_for_owner(conn, None, &owner, command);
    match step {
        FetchCommandTaskStep::Complete(mut plan) => {
            let renderer_output_predecessor = plan.take_renderer_output_predecessor();
            let (status, events) = plan.into_command_status_and_background_events();
            DevToolsCommandExecutionOutput::from_parts(
                status
                    .unwrap_or_else(|| {
                        Err(DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            "MissingFetchCommandResponse",
                        ))
                    })
                    .map(|()| success_result),
                events,
                renderer_output_predecessor,
            )
        }
        FetchCommandTaskStep::Pending(pending) => {
            let completed = pending.wait().await;
            complete_pending_devtools_fetch_command(conn, completed)
                .await
                .into_devtools_result_and_background_events(success_result)
        }
    }
}

fn devtools_fetch_success_result(command: &DevToolsCommand) -> DevToolsCommandResult {
    match command {
        DevToolsCommand::AddNetworkIntercept(command) => {
            DevToolsCommandResult::AddNetworkIntercept(DevToolsAddNetworkInterceptResult {
                intercept_id: command.intercept_id.clone(),
            })
        }
        _ => DevToolsCommandResult::Empty,
    }
}

fn start_devtools_fetch_command_for_owner(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsCommand,
) -> FetchCommandTaskStep {
    match &command {
        DevToolsCommand::AddNetworkIntercept(command) => {
            start_devtools_add_network_intercept_command(conn, command_id, owner, command)
        }
        DevToolsCommand::RemoveNetworkIntercept(command) => {
            start_devtools_remove_network_intercept_command(
                conn,
                command_id,
                owner,
                command.intercept_id.as_str(),
                command.context.protocol != DevToolsProtocol::Cdp
                    && command.context.target_id.is_none(),
            )
        }
        _ => commands::start_devtools_fetch_command_for_owner(conn, command_id, owner, command),
    }
}

fn fetch_devtools_command_owner(
    conn: &CdpConnection,
    command: &DevToolsCommand,
) -> Result<CommandOwnerScope, DevToolsError> {
    let (context, request_id) = match command {
        DevToolsCommand::AddNetworkIntercept(command) => {
            return fetch_config_devtools_command_owner(conn, &command.context);
        }
        DevToolsCommand::RemoveNetworkIntercept(command) => {
            return fetch_config_devtools_command_owner(conn, &command.context);
        }
        DevToolsCommand::ContinueInterceptedRequest(command) => {
            (&command.context, command.request_id.as_str())
        }
        DevToolsCommand::ContinueInterceptedResponse(command) => {
            (&command.context, command.request_id.as_str())
        }
        DevToolsCommand::ContinueWithAuth(command) => {
            (&command.context, command.request_id.as_str())
        }
        DevToolsCommand::FailInterceptedRequest(command) => {
            (&command.context, command.request_id.as_str())
        }
        DevToolsCommand::FulfillInterceptedRequest(command) => {
            (&command.context, command.request_id.as_str())
        }
        _ => return Ok(CommandOwnerScope::capture(conn, None)),
    };
    if context.protocol == DevToolsProtocol::Cdp {
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

fn fetch_config_devtools_command_owner(
    conn: &CdpConnection,
    context: &crate::devtools_runtime::DevToolsCommandContext,
) -> Result<CommandOwnerScope, DevToolsError> {
    if context.protocol == DevToolsProtocol::Cdp {
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
            PendingFetchCommandOperation::DocumentFetch(Ok(pending)),
        )),
        Ok(None) => FetchCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(message) if message == "BrowserContextNotLoaded" => FetchCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => FetchCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
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
    let intercept_session_id = if command.context.protocol == DevToolsProtocol::Cdp {
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
                PendingFetchCommandOperation::DocumentFetch(Ok(pending)),
            ))
        }
        Ok(None) => FetchCommandTaskStep::Complete(CommandOutputPlan::from_devtools_result(
            DevToolsCommandResult::AddNetworkIntercept(DevToolsAddNetworkInterceptResult {
                intercept_id: command.intercept_id.clone(),
            }),
        )),
        Err(message) if message == "BrowserContextNotLoaded" => FetchCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => FetchCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
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
                PendingFetchCommandOperation::DocumentFetch(Ok(pending)),
            ))
        }
        Ok(None) => FetchCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(message) if message == "NetworkInterceptNotFound" => FetchCommandTaskStep::Complete(
            CommandOutputPlan::error(-32000, "NetworkInterceptNotFound"),
        ),
        Err(message) if message == "BrowserContextNotLoaded" => FetchCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => FetchCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
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

pub(crate) async fn complete_pending_fetch_command(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> CommandOutputPlan {
    complete_pending_fetch_command_output(conn, completed)
        .await
        .into_output_plan()
}

async fn complete_pending_devtools_fetch_command(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> FetchCommandOutput {
    complete_pending_fetch_command_output(conn, completed).await
}

async fn complete_pending_fetch_command_output(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> FetchCommandOutput {
    complete_pending_fetch_command_inner(conn, completed).await
}

async fn complete_pending_fetch_command_inner(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> FetchCommandOutput {
    let mut out = FetchCommandOutput::default();
    let owner_scope = completed.owner_scope.clone();
    // Every Fetch operation that crossed the renderer Page boundary must make
    // its concrete publication a predecessor of the frontend response. Keep
    // this at the one dispatch join point: command-specific finish helpers
    // consume CompletedPageCommand and must not each recreate the ordering
    // contract.
    if let Some(predecessor) = completed.completed.renderer_output_predecessor() {
        out.set_renderer_output_predecessor(predecessor);
    }
    match completed.kind {
        PendingFetchCommandKind::Enable => {
            out.extend_plan_as_command_response(complete_enable_command(conn, completed));
        }
        PendingFetchCommandKind::AddNetworkIntercept { ref intercept_id } => {
            let result_intercept_id = intercept_id.clone();
            out.extend_plan_as_command_response(complete_fetch_config_update_command(
                conn,
                completed,
                DevToolsCommandResult::AddNetworkIntercept(DevToolsAddNetworkInterceptResult {
                    intercept_id: result_intercept_id.into(),
                }),
            ));
        }
        PendingFetchCommandKind::RemoveNetworkIntercept => {
            out.extend_plan_as_command_response(complete_fetch_config_update_command(
                conn,
                completed,
                DevToolsCommandResult::Empty,
            ));
        }
        PendingFetchCommandKind::Disable {
            pending_fetch_state,
        } => {
            complete_disable_command_async(
                conn,
                &owner_scope,
                completed.completed.into_document_fetch_completion(),
                *pending_fetch_state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::ContinueRequest { state } => {
            commands::complete_continue_request_command_async(
                conn,
                completed.completed.into_document_fetch_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::ContinueWithAuth { state } => {
            auth::complete_continue_with_auth_command_async(
                conn,
                &owner_scope,
                completed.completed.into_document_fetch_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::FailRequest { state } => {
            commands::complete_fail_request_command_async(
                conn,
                &owner_scope,
                completed.completed.into_document_fetch_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::FulfillRequest { state } => {
            commands::complete_fulfill_request_command_async(
                conn,
                &owner_scope,
                completed.completed.into_document_fetch_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::DispatchWebSocketMessage => {
            commands::complete_websocket_page_command(
                conn,
                completed.completed.into_document_fetch_completion(),
                &mut out,
            );
        }
        PendingFetchCommandKind::CloseWebSocket => {
            commands::complete_websocket_page_command(
                conn,
                completed.completed.into_document_fetch_completion(),
                &mut out,
            );
        }
        PendingFetchCommandKind::ContinueResponse { state } => {
            commands::complete_continue_response_command_async(
                conn,
                &owner_scope,
                completed.completed.into_document_fetch_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::GetResponseBody => {
            body_stream::complete_get_response_body_from_transfer(
                conn,
                &owner_scope,
                completed.completed,
                &mut out,
            );
        }
    }
    out
}

fn complete_enable_command(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> CommandOutputPlan {
    complete_fetch_config_update_command(conn, completed, DevToolsCommandResult::Empty)
}

fn complete_fetch_config_update_command(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
    result: DevToolsCommandResult,
) -> CommandOutputPlan {
    let Some(completed_page_command) = completed.completed.into_document_fetch_completion() else {
        return CommandOutputPlan::error(-32000, "Missing renderer completion");
    };
    let completion = match completed_page_command {
        Ok(completion) => completion,
        Err(error) => return CommandOutputPlan::error(-32000, error),
    };
    let finish = conn.finish_document_fetch_command(completion);
    match finish {
        Ok(_) => CommandOutputPlan::from_devtools_result(result),
        Err(error) => CommandOutputPlan::error(-32000, error),
    }
}

fn start_disable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> FetchCommandTaskStep {
    match conn.start_disable_fetch_for_session_owner(cmd.session_id) {
        Some((pending_fetch_state, pending)) => {
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new(
                conn,
                cmd.id,
                cmd.session_id,
                PendingFetchCommandKind::Disable {
                    pending_fetch_state: Box::new(pending_fetch_state),
                },
                match pending {
                    Ok(Some(pending)) => PendingFetchCommandOperation::DocumentFetch(Ok(pending)),
                    Ok(None) => PendingFetchCommandOperation::Ready,
                    Err(error) => PendingFetchCommandOperation::DocumentFetch(Err(error)),
                },
            ))
        }
        None => FetchCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        )),
    }
}

async fn complete_disable_command_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completed: Option<Result<CompletedDocumentFetchCommand, String>>,
    pending_fetch_state: FetchDisablePendingState,
    out: &mut FetchCommandOutput,
) {
    let renderer_result = completed
        .map(|completion| {
            let completion = completion?;
            conn.finish_document_fetch_command(completion).map(drop)
        })
        .transpose();
    match renderer_result {
        Ok(_) => out.push_success(),
        Err(error) => out.push_error(
            -32000,
            format!("failed to clear page fetch interception: {error}"),
        ),
    }
    // A disable error must not drop the requests already removed from the
    // session registry. Their original commands still need terminal results.

    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = pending_fetch_state;

    release_main_document_interceptions_neutrally_async(
        conn,
        out,
        owner.session_id(),
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
    )
    .await;
    for (_, pending) in pending_subresource_fetches {
        if let Ok(predecessor) = conn
            .fail_pending_subresource_fetch_for_owner_async(
                owner,
                pending.internal_id,
                "Fetch interception disabled".to_owned(),
            )
            .await
        {
            if let Some(predecessor) = predecessor {
                out.set_renderer_output_predecessor(predecessor);
            }
            let mut events = Vec::new();
            activity::flush_post_subresource_fetch_request_activity_background_events_async(
                conn,
                &mut events,
                owner.session_id(),
                &pending,
            )
            .await;
            out.extend_background_events(events);
        }
    }
    for (_, pending) in pending_subresource_auths {
        if let Ok(predecessor) = conn
            .fail_pending_subresource_auth_for_owner_async(
                owner,
                pending.internal_id,
                "Fetch interception disabled".to_owned(),
            )
            .await
        {
            if let Some(predecessor) = predecessor {
                out.set_renderer_output_predecessor(predecessor);
            }
            let mut events = Vec::new();
            activity::flush_post_subresource_auth_activity_background_events_async(
                conn,
                &mut events,
                owner.session_id(),
                &pending,
            )
            .await;
            out.extend_background_events(events);
        }
    }
    for (_, pending) in pending_subresource_responses {
        if let Ok(predecessor) = conn
            .fail_pending_subresource_response_for_owner_async(
                owner,
                pending.internal_id,
                "Fetch interception disabled".to_owned(),
            )
            .await
        {
            if let Some(predecessor) = predecessor {
                out.set_renderer_output_predecessor(predecessor);
            }
            let mut events = Vec::new();
            activity::flush_post_subresource_response_activity_background_events_async(
                conn,
                &mut events,
                owner.session_id(),
                &pending,
            )
            .await;
            out.extend_background_events(events);
        }
    }
}

async fn release_main_document_interceptions_neutrally_async(
    conn: &mut CdpConnection,
    out: &mut FetchCommandOutput,
    fallback_session_id: Option<&str>,
    pending_navigations: Vec<crate::conn::PendingFetchNavigation>,
    pending_auth_navigations: Vec<crate::conn::PendingFetchAuthNavigation>,
    pending_response_navigations: Vec<crate::conn::PendingFetchResponseNavigation>,
) {
    for pending in pending_navigations {
        let request = conn.take_navigation_request(pending.navigation_permit);
        navigation::continue_navigation_request_as_background_events_async(
            conn,
            out,
            crate::conn::ClaimedFetchNavigation::new(pending, request),
        )
        .await;
    }
    for pending in pending_auth_navigations {
        auth::default_navigation_auth_as_background_events_async(
            conn,
            out,
            fallback_session_id,
            pending,
        )
        .await;
    }
    for pending in pending_response_navigations {
        let transfer = conn.take_navigation_response(pending.permit);
        navigation::continue_navigation_response_neutrally_as_background_events_async(
            conn, out, pending, transfer,
        )
        .await;
    }
}

fn merge_optional_renderer_output_predecessors(
    mut first: Option<moli_core::RendererOutputFence>,
    second: Option<moli_core::RendererOutputFence>,
) -> Option<moli_core::RendererOutputFence> {
    if let Some(second) = second {
        second.merge_into_same_stream_tail(&mut first);
    }
    first
}

#[cfg(test)]
mod tests;
