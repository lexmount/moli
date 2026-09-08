use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::SinkExt;
use moli_protocol::{
    BackgroundCommandResponsePayload, BackgroundProtocolEvent,
    conn::RuntimeInspectorResponseReady,
    devtools_runtime::{
        AutomationEvent, DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
        DevToolsDomObjectReferenceCommand, DevToolsDomObjectReferenceOperation, DevToolsError,
        DevToolsErrorKind, DevToolsFrameId, DevToolsGetBrowserContextsCommand,
        DevToolsGetFrameTreesCommand, DevToolsGetLayoutMetricsCommand, DevToolsGetRealmsCommand,
        DevToolsGetTargetInfoCommand, DevToolsGetTargetsCommand, DevToolsNavigationWait,
        DevToolsProtocol, DevToolsRemoteHandleId, DevToolsSessionId,
        DevToolsSetFileInputFilesCommand, DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind,
        NavigationFrameEvent, NavigationFrameEventKind, NavigationLifecycleEvent,
        TargetLifecycleEvent, webdriver_bidi_navigation_id_from_loader_id,
    },
};
use moli_protocol_webdriver_bidi::{
    BidiCommandOutcome, BidiConnectionState, BidiDevToolsCommandDispatch, BidiErrorCode,
    BidiEventSourceHookPlan, BidiInputCommand, BidiInputCommandDispatch, BidiSessionRegistry,
    bidi_response_from_devtools_error, bidi_response_from_devtools_result, error_response,
    script_realm_created_event, success_response,
};
use moli_protocol_webdriver_classic::{
    CLASSIC_ELEMENT_REFERENCE_KEY, ClassicActionState, ClassicDevToolsCommandContext,
    ClassicElementOriginViewportPoints, ClassicError, ClassicErrorCode, ClassicViewportBounds,
    classic_element_id, element_center_from_geometry,
    perform_actions_ticks_with_state_and_viewport, release_actions_commands,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::time::sleep;
use tracing::warn;

use crate::cdp_scheduler::{
    CdpScheduler, CdpSchedulerEventReceivers, CompletedDevToolsNavigationExecution,
    DevToolsNavigationCommandProgress, DevToolsNavigationCommandWait,
    DevToolsRuntimeCommandProgress, PendingDevToolsNavigationLifecycle,
    PendingDevToolsRuntimeExecution, ProtocolOutputSequence, RendererOutputTransportFailure,
};

use super::AppState;
use super::webdriver_files::selected_files_from_paths;

pub(super) async fn ws_bidi_session_upgrade_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    let endpoint = match state.cdp_owner_registry.shared_owner() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            warn!(?error, "failed to attach BiDi frontend to DevTools owner");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    ws.on_upgrade(move |socket| async move {
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let attach = BidiFrontendAttach {
            socket,
            web_socket_url: state.bidi_ws_url,
            session_registry: state.bidi_session_registry,
            finished_tx,
        };
        if let Err(error) = endpoint.attach_bidi(attach) {
            warn!(?error, "failed to attach BiDi frontend");
            return;
        }
        let _ = finished_rx.await;
    })
}

