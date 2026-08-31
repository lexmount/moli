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
    BackgroundProtocolEvent, CdpConnection, CdpSessionRoute, Cmd, CommandOwnerScope,
    DevToolsCommandExecutionOutput, FetchInterceptionPattern,
    FetchRequestStage as ConnFetchRequestStage,
};
use crate::devtools_runtime::{
    DevToolsAddNetworkInterceptCommand, DevToolsAddNetworkInterceptResult, DevToolsCommand,
    DevToolsCommandResult, DevToolsError, DevToolsErrorKind, DevToolsNetworkInterceptPhase,
    DevToolsProtocol,
};
use crate::domains::actions::FetchAction;
use crate::domains::command_output::{CommandOutputPlan, devtools_error_from_cdp_error_parts};
use crate::domains::{activity, network};
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

pub(crate) struct PendingFetchCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    owner_scope: CommandOwnerScope,
    kind: PendingFetchCommandKind,
    pending: PendingFetchCommandOperation,
}

pub(crate) struct CompletedFetchCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
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
    Ready,
    Page(moli_core::page::PendingPageCommand),
    MaterializeResponseBody {
        request_id: String,
        transfer: Box<crate::conn::PausedDocumentTransfer>,
        limit: usize,
    },
}

enum CompletedFetchCommandOperation {
    Ready,
    Page(Box<Result<moli_core::page::CompletedPageCommand, String>>),
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
        // Fetch terminal commands may resume a renderer-initiated navigation.
        // Such a navigation has no command session of its own, so once this
        // command crosses an async boundary it needs the issuing Page route as
        // its implicit owner. Keep that route exact even if another Page or
        // BrowserContext becomes active before the completion is processed.
        let session_owner_route =
            match session_id.and_then(|session_id| conn.session_route(Some(session_id))) {
                Some(route @ CdpSessionRoute::PageTarget { .. }) => Some(route),
                Some(
                    CdpSessionRoute::Browser
                    | CdpSessionRoute::BrowserContext { .. }
                    | CdpSessionRoute::TabTarget { .. }
                    | CdpSessionRoute::SharedWorkerTarget { .. }
                    | CdpSessionRoute::DedicatedWorkerTarget { .. }
                    | CdpSessionRoute::ServiceWorkerTarget { .. },
                ) => None,
                None if session_id.is_none() => CommandOwnerScope::capture(conn, None)
                    .session_owner_route()
                    .cloned(),
                None => None,
            };
        Self {
            command_id,
            session_id: session_id.map(str::to_owned),
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                session_id,
                session_owner_route,
            ),
            kind,
            pending,
        }
    }

    pub(crate) async fn wait(self) -> CompletedFetchCommandDispatch {
        let completed = match self.pending {
            PendingFetchCommandOperation::Ready => CompletedFetchCommandOperation::Ready,
            PendingFetchCommandOperation::Page(pending) => CompletedFetchCommandOperation::Page(
                Box::new(pending.wait().await.map_err(|error| error.to_string())),
            ),
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
            session_id: self.session_id,
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
        self.session_id.as_deref()
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
            Self::Ready | Self::MaterializeResponseBody { .. } => None,
        }
    }

    fn into_page_completion(self) -> Option<Result<moli_core::page::CompletedPageCommand, String>> {
        match self {
            Self::Page(completed) => Some(*completed),
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
    let (owner_session_id, owner_route) = match fetch_devtools_command_session_ids(conn, &command) {
        Ok(session_ids) => session_ids,
        Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
    };
    let step = {
        let mut route_scope =
            conn.scoped_optional_none_session_owner_route_override(owner_route.clone());
        start_devtools_fetch_command(
            route_scope.conn_mut(),
            None,
            owner_session_id.as_deref(),
            command,
        )
    };
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
            let mut route_scope =
                conn.scoped_optional_none_session_owner_route_override(owner_route);
            complete_pending_devtools_fetch_command(route_scope.conn_mut(), completed)
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

fn start_devtools_fetch_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCommand,
) -> FetchCommandTaskStep {
    match &command {
        DevToolsCommand::AddNetworkIntercept(command) => {
            start_devtools_add_network_intercept_command(
                conn,
                command_id,
                command_session_id,
                command,
            )
        }
        DevToolsCommand::RemoveNetworkIntercept(command) => {
            start_devtools_remove_network_intercept_command(
                conn,
                command_id,
                command_session_id,
                command.intercept_id.as_str(),
                command.context.protocol != DevToolsProtocol::Cdp
                    && command.context.target_id.is_none(),
            )
        }
        _ => commands::start_devtools_fetch_command(conn, command_id, command_session_id, command),
    }
}

fn fetch_devtools_command_session_ids(
    conn: &CdpConnection,
    command: &DevToolsCommand,
) -> Result<(Option<String>, Option<CdpSessionRoute>), DevToolsError> {
    let (context, request_id) = match command {
        DevToolsCommand::AddNetworkIntercept(command) => {
            return fetch_config_devtools_command_session_ids(conn, command.context.clone());
        }
        DevToolsCommand::RemoveNetworkIntercept(command) => {
            return fetch_config_devtools_command_session_ids(conn, command.context.clone());
        }
        DevToolsCommand::ContinueInterceptedRequest(command) => {
            (command.context.clone(), command.request_id.as_str())
        }
        DevToolsCommand::ContinueInterceptedResponse(command) => {
            (command.context.clone(), command.request_id.as_str())
        }
        DevToolsCommand::ContinueWithAuth(command) => {
            (command.context.clone(), command.request_id.as_str())
        }
        DevToolsCommand::FailInterceptedRequest(command) => {
            (command.context.clone(), command.request_id.as_str())
        }
        DevToolsCommand::FulfillInterceptedRequest(command) => {
            (command.context.clone(), command.request_id.as_str())
        }
        _ => return Ok((None, None)),
    };
    let response_session_id = context
        .session_id
        .as_ref()
        .map(|session| session.as_str().to_owned());
    if context.protocol == DevToolsProtocol::Cdp {
        return Ok((response_session_id, None));
    }
    Ok((None, conn.pending_fetch_request_session_route(request_id)))
}

fn fetch_config_devtools_command_session_ids(
    conn: &CdpConnection,
    context: crate::devtools_runtime::DevToolsCommandContext,
) -> Result<(Option<String>, Option<CdpSessionRoute>), DevToolsError> {
    let response_session_id = context
        .session_id
        .as_ref()
        .map(|session| session.as_str().to_owned());
    if context.protocol == DevToolsProtocol::Cdp {
        return Ok((response_session_id, None));
    }
    let owner_route = if let Some(target_id) = context.target_id.as_ref() {
        Some(
            conn.target_session_route_for_target_id(target_id.as_str())
                .ok_or_else(|| {
                    DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget")
                })?,
        )
    } else {
        None
    };
    Ok((None, owner_route))
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
        Err(message) if message == "BrowserContextNotLoaded" => FetchCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => FetchCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    }
}

