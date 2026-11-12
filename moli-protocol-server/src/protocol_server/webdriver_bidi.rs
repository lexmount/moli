use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
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
use moli_core::{RendererOutputTransportMessage, runtime::NavigationRuntimeConfig};
use moli_protocol::{
    BackgroundCommandResponsePayload, BackgroundProtocolEvent, CdpInitialStoragePartition,
    automation::{
        AutomationCommand, AutomationContext, AutomationEvent, AutomationResult,
        DevToolsDomObjectReferenceCommand, DevToolsDomObjectReferenceOperation, DevToolsError,
        DevToolsErrorKind, DevToolsGetBrowserContextsCommand, DevToolsGetLayoutMetricsCommand,
        DevToolsGetRealmsCommand, DevToolsGetTargetInfoCommand, DevToolsNavigationWait,
        DevToolsRemoteHandleId, DevToolsSessionId, DevToolsSetFileInputFilesCommand,
        DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind, FrontendProtocol,
        webdriver_bidi_navigation_id_from_loader_id,
    },
    conn::{PendingAutomationCommandAdmission, RuntimeInspectorResponseReady},
};
use moli_protocol_webdriver_bidi::{
    BidiCommandOutcome, BidiConnectionState, BidiDevToolsCommandDispatch, BidiErrorCode,
    BidiEventSourceHookPlan, BidiInputCommand, BidiInputCommandDispatch, BidiSessionRegistry,
    bidi_response_from_devtools_error, bidi_response_from_devtools_result,
    bidi_user_context_from_browser_context_id, error_response, script_realm_created_event,
    success_response,
};
use moli_protocol_webdriver_classic::{
    CLASSIC_ELEMENT_REFERENCE_KEY, ClassicActionState, ClassicDevToolsCommandContext,
    ClassicElementOriginViewportPoints, ClassicError, ClassicErrorCode,
    ClassicUnhandledPromptBehavior, ClassicViewportBounds, classic_element_id,
    element_center_from_geometry, perform_actions_ticks_with_state_and_viewport,
    release_actions_commands,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

mod input_progress;
use input_progress::{BidiInputState, BidiPendingInput, wait_for_bidi_input_deadline};
use tracing::warn;

use crate::cdp_scheduler::{
    AutomationCommandProgress, AutomationWorkflowReady, CdpScheduler, CdpSchedulerEventReceivers,
    PendingAutomationExecution, ProtocolAdapterScheduler, ProtocolAdapterSchedulerAdvance,
    ProtocolAdapterSchedulerInput, ProtocolOutputSequence, RendererOutputTransportFailure,
};

use super::webdriver_files::selected_files_from_paths;
use super::{AppState, BrowserInstanceControl, protocol_local_executor::spawn_protocol_local_task};

pub(super) async fn ws_bidi_session_upgrade_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    let initial_storage_partition = state.initial_storage_partition();
    let web_socket_url = state.bidi_ws_url;
    let session_registry = state.bidi_session_registry;
    let control = state.control;
    let navigation_runtime_config = state.navigation_runtime_config;
    ws.on_upgrade(move |socket| {
        handle_bidi_session_socket(
            socket,
            web_socket_url,
            session_registry,
            initial_storage_partition,
            navigation_runtime_config,
            control,
        )
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
    let Some(unhandled_prompt_behavior) = state
        .classic_session_registry
        .unhandled_prompt_behavior(&session_id)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let web_socket_url = format!("{}/{}", state.bidi_ws_url.trim_end_matches('/'), session_id);
    ws.on_upgrade(move |socket| async move {
        let Some(_connection) = state.control.connection() else {
            return;
        };
        if !runtime
            .attach_bidi_socket(
                socket,
                web_socket_url,
                session_id,
                unhandled_prompt_behavior,
                state.bidi_session_registry,
            )
            .await
        {
            warn!("failed to attach WebDriver BiDi WebSocket to existing Classic runtime");
        }
    })
}

async fn handle_bidi_session_socket(
    socket: WebSocket,
    web_socket_url: String,
    session_registry: SharedBidiSessionRegistry,
    initial_storage_partition: CdpInitialStoragePartition,
    navigation_runtime_config: NavigationRuntimeConfig,
    control: BrowserInstanceControl,
) {
    let Some(_connection) = control.connection() else {
        return;
    };
    let completion = spawn_protocol_local_task("bidi-socket", move || {
        handle_bidi_session_socket_local(
            socket,
            web_socket_url,
            session_registry,
            initial_storage_partition,
            navigation_runtime_config,
            control,
        )
    })
    .await;
    if let Err(error) = completion {
        warn!(?error, "BiDi socket worker failed");
    }
}

async fn handle_bidi_session_socket_local(
    socket: WebSocket,
    web_socket_url: String,
    session_registry: SharedBidiSessionRegistry,
    initial_storage_partition: CdpInitialStoragePartition,
    navigation_runtime_config: NavigationRuntimeConfig,
    control: BrowserInstanceControl,
) {
    let mut actor = BidiSocketActor::new(socket, web_socket_url);
    let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
        initial_storage_partition,
        navigation_runtime_config,
    );
    actor.install_runtime_response_ready_sender(&mut scheduler);
    let mut adapter_scheduler = ProtocolAdapterScheduler::default();
    loop {
        adapter_scheduler.schedule_turn_if_needed(&scheduler, false);
        let input_deadline = actor.next_input_deadline();
        tokio::select! {
            biased;
            _ = control.shutdown_requested() => break,
            completion = receivers.protocol_work_completion_rx.recv() => {
                let Some(completion) = completion else { break; };
                if !actor.handle_protocol_work_completion(&mut scheduler, &mut receivers, completion).await { break; }
            }
            _ = receivers.state_events_ready.notified() => {
                scheduler.apply_protocol_state_events();
                if !actor.advance_pending_commands(&mut scheduler, &mut receivers).await {
                    break;
                }
            }
            _ = wait_for_bidi_input_deadline(input_deadline) => {
                if !actor.advance_pending_commands(&mut scheduler, &mut receivers).await { break; }
            }
            maybe_message = actor.socket.recv() => {
                let Some(message) = maybe_message else {
                    break;
                };
                if !actor.handle_socket_message(
                    &mut scheduler,
                    &mut receivers,
                    &session_registry,
                    message,
                ).await {
                    break;
                }
            }
            maybe_completion = receivers.background_navigation_completion_rx.recv() => {
                let Some(completion) = maybe_completion else {
                    break;
                };
                if !actor.handle_background_navigation_completion(
                    &mut scheduler,
                    &mut receivers,
                    completion,
                ).await {
                    break;
                }
            }
            maybe_event = receivers.background_event_rx.recv() => {
                let Some(event) = maybe_event else {
                    break;
                };
                let output = scheduler.route_background_event_around_inflight_navigation(event);
                if !actor
                    .send_or_route_protocol_output(&mut scheduler, &mut receivers, output, None)
                    .await
                {
                    break;
                }
            }
            maybe_publication = receivers.renderer_publication_rx.recv() => {
                let Some(publication) = maybe_publication else {
                    break;
                };
                if !actor.handle_renderer_publication(
                    &mut adapter_scheduler,
                    &mut scheduler,
                    &mut receivers,
                    publication,
                ).await {
                    break;
                }
            }
            maybe_ready = actor.automation_workflow_ready_rx.recv() => {
                let Some(ready) = maybe_ready else { break; };
                if !actor.handle_automation_workflow_ready(&mut scheduler, &mut receivers, ready).await { break; }
            }
            maybe_response = actor.runtime_response_ready_rx.recv() => {
                let Some(response) = maybe_response else {
                    break;
                };
                if !actor.handle_runtime_response_ready(&mut scheduler, &mut receivers, response).await {
                    break;
                }
            }
            input = adapter_scheduler.recv_input() => {
                if !actor.handle_adapter_scheduler_input(
                    &mut adapter_scheduler,
                    &mut scheduler,
                    &mut receivers,
                    input,
                ).await {
                    break;
                }
            }
        }
    }
    actor
        .release_event_sources(&mut scheduler, &mut receivers)
        .await;
    actor.release_session(&mut session_registry.lock());
}

pub(in crate::protocol_server) struct BidiSocketActor {
    socket: WebSocket,
    bidi: BidiConnectionState,
    input_action_states: BTreeMap<String, std::rc::Rc<std::cell::RefCell<BidiInputState>>>,
    pending_navigation_responses: Vec<BidiPendingNavigationResponse>,
    pending_commands: VecDeque<BidiPendingCommand>,
    runtime_response_ready_tx: mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    runtime_response_ready_rx: mpsc::UnboundedReceiver<RuntimeInspectorResponseReady>,
    automation_workflow_ready_tx: mpsc::UnboundedSender<AutomationWorkflowReady>,
    automation_workflow_ready_rx: mpsc::UnboundedReceiver<AutomationWorkflowReady>,
}

pub(in crate::protocol_server) enum BidiSocketActorInput {
    Socket(Option<Result<Message, axum::Error>>),
    AdapterScheduler(ProtocolAdapterSchedulerInput),
    RuntimeResponseReady(Option<Box<RuntimeInspectorResponseReady>>),
    AutomationWorkflowReady(Option<Box<AutomationWorkflowReady>>),
    InputDeadline,
}

impl BidiSocketActor {
    pub(in crate::protocol_server) fn new(socket: WebSocket, web_socket_url: String) -> Self {
        let (runtime_response_ready_tx, runtime_response_ready_rx) = mpsc::unbounded_channel();
        let (automation_workflow_ready_tx, automation_workflow_ready_rx) =
            mpsc::unbounded_channel();
        Self {
            socket,
            bidi: BidiConnectionState::with_web_socket_url(web_socket_url),
            input_action_states: BTreeMap::new(),
            pending_navigation_responses: Vec::new(),
            pending_commands: VecDeque::new(),
            runtime_response_ready_tx,
            runtime_response_ready_rx,
            automation_workflow_ready_tx,
            automation_workflow_ready_rx,
        }
    }

    pub(in crate::protocol_server) fn install_runtime_response_ready_sender(
        &self,
        scheduler: &mut CdpScheduler,
    ) {
        scheduler
            .set_runtime_inspector_response_ready_sender(self.runtime_response_ready_tx.clone());
    }

    pub(in crate::protocol_server) fn attach_existing_session(
        &mut self,
        session_id: String,
        registry: &mut BidiSessionRegistry,
    ) -> bool {
        self.bidi.attach_existing_session(session_id, registry)
    }

    pub(in crate::protocol_server) fn configure_existing_session_prompt_handlers(
        &mut self,
        scheduler: &mut CdpScheduler,
        behavior: &ClassicUnhandledPromptBehavior,
    ) -> Result<(), moli_protocol_webdriver_bidi::BidiError> {
        self.bidi
            .set_unhandled_prompt_behavior(&behavior.returned_capability())?;
        self.bidi.set_file_prompt_handler_for_script_commands(
            behavior.file_prompt_handler_for_bidi_script_commands(),
        );
        scheduler.register_bidi_javascript_dialog_handler(self.bidi.javascript_dialog_handlers());
        Ok(())
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
        // Release the handler before renderer cleanup can wait on an abandoned prompt.
        // An attached Classic session retains its independent registration.
        let _ = scheduler
            .unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverBidi);
        self.release_pending_commands(scheduler, receivers).await;
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

    async fn release_pending_commands(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) {
        while let Some(pending) = self.pending_commands.pop_front() {
            match pending.stage {
                Some(BidiPendingCommandStage::Executing(runtime_pending)) => {
                    let _output = scheduler
                        .cancel_automation_command_progress(receivers, runtime_pending)
                        .await;
                }
                Some(BidiPendingCommandStage::WaitingForNavigation { key, .. }) => {
                    scheduler
                        .release_navigation_completion(&pending.completion.command_context, &key);
                }
                _ => {}
            }
        }
    }

    /// Receives the BiDi-side inputs of an attached Classic session.
    ///
    /// The shared adapter scheduler remains outside the socket actor so a
    /// Classic-to-BiDi mode switch cannot replace its exact load residence.
    /// Selection order intentionally remains socket, adapter terminal/turn,
    /// then Runtime response, matching the pre-unification attached-session
    /// contract.
    pub(in crate::protocol_server) async fn recv_attached_input(
        &mut self,
        adapter_scheduler: &mut ProtocolAdapterScheduler,
    ) -> BidiSocketActorInput {
        let input_deadline = self.next_input_deadline();
        tokio::select! {
            biased;
            message = self.socket.recv() => BidiSocketActorInput::Socket(message),
            _ = wait_for_bidi_input_deadline(input_deadline) => BidiSocketActorInput::InputDeadline,
            input = adapter_scheduler.recv_input() => {
                BidiSocketActorInput::AdapterScheduler(input)
            }
            response = self.runtime_response_ready_rx.recv() => {
                BidiSocketActorInput::RuntimeResponseReady(response.map(Box::new))
            }
            ready = self.automation_workflow_ready_rx.recv() => {
                BidiSocketActorInput::AutomationWorkflowReady(ready.map(Box::new))
            }
        }
    }

    pub(in crate::protocol_server) async fn handle_socket_message(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        session_registry: &SharedBidiSessionRegistry,
        message: Result<Message, axum::Error>,
    ) -> bool {
        if !handle_bidi_socket_message(
            &mut self.socket,
            scheduler,
            receivers,
            &mut self.bidi,
            &mut self.input_action_states,
            session_registry,
            &mut self.pending_navigation_responses,
            &mut self.pending_commands,
            &self.automation_workflow_ready_tx,
            message,
        )
        .await
        {
            return false;
        }
        self.advance_pending_commands(scheduler, receivers).await
    }

    pub(in crate::protocol_server) async fn advance_pending_commands(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) -> bool {
        // Commands own independent waits. Iteration order is not an execution
        // dependency, and one slow context must not retain another's reply.
        loop {
            let mut progressed = false;
            for _ in 0..self.pending_commands.len() {
                let mut command = self
                    .pending_commands
                    .pop_front()
                    .expect("bounded pending snapshot");
                let stage = command.stage.take().expect("an owned pending stage");
                match stage {
                    BidiPendingCommandStage::WaitingForInputTurn => {
                        if !self.input_owner_is_current(scheduler, &command)
                            || self.input_turn_is_available(&command)
                        {
                            progressed = true;
                            if !self
                                .begin_input_command(scheduler, receivers, command)
                                .await
                            {
                                return false;
                            }
                        } else {
                            command.stage = Some(BidiPendingCommandStage::WaitingForInputTurn);
                            self.pending_commands.push_back(command);
                        }
                    }
                    BidiPendingCommandStage::InputPause(deadline) => {
                        if deadline <= Instant::now()
                            || !self.input_owner_is_current(scheduler, &command)
                        {
                            progressed = true;
                            if !self
                                .continue_input_actions(scheduler, receivers, command)
                                .await
                            {
                                return false;
                            }
                        } else {
                            command.stage = Some(BidiPendingCommandStage::InputPause(deadline));
                            self.pending_commands.push_back(command);
                        }
                    }
                    BidiPendingCommandStage::WaitingForDocument(admission) => {
                        if command.input.is_some()
                            && !self.input_owner_is_current(scheduler, &command)
                        {
                            progressed = true;
                            if !self
                                .fail_closed_input_command(scheduler, receivers, command)
                                .await
                            {
                                return false;
                            }
                            continue;
                        }
                        match scheduler
                            .resume_owned_automation_command_admission(
                                receivers,
                                &self.automation_workflow_ready_tx,
                                admission,
                            )
                            .await
                        {
                            Ok(progress) => {
                                progressed = true;
                                if !self
                                    .apply_pending_command_progress(
                                        scheduler, receivers, command, progress,
                                    )
                                    .await
                                {
                                    return false;
                                }
                            }
                            Err(admission) => {
                                command.stage =
                                    Some(BidiPendingCommandStage::WaitingForDocument(admission));
                                self.pending_commands.push_back(command);
                            }
                        }
                    }
                    BidiPendingCommandStage::NativeComplete(execution) => {
                        progressed = true;
                        if !self
                            .apply_pending_command_progress(
                                scheduler,
                                receivers,
                                command,
                                AutomationCommandProgress::Complete(execution),
                            )
                            .await
                        {
                            return false;
                        }
                    }
                    BidiPendingCommandStage::WaitingForNavigation { key, mut execution } => {
                        if let Some(result) = scheduler
                            .poll_navigation_completion(&command.completion.command_context, &key)
                        {
                            progressed = true;
                            scheduler.release_navigation_completion(
                                &command.completion.command_context,
                                &key,
                            );
                            if let Err(error) = result {
                                execution.result = Err(error);
                            }
                            if !self
                                .finish_pending_command(scheduler, receivers, command, *execution)
                                .await
                            {
                                return false;
                            }
                        } else {
                            command.stage = Some(BidiPendingCommandStage::WaitingForNavigation {
                                key,
                                execution,
                            });
                            self.pending_commands.push_back(command);
                        }
                    }
                    stage => {
                        command.stage = Some(stage);
                        self.pending_commands.push_back(command);
                    }
                }
            }
            if !progressed {
                break;
            }
        }
        true
    }

    pub(in crate::protocol_server) async fn handle_protocol_work_completion(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        completion: crate::cdp_scheduler::ProtocolWorkCompletion,
    ) -> bool {
        match scheduler.complete_protocol_work(completion) {
            Ok(output) => {
                self.send_or_route_protocol_output(scheduler, receivers, output, None)
                    .await
            }
            Err(error) => {
                tracing::error!(%error, "protocol owner work failed during recovery");
                false
            }
        }
    }

    pub(in crate::protocol_server) async fn handle_background_navigation_completion(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        completion: moli_protocol::BackgroundNavigationCompletion,
    ) -> bool {
        let output = scheduler
            .drain_background_navigation_completion_with_progress_barrier(completion, receivers)
            .await;
        match output {
            Ok(output) => {
                self.send_or_route_protocol_output(scheduler, receivers, output, None)
                    .await
            }
            Err(failure) => {
                let (output, _error) = failure.into_parts();
                let _ = self
                    .send_or_route_protocol_output(scheduler, receivers, output, None)
                    .await;
                false
            }
        }
    }

    pub(in crate::protocol_server) async fn handle_renderer_publication(
        &mut self,
        adapter_scheduler: &mut ProtocolAdapterScheduler,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        publication: RendererOutputTransportMessage,
    ) -> bool {
        let output = adapter_scheduler.ingest_renderer_publication(scheduler, publication);
        self.send_or_route_protocol_output(scheduler, receivers, output, None)
            .await
    }

    pub(in crate::protocol_server) async fn handle_runtime_response_ready(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        response: RuntimeInspectorResponseReady,
    ) -> bool {
        let output = scheduler.route_registered_runtime_inspector_response(response);
        self.send_or_route_protocol_output(scheduler, receivers, output, None)
            .await
    }

    pub(in crate::protocol_server) async fn handle_automation_workflow_ready(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        ready: AutomationWorkflowReady,
    ) -> bool {
        if !self
            .complete_pending_command_workflow(scheduler, receivers, ready)
            .await
        {
            return false;
        }
        self.advance_pending_commands(scheduler, receivers).await
    }

    pub(in crate::protocol_server) async fn handle_adapter_scheduler_input(
        &mut self,
        adapter_scheduler: &mut ProtocolAdapterScheduler,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        input: ProtocolAdapterSchedulerInput,
    ) -> bool {
        let output = match adapter_scheduler.advance_input(scheduler, input) {
            ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(output) => output,
            ProtocolAdapterSchedulerAdvance::Idle => ProtocolOutputSequence::empty(),
        };
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
        if !self
            .publish_pending_command_output(scheduler, receivers, output, owner_context)
            .await
        {
            return false;
        }
        self.advance_pending_commands(scheduler, receivers).await
    }

    async fn publish_pending_command_output(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        output: ProtocolOutputSequence,
        owner_context: Option<&str>,
    ) -> bool {
        let output = self.route_pending_command_output(scheduler, output);
        send_bidi_protocol_output(
            &mut self.socket,
            scheduler,
            receivers,
            &mut self.bidi,
            output,
            owner_context,
            &mut self.pending_navigation_responses,
        )
        .await
    }

    fn route_pending_command_output(
        &mut self,
        scheduler: &mut CdpScheduler,
        mut output: ProtocolOutputSequence,
    ) -> ProtocolOutputSequence {
        for command in &mut self.pending_commands {
            let stage = command
                .stage
                .take()
                .expect("a pending command retains its stage");
            command.stage = Some(match stage {
                BidiPendingCommandStage::Executing(pending) => {
                    let AutomationCommandProgress::Pending {
                        pending,
                        protocol_output,
                    } = scheduler.route_automation_command_output(pending, output)
                    else {
                        unreachable!("routing recorded output does not complete a Native wait")
                    };
                    output = protocol_output;
                    BidiPendingCommandStage::Executing(pending)
                }
                stage => stage,
            });
        }
        output
    }

    async fn complete_pending_command_workflow(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        ready: AutomationWorkflowReady,
    ) -> bool {
        let Some(index) = self.pending_commands.iter().position(|command| {
            matches!(command.stage.as_ref(), Some(BidiPendingCommandStage::Executing(pending)) if pending.command_id() == ready.command_id())
        }) else { return true; };
        let mut pending_command = self
            .pending_commands
            .remove(index)
            .expect("the matched command remains owned");
        let Some(BidiPendingCommandStage::Executing(pending)) = pending_command.stage.take() else {
            unreachable!("the matching Runtime notification requires an executing command");
        };
        let progress = scheduler
            .complete_automation_workflow_ready(
                receivers,
                &self.automation_workflow_ready_tx,
                pending,
                ready,
                ProtocolOutputSequence::empty(),
            )
            .await;
        self.apply_pending_command_progress(scheduler, receivers, pending_command, progress)
            .await
    }

    async fn apply_pending_command_progress(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut pending_command: BidiPendingCommand,
        progress: AutomationCommandProgress,
    ) -> bool {
        match progress {
            AutomationCommandProgress::Complete(mut execution) => {
                if pending_command.input.is_some() {
                    return self
                        .advance_input_command(scheduler, receivers, pending_command, *execution)
                        .await;
                }
                if let Some(key) = scheduler.navigation_completion_key(
                    &pending_command.completion.command_context,
                    pending_command.completion.navigation_wait,
                    &execution.result,
                ) {
                    if let Some(result) = scheduler.poll_navigation_completion(
                        &pending_command.completion.command_context,
                        &key,
                    ) {
                        scheduler.release_navigation_completion(
                            &pending_command.completion.command_context,
                            &key,
                        );
                        if let Err(error) = result {
                            execution.result = Err(error);
                        }
                    } else {
                        let output = std::mem::take(&mut execution.protocol_output);
                        pending_command.stage =
                            Some(BidiPendingCommandStage::WaitingForNavigation { key, execution });
                        self.pending_commands.push_back(pending_command);
                        return self
                            .publish_pending_command_output(scheduler, receivers, output, None)
                            .await;
                    }
                }
                self.finish_pending_command(scheduler, receivers, pending_command, *execution)
                    .await
            }
            AutomationCommandProgress::Pending {
                pending,
                protocol_output,
            } => {
                pending_command.stage = Some(BidiPendingCommandStage::Executing(pending));
                self.pending_commands.push_back(pending_command);
                let protocol_output = self.route_pending_command_output(scheduler, protocol_output);
                send_bidi_protocol_output(
                    &mut self.socket,
                    scheduler,
                    receivers,
                    &mut self.bidi,
                    protocol_output,
                    None,
                    &mut self.pending_navigation_responses,
                )
                .await
            }
        }
    }
    async fn finish_pending_command(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut pending: BidiPendingCommand,
        mut execution: crate::cdp_scheduler::AutomationExecution,
    ) -> bool {
        execution.protocol_output =
            self.route_pending_command_output(scheduler, execution.protocol_output);
        let output = complete_bidi_automation_command_execution(
            scheduler,
            receivers,
            &mut pending.completion,
            execution,
        )
        .await;
        send_completed_bidi_command(
            &mut self.socket,
            scheduler,
            receivers,
            &mut self.bidi,
            &mut self.pending_navigation_responses,
            pending,
            output,
        )
        .await
    }
}

async fn handle_bidi_socket_message(
    socket: &mut WebSocket,
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    input_action_states: &mut BTreeMap<String, std::rc::Rc<std::cell::RefCell<BidiInputState>>>,
    session_registry: &SharedBidiSessionRegistry,
    pending_navigation_responses: &mut Vec<BidiPendingNavigationResponse>,
    pending_commands: &mut VecDeque<BidiPendingCommand>,
    runtime_response_ready_tx: &mpsc::UnboundedSender<AutomationWorkflowReady>,
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
    let mut context_created_replay = Vec::new();
    let mut admission_event_sources = BidiDevToolsEventSources::default();
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
                        automation_command: None,
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
                        && let Some(validation) = validate_bidi_session_subscribe_targets(
                            scheduler,
                            &mut admission_event_sources,
                            bidi,
                            &message,
                        ) {
                        match validation {
                            BidiSessionSubscribeTargetValidation::Valid(targets) => {
                                targets.record_to_bidi_state(bidi);
                                let frames = scheduler.frame_index_snapshot();
                                bidi.record_frame_index_snapshot(frames.clone());
                                let previously_observed = bidi
                                    .context_created_events_for_frames(&frames)
                                    .into_iter()
                                    .filter_map(|event| {
                                        event["params"]["context"].as_str().map(str::to_owned)
                                    })
                                    .collect::<BTreeSet<_>>();
                                subscribe_hook_plan = message.get("params").and_then(|params| {
                                    bidi.subscribe_hook_plan_for_params(params).ok()
                                });
                                let mut registry = session_registry.lock();
                                let outcome = bidi
                                    .handle_message_with_session_registry(message, &mut registry);
                                if outcome.response["type"] == "success" {
                                    context_created_replay = bidi
                                        .context_created_events_for_frames(&frames)
                                        .into_iter()
                                        .filter(|event| {
                                            event["params"]["context"].as_str().is_some_and(
                                                |context| !previously_observed.contains(context),
                                            )
                                        })
                                        .collect();
                                }
                                outcome
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
                                    automation_command: None,
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
                automation_command: None,
                input_command: None,
            },
            None,
            None,
            None,
            None,
        ),
    };
    admission_event_sources.extend_protocol_output(apply_bidi_dialog_handler_lifecycle(
        scheduler,
        bidi,
        command_method.as_deref(),
        &outcome,
    ));
    let BidiCommandOutcome {
        response,
        channel: command_channel,
        close_connection,
        automation_command,
        input_command,
        ..
    } = outcome;
    let pending_navigation_candidate = automation_command.as_ref().and_then(|dispatch| {
        pending_navigation_response_for_dispatch(dispatch, command_channel.as_deref())
    });
    let command_start = {
        match (automation_command, input_command) {
            (Some(dispatch), None) => {
                start_bidi_automation_command(
                    scheduler,
                    receivers,
                    runtime_response_ready_tx,
                    bidi,
                    dispatch,
                    pending_navigation_candidate
                        .as_ref()
                        .map(|candidate| candidate.background_command_id),
                )
                .await
            }
            (None, Some(dispatch)) => {
                input_progress::start_bidi_input_command(
                    scheduler,
                    receivers,
                    runtime_response_ready_tx,
                    bidi,
                    input_action_states,
                    dispatch,
                )
                .await
            }
            (Some(_), Some(_)) => BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
                response: error_response(
                    None,
                    BidiErrorCode::UnsupportedOperation,
                    "BiDi command produced conflicting dispatches",
                ),
                event_sources: Vec::new(),
                post_response_event_sources: Vec::new(),
                event_context: None,
            }),
            (None, None) => BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
                response,
                event_sources: Vec::new(),
                post_response_event_sources: Vec::new(),
                event_context: None,
            }),
        }
    };
    let mut command_output = match command_start {
        BidiDevToolsCommandStart::Complete(mut output) => {
            let mut prefix = admission_event_sources.into_sources();
            prefix.append(&mut output.event_sources);
            output.event_sources = prefix;
            output
        }
        BidiDevToolsCommandStart::Pending(pending) => {
            let mut pending = *pending;
            admission_event_sources.append(std::mem::take(&mut pending.completion.event_sources));
            pending.completion.event_sources = admission_event_sources;
            pending.command_method = command_method;
            pending.command_params = command_params;
            pending.command_channel = command_channel;
            pending.pending_navigation_candidate = pending_navigation_candidate;
            let initial_sources =
                std::mem::take(&mut pending.completion.event_sources).into_sources();
            let pending_navigation_reply = take_pending_navigation_response_from_sources(
                pending_navigation_responses,
                &initial_sources,
            );
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
            if !send_bidi_json_events(socket, pending_navigation_reply).await {
                return false;
            }
            pending_commands.push_back(pending);
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
    let defer_current_response = pending_navigation_candidate.is_some()
        && bidi_response_is_missing_automation_command_result(&command_output.response)
        && sources_include_auth_required_pause(&command_output.event_sources);
    if defer_current_response {
        pending_navigation_responses.extend(pending_navigation_candidate);
    }
    let mut pending_response_from_event_sources = if defer_current_response {
        Vec::new()
    } else {
        take_pending_navigation_response_from_sources(
            pending_navigation_responses,
            &command_output.event_sources,
        )
    };
    let pending_response_from_post_response_event_sources = if defer_current_response {
        Vec::new()
    } else {
        take_pending_navigation_response_from_sources(
            pending_navigation_responses,
            &command_output.post_response_event_sources,
        )
    };
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
        bidi_events.extend(context_created_replay);
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
        let mut replay_event_sources = BidiDevToolsEventSources::default();
        let replay_events = replay_existing_bidi_realm_created_events(
            scheduler,
            receivers,
            &mut replay_event_sources,
            bidi,
            previous_realm_created_contexts.flatten(),
        )
        .await;
        let replay_event_sources = replay_event_sources.into_sources();
        pending_response_from_event_sources.extend(take_pending_navigation_response_from_sources(
            pending_navigation_responses,
            &replay_event_sources,
        ));
        bidi_events.extend(subscribed_bidi_events_from_devtools_event_sources(
            Some(&*scheduler),
            bidi,
            &replay_event_sources,
            None,
        ));
        bidi_events.extend(replay_events);
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
    if !send_bidi_json_events(socket, pending_response_from_event_sources).await {
        return false;
    }
    if !send_bidi_json_events(socket, post_response_bidi_events).await {
        return false;
    }
    if !send_bidi_json_events(socket, pending_response_from_post_response_event_sources).await {
        return false;
    }
    if close_connection || renderer_output_transport_terminal {
        let _ = socket.close().await;
        return false;
    }
    true
}

fn apply_bidi_dialog_handler_lifecycle(
    scheduler: &mut CdpScheduler,
    bidi: &BidiConnectionState,
    method: Option<&str>,
    outcome: &BidiCommandOutcome,
) -> ProtocolOutputSequence {
    if outcome.close_connection {
        return scheduler
            .unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverBidi);
    }
    if method == Some("session.new") && outcome.response["type"] == "success" {
        scheduler.register_bidi_javascript_dialog_handler(bidi.javascript_dialog_handlers());
    }
    ProtocolOutputSequence::empty()
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

async fn send_completed_bidi_command(
    socket: &mut WebSocket,
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    bidi: &mut BidiConnectionState,
    pending_navigation_responses: &mut Vec<BidiPendingNavigationResponse>,
    pending: BidiPendingCommand,
    mut command_output: BidiDevToolsCommandOutput,
) -> bool {
    let BidiPendingCommand {
        command_method,
        command_params,
        command_channel,
        pending_navigation_candidate,
        ..
    } = pending;
    let renderer_output_transport_terminal = receivers.renderer_publication_rx.is_closed();
    command_output.response =
        bidi_message_with_channel(command_output.response, command_channel.as_deref());
    let command_hook_plan = bidi.record_bidi_command_response(
        command_method.as_deref(),
        command_params.as_ref(),
        &command_output.response,
    );
    let defer_current_response = pending_navigation_candidate.is_some()
        && bidi_response_is_missing_automation_command_result(&command_output.response)
        && sources_include_auth_required_pause(&command_output.event_sources);
    if defer_current_response {
        pending_navigation_responses.extend(pending_navigation_candidate);
    }
    let pending_response_from_event_sources = if defer_current_response {
        Vec::new()
    } else {
        take_pending_navigation_response_from_sources(
            pending_navigation_responses,
            &command_output.event_sources,
        )
    };
    let pending_response_from_post_response_event_sources = if defer_current_response {
        Vec::new()
    } else {
        take_pending_navigation_response_from_sources(
            pending_navigation_responses,
            &command_output.post_response_event_sources,
        )
    };
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
    if !send_bidi_json_events(socket, pending_response_from_event_sources).await {
        return false;
    }
    if !send_bidi_json_events(socket, post_response_bidi_events).await {
        return false;
    }
    if !send_bidi_json_events(socket, pending_response_from_post_response_event_sources).await {
        return false;
    }
    !renderer_output_transport_terminal
}

use moli_protocol_webdriver_bidi::bidi_message_with_channel;

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
    if let Some(contexts) = plan.runtime_contexts() {
        if contexts.is_empty() {
            let runtime_enable_result =
                enable_bidi_runtime_protocol_sources(scheduler, receivers).await;
            let runtime_enable_output = materialize_bidi_event_source_hook_output(
                scheduler,
                bidi,
                events,
                runtime_enable_result,
                None,
            )?;
            if !runtime_enable_output.is_empty() && plan.runtime_events_enabled() {
                bidi.record_bidi_runtime_events_opened();
            }
            extend_bidi_events_from_protocol_output(
                Some(&*scheduler),
                bidi,
                events,
                runtime_enable_output,
                None,
            );
        } else {
            for context in contexts {
                let runtime_enable_result = scheduler
                    .enable_runtime_listener_for_target(receivers, context)
                    .await;
                let runtime_enable_output = materialize_bidi_event_source_hook_output(
                    scheduler,
                    bidi,
                    events,
                    runtime_enable_result,
                    Some(context),
                )?;
                if !runtime_enable_output.is_empty() && plan.records_runtime_context_ownership() {
                    bidi.record_bidi_runtime_event_source_opened(context);
                }
                extend_bidi_events_from_protocol_output(
                    Some(&*scheduler),
                    bidi,
                    events,
                    runtime_enable_output,
                    Some(context),
                );
            }
        }
    }
    if plan.runtime_events_disabled() {
        let runtime_disable_result =
            disable_bidi_runtime_protocol_sources(scheduler, receivers).await;
        let runtime_disable_output = materialize_bidi_event_source_hook_output(
            scheduler,
            bidi,
            events,
            runtime_disable_result,
            None,
        )?;
        if !runtime_disable_output.is_empty() {
            bidi.record_bidi_runtime_events_closed();
        }
    }
    if let Some(contexts) = plan.runtime_disabled_contexts() {
        for context in contexts {
            let runtime_disable_result = scheduler
                .disable_runtime_listener_for_target(receivers, context)
                .await;
            let runtime_disable_output = materialize_bidi_event_source_hook_output(
                scheduler,
                bidi,
                events,
                runtime_disable_result,
                Some(context),
            )?;
            if !runtime_disable_output.is_empty() {
                bidi.record_bidi_runtime_event_source_closed(context);
            }
        }
    }
    if let Some(contexts) = plan.network_contexts() {
        if contexts.is_empty() {
            let network_enable_result =
                enable_bidi_network_protocol_sources(scheduler, receivers).await;
            let network_enable_output = materialize_bidi_event_source_hook_output(
                scheduler,
                bidi,
                events,
                network_enable_result,
                None,
            )?;
            extend_bidi_events_from_protocol_output(
                Some(&*scheduler),
                bidi,
                events,
                network_enable_output,
                None,
            );
        } else {
            for context in contexts {
                if scheduler.enable_network_listener_for_target(context) {
                    bidi.record_bidi_network_event_source_opened(context);
                }
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
    pending_navigation_responses: &mut Vec<BidiPendingNavigationResponse>,
) -> bool {
    if output.is_empty() {
        return true;
    }
    let sources = BidiDevToolsEventSources::from_protocol_output(output).into_sources();
    let pending_response =
        take_pending_navigation_response_from_sources(pending_navigation_responses, &sources);
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
    if !send_bidi_json_events(socket, pending_response).await {
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
    stage: Option<BidiPendingCommandStage>,
    completion: BidiDevToolsCommandCompletion,
    input: Option<BidiPendingInput>,
}

enum BidiPendingCommandStage {
    WaitingForInputTurn,
    InputPause(Instant),
    NativeComplete(Box<crate::cdp_scheduler::AutomationExecution>),
    WaitingForNavigation {
        key: moli_protocol::DevToolsDocumentLifecycleWaitKey,
        execution: Box<crate::cdp_scheduler::AutomationExecution>,
    },
    WaitingForDocument(Box<PendingAutomationCommandAdmission>),
    Executing(Box<PendingAutomationExecution>),
}

struct BidiDevToolsCommandCompletion {
    id: u64,
    command_context: AutomationContext,
    navigation_wait: Option<DevToolsNavigationWait>,
    event_sources: BidiDevToolsEventSources,
    event_context: Option<String>,
}

#[derive(Debug, Clone)]
struct BidiPendingNavigationResponse {
    id: u64,
    url: String,
    channel: Option<String>,
    background_command_id: u64,
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
    ) -> Self {
        let (output, error) = failure.into_parts();
        event_sources.extend_protocol_output(output);
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
        event_context,
    }
}

impl BidiDevToolsEventSources {
    fn from_protocol_output(output: ProtocolOutputSequence) -> Self {
        let mut sources = Self::default();
        sources.extend_protocol_output(output);
        sources
    }

    fn extend_protocol_output(&mut self, output: ProtocolOutputSequence) {
        for event in output.into_background_events() {
            self.push_background_event(event);
        }
    }

    fn push_automation_event(&mut self, event: AutomationEvent) {
        self.sources
            .push(BidiDevToolsEventSource::AutomationEvent(Box::new(event)));
    }

    fn push_background_event(&mut self, event: BackgroundProtocolEvent) {
        if event.is_frame_lifecycle_projection() || event.is_javascript_dialog_opening_projection()
        {
            return;
        }
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
        let has_protocol_message = event.protocol_method().is_some();
        let (message, automation_event) = event.into_parts();
        match automation_event {
            Some(event) if !has_protocol_message => self.push_automation_event(event),
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

    fn into_sources(self) -> Vec<BidiDevToolsEventSource> {
        self.sources
    }
}

fn pending_navigation_response_for_dispatch(
    dispatch: &BidiDevToolsCommandDispatch,
    channel: Option<&str>,
) -> Option<BidiPendingNavigationResponse> {
    let AutomationCommand::Navigate(command) = &dispatch.command else {
        return None;
    };
    if command.wait == DevToolsNavigationWait::None {
        return None;
    }
    Some(BidiPendingNavigationResponse {
        id: dispatch.id,
        url: command.url.clone(),
        channel: channel.map(str::to_owned),
        background_command_id: dispatch.id,
    })
}

fn bidi_response_is_missing_automation_command_result(response: &serde_json::Value) -> bool {
    response["type"] == json!("error")
        && response["message"].as_str() == Some("MissingDevToolsCommandResult")
}

fn sources_include_auth_required_pause(sources: &[BidiDevToolsEventSource]) -> bool {
    sources.iter().any(|source| match source {
        BidiDevToolsEventSource::AutomationEvent(event) => {
            matches!(event.as_ref(), AutomationEvent::NetworkAuthRequired(_))
        }
        BidiDevToolsEventSource::CommandResponse { .. } => false,
        BidiDevToolsEventSource::ProtocolMessage(message) => {
            message["method"] == json!("Fetch.authRequired")
        }
        BidiDevToolsEventSource::ProtocolMessageWithAutomationEvent {
            message,
            automation_event,
        } => {
            matches!(
                automation_event.as_ref(),
                AutomationEvent::NetworkAuthRequired(_)
            ) || message["method"] == json!("Fetch.authRequired")
        }
    })
}

fn take_pending_navigation_response_from_sources(
    pending: &mut Vec<BidiPendingNavigationResponse>,
    sources: &[BidiDevToolsEventSource],
) -> Vec<serde_json::Value> {
    let mut responses = Vec::new();
    pending.retain(|pending_response| {
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
            });
        if let Some(response) = response {
            responses.push(response);
            false
        } else {
            true
        }
    });
    responses
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

fn subscribed_bidi_events_from_devtools_event_sources(
    scheduler: Option<&CdpScheduler>,
    bidi: &mut BidiConnectionState,
    sources: &[BidiDevToolsEventSource],
    owner_context: Option<&str>,
) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for source in sources {
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
    let sources = BidiDevToolsEventSources::from_protocol_output(output).into_sources();
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

async fn enable_bidi_runtime_protocol_sources(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
    scheduler
        .execute_internal_protocol_message(
            receivers,
            json!({
                "id": 0_u64,
                "method": "Runtime.enable",
                "params": {}
            }),
        )
        .await
}

async fn disable_bidi_runtime_protocol_sources(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
    scheduler
        .execute_internal_protocol_message(
            receivers,
            json!({
                "id": 0_u64,
                "method": "Runtime.disable",
                "params": {}
            }),
        )
        .await
}

async fn enable_bidi_network_protocol_sources(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
    scheduler
        .execute_internal_protocol_message(
            receivers,
            json!({
                "id": 0_u64,
                "method": "Network.enable",
                "params": {}
            }),
        )
        .await
}

async fn replay_existing_bidi_realm_created_events(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    event_sources: &mut BidiDevToolsEventSources,
    bidi: &mut BidiConnectionState,
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
        let events = replay_existing_bidi_realm_created_events_for_context(
            scheduler,
            receivers,
            event_sources,
            session_id,
            None,
        )
        .await;
        return bidi.subscribed_bidi_events_from_bidi_events_with_context(&events, None);
    } else {
        for context_id in contexts {
            let replayed = replay_existing_bidi_realm_created_events_for_context(
                scheduler,
                receivers,
                event_sources,
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
    receivers: &mut CdpSchedulerEventReceivers,
    event_sources: &mut BidiDevToolsEventSources,
    session_id: &str,
    context_id: Option<String>,
) -> Vec<serde_json::Value> {
    let command = AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: context_id.as_deref().map(DevToolsTargetId::from),
            browser_context_id: None,
        },
        realm_type: None,
    });
    let execution = scheduler
        .execute_automation_command_with_protocol_messages(receivers, command)
        .await;
    event_sources.extend_protocol_output(execution.protocol_output);
    let Ok(AutomationResult::Realms(result)) = execution.result else {
        return Vec::new();
    };
    result
        .realms
        .iter()
        .filter_map(script_realm_created_event)
        .collect()
}

async fn start_bidi_automation_command(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    runtime_response_ready_tx: &mpsc::UnboundedSender<AutomationWorkflowReady>,
    bidi: &BidiConnectionState,
    mut dispatch: BidiDevToolsCommandDispatch,
    background_command_id: Option<u64>,
) -> BidiDevToolsCommandStart {
    dispatch
        .command
        .set_webdriver_bidi_file_prompt_handler_for_script_command(
            bidi.file_prompt_handler_for_script_commands(),
        );
    let event_context = bidi_event_context_from_automation_command(&dispatch.command);
    // Consume ready facts here; Document readiness belongs to the pending
    // command and must not hold the socket's command-admission turn.
    let admission = drain_ready_bidi_background_navigation(scheduler, receivers).await;
    let mut event_sources = match admission {
        Ok(event_sources) => event_sources,
        Err(failure) => {
            let (event_sources, error) = failure.into_parts();
            return BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
                response: bidi_response_from_devtools_error(dispatch.id, error),
                event_sources: event_sources.into_sources(),
                post_response_event_sources: Vec::new(),
                event_context,
            });
        }
    };
    event_sources
        .extend_protocol_output(scheduler.complete_ready_protocol_residences_after_command());
    if let Some(error) = validate_bidi_top_level_context_command(
        scheduler,
        &mut event_sources,
        &dispatch.session_id,
        &dispatch.command,
    ) {
        return BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
            response: bidi_response_from_devtools_error(dispatch.id, error),
            event_sources: event_sources.into_sources(),
            post_response_event_sources: Vec::new(),
            event_context,
        });
    }
    let completion = BidiDevToolsCommandCompletion {
        id: dispatch.id,
        command_context: dispatch.command.context().clone(),
        navigation_wait: crate::cdp_scheduler::devtools_navigation_wait(&dispatch.command),
        event_sources,
        event_context,
    };
    let (stage, protocol_output) = match scheduler
        .start_owned_automation_command_progress(
            receivers,
            runtime_response_ready_tx,
            dispatch.command,
            background_command_id,
        )
        .await
    {
        Ok(AutomationCommandProgress::Complete(execution)) => (
            BidiPendingCommandStage::NativeComplete(execution),
            ProtocolOutputSequence::empty(),
        ),
        Ok(AutomationCommandProgress::Pending {
            pending,
            protocol_output,
        }) => (BidiPendingCommandStage::Executing(pending), protocol_output),
        Err(admission) => (
            BidiPendingCommandStage::WaitingForDocument(admission),
            ProtocolOutputSequence::empty(),
        ),
    };
    let mut completion = completion;
    completion
        .event_sources
        .extend_protocol_output(protocol_output);
    BidiDevToolsCommandStart::Pending(Box::new(BidiPendingCommand {
        command_method: None,
        command_params: None,
        command_channel: None,
        pending_navigation_candidate: None,
        stage: Some(stage),
        completion,
        input: None,
    }))
}