pub(super) async fn ws_bidi_existing_session_upgrade_handler(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !state.classic_session_registry.has_session(&session_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(runtime) = state.classic_session_registry.runtime_handle(&session_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if state
        .bidi_session_registry
        .lock()
        .contains_session(&session_id)
    {
        return StatusCode::CONFLICT.into_response();
    }
    let file_prompt_handler = state
        .classic_session_registry
        .file_prompt_handler_for_bidi_script_commands(&session_id)
        .map(str::to_owned);
    let web_socket_url = format!("{}/{}", state.bidi_ws_url.trim_end_matches('/'), session_id);
    ws.on_upgrade(move |socket| async move {
        if !runtime
            .attach_bidi_socket(
                socket,
                web_socket_url,
                session_id,
                file_prompt_handler,
                state.bidi_session_registry,
            )
            .await
        {
            warn!("failed to attach WebDriver BiDi WebSocket to existing Classic runtime");
        }
    })
}

mod service;
pub(crate) use service::{BidiFrontendAttach, BidiServiceFrontends};

pub(in crate::protocol_server) struct BidiSocketActor {
    socket: WebSocket,
    bidi: BidiConnectionState,
    input_action_states: BTreeMap<String, ClassicActionState>,
    pending_navigation_response: Option<BidiPendingNavigationResponse>,
    pending_command: Option<BidiPendingCommand>,
}

pub(crate) enum BidiSocketActorInput {
    Socket(Option<Result<Message, axum::Error>>),
    RuntimeCompletion(Box<moli_protocol::CompletedDevToolsRuntimeCommandDispatch>),
    RuntimeResponseReady(Option<Box<RuntimeInspectorResponseReady>>),
    NavigationCompletion(Result<Box<CompletedDevToolsNavigationExecution>, tokio::task::JoinError>),
}

impl BidiSocketActor {
    pub(in crate::protocol_server) fn new(socket: WebSocket, web_socket_url: String) -> Self {
        Self {
            socket,
            bidi: BidiConnectionState::with_web_socket_url(web_socket_url),
            input_action_states: BTreeMap::new(),
            pending_navigation_response: None,
            pending_command: None,
        }
    }

    pub(in crate::protocol_server) fn attach_existing_session(
        &mut self,
        session_id: String,
        registry: &mut BidiSessionRegistry,
    ) -> bool {
        self.bidi.attach_existing_session(session_id, registry)
    }

    pub(in crate::protocol_server) fn set_file_prompt_handler_for_script_commands(
        &mut self,
        handler: Option<&str>,
    ) {
        self.bidi
            .set_file_prompt_handler_for_script_commands(handler);
    }

    pub(in crate::protocol_server) fn release_session(
        &mut self,
        registry: &mut BidiSessionRegistry,
    ) {
        self.bidi.release_session(registry);
    }

    pub(in crate::protocol_server) async fn release_event_sources(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) {
        if let Some(mut pending) = self.pending_navigation_response.take()
            && let BidiNavigationReply::Lifecycle { wait, .. } = &mut pending.reply
        {
            scheduler.cancel_navigation_reply_wait(wait);
        }
        self.release_pending_command_state(scheduler);
        let plan = self.bidi.release_event_source_hook_plan();
        let mut events = Vec::new();
        let _ = append_bidi_event_source_hook_plan_events(
            scheduler,
            receivers,
            &mut self.bidi,
            &plan,
            &mut events,
        )
        .await;
    }

    fn release_pending_command_state(&mut self, scheduler: &mut CdpScheduler) {
        let Some(pending) = self.pending_command.take() else {
            return;
        };
        match pending.pending {
            Some(BidiPendingCommandWait::Runtime(runtime_pending)) => {
                scheduler.cancel_devtools_runtime_command(*runtime_pending);
            }
            Some(BidiPendingCommandWait::Navigation(wait)) => {
                scheduler.retain_detached_navigation(wait);
            }
            Some(BidiPendingCommandWait::NavigationLifecycle(wait)) => {
                scheduler.cancel_devtools_navigation_lifecycle(*wait);
            }
            None => {}
        }
        if let Some(previous_target_discovery) = pending.completion.previous_target_discovery {
            scheduler.replace_target_discovery_enabled(previous_target_discovery);
        }
    }

    pub(in crate::protocol_server) async fn handle_socket_message(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        session_registry: &SharedBidiSessionRegistry,
        message: Result<Message, axum::Error>,
    ) -> bool {
        handle_bidi_socket_message(
            &mut self.socket,
            scheduler,
            receivers,
            &mut self.bidi,
            &mut self.input_action_states,
            session_registry,
            &mut self.pending_navigation_response,
            &mut self.pending_command,
            message,
        )
        .await
    }

    pub(in crate::protocol_server) async fn handle_navigation_completion(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        completed: Result<Box<CompletedDevToolsNavigationExecution>, tokio::task::JoinError>,
    ) -> bool {
        let Some(mut command) = self.pending_command.take() else {
            return true;
        };
        let Some(BidiPendingCommandWait::Navigation(_wait)) = command.pending.take() else {
            unreachable!("navigation completion must belong to the pending navigation");
        };
        let completed = match completed {
            Ok(completed) => completed,
            Err(error) => {
                warn!(?error, "BiDi navigation waiter failed");
                return false;
            }
        };
        match scheduler
            .complete_devtools_navigation_command(receivers, *completed)
            .await
        {
            DevToolsNavigationCommandProgress::Complete(execution) => {
                complete_and_send_bidi_pending_command(
                    &mut self.socket,
                    scheduler,
                    receivers,
                    &mut self.bidi,
                    &mut self.pending_navigation_response,
                    command,
                    *execution,
                )
                .await
            }
            DevToolsNavigationCommandProgress::Pending(pending) => {
                command.pending = Some(BidiPendingCommandWait::Navigation(
                    DevToolsNavigationCommandWait::new(*pending),
                ));
                self.pending_command = Some(command);
                true
            }
            DevToolsNavigationCommandProgress::PendingLifecycle {
                pending,
                protocol_output,
            } => {
                command.pending = Some(BidiPendingCommandWait::NavigationLifecycle(pending));
                self.pending_command = Some(command);
                self.send_or_route_protocol_output(scheduler, receivers, protocol_output, None)
                    .await
            }
        }
    }

    async fn handle_runtime_completion(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        completed: moli_protocol::CompletedDevToolsRuntimeCommandDispatch,
    ) -> bool {
        let mut command = self
            .pending_command
            .take()
            .expect("Runtime completion owns its command");
        let pending = command.take_runtime_pending();
        let progress = scheduler
            .complete_devtools_runtime_command(receivers, pending, completed)
            .await;
        self.apply_pending_runtime_progress(scheduler, receivers, command, progress)
            .await
    }

    pub(in crate::protocol_server) async fn handle_runtime_response_ready(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        response: RuntimeInspectorResponseReady,
    ) -> bool {
        let command_id = response.command_id();
        let matches_pending_command = self
            .pending_command
            .as_ref()
            .and_then(|pending_command| pending_command.runtime_pending())
            .is_some_and(|pending| pending.command_id() == command_id);

        if matches_pending_command {
            return self
                .advance_pending_command_after_renderer_response(scheduler, receivers, response)
                .await;
        }

        let output = scheduler.route_registered_runtime_inspector_response(response);
        self.send_or_route_protocol_output(scheduler, receivers, output, None)
            .await
    }

    pub(in crate::protocol_server) async fn send_or_route_protocol_output(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        output: ProtocolOutputSequence,
        owner_context: Option<&str>,
    ) -> bool {
        if self.pending_command.as_ref().is_some_and(|command| {
            matches!(
                command.pending,
                Some(BidiPendingCommandWait::NavigationLifecycle(_))
            )
        }) {
            let mut command = self
                .pending_command
                .take()
                .expect("pending navigation lifecycle");
            let Some(BidiPendingCommandWait::NavigationLifecycle(pending)) = command.pending.take()
            else {
                unreachable!()
            };
            return match scheduler.advance_devtools_navigation_lifecycle(pending, output) {
                DevToolsNavigationCommandProgress::Complete(execution) => {
                    complete_and_send_bidi_pending_command(
                        &mut self.socket,
                        scheduler,
                        receivers,
                        &mut self.bidi,
                        &mut self.pending_navigation_response,
                        command,
                        *execution,
                    )
                    .await
                }
                DevToolsNavigationCommandProgress::PendingLifecycle {
                    pending,
                    protocol_output,
                } => {
                    command.pending = Some(BidiPendingCommandWait::NavigationLifecycle(pending));
                    self.pending_command = Some(command);
                    send_bidi_protocol_output(
                        &mut self.socket,
                        scheduler,
                        receivers,
                        &mut self.bidi,
                        protocol_output,
                        owner_context,
                        &mut self.pending_navigation_response,
                    )
                    .await
                }
                DevToolsNavigationCommandProgress::Pending(_) => {
                    unreachable!("a committed navigation cannot return to network admission")
                }
            };
        }
        if self
            .pending_command
            .as_ref()
            .and_then(BidiPendingCommand::runtime_pending)
            .is_none()
        {
            return send_bidi_protocol_output(
                &mut self.socket,
                scheduler,
                receivers,
                &mut self.bidi,
                output,
                owner_context,
                &mut self.pending_navigation_response,
            )
            .await;
        }
        self.advance_pending_command_after_protocol_output(scheduler, receivers, output)
            .await
    }

    async fn advance_pending_command_after_protocol_output(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        output: ProtocolOutputSequence,
    ) -> bool {
        let Some(mut pending_command) = self.pending_command.take() else {
            return send_bidi_protocol_output(
                &mut self.socket,
                scheduler,
                receivers,
                &mut self.bidi,
                output,
                None,
                &mut self.pending_navigation_response,
            )
            .await;
        };
        let pending = pending_command.take_runtime_pending();
        let progress = scheduler
            .advance_devtools_runtime_command_after_protocol_output(pending, output)
            .await;
        self.apply_pending_runtime_progress(scheduler, receivers, pending_command, progress)
            .await
    }

    async fn advance_pending_command_after_renderer_response(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        response: RuntimeInspectorResponseReady,
    ) -> bool {
        let Some(mut pending_command) = self.pending_command.take() else {
            let output = scheduler.route_registered_runtime_inspector_response(response);
            return send_bidi_protocol_output(
                &mut self.socket,
                scheduler,
                receivers,
                &mut self.bidi,
                output,
                None,
                &mut self.pending_navigation_response,
            )
            .await;
        };
        let pending = pending_command.take_runtime_pending();
        let progress = scheduler
            .advance_devtools_runtime_command_after_renderer_response(receivers, pending, response)
            .await;
        self.apply_pending_runtime_progress(scheduler, receivers, pending_command, progress)
            .await
    }

    async fn apply_pending_runtime_progress(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut pending_command: BidiPendingCommand,
        progress: DevToolsRuntimeCommandProgress,
    ) -> bool {
        match progress {
            DevToolsRuntimeCommandProgress::Complete(execution) => {
                complete_and_send_bidi_pending_command(
                    &mut self.socket,
                    scheduler,
                    receivers,
                    &mut self.bidi,
                    &mut self.pending_navigation_response,
                    pending_command,
                    *execution,
                )
                .await
            }
            DevToolsRuntimeCommandProgress::Pending {
                pending,
                protocol_output,
            } => {
                pending_command.pending = Some(BidiPendingCommandWait::Runtime(pending));
                let sent = send_bidi_protocol_output(
                    &mut self.socket,
                    scheduler,
                    receivers,
                    &mut self.bidi,
                    protocol_output,
                    None,
                    &mut self.pending_navigation_response,
                )
                .await;
                self.pending_command = Some(pending_command);
                sent
            }
        }
    }
}

async fn handle_bidi_socket_message(
    socket: &mut WebSocket,
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    input_action_states: &mut BTreeMap<String, ClassicActionState>,
    session_registry: &SharedBidiSessionRegistry,
    pending_navigation_response: &mut Option<BidiPendingNavigationResponse>,
    pending_command: &mut Option<BidiPendingCommand>,
    message: Result<Message, axum::Error>,
) -> bool {
    let payload: Result<serde_json::Value, _> = match message {
        Ok(Message::Text(text)) => serde_json::from_str(&text),
        Ok(Message::Binary(bytes)) => serde_json::from_slice(&bytes),
        Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => return true,
        Ok(Message::Close(_)) => return false,
        Err(error) => {
            warn!(?error, "BiDi WebSocket receive failed");
            return false;
        }
    };
    let (
        outcome,
        command_method,
        command_params,
        previous_realm_created_contexts,
        subscribe_hook_plan,
    ) = match payload {
        Ok(message) => {
            let command_method = message
                .get("method")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let command_params = message.get("params").cloned();
            match bidi_command_channel_from_message(&message) {
                Err(error_message) => {
                    let id = message.get("id").and_then(serde_json::Value::as_u64);
                    let outcome = BidiCommandOutcome {
                        response: error_response(id, BidiErrorCode::InvalidArgument, error_message),
                        session_id: bidi.session_id().map(ToOwned::to_owned),
                        channel: None,
                        close_connection: false,
                        devtools_command: None,
                        input_command: None,
                    };
                    (outcome, command_method, command_params, None, None)
                }
                Ok(command_channel) => {
                    let previous_realm_created_contexts = (command_method.as_deref()
                        == Some("session.subscribe"))
                    .then(|| bidi.replay_contexts_for_bidi_event("script.realmCreated"));
                    let mut subscribe_hook_plan = None;
                    let outcome = if command_method.as_deref() == Some("session.subscribe")
                        && let Some(validation) =
                            validate_bidi_session_subscribe_targets(scheduler, bidi, &message).await
                    {
                        match validation {
                            BidiSessionSubscribeTargetValidation::Valid(targets) => {
                                targets.record_to_bidi_state(bidi);
                                subscribe_hook_plan = message.get("params").and_then(|params| {
                                    bidi.subscribe_hook_plan_for_params(params).ok()
                                });
                                let mut registry = session_registry.lock();
                                bidi.handle_message_with_session_registry(message, &mut registry)
                            }
                            BidiSessionSubscribeTargetValidation::Error(response) => {
                                BidiCommandOutcome {
                                    response: bidi_message_with_channel(
                                        response,
                                        command_channel.as_deref(),
                                    ),
                                    session_id: bidi.session_id().map(ToOwned::to_owned),
                                    channel: command_channel.clone(),
                                    close_connection: false,
                                    devtools_command: None,
                                    input_command: None,
                                }
                            }
                        }
                    } else {
                        let mut registry = session_registry.lock();
                        bidi.handle_message_with_session_registry(message, &mut registry)
                    };
                    (
                        outcome,
                        command_method,
                        command_params,
                        previous_realm_created_contexts,
                        subscribe_hook_plan,
                    )
                }
            }
        }
        Err(error) => (
            BidiCommandOutcome {
                response: error_response(
                    None,
                    BidiErrorCode::InvalidArgument,
                    &format!("invalid JSON: {error}"),
                ),
                session_id: bidi.session_id().map(ToOwned::to_owned),
                channel: None,
                close_connection: false,
                devtools_command: None,
                input_command: None,
            },
            None,
            None,
            None,
            None,
        ),
    };
    let BidiCommandOutcome {
        response,
        channel: command_channel,
        close_connection,
        devtools_command,
        input_command,
        ..
    } = outcome;
    let pending_navigation_candidate = devtools_command.as_ref().and_then(|dispatch| {
        pending_navigation_response_for_dispatch(scheduler, dispatch, command_channel.as_deref())
    });
    let command_start = if pending_command.as_ref().is_some_and(|pending| {
        input_command.is_some()
            || devtools_command.as_ref().is_some_and(|dispatch| {
                pending.runtime_pending().is_some()
                    || bidi_devtools_command_uses_deferred_runtime_progress(&dispatch.command)
                    || matches!(
                        dispatch.command,
                        DevToolsCommand::Navigate(_)
                            | DevToolsCommand::Reload(_)
                            | DevToolsCommand::TraverseHistory(_)
                    )
            })
    }) {
        BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
            response: error_response(
                response.get("id").and_then(serde_json::Value::as_u64),
                BidiErrorCode::UnsupportedOperation,
                "another BiDi command is still pending",
            ),
            event_sources: Vec::new(),
            post_response_event_sources: Vec::new(),
            post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
            event_context: None,
        })
    } else {
        match (devtools_command, input_command) {
            (Some(dispatch), None) => {
                let observe_context_created = bidi
                    .subscribed_contexts_for_bidi_event("browsingContext.contextCreated")
                    .is_some();
                start_bidi_devtools_command(
                    scheduler,
                    receivers,
                    bidi,
                    dispatch,
                    pending_navigation_candidate
                        .as_ref()
                        .map(|candidate| candidate.background_command_id),
                    observe_context_created,
                    bidi.subscribed_contexts_for_bidi_event("browsingContext.load")
                        .is_some(),
                )
                .await
            }
            (None, Some(dispatch)) => BidiDevToolsCommandStart::Complete(
                execute_bidi_input_command(scheduler, receivers, input_action_states, dispatch)
                    .await,
            ),
            (Some(_), Some(_)) => BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
                response: error_response(
                    None,
                    BidiErrorCode::UnsupportedOperation,
                    "BiDi command produced conflicting dispatches",
                ),
                event_sources: Vec::new(),
                post_response_event_sources: Vec::new(),
                post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                event_context: None,
            }),
            (None, None) => BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
                response,
                event_sources: Vec::new(),
                post_response_event_sources: Vec::new(),
                post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                event_context: None,
            }),
        }
    };
    let mut command_output = match command_start {
        BidiDevToolsCommandStart::Complete(output) => output,
        BidiDevToolsCommandStart::Pending(pending) => {
            let mut pending = *pending;
            pending.command_method = command_method;
            pending.command_params = command_params;
            pending.command_channel = command_channel;
            pending.pending_navigation_candidate = pending_navigation_candidate;
            let initial_sources =
                std::mem::take(&mut pending.completion.event_sources).into_sources();
            let mut bidi_events = subscribed_bidi_events_from_devtools_event_sources(
                Some(&*scheduler),
                bidi,
                &initial_sources,
                pending.completion.event_context.as_deref(),
            );
            if !append_context_created_event_source_hook_events(
                scheduler,
                receivers,
                bidi,
                &mut bidi_events,
            )
            .await
            {
                let _ = send_bidi_json_events(socket, bidi_events).await;
                return false;
            }
            if !send_bidi_json_events(socket, bidi_events).await {
                return false;
            }
            *pending_command = Some(pending);
            return true;
        }
    };
    let renderer_output_transport_terminal = receivers.renderer_publication_rx.is_closed();
    command_output.response =
        bidi_message_with_channel(command_output.response, command_channel.as_deref());
    let command_hook_plan = bidi.record_bidi_command_response(
        command_method.as_deref(),
        command_params.as_ref(),
        &command_output.response,
    );
    let defer_current_response = pending_navigation_response.is_none()
        && pending_navigation_candidate
            .as_ref()
            .is_some_and(|pending| pending.matches_pause(&command_output));
    if defer_current_response {
        *pending_navigation_response = pending_navigation_candidate;
    }
    let pending_response_from_event_sources = (!defer_current_response)
        .then(|| {
            take_pending_navigation_response_from_sources(
                scheduler,
                pending_navigation_response,
                &command_output.event_sources,
            )
        })
        .flatten();
    let pending_response_from_post_response_event_sources = (!defer_current_response)
        .then(|| {
            take_pending_navigation_response_from_sources(
                scheduler,
                pending_navigation_response,
                &command_output.post_response_event_sources,
            )
        })
        .flatten();
    let mut bidi_events = subscribed_bidi_events_from_devtools_event_sources(
        Some(&*scheduler),
        bidi,
        &command_output.event_sources,
        command_output.event_context.as_deref(),
    );
    if !renderer_output_transport_terminal
        && !append_context_created_event_source_hook_events(
            scheduler,
            receivers,
            bidi,
            &mut bidi_events,
        )
        .await
    {
        let _ = send_bidi_json_events(socket, bidi_events).await;
        return false;
    }
    if command_method.as_deref() == Some("session.subscribe")
        && command_output.response["type"] == json!("success")
        && !renderer_output_transport_terminal
    {
        if let Some(plan) = subscribe_hook_plan.as_ref()
            && !append_bidi_event_source_hook_plan_events(
                scheduler,
                receivers,
                bidi,
                plan,
                &mut bidi_events,
            )
            .await
        {
            let _ = send_bidi_json_events(socket, bidi_events).await;
            return false;
        }
        bidi_events.extend(
            replay_existing_bidi_realm_created_events(
                scheduler,
                bidi,
                previous_realm_created_contexts.flatten(),
            )
            .await,
        );
        bidi_events.extend(bidi.replay_buffered_bidi_log_entry_events_for_subscriptions());
    }
    if !append_bidi_event_source_hook_plan_events(
        scheduler,
        receivers,
        bidi,
        &command_hook_plan,
        &mut bidi_events,
    )
    .await
    {
        let _ = send_bidi_json_events(socket, bidi_events).await;
        return false;
    }
    let mut post_response_bidi_events = subscribed_bidi_events_from_devtools_event_sources(
        Some(&*scheduler),
        bidi,
        &command_output.post_response_event_sources,
        command_output.event_context.as_deref(),
    );
    if !renderer_output_transport_terminal
        && !append_context_created_event_source_hook_events(
            scheduler,
            receivers,
            bidi,
            &mut post_response_bidi_events,
        )
        .await
    {
        let _ = send_bidi_json_events(socket, bidi_events).await;
        let _ = send_bidi_json_events(socket, post_response_bidi_events).await;
        return false;
    }
    if !send_bidi_json_events(socket, bidi_events).await {
        return false;
    }
    if !defer_current_response
        && socket
            .send(Message::Text(command_output.response.to_string().into()))
            .await
            .is_err()
    {
        return false;
    }
    if !send_bidi_json_events(socket, post_response_bidi_events).await {
        return false;
    }
    if let Some(response) =
        pending_response_from_event_sources.or(pending_response_from_post_response_event_sources)
        && socket
            .send(Message::Text(response.to_string().into()))
            .await
            .is_err()
    {
        return false;
    }
    if command_output
        .post_response_background_navigation_drain
        .waits_for_inflight_navigation()
        && !drain_and_send_bidi_navigation_after_response(
            socket,
            scheduler,
            receivers,
            bidi,
            pending_navigation_response,
            command_output.event_context.as_deref(),
        )
        .await
    {
        return false;
    }
    if close_connection || renderer_output_transport_terminal {
        let _ = socket.close().await;
        return false;
    }
    true
}

async fn send_bidi_json_events(socket: &mut WebSocket, events: Vec<serde_json::Value>) -> bool {
    for event in events {
        if socket
            .send(Message::Text(event.to_string().into()))
            .await
            .is_err()
        {
            return false;
        }
    }
    true
}