fn start_devtools_add_network_intercept_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: &DevToolsAddNetworkInterceptCommand,
) -> FetchCommandTaskStep {
    let (handle_auth_requests, auth_url_patterns, patterns) =
        network_intercept_fetch_config(command);
    let intercept_session_id = if command.context.protocol == DevToolsProtocol::Cdp {
        command_session_id.map(str::to_owned)
    } else {
        command
            .context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str().to_owned())
    };
    match conn.start_add_network_intercept_for_session_owner(
        command_session_id,
        intercept_session_id,
        command.intercept_id.as_str().to_owned(),
        handle_auth_requests,
        auth_url_patterns,
        patterns,
    ) {
        Ok(Some(pending)) => FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new(
            conn,
            command_id,
            command_session_id,
            PendingFetchCommandKind::AddNetworkIntercept {
                intercept_id: command.intercept_id.as_str().to_owned(),
            },
            PendingFetchCommandOperation::Page(pending),
        )),
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
    command_session_id: Option<&str>,
    intercept_id: &str,
    allow_global_lookup: bool,
) -> FetchCommandTaskStep {
    match conn.start_remove_network_intercept_for_session_owner(
        command_session_id,
        intercept_id,
        allow_global_lookup,
    ) {
        Ok(Some(pending)) => FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new(
            conn,
            command_id,
            command_session_id,
            PendingFetchCommandKind::RemoveNetworkIntercept,
            PendingFetchCommandOperation::Page(pending),
        )),
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
    let owner_scope = completed.owner_scope.clone();
    let mut route_scope = owner_scope.enter(conn);
    complete_pending_fetch_command_inner(route_scope.conn_mut(), completed).await
}