async fn complete_bidi_automation_command_execution(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    completion: &mut BidiDevToolsCommandCompletion,
    execution: crate::cdp_scheduler::AutomationExecution,
) -> BidiDevToolsCommandOutput {
    let id = completion.id;
    let mut event_sources = std::mem::take(&mut completion.event_sources);
    let event_context = completion.event_context.clone();
    event_sources.extend_protocol_output(execution.protocol_output);
    match drain_ready_bidi_background_navigation(scheduler, receivers).await {
        Ok(sources) => event_sources.append(sources),
        Err(failure) => {
            return bidi_command_output_from_renderer_transport_failure(
                id,
                event_context,
                event_sources,
                failure,
            );
        }
    }
    let response = match execution.result {
        Ok(result) => bidi_response_from_devtools_result(id, result),
        Err(error) => bidi_response_from_devtools_error(id, error),
    };
    let post_response_event_sources =
        match complete_bidi_post_response_protocol_residences(scheduler, receivers).await {
            Ok(sources) => sources,
            Err(failure) => {
                return bidi_command_output_from_renderer_transport_failure(
                    id,
                    event_context,
                    event_sources,
                    failure,
                );
            }
        };
    BidiDevToolsCommandOutput {
        response,
        event_sources: event_sources.into_sources(),
        post_response_event_sources: post_response_event_sources.into_sources(),
        event_context,
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
            if origin.contains_key("element")
                && origin.get("type").and_then(Value::as_str) != Some("element")
            {
                return Err(BidiInputPreparationError::Classic(ClassicError::new(
                    ClassicErrorCode::InvalidArgument,
                    "BiDi input element origin type must be element",
                )));
            }
            let element = origin
                .get("element")
                .and_then(Value::as_object)
                .unwrap_or(origin);
            let Some(shared_id) = element.get("sharedId") else {
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

fn bidi_resolve_subscribe_context(
    scheduler: &mut CdpScheduler,
    event_sources: &mut BidiDevToolsEventSources,
    session_id: &str,
    context_id: &str,
) -> Option<BidiValidatedSubscribeContext> {
    if let Some(owner) = scheduler.browsing_context_owner(context_id) {
        return Some(BidiValidatedSubscribeContext {
            context: owner.id,
            top_level_context: owner.page,
            user_context: owner.browser_context_id,
        });
    }
    // Worker contexts retain their existing Target-based validation.
    let info = bidi_target_info_for_target(scheduler, event_sources, session_id, context_id)?;
    Some(BidiValidatedSubscribeContext {
        context: context_id.to_owned(),
        top_level_context: context_id.to_owned(),
        user_context: info.browser_context_id.map(|id| id.into_string()),
    })
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
                Some(bidi_user_context_from_browser_context_id(
                    context.user_context.as_deref(),
                )),
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

fn validate_bidi_session_subscribe_targets(
    scheduler: &mut CdpScheduler,
    event_sources: &mut BidiDevToolsEventSources,
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
    let discover_contexts = contexts.is_empty();
    for context in contexts {
        let Some(resolved_context) =
            bidi_resolve_subscribe_context(scheduler, event_sources, session_id, &context)
        else {
            return Some(BidiSessionSubscribeTargetValidation::Error(error_response(
                id,
                BidiErrorCode::NoSuchFrame,
                "context not found",
            )));
        };
        validated.contexts.push(resolved_context);
    }
    if !user_contexts.is_empty() {
        let existing_user_contexts =
            bidi_existing_user_context_ids(scheduler, event_sources, session_id);
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
    }
    if discover_contexts {
        let contexts = bidi_existing_top_level_contexts(scheduler);
        validated
            .contexts
            .extend(contexts.into_iter().filter(|context| {
                validated.user_contexts.is_empty()
                    || validated.user_contexts.iter().any(|user_context| {
                        user_context
                            == bidi_user_context_from_browser_context_id(
                                context.user_context.as_deref(),
                            )
                    })
            }));
    }
    Some(BidiSessionSubscribeTargetValidation::Valid(validated))
}

fn bidi_existing_top_level_contexts(
    scheduler: &CdpScheduler,
) -> Vec<BidiValidatedSubscribeContext> {
    scheduler
        .frame_index_snapshot()
        .roots()
        .map(|record| BidiValidatedSubscribeContext {
            context: record.id.clone(),
            top_level_context: record.page.clone(),
            user_context: record.browser_context_id.clone(),
        })
        .collect()
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

fn bidi_existing_user_context_ids(
    scheduler: &mut CdpScheduler,
    event_sources: &mut BidiDevToolsEventSources,
    session_id: &str,
) -> BTreeSet<String> {
    let mut user_contexts = BTreeSet::from(["default".to_owned()]);
    let command = AutomationCommand::GetBrowserContexts(DevToolsGetBrowserContextsCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: None,
            browser_context_id: None,
        },
    });
    let execution = scheduler.execute_immediate_automation_command(command);
    event_sources.extend_protocol_output(execution.protocol_output);
    let result = execution.result;
    if let Ok(AutomationResult::GetBrowserContexts(result)) = result {
        user_contexts.extend(
            result
                .browser_context_ids
                .into_iter()
                .map(|browser_context_id| browser_context_id.into_string()),
        );
    }
    user_contexts
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
        | ClassicErrorCode::ElementClickIntercepted
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

fn validate_bidi_top_level_context_command(
    scheduler: &mut CdpScheduler,
    event_sources: &mut BidiDevToolsEventSources,
    session_id: &str,
    command: &AutomationCommand,
) -> Option<DevToolsError> {
    let target_id = match command {
        AutomationCommand::CreateTarget(command) => command.context.target_id.as_ref()?,
        AutomationCommand::CloseTarget(command) => &command.target_id,
        AutomationCommand::ActivateTarget(command) => &command.target_id,
        AutomationCommand::TraverseHistory(command) => command.context.target_id.as_ref()?,
        _ => return None,
    };
    let target_info =
        bidi_target_info_for_target(scheduler, event_sources, session_id, target_id.as_str());
    match target_info {
        Some(info) if matches!(info.kind, DevToolsTargetKind::Frame) => Some(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "ChildFrameContextNotSupported",
        )),
        Some(_) => None,
        None if scheduler
            .browsing_context_owner(target_id.as_str())
            .is_some() =>
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

fn bidi_target_info_for_target(
    scheduler: &mut CdpScheduler,
    event_sources: &mut BidiDevToolsEventSources,
    session_id: &str,
    target_id: &str,
) -> Option<DevToolsTargetInfo> {
    let command = AutomationCommand::GetTargetInfo(DevToolsGetTargetInfoCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: Some(DevToolsTargetId::from(target_id)),
            browser_context_id: None,
        },
        target_id: Some(DevToolsTargetId::from(target_id)),
    });
    let execution = scheduler.execute_immediate_automation_command(command);
    event_sources.extend_protocol_output(execution.protocol_output);
    let Ok(AutomationResult::GetTargetInfo(result)) = execution.result else {
        return None;
    };
    Some(result.target_info)
}

fn bidi_event_context_from_automation_command(command: &AutomationCommand) -> Option<String> {
    match command {
        AutomationCommand::Navigate(command) => command.context.target_id.as_ref(),
        AutomationCommand::Reload(command) => command.context.target_id.as_ref(),
        AutomationCommand::EvaluateScript(command) => command.context.target_id.as_ref(),
        AutomationCommand::CallFunction(command) => command.context.target_id.as_ref(),
        AutomationCommand::GetRealms(command) => command.context.target_id.as_ref(),
        _ => None,
    }
    .map(|target_id| target_id.as_str().to_owned())
}

async fn complete_bidi_post_response_protocol_residences(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
) -> Result<BidiDevToolsEventSources, BidiRendererOutputTransportFailure> {
    let mut event_sources = BidiDevToolsEventSources::from_protocol_output(
        scheduler.complete_ready_protocol_residences_after_command(),
    );
    event_sources.append(drain_ready_bidi_background_navigation(scheduler, receivers).await?);
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
    );
    while let Ok(completion) = receivers.background_navigation_completion_rx.try_recv() {
        match scheduler
            .drain_background_navigation_completion_with_progress_barrier(completion, receivers)
            .await
        {
            Ok(output) => event_sources.extend_protocol_output(output),
            Err(failure) => {
                return Err(BidiRendererOutputTransportFailure::from_renderer(
                    event_sources,
                    failure,
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
    use moli_protocol::automation::{PageJavaScriptDialogOpeningEvent, RuntimeConsoleEvent};
    use serde_json::json;

    use super::*;

    fn bidi_with_prompt_subscription() -> (BidiConnectionState, BidiSessionRegistry) {
        let mut bidi = BidiConnectionState::new();
        let mut registry = BidiSessionRegistry::new();
        for (id, method, params) in [
            (1, "session.new", json!({})),
            (
                2,
                "session.subscribe",
                json!({"events": ["browsingContext.userPromptOpened"]}),
            ),
        ] {
            let outcome = bidi.handle_message_with_session_registry(
                json!({"id": id, "method": method, "params": params}),
                &mut registry,
            );
            assert_eq!(outcome.response["type"], "success", "{method}");
        }
        (bidi, registry)
    }

    #[test]
    fn protocol_output_hook_delivers_one_native_dialog_opening_with_two_cdp_projections() {
        let (mut bidi, _registry) = bidi_with_prompt_subscription();
        let opening = PageJavaScriptDialogOpeningEvent {
            frame_id: Some("TID-prompt".into()),
            url: "https://example.test/".to_owned(),
            message: "one physical prompt".to_owned(),
            dialog_type: "prompt".to_owned(),
            has_browser_handler: false,
            default_prompt: "default prompt".to_owned(),
        };
        let mut sources = BidiDevToolsEventSources::default();
        sources.push_automation_event(AutomationEvent::PageJavaScriptDialogOpening(
            opening.clone(),
        ));
        for session in ["SID-first", "SID-peer"] {
            sources.push_background_event(BackgroundProtocolEvent::page_javascript_dialog_opening(
                Some(session),
                opening.clone(),
            ));
        }
        let events = subscribed_bidi_events_from_devtools_event_sources(
            None,
            &mut bidi,
            &sources.into_sources(),
            None,
        );
        assert_eq!(
            events,
            vec![json!({
                "type": "event",
                "method": "browsingContext.userPromptOpened",
                "params": {
                    "context": "TID-prompt", "type": "prompt", "message": "one physical prompt",
                    "handler": "dismiss", "defaultValue": "default prompt",
                }
            })]
        );
    }

    #[test]
    fn protocol_output_hook_preserves_raw_dialog_opening_bridge() {
        let (mut bidi, _registry) = bidi_with_prompt_subscription();
        let output = ProtocolOutputSequence::from_background_event(
            BackgroundProtocolEvent::immediate(json!({
                "sessionId": "SID-raw",
                "method": "Page.javascriptDialogOpening",
                "params": {
                    "frameId": "TID-raw", "type": "alert", "message": "raw CDP input",
                }
            })),
        );
        let mut events = Vec::new();
        extend_bidi_events_from_protocol_output(None, &mut bidi, &mut events, output, None);
        assert_eq!(
            events,
            vec![json!({
                "type": "event",
                "method": "browsingContext.userPromptOpened",
                "params": {"context": "TID-raw", "type": "alert", "message": "raw CDP input", "handler": "dismiss"}
            })]
        );
    }

    #[test]
    fn bidi_dialog_handler_follows_successful_session_creation_and_end() {
        let (mut scheduler, _) = CdpScheduler::new(moli_protocol::CdpConnection::new());
        let mut bidi = BidiConnectionState::new();
        let mut registry = BidiSessionRegistry::new();
        for (id, method, params, expected_available) in [
            (1, "session.status", json!({}), false),
            (2, "session.new", json!({"capabilities": false}), false),
            (3, "session.new", json!({}), true),
            (4, "session.new", json!({}), true),
            (5, "session.end", json!({}), false),
            (6, "session.end", json!({}), false),
        ] {
            let outcome = bidi.handle_message_with_session_registry(
                json!({"id": id, "method": method, "params": params}),
                &mut registry,
            );
            let _ =
                apply_bidi_dialog_handler_lifecycle(&mut scheduler, &bidi, Some(method), &outcome);
            assert_eq!(
                scheduler.has_automation_javascript_dialog_handler(),
                expected_available,
                "{method}: {:?}",
                outcome.response
            );
        }
    }

    #[test]
    fn bidi_session_end_keeps_an_independent_classic_dialog_handler() {
        let (mut scheduler, _) = CdpScheduler::new(moli_protocol::CdpConnection::new());
        assert!(scheduler.register_automation_javascript_dialog_handler(
            FrontendProtocol::WebDriverClassic,
            true
        ));
        let mut bidi = BidiConnectionState::new();
        let mut registry = BidiSessionRegistry::new();
        for (id, method) in [(1, "session.new"), (2, "session.end")] {
            let outcome = bidi.handle_message_with_session_registry(
                json!({"id": id, "method": method, "params": {}}),
                &mut registry,
            );
            assert_eq!(outcome.response["type"], "success");
            let _ =
                apply_bidi_dialog_handler_lifecycle(&mut scheduler, &bidi, Some(method), &outcome);
            assert!(scheduler.has_automation_javascript_dialog_handler());
        }
        let _ = scheduler
            .unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverClassic);
        assert!(!scheduler.has_automation_javascript_dialog_handler());
    }

    #[test]
    fn webdriver_classic_click_errors_use_existing_bidi_input_error_mapping() {
        for (classic, expected) in [
            (
                ClassicErrorCode::ElementClickIntercepted,
                "invalid argument",
            ),
            (ClassicErrorCode::ElementNotInteractable, "invalid argument"),
            (ClassicErrorCode::InvalidArgument, "invalid argument"),
            (ClassicErrorCode::NoSuchElement, "no such node"),
            (ClassicErrorCode::UnknownError, "unknown error"),
        ] {
            let response = bidi_response_from_classic_input_error(
                42,
                ClassicError::new(classic, "input diagnostic"),
            );
            assert_eq!(response["type"], json!("error"));
            assert_eq!(response["id"], json!(42));
            assert_eq!(response["error"], json!(expected));
            assert_eq!(response["message"], json!("input diagnostic"));
        }
    }

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

    #[test]
    fn pending_navigation_response_consumes_typed_command_response() {
        let mut pending = vec![BidiPendingNavigationResponse {
            id: 5,
            url: "https://example.test/auth".to_owned(),
            channel: Some("chan".to_owned()),
            background_command_id: 42,
        }];
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

        let response = take_pending_navigation_response_from_sources(&mut pending, &sources)
            .pop()
            .expect("typed command response should complete pending navigation");

        assert!(pending.is_empty());
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

    #[test]
    fn pending_navigation_response_falls_back_to_raw_protocol_error() {
        let mut pending = vec![BidiPendingNavigationResponse {
            id: 7,
            url: "https://example.test/error".to_owned(),
            channel: Some("chan".to_owned()),
            background_command_id: 42,
        }];
        let sources = vec![BidiDevToolsEventSource::ProtocolMessage(json!({
            "id": 42_u64,
            "error": {
                "code": -32000,
                "message": "navigation failed before typed response"
            }
        }))];

        let response = take_pending_navigation_response_from_sources(&mut pending, &sources)
            .pop()
            .expect("raw protocol error should complete pending navigation as a fallback");

        assert!(pending.is_empty());
        assert_eq!(response["type"], json!("error"));
        assert_eq!(response["id"], json!(7_u64));
        assert_eq!(response["goog:channel"], json!("chan"));
        assert_eq!(response["error"], json!("unsupported operation"));
        assert_eq!(
            response["message"],
            json!("navigation failed before typed response")
        );
    }
    #[test]
    fn pending_navigation_responses_keep_independent_command_owners() {
        let mut pending = [41, 42, 43]
            .map(|id| BidiPendingNavigationResponse {
                id,
                background_command_id: id + 100,
                url: format!("https://example.test/{id}"),
                channel: Some(format!("channel-{id}")),
            })
            .to_vec();
        let sources = [42, 41].map(|id| BidiDevToolsEventSource::CommandResponse {
            command_id: Some(id + 100),
            response: BackgroundCommandResponsePayload::Success {
                result: json!({"navigation":format!("navigation-{id}")}),
            },
        });
        let responses = take_pending_navigation_response_from_sources(&mut pending, &sources);
        assert_eq!(responses.len(), 2);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, 43);
        for response in responses {
            let id = response["id"].as_u64().unwrap();
            assert_eq!(response["goog:channel"], format!("channel-{id}"));
            assert_eq!(
                response["result"]["url"],
                format!("https://example.test/{id}")
            );
            assert_eq!(response["result"]["navigation"], format!("navigation-{id}"));
        }
        assert!(
            take_pending_navigation_response_from_sources(&mut pending, &sources).is_empty(),
            "duplicate notification has no completion owner"
        );
    }
}