async fn complete_and_send_bidi_pending_command(
    socket: &mut WebSocket,
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    pending_navigation_response: &mut Option<BidiPendingNavigationResponse>,
    pending: BidiPendingCommand,
    execution: crate::cdp_scheduler::DevToolsCommandExecution,
) -> bool {
    let BidiPendingCommand {
        command_method,
        command_params,
        command_channel,
        pending_navigation_candidate,
        pending: _,
        completion,
    } = pending;
    let mut command_output =
        complete_bidi_devtools_command_execution(scheduler, receivers, completion, execution).await;
    let renderer_output_transport_terminal = receivers.renderer_publication_rx.is_closed();
    command_output.response =
        bidi_message_with_channel(command_output.response, command_channel.as_deref());
    let command_hook_plan = bidi.record_bidi_command_response(
        command_method.as_deref(),
        command_params.as_ref(),
        &command_output.response,
    );
    let defer_current_response = pending_navigation_response.is_none()
        && pending_navigation_candidate
            .as_ref()
            .is_some_and(|pending| pending.matches_pause(&command_output));
    if defer_current_response {
        *pending_navigation_response = pending_navigation_candidate;
    }
    let pending_response_from_event_sources = (!defer_current_response)
        .then(|| {
            take_pending_navigation_response_from_sources(
                scheduler,
                pending_navigation_response,
                &command_output.event_sources,
            )
        })
        .flatten();
    let pending_response_from_post_response_event_sources = (!defer_current_response)
        .then(|| {
            take_pending_navigation_response_from_sources(
                scheduler,
                pending_navigation_response,
                &command_output.post_response_event_sources,
            )
        })
        .flatten();
    let mut bidi_events = subscribed_bidi_events_from_devtools_event_sources(
        Some(&*scheduler),
        bidi,
        &command_output.event_sources,
        command_output.event_context.as_deref(),
    );
    if !renderer_output_transport_terminal
        && !append_context_created_event_source_hook_events(
            scheduler,
            receivers,
            bidi,
            &mut bidi_events,
        )
        .await
    {
        let _ = send_bidi_json_events(socket, bidi_events).await;
        return false;
    }
    if !append_bidi_event_source_hook_plan_events(
        scheduler,
        receivers,
        bidi,
        &command_hook_plan,
        &mut bidi_events,
    )
    .await
    {
        let _ = send_bidi_json_events(socket, bidi_events).await;
        return false;
    }
    let mut post_response_bidi_events = subscribed_bidi_events_from_devtools_event_sources(
        Some(&*scheduler),
        bidi,
        &command_output.post_response_event_sources,
        command_output.event_context.as_deref(),
    );
    if !renderer_output_transport_terminal
        && !append_context_created_event_source_hook_events(
            scheduler,
            receivers,
            bidi,
            &mut post_response_bidi_events,
        )
        .await
    {
        let _ = send_bidi_json_events(socket, bidi_events).await;
        let _ = send_bidi_json_events(socket, post_response_bidi_events).await;
        return false;
    }
    if !send_bidi_json_events(socket, bidi_events).await {
        return false;
    }
    if !defer_current_response
        && socket
            .send(Message::Text(command_output.response.to_string().into()))
            .await
            .is_err()
    {
        return false;
    }
    if !send_bidi_json_events(socket, post_response_bidi_events).await {
        return false;
    }
    if let Some(response) =
        pending_response_from_event_sources.or(pending_response_from_post_response_event_sources)
        && socket
            .send(Message::Text(response.to_string().into()))
            .await
            .is_err()
    {
        return false;
    }
    if command_output
        .post_response_background_navigation_drain
        .waits_for_inflight_navigation()
        && !drain_and_send_bidi_navigation_after_response(
            socket,
            scheduler,
            receivers,
            bidi,
            pending_navigation_response,
            command_output.event_context.as_deref(),
        )
        .await
    {
        return false;
    }
    !renderer_output_transport_terminal
}

/// Completes the optional navigation tail after a BiDi command response is on
/// the wire.
///
/// A renderer-output transport terminal cannot retroactively replace that
/// response. We still deliver the concrete FIFO prefix admitted before the
/// terminal, then return `false` so the caller closes the connection instead
/// of issuing any later successful response from an incomplete output stream.
async fn drain_and_send_bidi_navigation_after_response(
    socket: &mut WebSocket,
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    pending_navigation_response: &mut Option<BidiPendingNavigationResponse>,
    event_context: Option<&str>,
) -> bool {
    let (background_navigation_sources, transport_is_live) =
        match drain_bidi_background_navigation_before_command(scheduler, receivers).await {
            Ok(sources) => (sources.into_sources(), true),
            Err(failure) => {
                let (sources, _terminal_error) = failure.into_parts();
                (sources.into_sources(), false)
            }
        };
    let pending_response = transport_is_live
        .then(|| {
            take_pending_navigation_response_from_sources(
                scheduler,
                pending_navigation_response,
                &background_navigation_sources,
            )
        })
        .flatten();
    let mut background_navigation_events = subscribed_bidi_events_from_devtools_event_sources(
        Some(&*scheduler),
        bidi,
        &background_navigation_sources,
        event_context,
    );
    if transport_is_live
        && !append_context_created_event_source_hook_events(
            scheduler,
            receivers,
            bidi,
            &mut background_navigation_events,
        )
        .await
    {
        let _ = send_bidi_json_events(socket, background_navigation_events).await;
        return false;
    }
    if !send_bidi_json_events(socket, background_navigation_events).await {
        return false;
    }
    if let Some(response) = pending_response
        && socket
            .send(Message::Text(response.to_string().into()))
            .await
            .is_err()
    {
        return false;
    }
    transport_is_live
}

fn bidi_message_with_channel(
    mut message: serde_json::Value,
    channel: Option<&str>,
) -> serde_json::Value {
    if let Some(channel) = channel
        && let Some(message) = message.as_object_mut()
    {
        message.insert("goog:channel".to_owned(), json!(channel));
    }
    message
}

fn bidi_command_channel_from_message(message: &serde_json::Value) -> Result<Option<String>, &str> {
    match message.get("goog:channel") {
        Some(serde_json::Value::String(channel)) if channel.is_empty() => Ok(None),
        Some(serde_json::Value::String(channel)) => Ok(Some(channel.clone())),
        Some(_) => Err("goog:channel must be a string"),
        None => Ok(None),
    }
}

async fn append_bidi_event_source_hook_plan_events(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    plan: &BidiEventSourceHookPlan,
    events: &mut Vec<Value>,
) -> bool {
    match try_append_bidi_event_source_hook_plan_events(scheduler, receivers, bidi, plan, events)
        .await
    {
        Ok(()) => true,
        Err(error) => {
            warn!(
                ?error,
                "BiDi event-source owner turn lost its renderer transport"
            );
            false
        }
    }
}

async fn try_append_bidi_event_source_hook_plan_events(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    plan: &BidiEventSourceHookPlan,
    events: &mut Vec<Value>,
) -> Result<(), DevToolsError> {
    // Resolve a global subscription to concrete sources once, so global and
    // scoped subscriptions share the same last-observer lifetime.
    let all_contexts = if plan.runtime_contexts().is_some_and(<[String]>::is_empty)
        || plan.network_contexts().is_some_and(<[String]>::is_empty)
    {
        let execution = scheduler
            .execute_devtools_command_with_protocol_messages(DevToolsCommand::GetTargets(
                DevToolsGetTargetsCommand {
                    context: DevToolsCommandContext {
                        protocol: DevToolsProtocol::WebDriverBidi,
                        session_id: bidi.session_id().map(DevToolsSessionId::from),
                        target_id: None,
                        browser_context_id: None,
                    },
                    root: None,
                    max_depth: None,
                    filter: None,
                },
            ))
            .await;
        extend_bidi_events_from_protocol_output(
            Some(&*scheduler),
            bidi,
            events,
            execution.protocol_output,
            None,
        );
        match execution.result? {
            DevToolsCommandResult::GetTargets(result) => result
                .targets
                .into_iter()
                .filter(|target| {
                    matches!(
                        target.kind,
                        DevToolsTargetKind::Page
                            | DevToolsTargetKind::Worker
                            | DevToolsTargetKind::SharedWorker
                            | DevToolsTargetKind::ServiceWorker
                    )
                })
                .filter_map(|target| target.target_id.map(|id| id.as_str().to_owned()))
                .collect::<Vec<_>>(),
            _ => unreachable!("GetTargets must return target metadata"),
        }
    } else {
        Vec::new()
    };
    if let Some(contexts) = plan.runtime_contexts() {
        for context in if contexts.is_empty() {
            &all_contexts
        } else {
            contexts
        } {
            let result = scheduler
                .enable_runtime_listener_for_target(receivers, context)
                .await;
            let output = materialize_bidi_event_source_hook_output(
                scheduler,
                bidi,
                events,
                result,
                Some(context),
            )?;
            if !output.is_empty() && plan.records_runtime_context_ownership() {
                bidi.record_bidi_runtime_event_source_opened(context);
            }
            extend_bidi_events_from_protocol_output(
                Some(&*scheduler),
                bidi,
                events,
                output,
                Some(context),
            );
        }
    }
    if let Some(contexts) = plan.runtime_disabled_contexts() {
        for context in contexts {
            let result = scheduler
                .disable_runtime_listener_for_target(receivers, context)
                .await;
            let output = materialize_bidi_event_source_hook_output(
                scheduler,
                bidi,
                events,
                result,
                Some(context),
            )?;
            bidi.record_bidi_runtime_event_source_closed(context);
            extend_bidi_events_from_protocol_output(
                Some(&*scheduler),
                bidi,
                events,
                output,
                Some(context),
            );
        }
    }
    if let Some(contexts) = plan.network_contexts() {
        for context in if contexts.is_empty() {
            &all_contexts
        } else {
            contexts
        } {
            if scheduler.enable_network_listener_for_target(context) {
                bidi.record_bidi_network_event_source_opened(context);
            }
        }
    }
    if let Some(contexts) = plan.network_disabled_contexts() {
        for context in contexts {
            if scheduler.disable_network_listener_for_target(context) {
                bidi.record_bidi_network_event_source_closed(context);
            }
        }
    }
    if let Some(contexts) = plan.file_dialog_opened_contexts() {
        for context in contexts {
            if scheduler.enable_file_dialog_opened_listener_for_target(context) {
                bidi.record_bidi_file_dialog_opened_source_opened(context);
            }
        }
    }
    if let Some(contexts) = plan.file_dialog_opened_disabled_contexts() {
        for context in contexts {
            if scheduler.disable_file_dialog_opened_listener_for_target(context) {
                bidi.record_bidi_file_dialog_opened_source_closed(context);
            }
        }
    }
    if plan.download_events_enabled() && scheduler.enable_webdriver_bidi_download_events() {
        bidi.record_bidi_download_event_source_opened();
    }
    if plan.download_events_disabled() && scheduler.disable_webdriver_bidi_download_events() {
        bidi.record_bidi_download_event_source_closed();
    }
    Ok(())
}

fn materialize_bidi_event_source_hook_output(
    scheduler: &CdpScheduler,
    bidi: &mut BidiConnectionState,
    events: &mut Vec<Value>,
    result: Result<ProtocolOutputSequence, RendererOutputTransportFailure>,
    owner_context: Option<&str>,
) -> Result<ProtocolOutputSequence, DevToolsError> {
    match result {
        Ok(output) => Ok(output),
        Err(failure) => {
            let (output, error) = failure.into_parts();
            extend_bidi_events_from_protocol_output(
                Some(scheduler),
                bidi,
                events,
                output,
                owner_context,
            );
            Err(error)
        }
    }
}

async fn send_bidi_protocol_output(
    socket: &mut WebSocket,
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    output: ProtocolOutputSequence,
    owner_context: Option<&str>,
    pending_navigation_response: &mut Option<BidiPendingNavigationResponse>,
) -> bool {
    if output.is_empty() && pending_navigation_response.is_none() {
        return true;
    }
    let sources =
        BidiDevToolsEventSources::from_protocol_output(output, Some(&*scheduler)).into_sources();
    let pending_response = take_pending_navigation_response_from_sources(
        scheduler,
        pending_navigation_response,
        &sources,
    );
    let mut events = subscribed_bidi_events_from_devtools_event_sources(
        Some(&*scheduler),
        bidi,
        &sources,
        owner_context,
    );
    if !append_context_created_event_source_hook_events(scheduler, receivers, bidi, &mut events)
        .await
    {
        let _ = send_bidi_json_events(socket, events).await;
        return false;
    }
    if !send_bidi_json_events(socket, events).await {
        return false;
    }
    if let Some(response) = pending_response
        && socket
            .send(Message::Text(response.to_string().into()))
            .await
            .is_err()
    {
        return false;
    }
    true
}

async fn append_context_created_event_source_hook_events(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    events: &mut Vec<serde_json::Value>,
) -> bool {
    let plan = bidi.context_created_event_source_hook_plan(events);
    append_bidi_event_source_hook_plan_events(scheduler, receivers, bidi, &plan, events).await
}