async fn complete_pending_fetch_command_inner(
    conn: &mut CdpConnection,
    completed: CompletedFetchCommandDispatch,
) -> FetchCommandOutput {
    let mut out = FetchCommandOutput::default();
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
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                *pending_fetch_state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::ContinueRequest { state } => {
            commands::complete_continue_request_command_async(
                conn,
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::ContinueWithAuth { state } => {
            auth::complete_continue_with_auth_command_async(
                conn,
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::FailRequest { state } => {
            commands::complete_fail_request_command_async(
                conn,
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::FulfillRequest { state } => {
            commands::complete_fulfill_request_command_async(
                conn,
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::DispatchWebSocketMessage { operation } => {
            commands::complete_websocket_page_command(
                conn,
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                operation,
                &mut out,
            );
        }
        PendingFetchCommandKind::CloseWebSocket => {
            commands::complete_websocket_page_command(
                conn,
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                commands::PendingWebSocketCommandOperation::Close,
                &mut out,
            );
        }
        PendingFetchCommandKind::ContinueResponse { state } => {
            commands::complete_continue_response_command_async(
                conn,
                completed.session_id.as_deref(),
                completed.completed.into_page_completion(),
                *state,
                &mut out,
            )
            .await;
        }
        PendingFetchCommandKind::GetResponseBody => {
            body_stream::complete_get_response_body_from_transfer(
                conn,
                completed.session_id.as_deref(),
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
    let Some(completed_page_command) = completed.completed.into_page_completion() else {
        return CommandOutputPlan::error(-32000, "Missing renderer completion");
    };
    let completion = match completed_page_command {
        Ok(completion) => completion,
        Err(error) => return CommandOutputPlan::error(-32000, error),
    };
    let page = match conn.loaded_page_mut_for_protocol_access(completed.session_id.as_deref()) {
        Ok(page) => page,
        Err(message) if message == "NoDocumentLoaded" => {
            return CommandOutputPlan::from_devtools_result(result);
        }
        Err(message) => return CommandOutputPlan::error(-32000, message),
    };
    match page.finish_set_fetch_subresource_interception(completion) {
        Ok(()) => CommandOutputPlan::from_devtools_result(result),
        Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
    }
}

fn start_disable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> FetchCommandTaskStep {
    match conn.start_disable_fetch_for_session_owner(cmd.session_id) {
        Ok(Some((pending_fetch_state, pending))) => {
            FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new(
                conn,
                cmd.id,
                cmd.session_id,
                PendingFetchCommandKind::Disable {
                    pending_fetch_state: Box::new(pending_fetch_state),
                },
                pending
                    .map(PendingFetchCommandOperation::Page)
                    .unwrap_or(PendingFetchCommandOperation::Ready),
            ))
        }
        Ok(None) => FetchCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        )),
        Err(error) => FetchCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("failed to clear page fetch interception: {error}"),
        )),
    }
}

async fn complete_disable_command_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    completed: Option<Result<moli_core::page::CompletedPageCommand, String>>,
    pending_fetch_state: FetchDisablePendingState,
    out: &mut FetchCommandOutput,
) {
    if let Some(completed) = completed {
        let completion = match completed {
            Ok(completion) => completion,
            Err(error) => {
                out.push_error(
                    -32000,
                    format!("failed to clear page fetch interception: {error}"),
                );
                return;
            }
        };
        match conn.loaded_page_mut_for_protocol_access(session_id) {
            Ok(page) => {
                if let Err(error) = page.finish_set_fetch_subresource_interception(completion) {
                    out.push_error(
                        -32000,
                        format!("failed to clear page fetch interception: {error}"),
                    );
                    return;
                }
            }
            Err(message) if message == "NoDocumentLoaded" => {}
            Err(message) => {
                out.push_error(-32000, message);
                return;
            }
        }
    }

    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = pending_fetch_state;

    out.push_success();
    for pending in pending_navigations {
        let token = pending.document_navigation_token;
        let navigation_state = pending.navigation;
        let navigation = network::materialize_navigation_load_result(
            conn,
            &navigation_state,
            Err("Fetch interception disabled".to_owned()),
        );
        navigation::complete_tokened_materialized_navigation_as_background_events_async(
            conn,
            out,
            token,
            navigation_state,
            navigation,
        )
        .await;
    }
    for pending in pending_auth_navigations {
        let token = pending.document_navigation_token;
        let navigation_state = pending.navigation;
        let navigation = network::materialize_navigation_load_result(
            conn,
            &navigation_state,
            Err("Fetch interception disabled".to_owned()),
        );
        navigation::complete_tokened_materialized_navigation_as_background_events_async(
            conn,
            out,
            token,
            navigation_state,
            navigation,
        )
        .await;
    }
    for pending in pending_response_navigations {
        let (token, navigation, result) = pending.fail("Fetch interception disabled".to_owned());
        let result = network::materialize_navigation_load_result(conn, &navigation, result);
        navigation::complete_tokened_materialized_navigation_as_background_events_async(
            conn, out, token, navigation, result,
        )
        .await;
    }
    for (_, pending) in pending_subresource_fetches {
        if let Ok(predecessor) = conn
            .fail_pending_subresource_fetch_for_session_owner_async(
                session_id,
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
                session_id,
                &pending,
            )
            .await;
            out.extend_background_events(events);
        }
    }
    for (_, pending) in pending_subresource_auths {
        if let Ok(predecessor) = conn
            .fail_pending_subresource_auth_for_session_owner_async(
                session_id,
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
                session_id,
                &pending,
            )
            .await;
            out.extend_background_events(events);
        }
    }
    for (_, pending) in pending_subresource_responses {
        if let Ok(predecessor) = conn
            .fail_pending_subresource_response_for_session_owner_async(
                session_id,
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
                session_id,
                &pending,
            )
            .await;
            out.extend_background_events(events);
        }
    }
}

#[cfg(test)]
mod tests;