struct BidiDevToolsCommandOutput {
    response: serde_json::Value,
    event_sources: Vec<BidiDevToolsEventSource>,
    post_response_event_sources: Vec<BidiDevToolsEventSource>,
    post_response_background_navigation_drain: BidiBackgroundNavigationDrain,
    event_context: Option<String>,
}

enum BidiDevToolsCommandStart {
    Complete(BidiDevToolsCommandOutput),
    Pending(Box<BidiPendingCommand>),
}

struct BidiPendingCommand {
    command_method: Option<String>,
    command_params: Option<serde_json::Value>,
    command_channel: Option<String>,
    pending_navigation_candidate: Option<BidiPendingNavigationResponse>,
    pending: Option<BidiPendingCommandWait>,
    completion: BidiDevToolsCommandCompletion,
}

enum BidiPendingCommandWait {
    Runtime(Box<PendingDevToolsRuntimeExecution>),
    Navigation(DevToolsNavigationCommandWait),
    NavigationLifecycle(Box<PendingDevToolsNavigationLifecycle>),
}

impl BidiPendingCommand {
    fn runtime_pending(&self) -> Option<&PendingDevToolsRuntimeExecution> {
        match self.pending.as_ref()? {
            BidiPendingCommandWait::Runtime(pending) => Some(pending),
            BidiPendingCommandWait::Navigation(_)
            | BidiPendingCommandWait::NavigationLifecycle(_) => None,
        }
    }

    fn take_runtime_pending(&mut self) -> Box<PendingDevToolsRuntimeExecution> {
        match self.pending.take() {
            Some(BidiPendingCommandWait::Runtime(pending)) => pending,
            _ => unreachable!("pending Runtime command must retain its continuation"),
        }
    }
}

async fn recv_bidi_command_completion(
    pending: &mut Option<BidiPendingCommand>,
) -> BidiSocketActorInput {
    match pending
        .as_mut()
        .and_then(|pending| pending.pending.as_mut())
    {
        Some(BidiPendingCommandWait::Navigation(wait)) => {
            BidiSocketActorInput::NavigationCompletion(wait.await.map(Box::new))
        }
        Some(BidiPendingCommandWait::Runtime(wait)) => {
            BidiSocketActorInput::RuntimeCompletion(Box::new(wait.wait().await))
        }
        _ => std::future::pending().await,
    }
}

struct BidiDevToolsCommandCompletion {
    id: u64,
    session_id: String,
    event_sources: BidiDevToolsEventSources,
    event_context: Option<String>,
    close_target_event: Option<TargetLifecycleEvent>,
    create_target_browser_context_id:
        Option<moli_protocol::devtools_runtime::DevToolsBrowserContextId>,
    observe_browsing_context_load: bool,
    script_may_create_targets: bool,
    previous_target_discovery: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BidiBackgroundNavigationDrain {
    None,
    ScriptCreatedTargetLoadSubscription,
}

impl BidiBackgroundNavigationDrain {
    fn waits_for_inflight_navigation(self) -> bool {
        matches!(self, Self::ScriptCreatedTargetLoadSubscription)
    }
}

struct BidiPendingNavigationResponse {
    id: u64,
    target_id: String,
    url: String,
    channel: Option<String>,
    background_command_id: u64,
    wait: DevToolsNavigationWait,
    reply: BidiNavigationReply,
}

enum BidiNavigationReply {
    Command,
    Lifecycle {
        response: Value,
        wait: Box<crate::cdp_scheduler::DevToolsNavigationReplyWait>,
    },
}

enum BidiDevToolsEventSource {
    ProtocolMessage(serde_json::Value),
    ProtocolMessageWithAutomationEvent {
        message: serde_json::Value,
        automation_event: Box<AutomationEvent>,
    },
    CommandResponse {
        command_id: Option<u64>,
        response: BackgroundCommandResponsePayload,
    },
    AutomationEvent(Box<AutomationEvent>),
}

#[derive(Default)]
struct BidiDevToolsEventSources {
    sources: Vec<BidiDevToolsEventSource>,
}

struct BidiRendererOutputTransportFailure {
    event_sources: BidiDevToolsEventSources,
    error: DevToolsError,
}

impl BidiRendererOutputTransportFailure {
    fn from_renderer(
        mut event_sources: BidiDevToolsEventSources,
        failure: crate::cdp_scheduler::RendererOutputTransportFailure,
        scheduler: &CdpScheduler,
    ) -> Self {
        let (output, error) = failure.into_parts();
        event_sources.extend_protocol_output(output, Some(scheduler));
        Self {
            event_sources,
            error,
        }
    }

    fn into_parts(self) -> (BidiDevToolsEventSources, DevToolsError) {
        (self.event_sources, self.error)
    }
}

fn bidi_command_output_from_renderer_transport_failure(
    id: u64,
    event_context: Option<String>,
    mut preceding_sources: BidiDevToolsEventSources,
    failure: BidiRendererOutputTransportFailure,
) -> BidiDevToolsCommandOutput {
    let (failure_sources, error) = failure.into_parts();
    preceding_sources.append(failure_sources);
    BidiDevToolsCommandOutput {
        response: bidi_response_from_devtools_error(id, error),
        event_sources: preceding_sources.into_sources(),
        post_response_event_sources: Vec::new(),
        post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
        event_context,
    }
}

impl BidiDevToolsEventSources {
    fn from_protocol_output(
        output: ProtocolOutputSequence,
        scheduler: Option<&CdpScheduler>,
    ) -> Self {
        let mut sources = Self::default();
        sources.extend_protocol_output(output, scheduler);
        sources
    }

    fn extend_protocol_output(
        &mut self,
        output: ProtocolOutputSequence,
        scheduler: Option<&CdpScheduler>,
    ) {
        let output = match scheduler {
            Some(scheduler) => scheduler.publish_bidi_protocol_output(output),
            None => output,
        };
        for event in output.into_background_events() {
            self.push_background_event(event);
        }
    }

    fn push_automation_event(&mut self, event: AutomationEvent) {
        self.sources
            .push(BidiDevToolsEventSource::AutomationEvent(Box::new(event)));
    }

    fn push_background_event(&mut self, event: BackgroundProtocolEvent) {
        let event = match event.into_command_response_payload() {
            Ok((command_id, _, response)) => {
                self.sources.push(BidiDevToolsEventSource::CommandResponse {
                    command_id,
                    response,
                });
                return;
            }
            Err(event) => event,
        };
        let (message, automation_event) = event.into_parts();
        match automation_event {
            Some(event) => self.sources.push(
                BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent {
                    message,
                    automation_event: Box::new(event),
                },
            ),
            None => self
                .sources
                .push(BidiDevToolsEventSource::ProtocolMessage(message)),
        }
    }

    fn append(&mut self, mut other: Self) {
        self.sources.append(&mut other.sources);
    }

    fn push_initial_load_events_for_script_created_targets(&mut self) {
        let load_events = self
            .sources
            .iter()
            .filter_map(|source| match source {
                BidiDevToolsEventSource::ProtocolMessage(message) => {
                    initial_load_events_for_target_created_message(message)
                }
                BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent { message, .. } => {
                    initial_load_events_for_target_created_message(message)
                }
                BidiDevToolsEventSource::CommandResponse { .. } => None,
                BidiDevToolsEventSource::AutomationEvent(_) => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        for event in load_events {
            self.push_automation_event(event);
        }
    }

    fn into_sources(self) -> Vec<BidiDevToolsEventSource> {
        self.sources
    }
}

fn pending_navigation_response_for_dispatch(
    scheduler: &mut CdpScheduler,
    dispatch: &BidiDevToolsCommandDispatch,
    channel: Option<&str>,
) -> Option<BidiPendingNavigationResponse> {
    let DevToolsCommand::Navigate(command) = &dispatch.command else {
        return None;
    };
    if command.wait == DevToolsNavigationWait::None {
        return None;
    }
    Some(BidiPendingNavigationResponse {
        id: dispatch.id,
        target_id: command.context.target_id.as_ref()?.as_str().to_owned(),
        url: command.url.clone(),
        channel: channel.map(str::to_owned),
        background_command_id: scheduler.next_automation_command_id(),
        wait: command.wait,
        reply: BidiNavigationReply::Command,
    })
}

fn bidi_response_is_missing_devtools_command_result(response: &serde_json::Value) -> bool {
    response["type"] == json!("error")
        && response["message"].as_str() == Some("MissingDevToolsCommandResult")
}

impl BidiPendingNavigationResponse {
    fn matches_pause(&self, output: &BidiDevToolsCommandOutput) -> bool {
        if output.response["type"] != "success"
            && !bidi_response_is_missing_devtools_command_result(&output.response)
        {
            return false;
        }
        output.event_sources.iter().any(|source| {
            let event = match source {
                BidiDevToolsEventSource::AutomationEvent(event)
                | BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent { automation_event: event, .. } => event.as_ref(),
                BidiDevToolsEventSource::ProtocolMessage(message) => {
                    return matches!(message["method"].as_str(), Some("Fetch.authRequired" | "Fetch.requestPaused"))
                        && message["params"]["frameId"] == self.target_id
                        && message["params"]["resourceType"] == "Document";
                }
                BidiDevToolsEventSource::CommandResponse { .. } => return false,
            };
            matches!(event, AutomationEvent::NetworkAuthRequired(event) | AutomationEvent::RequestPaused(event)
                if event.target_id.as_str() == self.target_id
                && event.resource_type == Some(moli_protocol::devtools_runtime::DevToolsNetworkResourceType::Document))
        })
    }
}

fn take_pending_navigation_response_from_sources(
    scheduler: &mut CdpScheduler,
    pending: &mut Option<BidiPendingNavigationResponse>,
    sources: &[BidiDevToolsEventSource],
) -> Option<serde_json::Value> {
    let pending_response = pending.as_mut()?;
    if matches!(pending_response.reply, BidiNavigationReply::Command) {
        let response = sources
            .iter()
            .find_map(|source| match source {
                BidiDevToolsEventSource::CommandResponse {
                    command_id,
                    response,
                } => pending_navigation_response_from_command_response(
                    pending_response,
                    *command_id,
                    response,
                ),
                BidiDevToolsEventSource::ProtocolMessage(_)
                | BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent { .. }
                | BidiDevToolsEventSource::AutomationEvent(_) => None,
            })
            .or_else(|| {
                sources.iter().find_map(|source| match source {
                    BidiDevToolsEventSource::ProtocolMessage(message)
                    | BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent {
                        message, ..
                    } => {
                        pending_navigation_response_from_protocol_message(pending_response, message)
                    }
                    BidiDevToolsEventSource::CommandResponse { .. }
                    | BidiDevToolsEventSource::AutomationEvent(_) => None,
                })
            })?;
        let wait = (response["type"] == "success")
            .then(|| {
                scheduler.navigation_reply_wait(
                    &pending_response.target_id,
                    response["result"]["navigation"].as_str(),
                    pending_response.wait,
                )
            })
            .flatten();
        if let Some(wait) = wait {
            pending_response.reply = BidiNavigationReply::Lifecycle {
                response,
                wait: Box::new(wait),
            };
        } else {
            *pending = None;
            return Some(response);
        }
    }
    let BidiNavigationReply::Lifecycle { wait, .. } = &mut pending_response.reply else {
        unreachable!()
    };
    for source in sources {
        match source {
            BidiDevToolsEventSource::AutomationEvent(event)
            | BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent {
                automation_event: event,
                ..
            } => wait.observe_lifecycle_event(event),
            BidiDevToolsEventSource::ProtocolMessage(_)
            | BidiDevToolsEventSource::CommandResponse { .. } => {}
        }
    }
    let result = scheduler.poll_navigation_reply_wait(wait)?;
    let pending_response = pending.take().expect("completed navigation reply");
    let BidiNavigationReply::Lifecycle { response, .. } = pending_response.reply else {
        unreachable!()
    };
    Some(match result {
        Ok(()) => response,
        Err(error) => bidi_message_with_channel(
            bidi_response_from_devtools_error(pending_response.id, error),
            pending_response.channel.as_deref(),
        ),
    })
}

fn pending_navigation_response_from_command_response(
    pending: &BidiPendingNavigationResponse,
    command_id: Option<u64>,
    response: &BackgroundCommandResponsePayload,
) -> Option<serde_json::Value> {
    if command_id != Some(pending.background_command_id) {
        return None;
    }
    match response {
        BackgroundCommandResponsePayload::Success { result } => {
            let navigation = bidi_navigation_value_from_command_result(result);
            Some(bidi_message_with_channel(
                success_response(
                    pending.id,
                    json!({
                        "navigation": navigation,
                        "url": pending.url,
                    }),
                ),
                pending.channel.as_deref(),
            ))
        }
        BackgroundCommandResponsePayload::Error { message, .. } => Some(bidi_message_with_channel(
            error_response(
                Some(pending.id),
                BidiErrorCode::UnsupportedOperation,
                message,
            ),
            pending.channel.as_deref(),
        )),
    }
}

fn pending_navigation_response_from_protocol_message(
    pending: &BidiPendingNavigationResponse,
    message: &serde_json::Value,
) -> Option<serde_json::Value> {
    if message.get("id").and_then(serde_json::Value::as_u64) != Some(pending.background_command_id)
    {
        return None;
    }
    if let Some(result) = message.get("result") {
        let navigation = bidi_navigation_value_from_command_result(result);
        return Some(bidi_message_with_channel(
            success_response(
                pending.id,
                json!({
                    "navigation": navigation,
                    "url": pending.url,
                }),
            ),
            pending.channel.as_deref(),
        ));
    }
    let error = message.get("error")?;
    let error_message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("navigation command failed");
    Some(bidi_message_with_channel(
        error_response(
            Some(pending.id),
            BidiErrorCode::UnsupportedOperation,
            error_message,
        ),
        pending.channel.as_deref(),
    ))
}

fn bidi_navigation_value_from_command_result(result: &serde_json::Value) -> serde_json::Value {
    if let Some(navigation) = result.get("navigation") {
        return navigation.clone();
    }
    result
        .get("loaderId")
        .and_then(serde_json::Value::as_str)
        .map(webdriver_bidi_navigation_id_from_loader_id)
        .map(|navigation_id| json!(navigation_id.into_string()))
        .unwrap_or(serde_json::Value::Null)
}

fn initial_load_events_for_target_created_message(
    message: &serde_json::Value,
) -> Option<Vec<AutomationEvent>> {
    if message.get("method").and_then(serde_json::Value::as_str) != Some("Target.targetCreated") {
        return None;
    }
    let target_info = &message["params"]["targetInfo"];
    if target_info.get("type").and_then(serde_json::Value::as_str) != Some("page") {
        return None;
    }
    let target_id = target_info.get("targetId")?.as_str()?;
    let url = target_info
        .get("url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let target_id = DevToolsTargetId::from(target_id);
    let frame_id = DevToolsFrameId::from(target_id.as_str());
    Some(vec![
        AutomationEvent::NavigationFrame(NavigationFrameEvent {
            target_id: target_id.clone(),
            frame_id: frame_id.clone(),
            parent_frame_id: None,
            loader_id: None,
            url: url.clone(),
            kind: NavigationFrameEventKind::Navigated,
            frame_name: None,
            security_origin: None,
            secure_context_type: None,
        }),
        AutomationEvent::Load(NavigationLifecycleEvent {
            target_id,
            frame_id,
            navigation_id: None,
            loader_id: None,
            url,
            timestamp: 0.0,
        }),
    ])
}

fn subscribed_bidi_events_from_devtools_event_sources(
    scheduler: Option<&CdpScheduler>,
    bidi: &mut BidiConnectionState,
    sources: &[BidiDevToolsEventSource],
    owner_context: Option<&str>,
) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for source in sources {
        let automation = match source {
            BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent {
                automation_event,
                ..
            }
            | BidiDevToolsEventSource::AutomationEvent(automation_event) => {
                Some(automation_event.as_ref())
            }
            _ => None,
        };
        if scheduler.is_some_and(|scheduler| {
            !scheduler.webdriver_automation_event_is_visible(
                bidi.session_id(),
                automation,
                owner_context,
            )
        }) {
            continue;
        }
        match source {
            BidiDevToolsEventSource::ProtocolMessage(message) => {
                let message_owner_context =
                    bidi_protocol_message_owner_context(scheduler, message, owner_context);
                events.extend(
                    bidi.subscribed_bidi_events_from_protocol_messages_with_context(
                        std::iter::once(message),
                        message_owner_context.as_deref(),
                    ),
                );
            }
            BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent {
                message,
                automation_event,
            } => {
                let message_owner_context =
                    bidi_protocol_message_owner_context(scheduler, message, owner_context);
                bidi.record_protocol_message_state(message, message_owner_context.as_deref());
                events.extend(
                    bidi.subscribed_bidi_events_from_automation_events_with_context(
                        std::iter::once(automation_event.as_ref()),
                        message_owner_context.as_deref(),
                    ),
                );
            }
            BidiDevToolsEventSource::CommandResponse { .. } => {}
            BidiDevToolsEventSource::AutomationEvent(event) => {
                events.extend(
                    bidi.subscribed_bidi_events_from_automation_events(std::iter::once(
                        event.as_ref(),
                    )),
                );
            }
        }
    }
    events
}

fn extend_bidi_events_from_protocol_output(
    scheduler: Option<&CdpScheduler>,
    bidi: &mut BidiConnectionState,
    events: &mut Vec<serde_json::Value>,
    output: ProtocolOutputSequence,
    owner_context: Option<&str>,
) {
    let sources = BidiDevToolsEventSources::from_protocol_output(output, scheduler).into_sources();
    events.extend(subscribed_bidi_events_from_devtools_event_sources(
        scheduler,
        bidi,
        &sources,
        owner_context,
    ));
}

fn bidi_protocol_message_owner_context(
    scheduler: Option<&CdpScheduler>,
    message: &serde_json::Value,
    fallback_owner_context: Option<&str>,
) -> Option<String> {
    message
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .and_then(|session_id| {
            scheduler.and_then(|scheduler| scheduler.worker_target_id_for_session(Some(session_id)))
        })
        .or_else(|| fallback_owner_context.map(str::to_owned))
}

async fn replay_existing_bidi_realm_created_events(
    scheduler: &mut CdpScheduler,
    bidi: &BidiConnectionState,
    previous_contexts: Option<Vec<String>>,
) -> Vec<serde_json::Value> {
    let Some(contexts) = replay_contexts_for_new_subscription(
        bidi.replay_contexts_for_bidi_event("script.realmCreated"),
        previous_contexts,
    ) else {
        return Vec::new();
    };
    let Some(session_id) = bidi.session_id() else {
        return Vec::new();
    };
    let mut events = Vec::new();
    if contexts.is_empty() {
        let events =
            replay_existing_bidi_realm_created_events_for_context(scheduler, session_id, None)
                .await;
        return bidi.subscribed_bidi_events_from_bidi_events_with_context(&events, None);
    } else {
        for context_id in contexts {
            let replayed = replay_existing_bidi_realm_created_events_for_context(
                scheduler,
                session_id,
                Some(context_id.clone()),
            )
            .await;
            events.extend(bidi.subscribed_bidi_events_from_bidi_events_with_context(
                &replayed,
                Some(&context_id),
            ));
        }
    }
    events
}

fn replay_contexts_for_new_subscription(
    current_contexts: Option<Vec<String>>,
    previous_contexts: Option<Vec<String>>,
) -> Option<Vec<String>> {
    let current_contexts = current_contexts?;
    match (current_contexts.is_empty(), previous_contexts) {
        (_, Some(previous)) if previous.is_empty() => None,
        (true, _) => Some(Vec::new()),
        (false, Some(previous)) => {
            let previous = previous
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            let contexts = current_contexts
                .into_iter()
                .filter(|context| !previous.contains(context))
                .collect::<Vec<_>>();
            (!contexts.is_empty()).then_some(contexts)
        }
        (false, None) => Some(current_contexts),
    }
}

async fn replay_existing_bidi_realm_created_events_for_context(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    context_id: Option<String>,
) -> Vec<serde_json::Value> {
    let command = DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: context_id.as_deref().map(DevToolsTargetId::from),
            browser_context_id: None,
        },
        realm_type: None,
    });
    let execution = scheduler
        .execute_devtools_command_with_protocol_messages(command)
        .await;
    let Ok(DevToolsCommandResult::Realms(result)) = execution.result else {
        return Vec::new();
    };
    result
        .realms
        .iter()
        .filter_map(script_realm_created_event)
        .collect()
}

async fn start_bidi_devtools_command(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &BidiConnectionState,
    mut dispatch: BidiDevToolsCommandDispatch,
    background_command_id: Option<u64>,
    observe_context_created: bool,
    observe_browsing_context_load: bool,
) -> BidiDevToolsCommandStart {
    dispatch
        .command
        .set_webdriver_bidi_file_prompt_handler_for_script_command(
            bidi.file_prompt_handler_for_script_commands(),
        );
    let event_context = bidi_event_context_from_devtools_command(&dispatch.command);
    let close_target_id = match &dispatch.command {
        DevToolsCommand::CloseTarget(command) => Some(command.target_id.as_str().to_owned()),
        _ => None,
    };
    let script_may_create_targets = matches!(
        &dispatch.command,
        DevToolsCommand::EvaluateScript(_) | DevToolsCommand::CallFunction(_)
    );
    let mut event_sources =
        match drain_bidi_background_navigation_before_command(scheduler, receivers).await {
            Ok(event_sources) => event_sources,
            Err(failure) => {
                let (event_sources, error) = failure.into_parts();
                return BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
                    response: bidi_response_from_devtools_error(dispatch.id, error),
                    event_sources: event_sources.into_sources(),
                    post_response_event_sources: Vec::new(),
                    post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                    event_context,
                });
            }
        };
    event_sources.extend_protocol_output(
        scheduler
            .complete_ready_protocol_residences_after_command()
            .await,
        Some(&*scheduler),
    );
    let close_target_event = if let Some(target_id) = close_target_id.as_deref() {
        bidi_target_lifecycle_event_for_target(scheduler, &dispatch.session_id, target_id).await
    } else {
        None
    };
    if let Some(error) =
        validate_bidi_top_level_context_command(scheduler, &dispatch.session_id, &dispatch.command)
            .await
    {
        return BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
            response: bidi_response_from_devtools_error(dispatch.id, error),
            event_sources: event_sources.into_sources(),
            post_response_event_sources: Vec::new(),
            post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
            event_context,
        });
    }
    let previous_target_discovery = ((observe_context_created || observe_browsing_context_load)
        && script_may_create_targets)
        .then(|| scheduler.replace_target_discovery_enabled(true));
    let create_target_browser_context_id = match &dispatch.command {
        DevToolsCommand::CreateTarget(command) => command.browser_context_id.clone(),
        _ => None,
    };
    let completion = BidiDevToolsCommandCompletion {
        id: dispatch.id,
        session_id: dispatch.session_id.clone(),
        event_sources,
        event_context,
        close_target_event,
        create_target_browser_context_id,
        observe_browsing_context_load,
        script_may_create_targets,
        previous_target_discovery,
    };
    if bidi_devtools_command_uses_deferred_runtime_progress(&dispatch.command) {
        return match scheduler
            .start_devtools_runtime_command(receivers, dispatch.command)
            .await
        {
            DevToolsRuntimeCommandProgress::Complete(execution) => {
                BidiDevToolsCommandStart::Complete(
                    complete_bidi_devtools_command_execution(
                        scheduler, receivers, completion, *execution,
                    )
                    .await,
                )
            }
            DevToolsRuntimeCommandProgress::Pending {
                pending,
                protocol_output,
            } => {
                let mut completion = completion;
                completion
                    .event_sources
                    .extend_protocol_output(protocol_output, Some(&*scheduler));
                BidiDevToolsCommandStart::Pending(Box::new(BidiPendingCommand {
                    command_method: None,
                    command_params: None,
                    command_channel: None,
                    pending_navigation_candidate: None,
                    pending: Some(BidiPendingCommandWait::Runtime(pending)),
                    completion,
                }))
            }
        };
    }
    if matches!(
        dispatch.command,
        DevToolsCommand::Navigate(_)
            | DevToolsCommand::Reload(_)
            | DevToolsCommand::TraverseHistory(_)
    ) {
        return match scheduler
            .start_devtools_navigation_command(receivers, dispatch.command, background_command_id)
            .await
        {
            DevToolsNavigationCommandProgress::Complete(execution) => {
                BidiDevToolsCommandStart::Complete(
                    complete_bidi_devtools_command_execution(
                        scheduler, receivers, completion, *execution,
                    )
                    .await,
                )
            }
            DevToolsNavigationCommandProgress::Pending(pending) => {
                BidiDevToolsCommandStart::Pending(Box::new(BidiPendingCommand {
                    command_method: None,
                    command_params: None,
                    command_channel: None,
                    pending_navigation_candidate: None,
                    pending: Some(BidiPendingCommandWait::Navigation(
                        DevToolsNavigationCommandWait::new(*pending),
                    )),
                    completion,
                }))
            }
            DevToolsNavigationCommandProgress::PendingLifecycle {
                pending,
                protocol_output,
            } => {
                let mut completion = completion;
                completion
                    .event_sources
                    .extend_protocol_output(protocol_output, Some(&*scheduler));
                BidiDevToolsCommandStart::Pending(Box::new(BidiPendingCommand {
                    command_method: None,
                    command_params: None,
                    command_channel: None,
                    pending_navigation_candidate: None,
                    pending: Some(BidiPendingCommandWait::NavigationLifecycle(pending)),
                    completion,
                }))
            }
        };
    }
    let execution = scheduler
        .execute_devtools_command_with_external_load_wait_and_protocol_messages_background_command_id(
            receivers,
            dispatch.command,
            background_command_id,
        )
        .await;
    BidiDevToolsCommandStart::Complete(
        complete_bidi_devtools_command_execution(scheduler, receivers, completion, execution).await,
    )
}

async fn complete_bidi_devtools_command_execution(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    completion: BidiDevToolsCommandCompletion,
    execution: crate::cdp_scheduler::DevToolsCommandExecution,
) -> BidiDevToolsCommandOutput {
    let BidiDevToolsCommandCompletion {
        id,
        session_id,
        mut event_sources,
        event_context,
        close_target_event,
        create_target_browser_context_id,
        observe_browsing_context_load,
        script_may_create_targets,
        previous_target_discovery,
    } = completion;
    event_sources.extend_protocol_output(execution.protocol_output, Some(&*scheduler));
    match drain_ready_bidi_background_navigation(scheduler, receivers).await {
        Ok(sources) => event_sources.append(sources),
        Err(failure) => {
            if let Some(previous_target_discovery) = previous_target_discovery {
                scheduler.replace_target_discovery_enabled(previous_target_discovery);
            }
            return bidi_command_output_from_renderer_transport_failure(
                id,
                event_context,
                event_sources,
                failure,
            );
        }
    }
    let created_target_id = match &execution.result {
        Ok(DevToolsCommandResult::CreateTarget(result)) => {
            Some(result.target_id.as_str().to_owned())
        }
        _ => None,
    };
    let close_succeeded = matches!(&execution.result, Ok(DevToolsCommandResult::CloseTarget(_)));
    if let Some(target_id) = created_target_id.as_deref() {
        let mut event =
            match bidi_target_lifecycle_event_for_target(scheduler, &session_id, target_id).await {
                Some(event) => event,
                None => TargetLifecycleEvent {
                    target_id: DevToolsTargetId::from(target_id),
                    browser_context_id: create_target_browser_context_id.clone(),
                    kind: DevToolsTargetKind::Page,
                    url: "about:blank".to_owned(),
                    target_info: None,
                },
            };
        if let Some(browser_context_id) = create_target_browser_context_id {
            event.browser_context_id = Some(browser_context_id);
            event.target_info = None;
        }
        event_sources.push_automation_event(AutomationEvent::TargetCreated(event));
    }
    if close_succeeded && let Some(event) = close_target_event {
        event_sources.push_automation_event(AutomationEvent::TargetDestroyed(event));
    }
    if observe_browsing_context_load && script_may_create_targets {
        event_sources.push_initial_load_events_for_script_created_targets();
    }
    let response = match execution.result {
        Ok(result) => bidi_response_from_devtools_result(id, result),
        Err(error) => bidi_response_from_devtools_error(id, error),
    };
    let mut post_response_event_sources =
        match complete_bidi_post_response_protocol_residences(scheduler, receivers).await {
            Ok(sources) => sources,
            Err(failure) => {
                if let Some(previous_target_discovery) = previous_target_discovery {
                    scheduler.replace_target_discovery_enabled(previous_target_discovery);
                }
                return bidi_command_output_from_renderer_transport_failure(
                    id,
                    event_context,
                    event_sources,
                    failure,
                );
            }
        };
    if observe_browsing_context_load && script_may_create_targets {
        post_response_event_sources.push_initial_load_events_for_script_created_targets();
    }
    if let Some(previous_target_discovery) = previous_target_discovery {
        scheduler.replace_target_discovery_enabled(previous_target_discovery);
    }
    let post_response_background_navigation_drain = if observe_browsing_context_load
        && script_may_create_targets
        && scheduler.has_inflight_background_navigation()
    {
        BidiBackgroundNavigationDrain::ScriptCreatedTargetLoadSubscription
    } else {
        BidiBackgroundNavigationDrain::None
    };
    BidiDevToolsCommandOutput {
        response,
        event_sources: event_sources.into_sources(),
        post_response_event_sources: post_response_event_sources.into_sources(),
        post_response_background_navigation_drain,
        event_context,
    }
}

fn bidi_devtools_command_uses_deferred_runtime_progress(command: &DevToolsCommand) -> bool {
    matches!(
        command,
        DevToolsCommand::GetRealms(_)
            | DevToolsCommand::EvaluateScript(_)
            | DevToolsCommand::CallFunction(_)
            | DevToolsCommand::LocateNodes(_)
            | DevToolsCommand::ReleaseObjects(_)
    )
}

async fn execute_bidi_input_command(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    action_states: &mut BTreeMap<String, ClassicActionState>,
    dispatch: BidiInputCommandDispatch,
) -> BidiDevToolsCommandOutput {
    let mut event_sources =
        match drain_bidi_background_navigation_before_command(scheduler, receivers).await {
            Ok(event_sources) => event_sources,
            Err(failure) => {
                let (event_sources, error) = failure.into_parts();
                return BidiDevToolsCommandOutput {
                    response: bidi_response_from_devtools_error(dispatch.id, error),
                    event_sources: event_sources.into_sources(),
                    post_response_event_sources: Vec::new(),
                    post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                    event_context: Some(dispatch.context),
                };
            }
        };
    event_sources.extend_protocol_output(
        scheduler
            .complete_ready_protocol_residences_after_command()
            .await,
        Some(&*scheduler),
    );
    if !bidi_context_exists(scheduler, &dispatch.session_id, &dispatch.context).await {
        return BidiDevToolsCommandOutput {
            response: bidi_response_from_devtools_error(
                dispatch.id,
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"),
            ),
            event_sources: event_sources.into_sources(),
            post_response_event_sources: Vec::new(),
            post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
            event_context: Some(dispatch.context),
        };
    }

    let context = ClassicDevToolsCommandContext::with_protocol_and_target_id(
        DevToolsProtocol::WebDriverBidi,
        &dispatch.session_id,
        &dispatch.context,
    );
    let command = dispatch.command.clone();
    if let BidiInputCommand::SetFiles { params } = command {
        return execute_bidi_set_files_command(
            scheduler,
            receivers,
            dispatch,
            params,
            event_sources,
        )
        .await;
    }
    let viewport_bounds = match bidi_input_viewport_bounds(
        scheduler,
        &dispatch.session_id,
        &dispatch.context,
    )
    .await
    {
        Ok(viewport_bounds) => viewport_bounds,
        Err(error) => {
            return BidiDevToolsCommandOutput {
                response: bidi_response_from_devtools_error(dispatch.id, error),
                event_sources: event_sources.into_sources(),
                post_response_event_sources: Vec::new(),
                post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                event_context: Some(dispatch.context),
            };
        }
    };
    let ticks = match command {
        BidiInputCommand::PerformActions { params } => {
            let (params, element_origins) = match bidi_input_classic_params_and_element_origins(
                scheduler,
                receivers,
                &dispatch.session_id,
                &dispatch.context,
                params,
                &mut event_sources,
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(BidiInputPreparationError::Classic(error)) => {
                    return BidiDevToolsCommandOutput {
                        response: bidi_response_from_classic_input_error(dispatch.id, error),
                        event_sources: event_sources.into_sources(),
                        post_response_event_sources: Vec::new(),
                        post_response_background_navigation_drain:
                            BidiBackgroundNavigationDrain::None,
                        event_context: Some(dispatch.context),
                    };
                }
                Err(BidiInputPreparationError::DevTools(error)) => {
                    return BidiDevToolsCommandOutput {
                        response: bidi_response_from_devtools_error(dispatch.id, error),
                        event_sources: event_sources.into_sources(),
                        post_response_event_sources: Vec::new(),
                        post_response_background_navigation_drain:
                            BidiBackgroundNavigationDrain::None,
                        event_context: Some(dispatch.context),
                    };
                }
            };
            let action_state = action_states.entry(dispatch.context.clone()).or_default();
            perform_actions_ticks_with_state_and_viewport(
                &context,
                &params,
                &element_origins,
                Some(viewport_bounds),
                action_state,
            )
        }
        BidiInputCommand::ReleaseActions => {
            let action_state = action_states.entry(dispatch.context.clone()).or_default();
            Ok(vec![moli_protocol_webdriver_classic::ClassicActionTick {
                commands: release_actions_commands(&context, action_state),
                duration_ms: 0,
            }])
        }
        BidiInputCommand::SetFiles { .. } => unreachable!("input.setFiles returns before actions"),
    };
    let ticks = match ticks {
        Ok(ticks) => ticks,
        Err(error) => {
            return BidiDevToolsCommandOutput {
                response: bidi_response_from_classic_input_error(dispatch.id, error),
                event_sources: event_sources.into_sources(),
                post_response_event_sources: Vec::new(),
                post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                event_context: Some(dispatch.context),
            };
        }
    };

    for tick in ticks {
        for command in tick.commands {
            let execution = scheduler
                .execute_devtools_command_with_external_load_wait_and_protocol_messages(
                    receivers, command,
                )
                .await;
            event_sources.extend_protocol_output(execution.protocol_output, Some(&*scheduler));
            match execution.result {
                Ok(DevToolsCommandResult::Empty) => {}
                Ok(_) => {
                    return BidiDevToolsCommandOutput {
                        response: bidi_response_from_devtools_error(
                            dispatch.id,
                            DevToolsError::new(
                                DevToolsErrorKind::Internal,
                                "input action returned an unexpected result",
                            ),
                        ),
                        event_sources: event_sources.into_sources(),
                        post_response_event_sources: Vec::new(),
                        post_response_background_navigation_drain:
                            BidiBackgroundNavigationDrain::None,
                        event_context: Some(dispatch.context),
                    };
                }
                Err(error) => {
                    return BidiDevToolsCommandOutput {
                        response: bidi_response_from_devtools_error(dispatch.id, error),
                        event_sources: event_sources.into_sources(),
                        post_response_event_sources: Vec::new(),
                        post_response_background_navigation_drain:
                            BidiBackgroundNavigationDrain::None,
                        event_context: Some(dispatch.context),
                    };
                }
            }
        }
        if tick.duration_ms > 0 {
            sleep(Duration::from_millis(tick.duration_ms)).await;
        }
    }
    let post_response_event_sources =
        match complete_bidi_post_response_protocol_residences(scheduler, receivers).await {
            Ok(sources) => sources,
            Err(failure) => {
                return bidi_command_output_from_renderer_transport_failure(
                    dispatch.id,
                    Some(dispatch.context),
                    event_sources,
                    failure,
                );
            }
        };
    BidiDevToolsCommandOutput {
        response: success_response(dispatch.id, json!({})),
        event_sources: event_sources.into_sources(),
        post_response_event_sources: post_response_event_sources.into_sources(),
        post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
        event_context: Some(dispatch.context),
    }
}

async fn execute_bidi_set_files_command(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    dispatch: BidiInputCommandDispatch,
    params: Value,
    mut event_sources: BidiDevToolsEventSources,
) -> BidiDevToolsCommandOutput {
    let (shared_id, file_paths) = match parse_bidi_set_files_params(&params) {
        Ok(parsed) => parsed,
        Err(message) => {
            return BidiDevToolsCommandOutput {
                response: error_response(
                    Some(dispatch.id),
                    BidiErrorCode::InvalidArgument,
                    &message,
                ),
                event_sources: event_sources.into_sources(),
                post_response_event_sources: Vec::new(),
                post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                event_context: Some(dispatch.context),
            };
        }
    };
    let files = match selected_files_from_paths(&file_paths, "input.setFiles") {
        Ok(files) => files,
        Err(error) => {
            return BidiDevToolsCommandOutput {
                response: bidi_response_from_devtools_error(dispatch.id, error),
                event_sources: event_sources.into_sources(),
                post_response_event_sources: Vec::new(),
                post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
                event_context: Some(dispatch.context),
            };
        }
    };
    let command = DevToolsCommand::SetFileInputFiles(DevToolsSetFileInputFilesCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(dispatch.session_id.as_str())),
            target_id: Some(DevToolsTargetId::from(dispatch.context.as_str())),
            browser_context_id: None,
        },
        object_id: DevToolsRemoteHandleId::from(shared_id.as_str()),
        files,
        append: false,
    });
    let execution = scheduler
        .execute_devtools_command_with_external_load_wait_and_protocol_messages(receivers, command)
        .await;
    event_sources.extend_protocol_output(execution.protocol_output, Some(&*scheduler));
    let response = match execution.result {
        Ok(DevToolsCommandResult::Empty) => success_response(dispatch.id, json!({})),
        Ok(_) => bidi_response_from_devtools_error(
            dispatch.id,
            DevToolsError::new(
                DevToolsErrorKind::Internal,
                "input.setFiles returned an unexpected result",
            ),
        ),
        Err(error) => bidi_response_from_devtools_error(dispatch.id, error),
    };
    let post_response_event_sources =
        match complete_bidi_post_response_protocol_residences(scheduler, receivers).await {
            Ok(sources) => sources,
            Err(failure) => {
                return bidi_command_output_from_renderer_transport_failure(
                    dispatch.id,
                    Some(dispatch.context),
                    event_sources,
                    failure,
                );
            }
        };
    BidiDevToolsCommandOutput {
        response,
        event_sources: event_sources.into_sources(),
        post_response_event_sources: post_response_event_sources.into_sources(),
        post_response_background_navigation_drain: BidiBackgroundNavigationDrain::None,
        event_context: Some(dispatch.context),
    }
}

fn parse_bidi_set_files_params(params: &Value) -> Result<(String, Vec<String>), String> {
    let element = params
        .get("element")
        .and_then(Value::as_object)
        .ok_or_else(|| "element must be an object".to_owned())?;
    let shared_id = element
        .get("sharedId")
        .and_then(Value::as_str)
        .filter(|shared_id| !shared_id.is_empty())
        .ok_or_else(|| "element.sharedId must be a string".to_owned())?
        .to_owned();
    let files = params
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "files must be an array".to_owned())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "files entries must be strings".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((shared_id, files))
}

enum BidiInputPreparationError {
    Classic(ClassicError),
    DevTools(DevToolsError),
}

async fn bidi_input_classic_params_and_element_origins(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    context_id: &str,
    params: Value,
    event_sources: &mut BidiDevToolsEventSources,
) -> Result<(Value, ClassicElementOriginViewportPoints), BidiInputPreparationError> {
    let mut params = params;
    let shared_origins = rewrite_bidi_element_origins_for_classic_actions(&mut params)?;
    let mut element_origins = ClassicElementOriginViewportPoints::new();
    for (shared_id, classic_id) in shared_origins {
        let point = bidi_input_element_origin_viewport_point(
            scheduler,
            receivers,
            session_id,
            context_id,
            &shared_id,
            event_sources,
        )
        .await?;
        element_origins.insert(classic_id, point);
    }
    Ok((params, element_origins))
}

fn rewrite_bidi_element_origins_for_classic_actions(
    params: &mut Value,
) -> Result<BTreeMap<String, String>, BidiInputPreparationError> {
    let mut shared_origins = BTreeMap::new();
    let Some(actions) = params.get_mut("actions").and_then(Value::as_array_mut) else {
        return Ok(shared_origins);
    };
    for source in actions {
        let Some(source) = source.as_object_mut() else {
            continue;
        };
        let source_type = source
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !matches!(source_type.as_deref(), Some("pointer" | "wheel")) {
            continue;
        }
        let Some(source_actions) = source.get_mut("actions").and_then(Value::as_array_mut) else {
            continue;
        };
        for action in source_actions {
            let Some(action) = action.as_object_mut() else {
                continue;
            };
            let action_type = action.get("type").and_then(Value::as_str);
            if !matches!(
                (source_type.as_deref(), action_type),
                (Some("pointer"), Some("pointerMove")) | (Some("wheel"), Some("scroll"))
            ) {
                continue;
            }
            let Some(origin) = action.get_mut("origin").and_then(Value::as_object_mut) else {
                continue;
            };
            let Some(shared_id) = origin.get("sharedId") else {
                continue;
            };
            let Some(shared_id) = shared_id.as_str() else {
                return Err(BidiInputPreparationError::Classic(ClassicError::new(
                    ClassicErrorCode::InvalidArgument,
                    "BiDi input element origin sharedId must be a string",
                )));
            };
            let next_id = shared_origins.len().saturating_add(1);
            let classic_id = shared_origins
                .entry(shared_id.to_owned())
                .or_insert_with(|| classic_element_id(next_id as u32))
                .clone();
            origin.clear();
            origin.insert(CLASSIC_ELEMENT_REFERENCE_KEY.to_owned(), json!(classic_id));
        }
    }
    Ok(shared_origins)
}

async fn bidi_input_element_origin_viewport_point(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    session_id: &str,
    context_id: &str,
    shared_id: &str,
    event_sources: &mut BidiDevToolsEventSources,
) -> Result<moli_protocol_webdriver_classic::ClassicViewportPoint, BidiInputPreparationError> {
    let command = DevToolsCommand::DomObjectReference(DevToolsDomObjectReferenceCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: Some(DevToolsTargetId::from(context_id)),
            browser_context_id: None,
        },
        object_id: DevToolsRemoteHandleId::from(shared_id),
        operation: DevToolsDomObjectReferenceOperation::GetBoxModel,
    });
    let execution = scheduler
        .execute_devtools_command_with_external_load_wait_and_protocol_messages(receivers, command)
        .await;
    event_sources.extend_protocol_output(execution.protocol_output, Some(&*scheduler));
    match execution.result {
        Ok(DevToolsCommandResult::DomGeometry(geometry)) => {
            element_center_from_geometry(&geometry).map_err(BidiInputPreparationError::Classic)
        }
        Ok(_) => Err(BidiInputPreparationError::DevTools(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "input element origin geometry returned an unexpected result",
        ))),
        Err(error) => Err(BidiInputPreparationError::DevTools(error)),
    }
}

async fn bidi_resolve_subscribe_context(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    context_id: &str,
) -> Option<BidiValidatedSubscribeContext> {
    if let Some(info) = bidi_target_info_for_target(scheduler, session_id, context_id).await {
        let user_context = info
            .browser_context_id
            .as_ref()
            .map(|browser_context_id| browser_context_id.as_str().to_owned());
        return Some(BidiValidatedSubscribeContext {
            context: context_id.to_owned(),
            top_level_context: context_id.to_owned(),
            user_context,
        });
    }
    let top_level_context =
        bidi_top_level_context_for_frame_tree_context(scheduler, session_id, context_id).await?;
    let user_context = bidi_target_info_for_target(scheduler, session_id, &top_level_context)
        .await
        .and_then(|info| {
            info.browser_context_id
                .as_ref()
                .map(|browser_context_id| browser_context_id.as_str().to_owned())
        });
    Some(BidiValidatedSubscribeContext {
        context: context_id.to_owned(),
        top_level_context,
        user_context,
    })
}

async fn bidi_context_exists(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    context_id: &str,
) -> bool {
    bidi_resolve_subscribe_context(scheduler, session_id, context_id)
        .await
        .is_some()
}

#[derive(Debug, Clone, Default)]
struct BidiValidatedSubscribeTargets {
    contexts: Vec<BidiValidatedSubscribeContext>,
    user_contexts: Vec<String>,
}

impl BidiValidatedSubscribeTargets {
    fn record_to_bidi_state(self, bidi: &mut BidiConnectionState) {
        for context in self.contexts {
            bidi.record_known_bidi_subscription_context(
                &context.context,
                &context.top_level_context,
                context.user_context.as_deref(),
            );
        }
        for user_context in self.user_contexts {
            bidi.record_known_bidi_user_context(&user_context);
        }
    }
}

#[derive(Debug, Clone)]
struct BidiValidatedSubscribeContext {
    context: String,
    top_level_context: String,
    user_context: Option<String>,
}

enum BidiSessionSubscribeTargetValidation {
    Valid(BidiValidatedSubscribeTargets),
    Error(Value),
}

async fn validate_bidi_session_subscribe_targets(
    scheduler: &mut CdpScheduler,
    bidi: &BidiConnectionState,
    message: &Value,
) -> Option<BidiSessionSubscribeTargetValidation> {
    let id = message.get("id").and_then(Value::as_u64);
    let session_id = bidi.session_id()?;
    let (contexts, user_contexts) =
        bidi_session_subscribe_target_arrays_for_validation(message.get("params")?)?;
    if !bidi_session_subscribe_protocol_shape_is_valid(bidi, message, &contexts, &user_contexts) {
        return None;
    }
    let mut validated = BidiValidatedSubscribeTargets::default();
    for context in contexts {
        let Some(resolved_context) =
            bidi_resolve_subscribe_context(scheduler, session_id, &context).await
        else {
            return Some(BidiSessionSubscribeTargetValidation::Error(error_response(
                id,
                BidiErrorCode::NoSuchFrame,
                "context not found",
            )));
        };
        validated.contexts.push(resolved_context);
    }
    if user_contexts.is_empty() {
        return Some(BidiSessionSubscribeTargetValidation::Valid(validated));
    }
    let existing_user_contexts = bidi_existing_user_context_ids(scheduler, session_id).await;
    for user_context in user_contexts {
        if !existing_user_contexts.contains(&user_context) {
            return Some(BidiSessionSubscribeTargetValidation::Error(error_response(
                id,
                BidiErrorCode::NoSuchUserContext,
                "user context not found",
            )));
        }
        validated.user_contexts.push(user_context);
    }
    Some(BidiSessionSubscribeTargetValidation::Valid(validated))
}

fn bidi_session_subscribe_protocol_shape_is_valid(
    bidi: &BidiConnectionState,
    message: &Value,
    contexts: &[String],
    user_contexts: &[String],
) -> bool {
    let mut probe = bidi.clone();
    for context in contexts {
        probe.record_known_bidi_subscription_context(context, context, Some("default"));
    }
    for user_context in user_contexts {
        probe.record_known_bidi_user_context(user_context);
    }
    let mut registry = BidiSessionRegistry::new();
    probe
        .handle_message_with_session_registry(message.clone(), &mut registry)
        .response
        .get("type")
        == Some(&json!("success"))
}

fn bidi_session_subscribe_target_arrays_for_validation(
    params: &Value,
) -> Option<(Vec<String>, Vec<String>)> {
    if !params.is_object() {
        return None;
    }
    let contexts = bidi_optional_non_empty_string_array_for_validation(params, "contexts")?;
    let user_contexts =
        bidi_optional_non_empty_string_array_for_validation(params, "userContexts")?;
    if contexts.is_some() && user_contexts.is_some() {
        return None;
    }
    Some((
        contexts.unwrap_or_default(),
        user_contexts.unwrap_or_default(),
    ))
}

fn bidi_optional_non_empty_string_array_for_validation(
    params: &Value,
    field: &str,
) -> Option<Option<Vec<String>>> {
    let Some(value) = params.get(field) else {
        return Some(None);
    };
    let values = value.as_array()?;
    if values.is_empty() {
        return None;
    }
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        result.push(value.as_str()?.to_owned());
    }
    Some(Some(result))
}

async fn bidi_existing_user_context_ids(
    scheduler: &mut CdpScheduler,
    session_id: &str,
) -> BTreeSet<String> {
    let mut user_contexts = BTreeSet::from(["default".to_owned()]);
    let command = DevToolsCommand::GetBrowserContexts(DevToolsGetBrowserContextsCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: None,
            browser_context_id: None,
        },
    });
    let result = scheduler
        .execute_devtools_command_with_protocol_messages(command)
        .await
        .result;
    if let Ok(DevToolsCommandResult::GetBrowserContexts(result)) = result {
        user_contexts.extend(
            result
                .browser_context_ids
                .into_iter()
                .map(|browser_context_id| browser_context_id.into_string()),
        );
    }
    user_contexts
}

async fn bidi_input_viewport_bounds(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    context_id: &str,
) -> Result<ClassicViewportBounds, DevToolsError> {
    let command = DevToolsCommand::GetLayoutMetrics(DevToolsGetLayoutMetricsCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: Some(DevToolsTargetId::from(context_id)),
            browser_context_id: None,
        },
    });
    match scheduler
        .execute_devtools_command_with_protocol_messages(command)
        .await
        .result
    {
        Ok(DevToolsCommandResult::LayoutMetrics(result)) => Ok(ClassicViewportBounds::new(
            result.layout_viewport_width,
            result.layout_viewport_height,
        )),
        Ok(_) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "layout metrics returned an unexpected result",
        )),
        Err(error) => Err(error),
    }
}

fn bidi_response_from_classic_input_error(id: u64, error: ClassicError) -> serde_json::Value {
    let code = match error.code {
        ClassicErrorCode::InvalidSessionId => BidiErrorCode::InvalidSessionId,
        ClassicErrorCode::NoSuchElement => BidiErrorCode::NoSuchNode,
        ClassicErrorCode::NoSuchWindow => BidiErrorCode::NoSuchFrame,
        ClassicErrorCode::UnsupportedOperation => BidiErrorCode::UnsupportedOperation,
        ClassicErrorCode::UnknownError => BidiErrorCode::UnknownError,
        ClassicErrorCode::InvalidArgument
        | ClassicErrorCode::MoveTargetOutOfBounds
        | ClassicErrorCode::ElementNotInteractable
        | ClassicErrorCode::InvalidElementState
        | ClassicErrorCode::StaleElementReference
        | ClassicErrorCode::NoSuchFrame
        | ClassicErrorCode::InvalidSelector
        | ClassicErrorCode::JavascriptError
        | ClassicErrorCode::NoSuchAlert
        | ClassicErrorCode::NoSuchCookie
        | ClassicErrorCode::NoSuchShadowRoot
        | ClassicErrorCode::ScriptTimeout
        | ClassicErrorCode::SessionNotCreated
        | ClassicErrorCode::DetachedShadowRoot
        | ClassicErrorCode::Timeout
        | ClassicErrorCode::UnknownCommand
        | ClassicErrorCode::UnexpectedAlertOpen
        | ClassicErrorCode::InvalidCookieDomain => BidiErrorCode::InvalidArgument,
    };
    error_response(Some(id), code, &error.message)
}

async fn validate_bidi_top_level_context_command(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    command: &DevToolsCommand,
) -> Option<DevToolsError> {
    let target_id = match command {
        DevToolsCommand::CreateTarget(command) => command.context.target_id.as_ref()?,
        DevToolsCommand::CloseTarget(command) => &command.target_id,
        DevToolsCommand::ActivateTarget(command) => &command.target_id,
        DevToolsCommand::TraverseHistory(command) => command.context.target_id.as_ref()?,
        _ => return None,
    };
    let target_info = bidi_target_info_for_target(scheduler, session_id, target_id.as_str()).await;
    match target_info {
        Some(info) if matches!(info.kind, DevToolsTargetKind::Frame) => Some(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "ChildFrameContextNotSupported",
        )),
        Some(_) => None,
        None if bidi_frame_tree_contains_target(scheduler, session_id, target_id.as_str())
            .await =>
        {
            Some(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "ChildFrameContextNotSupported",
            ))
        }
        None => Some(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        )),
    }
}

async fn bidi_frame_tree_contains_target(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    target_id: &str,
) -> bool {
    bidi_top_level_context_for_frame_tree_context(scheduler, session_id, target_id)
        .await
        .is_some()
}

async fn bidi_top_level_context_for_frame_tree_context(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    target_id: &str,
) -> Option<String> {
    let command = DevToolsCommand::GetFrameTrees(DevToolsGetFrameTreesCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: None,
            browser_context_id: None,
        },
        max_depth: None,
    });
    let execution = scheduler
        .execute_devtools_command_with_protocol_messages(command)
        .await;
    let Ok(DevToolsCommandResult::GetFrameTrees(result)) = execution.result else {
        return None;
    };
    result
        .frame_trees
        .iter()
        .find_map(|tree| frame_tree_top_level_context_for_target(&tree.frame_tree, target_id))
}

fn frame_tree_top_level_context_for_target(
    frame_tree: &serde_json::Value,
    target_id: &str,
) -> Option<String> {
    let top_level_context = frame_tree
        .get("frame")
        .and_then(|frame| frame.get("id"))
        .and_then(serde_json::Value::as_str)?;
    if top_level_context == target_id {
        return Some(top_level_context.to_owned());
    }
    frame_tree_contains_target(frame_tree, target_id).then(|| top_level_context.to_owned())
}

fn frame_tree_contains_target(frame_tree: &serde_json::Value, target_id: &str) -> bool {
    frame_tree
        .get("frame")
        .and_then(|frame| frame.get("id"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| id == target_id)
        || frame_tree
            .get("childFrames")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|child| frame_tree_contains_target(child, target_id))
}

async fn bidi_target_lifecycle_event_for_target(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    target_id: &str,
) -> Option<TargetLifecycleEvent> {
    let result = bidi_target_info_for_target(scheduler, session_id, target_id).await?;
    target_lifecycle_event_from_target_info(result)
}

async fn bidi_target_info_for_target(
    scheduler: &mut CdpScheduler,
    session_id: &str,
    target_id: &str,
) -> Option<DevToolsTargetInfo> {
    let command = DevToolsCommand::GetTargetInfo(DevToolsGetTargetInfoCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: Some(DevToolsTargetId::from(target_id)),
            browser_context_id: None,
        },
        target_id: Some(DevToolsTargetId::from(target_id)),
    });
    let execution = scheduler
        .execute_devtools_command_with_protocol_messages(command)
        .await;
    let Ok(DevToolsCommandResult::GetTargetInfo(result)) = execution.result else {
        return None;
    };
    Some(result.target_info)
}

fn target_lifecycle_event_from_target_info(
    info: DevToolsTargetInfo,
) -> Option<TargetLifecycleEvent> {
    let target_id = info.target_id.clone()?;
    Some(TargetLifecycleEvent {
        target_id,
        browser_context_id: info.browser_context_id.clone(),
        kind: info.kind,
        url: info.url.clone(),
        target_info: Some(info),
    })
}

fn bidi_event_context_from_devtools_command(command: &DevToolsCommand) -> Option<String> {
    match command {
        DevToolsCommand::Navigate(command) => command.context.target_id.as_ref(),
        DevToolsCommand::Reload(command) => command.context.target_id.as_ref(),
        DevToolsCommand::EvaluateScript(command) => command.context.target_id.as_ref(),
        DevToolsCommand::CallFunction(command) => command.context.target_id.as_ref(),
        DevToolsCommand::GetRealms(command) => command.context.target_id.as_ref(),
        _ => None,
    }
    .map(|target_id| target_id.as_str().to_owned())
}

async fn complete_bidi_post_response_protocol_residences(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) -> Result<BidiDevToolsEventSources, BidiRendererOutputTransportFailure> {
    let mut event_sources = BidiDevToolsEventSources::from_protocol_output(
        scheduler
            .complete_ready_protocol_residences_after_command()
            .await,
        Some(&*scheduler),
    );
    event_sources.append(drain_ready_bidi_background_navigation(scheduler, receivers).await?);
    Ok(event_sources)
}

async fn drain_bidi_background_navigation_before_command(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) -> Result<BidiDevToolsEventSources, BidiRendererOutputTransportFailure> {
    let mut event_sources = drain_ready_bidi_background_navigation(scheduler, receivers).await?;
    while scheduler.has_inflight_background_navigation() {
        let Some(completion) = receivers.background_navigation_completion_rx.recv().await else {
            return Ok(event_sources);
        };
        match scheduler
            .drain_background_navigation_completion_with_progress_barrier(completion, receivers)
            .await
        {
            Ok(output) => event_sources.extend_protocol_output(output, Some(&*scheduler)),
            Err(failure) => {
                return Err(BidiRendererOutputTransportFailure::from_renderer(
                    event_sources,
                    failure,
                    scheduler,
                ));
            }
        }
        event_sources.append(drain_ready_bidi_background_navigation(scheduler, receivers).await?);
    }
    Ok(event_sources)
}

async fn drain_ready_bidi_background_navigation(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) -> Result<BidiDevToolsEventSources, BidiRendererOutputTransportFailure> {
    let mut event_sources = BidiDevToolsEventSources::default();
    event_sources.extend_protocol_output(
        scheduler
            .drain_background_events_around_inflight_navigation(&mut receivers.background_event_rx),
        Some(&*scheduler),
    );
    while let Ok(completion) = receivers.background_navigation_completion_rx.try_recv() {
        match scheduler
            .drain_background_navigation_completion_with_progress_barrier(completion, receivers)
            .await
        {
            Ok(output) => event_sources.extend_protocol_output(output, Some(&*scheduler)),
            Err(failure) => {
                return Err(BidiRendererOutputTransportFailure::from_renderer(
                    event_sources,
                    failure,
                    scheduler,
                ));
            }
        }
    }
    Ok(event_sources)
}

#[derive(Debug, Clone, Default)]
pub(super) struct SharedBidiSessionRegistry {
    inner: Arc<Mutex<BidiSessionRegistry>>,
}

impl SharedBidiSessionRegistry {
    pub(in crate::protocol_server) fn lock(
        &self,
    ) -> parking_lot::MutexGuard<'_, BidiSessionRegistry> {
        self.inner.lock()
    }
}

#[cfg(test)]
mod tests {
    use moli_protocol::CdpInitialStoragePartition;
    use moli_protocol::devtools_runtime::RuntimeConsoleEvent;
    use serde_json::json;

    use super::*;

    #[test]
    fn protocol_output_hook_prefers_automation_sidecar_over_protocol_message_parse() {
        let mut bidi = BidiConnectionState::new();
        let mut registry = BidiSessionRegistry::new();
        let session = bidi.handle_message_with_session_registry(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            }),
            &mut registry,
        );
        assert_eq!(session.response["type"], json!("success"));
        let subscribe = bidi.handle_message_with_session_registry(
            json!({
                "id": 2_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"]
                }
            }),
            &mut registry,
        );
        assert_eq!(subscribe.response["type"], json!("success"));

        let protocol_message = json!({
            "method": "Runtime.consoleAPICalled",
            "params": {
                "type": "log",
                "args": [{"type": "string", "value": "protocol text"}],
                "executionContextId": 7,
                "timestamp": 1.0
            }
        });
        let sidecar = AutomationEvent::RuntimeConsoleApiCalled(RuntimeConsoleEvent {
            target_id: None,
            console_type: "log".to_owned(),
            text: "sidecar text".to_owned(),
            args: vec![json!({"type": "string", "value": "sidecar text"})],
            stack: None,
            stack_trace: None,
            execution_context_id: Some(7),
            timestamp: Some(1.0),
        });
        let output = ProtocolOutputSequence::from_background_event(
            BackgroundProtocolEvent::immediate_automation_event(protocol_message, sidecar),
        );
        let mut events = Vec::new();

        extend_bidi_events_from_protocol_output(None, &mut bidi, &mut events, output, None);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["method"], json!("log.entryAdded"));
        assert_eq!(events[0]["params"]["text"], json!("sidecar text"));
        assert_ne!(events[0]["params"]["text"], json!("protocol text"));
    }

    #[tokio::test]
    async fn pending_navigation_response_consumes_typed_command_response() {
        let service = moli_core::browser::BrowserService::start().unwrap();
        let (mut scheduler, _) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let mut pending = Some(BidiPendingNavigationResponse {
            id: 5,
            target_id: "TID-1".to_owned(),
            url: "https://example.test/auth".to_owned(),
            channel: Some("chan".to_owned()),
            background_command_id: 42,
            wait: DevToolsNavigationWait::DocumentInstalled,
            reply: BidiNavigationReply::Command,
        });
        let sources = vec![
            BidiDevToolsEventSource::ProtocolMessage(json!({
                "id": 42_u64,
                "result": {
                    "navigation": "navigation-from-protocol-message"
                }
            })),
            BidiDevToolsEventSource::CommandResponse {
                command_id: Some(42),
                response: BackgroundCommandResponsePayload::Success {
                    result: json!({
                        "navigation": "navigation-from-typed-command-response",
                    }),
                },
            },
        ];

        let response =
            take_pending_navigation_response_from_sources(&mut scheduler, &mut pending, &sources)
                .expect("typed command response should complete pending navigation");

        assert!(pending.is_none());
        assert_eq!(response["type"], json!("success"));
        assert_eq!(response["id"], json!(5_u64));
        assert_eq!(response["goog:channel"], json!("chan"));
        assert_eq!(
            response["result"]["navigation"],
            json!("navigation-from-typed-command-response")
        );
        assert_eq!(
            response["result"]["url"],
            json!("https://example.test/auth")
        );
    }

    #[tokio::test]
    async fn pending_navigation_response_rejects_success_for_closed_target() {
        let service = moli_core::browser::BrowserService::start().unwrap();
        let (mut scheduler, _) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let mut pending = Some(BidiPendingNavigationResponse {
            id: 7,
            target_id: "closed-target".to_owned(),
            url: "https://example.test/closed".to_owned(),
            channel: Some("original-channel".to_owned()),
            background_command_id: 42,
            wait: DevToolsNavigationWait::Load,
            reply: BidiNavigationReply::Command,
        });
        let sources = [BidiDevToolsEventSource::CommandResponse {
            command_id: Some(42),
            response: BackgroundCommandResponsePayload::Success {
                result: json!({"loaderId": "original-loader"}),
            },
        }];
        let response =
            take_pending_navigation_response_from_sources(&mut scheduler, &mut pending, &sources)
                .expect("closed target must settle even without a lifecycle publication");
        assert_eq!(response["type"], "error");
        assert_eq!(response["id"], 7);
        assert_eq!(response["goog:channel"], "original-channel");
        assert_eq!(response["message"], "Target closed before navigation load");
        assert!(pending.is_none());
        assert!(
            take_pending_navigation_response_from_sources(&mut scheduler, &mut pending, &sources)
                .is_none()
        );
    }

    #[tokio::test]
    async fn pending_navigation_response_falls_back_to_raw_protocol_error() {
        let service = moli_core::browser::BrowserService::start().unwrap();
        let (mut scheduler, _) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let mut pending = Some(BidiPendingNavigationResponse {
            id: 7,
            target_id: "TID-1".to_owned(),
            url: "https://example.test/error".to_owned(),
            channel: Some("chan".to_owned()),
            background_command_id: 42,
            wait: DevToolsNavigationWait::DocumentInstalled,
            reply: BidiNavigationReply::Command,
        });
        let sources = vec![BidiDevToolsEventSource::ProtocolMessage(json!({
            "id": 42_u64,
            "error": {
                "code": -32000,
                "message": "navigation failed before typed response"
            }
        }))];

        let response =
            take_pending_navigation_response_from_sources(&mut scheduler, &mut pending, &sources)
                .expect("raw protocol error should complete pending navigation as a fallback");

        assert!(pending.is_none());
        assert_eq!(response["type"], json!("error"));
        assert_eq!(response["id"], json!(7_u64));
        assert_eq!(response["goog:channel"], json!("chan"));
        assert_eq!(response["error"], json!("unsupported operation"));
        assert_eq!(
            response["message"],
            json!("navigation failed before typed response")
        );
    }
}
