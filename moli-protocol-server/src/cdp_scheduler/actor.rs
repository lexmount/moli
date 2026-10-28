use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::Instant,
};

use anyhow::{Context as _, Result};
use moli_core::{RendererOutputFence, RendererOutputTransportMessage};
use moli_protocol::{
    BackgroundNavigationCompletion, BackgroundProtocolEvent, CdpSchedulerEvent,
    CommandDispatchContext, CommandWaitLifetime, CompletedCdpCommandDispatch,
    CompletedPageScreencastCapture, ParsedCdpCommand, PendingCdpCommandDispatch,
    conn::{RuntimeInspectorAsyncCompletionReceiver, RuntimeInspectorResponseReady},
};
use serde_json::Value;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;

use crate::{
    cdp_frontend::{
        CdpControlResource, CdpControlResourceLease, CdpCreatedTarget, CdpFrontendCommand,
        CdpFrontendControlRequest, CdpFrontendReceivers, CdpFrontendRegistration,
        CdpInternalCommandResult,
    },
    cdp_frontend_router::{CdpFrontendRouter, CdpPreparedFrontendCommand},
};

use super::command_task::{
    CommandWaitCompletion, CommandWaitFailure, CommandWaitIdentity, CommandWaitTask,
    spawn_command_wait, spawn_owned_wait,
};
use super::frontend_control::CdpFrontendControlState;
use super::{
    CdpBackgroundEventReceiver, CdpBackgroundNavigationCompletionReceiver, CdpCookieSnapshot,
    CdpOwnerActorLifecycle, CdpRendererPublicationReceiver, CdpScheduler,
    CdpSchedulerEventReceivers, CommandDispatchState, CommandOutputReleasePermit,
    CommandStartAction, CommandTaskStep, CommandTurnOutput, ProtocolAdapterScheduler,
    ProtocolAdapterSchedulerAdvance, ProtocolAdapterSchedulerInput, ProtocolOutputSequence,
};

struct PendingRuntimeDeferredReplyState {
    pending: PendingCdpCommandDispatch,
    dispatch: CommandDispatchState,
    metadata: InFlightCommandMetadata,
    output_release_permit: CommandOutputReleasePermit,
    command_context: CommandDispatchContext,
    output_session_id: Option<String>,
    response_task: Option<CommandWaitTask>,
}

#[derive(Clone)]
struct InFlightCommandMetadata {
    method: Option<String>,
    id: Option<u64>,
    session_id: Option<String>,
    started: Option<Instant>,
    executes_page_javascript: bool,
    command_output_session_id: Option<String>,
    internal_response: Option<Rc<RefCell<Option<InternalCommandResponse>>>>,
}

struct InternalCommandResponse {
    sender: oneshot::Sender<Result<CdpInternalCommandResult>>,
    registration: Option<CdpFrontendRegistration>,
    owner: mpsc::WeakUnboundedSender<CdpFrontendControlRequest>,
}

struct InFlightCommandState {
    metadata: InFlightCommandMetadata,
    dispatch: CommandDispatchState,
    output_release_permit: CommandOutputReleasePermit,
    command_context: CommandDispatchContext,
    pending_turn: u64,
    owner_session_id: Option<String>,
    wait_lifetime: Option<CommandWaitLifetime>,
    wait_task: CommandWaitTask,
}

enum CompletedActorWork {
    Command(CompletedCdpCommandDispatch),
    FrontendControl,
}

type PendingCommandCompletion = CommandWaitCompletion<CompletedActorWork>;

struct BlockedCommandDispatch {
    command: ParsedCdpCommand,
    metadata: InFlightCommandMetadata,
}

enum InFlightWork {
    Command(Box<InFlightCommandState>),
    FrontendControl(CommandWaitTask),
}

impl InFlightWork {
    fn command(&self) -> Option<&InFlightCommandState> {
        match self {
            Self::Command(state) => Some(state),
            Self::FrontendControl(_) => None,
        }
    }

    fn turn(&self) -> u64 {
        match self {
            Self::Command(state) => state.pending_turn,
            Self::FrontendControl(_) => 0,
        }
    }

    fn into_wait_task(self) -> CommandWaitTask {
        match self {
            Self::Command(state) => state.wait_task,
            Self::FrontendControl(task) => task,
        }
    }
}

type InFlightCommands = HashMap<u64, InFlightWork>;

fn page_javascript_owner_is_blocked(
    scheduler: &CdpScheduler,
    in_flight_commands: &InFlightCommands,
) -> bool {
    scheduler.has_pending_javascript_dialog()
        || in_flight_commands
            .values()
            .filter_map(InFlightWork::command)
            .any(|state| state.metadata.executes_page_javascript)
}

enum SchedulerInput {
    ProtocolWorkCompletion(super::protocol_task::ProtocolWorkCompletion),
    ProtocolStateRequest(moli_protocol::conn::ProtocolStateRequest),
    DocumentActivityCompletion(moli_protocol::conn::CompletedDocumentActivityUpdate),
    BackgroundNavigationCompletion(BackgroundNavigationCompletion),
    BackgroundEvent(BackgroundProtocolEvent),
    RendererPublication(RendererOutputTransportMessage),
    DeferredRuntimeInspectorResponse(Box<RuntimeInspectorResponseReady>),
    AdapterScheduler(ProtocolAdapterSchedulerInput),
}

struct SchedulerInputReceivers {
    protocol_state_request_rx: moli_protocol::conn::ProtocolStateRequestReceiver,
    protocol_work_completion_rx:
        mpsc::UnboundedReceiver<super::protocol_task::ProtocolWorkCompletion>,
    document_activity_completion_rx:
        mpsc::UnboundedReceiver<moli_protocol::conn::CompletedDocumentActivityUpdate>,
    background_event_rx: CdpBackgroundEventReceiver,
    background_navigation_completion_rx: CdpBackgroundNavigationCompletionReceiver,
    renderer_publication_rx: CdpRendererPublicationReceiver,
    buffered_renderer_publications: VecDeque<RendererOutputTransportMessage>,
    ready_background_inputs_before_runtime_response: VecDeque<SchedulerInput>,
}

impl SchedulerInputReceivers {
    fn new(receivers: CdpSchedulerEventReceivers) -> Self {
        Self {
            protocol_state_request_rx: receivers.protocol_state_request_rx,
            protocol_work_completion_rx: receivers.protocol_work_completion_rx,
            document_activity_completion_rx: receivers.document_activity_completion_rx,
            background_event_rx: receivers.background_event_rx,
            background_navigation_completion_rx: receivers.background_navigation_completion_rx,
            renderer_publication_rx: receivers.renderer_publication_rx,
            buffered_renderer_publications: VecDeque::new(),
            ready_background_inputs_before_runtime_response: VecDeque::new(),
        }
    }

    fn queue_ready_background_inputs_before_runtime_response(
        &mut self,
        response: RuntimeInspectorResponseReady,
    ) {
        // Chromium delivers protocol events produced while running JavaScript
        // before the Runtime command response. Snapshot the already-ready
        // protocol events from their separate channels, then close the finite
        // batch with the response so a busy producer cannot starve it.
        //
        // Renderer publications are deliberately not copied into this queue.
        // A correlated response carries an exact concrete output cursor; the
        // cursor fence below consumes only concrete stream traffic until that
        // position is admitted.
        while let Ok(completion) = self.document_activity_completion_rx.try_recv() {
            self.ready_background_inputs_before_runtime_response
                .push_back(SchedulerInput::DocumentActivityCompletion(completion));
        }
        while let Ok(completion) = self.background_navigation_completion_rx.try_recv() {
            self.ready_background_inputs_before_runtime_response
                .push_back(SchedulerInput::BackgroundNavigationCompletion(completion));
        }
        while let Ok(event) = self.background_event_rx.try_recv() {
            self.ready_background_inputs_before_runtime_response
                .push_back(SchedulerInput::BackgroundEvent(event));
        }
        self.ready_background_inputs_before_runtime_response
            .push_back(SchedulerInput::DeferredRuntimeInspectorResponse(Box::new(
                response,
            )));
    }
}

impl SchedulerInputReceivers {
    fn take_buffered_renderer_publication(&mut self) -> Option<RendererOutputTransportMessage> {
        self.buffered_renderer_publications.pop_front()
    }

    async fn recv(
        &mut self,
        deferred_runtime_response_rx: &mut mpsc::UnboundedReceiver<RuntimeInspectorResponseReady>,
        has_pending_runtime_deferred_reply: bool,
        adapter_scheduler: &mut ProtocolAdapterScheduler,
        page_javascript_blocked: bool,
    ) -> Option<SchedulerInput> {
        if let Some(input) = self
            .ready_background_inputs_before_runtime_response
            .pop_front()
        {
            return Some(input);
        }
        if let Some(publication) = self.take_buffered_renderer_publication() {
            return Some(SchedulerInput::RendererPublication(publication));
        }
        if has_pending_runtime_deferred_reply {
            tokio::select! {
                biased;
                completion = self.protocol_work_completion_rx.recv(), if !self.protocol_work_completion_rx.is_closed() => {
                    completion.map(SchedulerInput::ProtocolWorkCompletion)
                }
                maybe_request = self.protocol_state_request_rx.recv(), if !self.protocol_state_request_rx.is_closed() => {
                    maybe_request.map(SchedulerInput::ProtocolStateRequest)
                }
                maybe_response = deferred_runtime_response_rx.recv() => {
                    let response = maybe_response?;
                    self.queue_ready_background_inputs_before_runtime_response(response);
                    self.ready_background_inputs_before_runtime_response.pop_front()
                }
                maybe_completion = self.document_activity_completion_rx.recv() => {
                    maybe_completion.map(SchedulerInput::DocumentActivityCompletion)
                }
                maybe_completion = self.background_navigation_completion_rx.recv() => {
                    maybe_completion.map(SchedulerInput::BackgroundNavigationCompletion)
                }
                maybe_event = self.background_event_rx.recv() => {
                    maybe_event.map(SchedulerInput::BackgroundEvent)
                }
                maybe_publication = self.renderer_publication_rx.recv() => {
                    maybe_publication.map(SchedulerInput::RendererPublication)
                }
                input = adapter_scheduler.recv_input(), if !page_javascript_blocked => {
                    Some(SchedulerInput::AdapterScheduler(input))
                }
            }
        } else {
            tokio::select! {
                biased;
                completion = self.protocol_work_completion_rx.recv(), if !self.protocol_work_completion_rx.is_closed() => {
                    completion.map(SchedulerInput::ProtocolWorkCompletion)
                }
                maybe_request = self.protocol_state_request_rx.recv(), if !self.protocol_state_request_rx.is_closed() => {
                    maybe_request.map(SchedulerInput::ProtocolStateRequest)
                }
                maybe_completion = self.document_activity_completion_rx.recv() => {
                    maybe_completion.map(SchedulerInput::DocumentActivityCompletion)
                }
                maybe_completion = self.background_navigation_completion_rx.recv() => {
                    maybe_completion.map(SchedulerInput::BackgroundNavigationCompletion)
                }
                maybe_event = self.background_event_rx.recv() => {
                    maybe_event.map(SchedulerInput::BackgroundEvent)
                }
                maybe_publication = self.renderer_publication_rx.recv() => {
                    maybe_publication.map(SchedulerInput::RendererPublication)
                }
                maybe_response = deferred_runtime_response_rx.recv() => {
                    maybe_response.map(|response| SchedulerInput::DeferredRuntimeInspectorResponse(Box::new(response)))
                }
                input = adapter_scheduler.recv_input(), if !page_javascript_blocked => {
                    Some(SchedulerInput::AdapterScheduler(input))
                }
            }
        }
    }
}

pub(crate) fn spawn_cdp_scheduler_actor(
    scheduler: CdpScheduler,
    receivers: CdpSchedulerEventReceivers,
    frontend_router: CdpFrontendRouter,
    frontend_receivers: CdpFrontendReceivers,
    owner_lifecycle: Option<CdpOwnerActorLifecycle>,
) -> JoinHandle<CdpCookieSnapshot> {
    tokio::task::spawn_local(run_cdp_scheduler_actor(
        scheduler,
        receivers,
        frontend_router,
        frontend_receivers,
        owner_lifecycle,
    ))
}

async fn run_cdp_scheduler_actor(
    mut scheduler: CdpScheduler,
    receivers: CdpSchedulerEventReceivers,
    frontend_router: CdpFrontendRouter,
    mut frontend_receivers: CdpFrontendReceivers,
    owner_lifecycle: Option<CdpOwnerActorLifecycle>,
) -> CdpCookieSnapshot {
    let mut scheduler_input_rx = SchedulerInputReceivers::new(receivers);
    let (deferred_runtime_response_tx, mut deferred_runtime_response_rx) =
        mpsc::unbounded_channel();
    scheduler
        .conn
        .set_runtime_inspector_response_ready_sender(deferred_runtime_response_tx.clone());
    let mut adapter_scheduler = ProtocolAdapterScheduler::default();
    let mut pending_runtime_deferred_replies: VecDeque<PendingRuntimeDeferredReplyState> =
        VecDeque::new();
    let (pending_command_completion_tx, mut pending_command_completion_rx) =
        mpsc::unbounded_channel();
    let (page_screencast_completion_tx, mut page_screencast_completion_rx) =
        mpsc::unbounded_channel::<CompletedPageScreencastCapture>();
    let mut in_flight_commands = InFlightCommands::new();
    let mut blocked_commands = VecDeque::new();
    let mut next_in_flight_command_token = 0_u64;
    let frontend_control = CdpFrontendControlState::new(frontend_receivers.control_tx.clone());

    loop {
        if scheduler_input_rx.renderer_publication_rx.is_closed() {
            break;
        }
        for state in in_flight_commands
            .values()
            .filter_map(InFlightWork::command)
        {
            if state
                .owner_session_id
                .as_deref()
                .is_some_and(|session_id| !scheduler.conn.has_attached_devtools_session(session_id))
                || state
                    .wait_lifetime
                    .as_ref()
                    .is_some_and(|lifetime| !lifetime.is_active())
            {
                state.wait_task.abort();
            }
        }
        for pending in &pending_runtime_deferred_replies {
            if pending
                .output_session_id
                .as_deref()
                .is_some_and(|session_id| !scheduler.conn.has_attached_devtools_session(session_id))
                && let Some(task) = &pending.response_task
            {
                task.abort();
            }
        }
        let page_javascript_blocked =
            page_javascript_owner_is_blocked(&scheduler, &in_flight_commands);
        adapter_scheduler.schedule_turn_if_needed(&scheduler, page_javascript_blocked);
        let page_screencast_deadline = scheduler.next_page_screencast_deadline();
        tokio::select! {
            biased;
            maybe_completion = pending_command_completion_rx.recv(), if !in_flight_commands.is_empty() => {
                let Some(completion) = maybe_completion else {
                    break;
                };
                if !handle_pending_command_completion(
                    &frontend_router,
                    &mut scheduler,
                    &mut scheduler_input_rx,
                    &mut pending_runtime_deferred_replies,
                    &deferred_runtime_response_tx,
                    &mut adapter_scheduler,
                    &pending_command_completion_tx,
                    &mut in_flight_commands,
                    completion,
                )
                .await
                {
                    break;
                }
                if !drain_blocked_commands_after_navigation_gate(
                    &frontend_router,
                    &mut scheduler,
                    &mut scheduler_input_rx,
                    &mut pending_runtime_deferred_replies,
                    &deferred_runtime_response_tx,
                    &mut adapter_scheduler,
                    &pending_command_completion_tx,
                    &mut in_flight_commands,
                    &mut blocked_commands,
                    &mut next_in_flight_command_token,
                )
                .await
                {
                    break;
                }
            }
            maybe_completion = page_screencast_completion_rx.recv() => {
                let Some(completion) = maybe_completion else {
                    break;
                };
                if let Some(frame) = scheduler.complete_page_screencast_capture(
                    completion,
                    TokioInstant::now(),
                ) {
                    let super::ScheduledPageScreencastFrame {
                        event,
                        session_id,
                        generation,
                        visual_state,
                    } = frame;
                    let output = scheduler.route_background_event_around_inflight_navigation(event);
                    if !flush_protocol_output_with_runtime_deferred_reply_routing(
                        &frontend_router,
                        &mut scheduler,
                        &mut pending_runtime_deferred_replies,
                        output,
                    )
                    .await
                    {
                        break;
                    }
                    scheduler.note_page_screencast_frame_emitted(
                        &session_id,
                        generation,
                        visual_state,
                        TokioInstant::now(),
                    );
                }
            }
            maybe_request = frontend_receivers.control_rx.recv() => {
                let Some(request) = maybe_request else {
                    break;
                };
                match request {
                    CdpFrontendControlRequest::Shutdown => break,
                    CdpFrontendControlRequest::TargetDestroyed { target_id } => {
                        frontend_router.unregister_frontends_for_target(&target_id);
                    }
                    CdpFrontendControlRequest::EnsureDefaultTargetPublished { completion_tx } => {
                        scheduler.conn.publish_default_browser_target();
                        let _ = completion_tx.send(Ok(()));
                    }
                    CdpFrontendControlRequest::Checkpoint => {
                        super::frontend_control::send_cookie_checkpoint(&mut scheduler, owner_lifecycle.as_ref());
                    }
                    CdpFrontendControlRequest::ExecuteInternalCommand { command, registration, completion_tx } => {
                        let command = *command;
                        let mut metadata = command_metadata(&command);
                        metadata.internal_response = Some(Rc::new(RefCell::new(Some(InternalCommandResponse { sender: completion_tx, registration, owner: frontend_receivers.control_tx.clone() }))));
                        if !handle_client_command_with_interleaved_output(
                            &frontend_router, &mut scheduler, &mut scheduler_input_rx, command, metadata,
                            &mut pending_runtime_deferred_replies, &deferred_runtime_response_tx,
                            &mut adapter_scheduler, &pending_command_completion_tx, &mut in_flight_commands,
                            &mut blocked_commands, &mut next_in_flight_command_token,
                        ).await { break; }
                    }
                    request => {
                        let token = take_next_in_flight_command_token(&mut next_in_flight_command_token);
                        let control = frontend_control.clone();
                        let router = frontend_router.clone();
                        let wait = async move {
                            control.handle_request(request, router).await;
                            CompletedActorWork::FrontendControl
                        };
                        let task = spawn_command_wait(CommandWaitIdentity { token, turn: 0 }, wait,
                            &pending_command_completion_tx);
                        in_flight_commands.insert(token, InFlightWork::FrontendControl(task));
                    }
                }
            }
            maybe_command = frontend_receivers.command_rx.recv() => {
                let Some(command) = maybe_command else {
                    break;
                };
                if !handle_frontend_command(
                    command,
                    &frontend_router,
                    &mut scheduler,
                    &mut scheduler_input_rx,
                    &mut pending_runtime_deferred_replies,
                    &deferred_runtime_response_tx,
                    &mut adapter_scheduler,
                    &pending_command_completion_tx,
                    &mut in_flight_commands,
                    &mut blocked_commands,
                    &mut next_in_flight_command_token,
                    owner_lifecycle.as_ref(),
                )
                .await
                {
                    break;
                }
            }
            _ = wait_for_page_screencast_deadline(page_screencast_deadline), if page_screencast_deadline.is_some() => {
                for capture in scheduler
                    .start_due_page_screencast_captures(TokioInstant::now())
                {
                    let completion_tx = page_screencast_completion_tx.clone();
                    tokio::task::spawn_local(async move {
                        let _ = completion_tx.send(capture.wait().await);
                    });
                }
            }
            maybe_input = scheduler_input_rx.recv(
                &mut deferred_runtime_response_rx,
                !pending_runtime_deferred_replies.is_empty(),
                &mut adapter_scheduler,
                page_javascript_blocked,
            ) => {
                let Some(input) = maybe_input else {
                    break;
                };
                trace_scheduler_input(&input, "scheduler_input_received");
                if !handle_scheduler_input(
                    &frontend_router,
                    &mut scheduler,
                    &mut scheduler_input_rx,
                    &mut pending_runtime_deferred_replies,
                    &deferred_runtime_response_tx,
                    &mut adapter_scheduler,
                    &pending_command_completion_tx,
                    &mut in_flight_commands,
                    &mut blocked_commands,
                    &mut next_in_flight_command_token,
                    input,
                )
                .await
                {
                    break;
                }
            }
        }
    }
    // Revoke owned waits before releasing the connection. Their cancellation
    // completions can no longer consume an in-flight entry or publish a reply.
    let mut waits = in_flight_commands
        .drain()
        .map(|(_, state)| state.into_wait_task())
        .collect::<Vec<_>>();
    waits.extend(
        pending_runtime_deferred_replies
            .drain(..)
            .filter_map(|pending| pending.response_task),
    );
    waits.extend(scheduler.conn.take_background_navigation_tasks());
    waits.extend(scheduler.take_protocol_work_tasks());
    for task in &waits {
        task.abort();
    }
    for task in waits {
        task.cancel_and_join().await;
    }
    CdpCookieSnapshot::from_profile_backed_cookies(scheduler.snapshot_profile_backed_cookies())
}

async fn wait_for_page_screencast_deadline(deadline: Option<TokioInstant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn handle_scheduler_input(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    blocked_commands: &mut VecDeque<BlockedCommandDispatch>,
    next_in_flight_command_token: &mut u64,
    input: SchedulerInput,
) -> bool {
    let input_kind = scheduler_input_kind(&input);
    let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
    if trace_started.is_some() {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "scheduler_input_start",
            input = input_kind,
        );
    }
    let ok = match input {
        SchedulerInput::ProtocolWorkCompletion(completion) => {
            flush_owned_protocol_work(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                completion,
            )
            .await
        }
        SchedulerInput::ProtocolStateRequest(request) => {
            scheduler.apply_protocol_state_request(request);
            true
        }
        SchedulerInput::DocumentActivityCompletion(completion) => {
            let outcome = scheduler.conn.complete_document_activity_update(completion);
            let (output, post_output, boundary, predecessor) =
                scheduler.materialize_renderer_owner_turn_outcome(outcome);
            assert!(boundary.is_none() && post_output.is_empty());
            if !flush_renderer_publication_predecessor(
                frontend_router,
                scheduler,
                scheduler_input_rx,
                pending_runtime_deferred_replies,
                adapter_scheduler,
                predecessor.as_ref(),
            )
            .await
            {
                return false;
            }
            flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await
        }
        SchedulerInput::BackgroundNavigationCompletion(_) => {
            if !flush_background_completion_input(
                frontend_router,
                scheduler,
                scheduler_input_rx,
                pending_runtime_deferred_replies,
                adapter_scheduler,
                input,
            )
            .await
            {
                return false;
            }
            drain_blocked_commands_after_navigation_gate(
                frontend_router,
                scheduler,
                scheduler_input_rx,
                pending_runtime_deferred_replies,
                deferred_runtime_response_tx,
                adapter_scheduler,
                pending_command_completion_tx,
                in_flight_commands,
                blocked_commands,
                next_in_flight_command_token,
            )
            .await
        }
        SchedulerInput::BackgroundEvent(event) => {
            let output = scheduler.route_background_event_around_inflight_navigation(event);
            flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await
        }
        SchedulerInput::DeferredRuntimeInspectorResponse(response) => {
            let renderer_output_predecessor = response.renderer_output_predecessor();
            if !flush_renderer_publication_predecessor(
                frontend_router,
                scheduler,
                scheduler_input_rx,
                pending_runtime_deferred_replies,
                adapter_scheduler,
                renderer_output_predecessor.as_ref(),
            )
            .await
            {
                return false;
            }
            handle_deferred_runtime_inspector_response_result(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                deferred_runtime_response_tx,
                *response,
            )
            .await
        }
        SchedulerInput::RendererPublication(publication) => {
            ingest_and_flush_renderer_publication(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                adapter_scheduler,
                publication,
            )
            .await
        }
        SchedulerInput::AdapterScheduler(input) => {
            handle_adapter_scheduler_input(
                frontend_router,
                scheduler,
                adapter_scheduler,
                pending_runtime_deferred_replies,
                input,
            )
            .await
        }
    };
    if let Some(started) = trace_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "scheduler_input_done",
            input = input_kind,
            ok,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
    ok
}

async fn flush_owned_protocol_work(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    completion: super::protocol_task::ProtocolWorkCompletion,
) -> bool {
    match scheduler.complete_protocol_work(completion) {
        Ok(output) => {
            flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await
        }
        Err(error) => {
            tracing::error!(%error, "protocol owner work failed during recovery");
            false
        }
    }
}

async fn flush_renderer_publication_predecessor(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    predecessor: Option<&RendererOutputFence>,
) -> bool {
    let Some(predecessor) = predecessor else {
        return true;
    };
    while !scheduler
        .conn
        .renderer_output_cursor_is_projected(predecessor.cursor())
    {
        if let Some(publication) = scheduler_input_rx.take_buffered_renderer_publication() {
            if !ingest_and_flush_renderer_publication(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                adapter_scheduler,
                publication,
            )
            .await
            {
                return false;
            }
            continue;
        }
        tokio::select! {
            biased;
            request = scheduler_input_rx.protocol_state_request_rx.recv(), if !scheduler_input_rx.protocol_state_request_rx.is_closed() => {
                let Some(request) = request else { return false; };
                scheduler.apply_protocol_state_request(request);
            }
            completion = scheduler_input_rx.protocol_work_completion_rx.recv() => {
                let Some(completion) = completion else { return false; };
                if !flush_owned_protocol_work(frontend_router, scheduler, pending_runtime_deferred_replies, completion).await { return false; }
            }
            publication = scheduler_input_rx.renderer_publication_rx.recv() => {
                let Some(publication) = publication else { return false; };
                if !ingest_and_flush_renderer_publication(frontend_router, scheduler,
                    pending_runtime_deferred_replies, adapter_scheduler, publication).await { return false; }
            }
        }
    }
    true
}

async fn ingest_and_flush_renderer_publication(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    publication: RendererOutputTransportMessage,
) -> bool {
    let output = adapter_scheduler
        .ingest_renderer_publication(scheduler, publication)
        .await;
    flush_protocol_output_with_runtime_deferred_reply_routing(
        frontend_router,
        scheduler,
        pending_runtime_deferred_replies,
        output,
    )
    .await
}

async fn flush_background_completion_input(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    input: SchedulerInput,
) -> bool {
    let (prefix_output, mut completion_output, renderer_output_predecessor) = match input {
        SchedulerInput::BackgroundNavigationCompletion(completion) => {
            materialize_background_navigation_completion_output(
                scheduler,
                completion,
                &mut scheduler_input_rx.background_event_rx,
            )
            .await
        }
        _ => unreachable!("only background completion inputs are routed here"),
    };
    // The prefix contains facts that predate the renderer Page commit,
    // including frameStartedNavigating and an early Page.navigate response.
    // Flush it before waiting on the independent renderer transport. Otherwise
    // the exact commit cursor would incorrectly reorder new-realm output in
    // front of the navigation that created that realm.
    if !prefix_output.is_empty()
        && !flush_protocol_output_with_runtime_deferred_reply_routing(
            frontend_router,
            scheduler,
            pending_runtime_deferred_replies,
            prefix_output,
        )
        .await
    {
        return false;
    }
    if !flush_renderer_publication_predecessor(
        frontend_router,
        scheduler,
        scheduler_input_rx,
        pending_runtime_deferred_replies,
        adapter_scheduler,
        renderer_output_predecessor.as_ref(),
    )
    .await
    {
        return false;
    }
    if let Some(predecessor) = renderer_output_predecessor {
        let mut renderer_prefix = scheduler
            .complete_renderer_output_predecessor_before_runtime_response(&predecessor)
            .await;
        scheduler.append_navigation_gate_release_before_renderer_boundary(&mut renderer_prefix);
        renderer_prefix.append(completion_output);
        completion_output = renderer_prefix;
    }
    flush_protocol_output_with_runtime_deferred_reply_routing(
        frontend_router,
        scheduler,
        pending_runtime_deferred_replies,
        completion_output,
    )
    .await
}

enum RuntimeDeferredReplyFlushItem {
    Protocol(ProtocolOutputSequence),
    FinishCommand(RuntimeDeferredReplyCompletedCommand),
}

struct RuntimeDeferredReplyCompletedCommand {
    post_flush_scheduler_events: Vec<CdpSchedulerEvent>,
    output_release_permit: Option<CommandOutputReleasePermit>,
}

struct RuntimeDeferredReplyCompletion {
    output: CommandTurnOutput,
    output_session_id: Option<String>,
}

impl PendingRuntimeDeferredReplyState {
    fn new(
        pending: PendingCdpCommandDispatch,
        dispatch: CommandDispatchState,
        metadata: InFlightCommandMetadata,
        output_release_permit: CommandOutputReleasePermit,
        command_context: CommandDispatchContext,
    ) -> Self {
        let output_session_id = pending.session_id().map(str::to_owned);
        Self {
            pending,
            dispatch,
            metadata,
            output_release_permit,
            command_context,
            output_session_id,
            response_task: None,
        }
    }
}

impl RuntimeDeferredReplyCompletion {
    async fn settle_exact_outputs_before_response(mut self, scheduler: &mut CdpScheduler) -> Self {
        if let Some(predecessor) = self.output.take_renderer_output_predecessor() {
            assert!(
                scheduler
                    .conn
                    .renderer_output_cursor_is_projected(predecessor.cursor()),
                "a deferred Runtime response must not cross its renderer owner edge before its command-turn cursor is projected"
            );
            let output = scheduler
                .complete_renderer_output_predecessor_before_runtime_response(&predecessor)
                .await;
            self.output.prepend_protocol_output(output);
        }
        let output =
            scheduler.project_protocol_local_command_outputs_now(self.output_session_id.as_deref());
        self.output.prepend_protocol_output(output);
        self
    }

    fn into_parts(
        self,
    ) -> (
        ProtocolOutputSequence,
        Vec<BackgroundProtocolEvent>,
        RuntimeDeferredReplyCompletedCommand,
    ) {
        let (
            completion_output,
            post_renderer_output,
            renderer_output_boundary,
            post_response_events,
            post_flush_scheduler_events,
            renderer_output_predecessor,
            output_release_permit,
        ) = self.output.into_parts();
        debug_assert!(
            renderer_output_predecessor.is_none(),
            "deferred Runtime response consumed its renderer output predecessor"
        );
        debug_assert!(
            renderer_output_boundary.is_none() && post_renderer_output.is_empty(),
            "a deferred Runtime reply cannot own a navigation renderer insertion boundary"
        );
        (
            completion_output,
            post_response_events,
            RuntimeDeferredReplyCompletedCommand {
                post_flush_scheduler_events,
                output_release_permit,
            },
        )
    }
}

async fn flush_protocol_output_with_runtime_deferred_reply_routing(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    output: ProtocolOutputSequence,
) -> bool {
    let mut pending_flush = VecDeque::new();
    pending_flush.push_back(RuntimeDeferredReplyFlushItem::Protocol(output));

    while let Some(item) = pending_flush.pop_front() {
        match item {
            RuntimeDeferredReplyFlushItem::Protocol(mut output) => loop {
                if output.is_empty() {
                    break;
                }
                if let Some((prefix, response)) = output.split_next_runtime_response_ready() {
                    let advance = match complete_runtime_deferred_reply_for_renderer_response(
                        scheduler,
                        pending_runtime_deferred_replies,
                        response,
                    )
                    .await
                    {
                        Ok(advance) => advance,
                        Err(response) => {
                            let mut routed =
                                route_unmatched_runtime_inspector_response(scheduler, response);
                            routed.append(output);
                            if !routed.is_empty() {
                                pending_flush
                                    .push_front(RuntimeDeferredReplyFlushItem::Protocol(routed));
                            }
                            break;
                        }
                    };
                    match advance {
                        RuntimeDeferredReplyAdvance::Pending(mut pending) => {
                            let output =
                                take_runtime_deferred_initial_protocol_output(&mut pending);
                            if !output.is_empty() {
                                pending_flush
                                    .push_front(RuntimeDeferredReplyFlushItem::Protocol(output));
                            }
                            if !prefix.is_empty() {
                                pending_flush
                                    .push_front(RuntimeDeferredReplyFlushItem::Protocol(prefix));
                            }
                            pending_runtime_deferred_replies.push_back(*pending);
                        }
                        RuntimeDeferredReplyAdvance::Complete(completion) => {
                            let completion = (*completion)
                                .settle_exact_outputs_before_response(scheduler)
                                .await;
                            let mut remaining_output = prefix;
                            remaining_output.append(output);
                            enqueue_runtime_deferred_reply_completion_flush(
                                &mut pending_flush,
                                completion,
                                remaining_output,
                            );
                            break;
                        }
                    }
                    break;
                }
                let command_ids =
                    pending_runtime_deferred_reply_command_ids(pending_runtime_deferred_replies);
                if command_ids.is_empty() {
                    frontend_router.enqueue_protocol_output_sequence(output);
                    break;
                }
                let Some((prefix, command_id, event)) =
                    output.split_next_protocol_message_with_any_id(&command_ids)
                else {
                    frontend_router.enqueue_protocol_output_sequence(output);
                    break;
                };
                if !prefix.is_empty() {
                    frontend_router.enqueue_protocol_output_sequence(prefix);
                }
                let advance = match fail_runtime_deferred_reply_for_loose_protocol_response(
                    scheduler,
                    pending_runtime_deferred_replies,
                    command_id,
                    event,
                )
                .await
                {
                    Ok(advance) => advance,
                    Err(event) => {
                        tracing::debug!(
                            command_id,
                            "runtime deferred reply router saw a response without a pending state"
                        );
                        let mut recovered = ProtocolOutputSequence::from_background_event(event);
                        recovered.append(output);
                        pending_flush
                            .push_front(RuntimeDeferredReplyFlushItem::Protocol(recovered));
                        break;
                    }
                };
                match advance {
                    RuntimeDeferredReplyAdvance::Pending(mut pending) => {
                        let output = take_runtime_deferred_initial_protocol_output(&mut pending);
                        if !output.is_empty() {
                            pending_flush
                                .push_front(RuntimeDeferredReplyFlushItem::Protocol(output));
                        }
                        pending_runtime_deferred_replies.push_back(*pending);
                    }
                    RuntimeDeferredReplyAdvance::Complete(completion) => {
                        let completion = (*completion)
                            .settle_exact_outputs_before_response(scheduler)
                            .await;
                        enqueue_runtime_deferred_reply_completion_flush(
                            &mut pending_flush,
                            completion,
                            output,
                        );
                        break;
                    }
                }
            },
            RuntimeDeferredReplyFlushItem::FinishCommand(completed) => {
                let followup_output =
                    finish_runtime_deferred_reply_completed_command(scheduler, completed).await;
                if !followup_output.is_empty() {
                    pending_flush
                        .push_front(RuntimeDeferredReplyFlushItem::Protocol(followup_output));
                }
            }
        }
    }
    true
}

fn enqueue_runtime_deferred_reply_completion_flush(
    pending_flush: &mut VecDeque<RuntimeDeferredReplyFlushItem>,
    completion: RuntimeDeferredReplyCompletion,
    remaining_output: ProtocolOutputSequence,
) {
    let (completion_output, post_response_events, completed) = completion.into_parts();
    if !remaining_output.is_empty() {
        pending_flush.push_front(RuntimeDeferredReplyFlushItem::Protocol(remaining_output));
    }
    pending_flush.push_front(RuntimeDeferredReplyFlushItem::FinishCommand(completed));
    if !post_response_events.is_empty() {
        pending_flush.push_front(RuntimeDeferredReplyFlushItem::Protocol(
            ProtocolOutputSequence::from_background_events(post_response_events),
        ));
    }
    pending_flush.push_front(RuntimeDeferredReplyFlushItem::Protocol(completion_output));
}

async fn finish_runtime_deferred_reply_completed_command(
    scheduler: &mut CdpScheduler,
    completed: RuntimeDeferredReplyCompletedCommand,
) -> ProtocolOutputSequence {
    let mut followup_output = scheduler
        .finish_command_dispatch_output_flush(
            completed.post_flush_scheduler_events,
            completed.output_release_permit,
        )
        .await;
    followup_output.append(
        scheduler
            .complete_ready_protocol_residences_after_command()
            .await,
    );
    followup_output
}

fn pending_runtime_deferred_reply_command_ids(
    pending_runtime_deferred_replies: &VecDeque<PendingRuntimeDeferredReplyState>,
) -> Vec<u64> {
    pending_runtime_deferred_replies
        .iter()
        .filter_map(|pending| pending.pending.command_id())
        .collect()
}

async fn fail_runtime_deferred_reply_for_loose_protocol_response(
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    command_id: u64,
    event: BackgroundProtocolEvent,
) -> Result<RuntimeDeferredReplyAdvance, BackgroundProtocolEvent> {
    let Some(index) = pending_runtime_deferred_replies
        .iter()
        .position(|pending| pending.pending.command_id() == Some(command_id))
    else {
        return Err(event);
    };
    let pending = pending_runtime_deferred_replies
        .remove(index)
        .expect("pending runtime deferred reply index came from position()");
    let event_message = event
        .protocol_message()
        .cloned()
        .unwrap_or_else(|| event.clone().into_protocol_message());
    tracing::warn!(
        command_id,
        message = ?event_message,
        "runtime deferred reply saw a loose protocol response; deferred replies must complete through the typed renderer response receiver"
    );
    let output_session_id = pending.output_session_id;
    pending
        .pending
        .forget_scheduler_deferred_inspector_reply(&mut scheduler.conn);
    let command_output = pending
        .dispatch
        .complete_with_turn_output(CommandTurnOutput::new(
            ProtocolOutputSequence::from_background_event(BackgroundProtocolEvent::command_error(
                Some(command_id),
                None,
                -32000,
                "RuntimeDeferredReplyLooseProtocolResponse".to_owned(),
                None,
            )),
            Vec::new(),
        ))
        .with_output_release_permit(pending.output_release_permit);
    Ok(RuntimeDeferredReplyAdvance::Complete(Box::new(
        RuntimeDeferredReplyCompletion {
            output: command_output,
            output_session_id,
        },
    )))
}

// An unmatched response goes back to the caller for routing, without boxing.
#[allow(clippy::result_large_err)]
async fn complete_runtime_deferred_reply_for_renderer_response(
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    response: RuntimeInspectorResponseReady,
) -> Result<RuntimeDeferredReplyAdvance, RuntimeInspectorResponseReady> {
    let command_id = response.command_id();
    let renderer_output_predecessor = response.renderer_output_predecessor();
    let Some(index) = pending_runtime_deferred_replies
        .iter()
        .position(|pending| pending.pending.command_id() == Some(command_id))
    else {
        return Err(response);
    };
    let mut pending = pending_runtime_deferred_replies
        .remove(index)
        .expect("pending runtime deferred reply index came from position()");
    pending
        .pending
        .route_scheduler_deferred_inspector_response(&mut scheduler.conn, response)
        .await;
    Ok(
        complete_runtime_deferred_reply_state(scheduler, pending, renderer_output_predecessor)
            .await,
    )
}

async fn flush_runtime_deferred_reply_advance(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    advance: RuntimeDeferredReplyAdvance,
) -> bool {
    match advance {
        RuntimeDeferredReplyAdvance::Pending(pending) => {
            enqueue_pending_runtime_deferred_reply_state(
                pending_runtime_deferred_replies,
                *pending,
                deferred_runtime_response_tx,
            );
            true
        }
        RuntimeDeferredReplyAdvance::Complete(completion) => {
            let completion = (*completion)
                .settle_exact_outputs_before_response(scheduler)
                .await;
            let (completion_output, post_response_events, completed) = completion.into_parts();
            frontend_router.enqueue_protocol_output_sequence(completion_output);
            if !post_response_events.is_empty()
                && !flush_protocol_output_with_runtime_deferred_reply_routing(
                    frontend_router,
                    scheduler,
                    pending_runtime_deferred_replies,
                    ProtocolOutputSequence::from_background_events(post_response_events),
                )
                .await
            {
                return false;
            }
            let followup_output =
                finish_runtime_deferred_reply_completed_command(scheduler, completed).await;
            if !followup_output.is_empty() {
                flush_protocol_output_with_runtime_deferred_reply_routing(
                    frontend_router,
                    scheduler,
                    pending_runtime_deferred_replies,
                    followup_output,
                )
                .await
            } else {
                true
            }
        }
    }
}

enum RuntimeDeferredReplyAdvance {
    Pending(Box<PendingRuntimeDeferredReplyState>),
    Complete(Box<RuntimeDeferredReplyCompletion>),
}

async fn complete_runtime_deferred_reply_state(
    scheduler: &mut CdpScheduler,
    mut pending: PendingRuntimeDeferredReplyState,
    renderer_output_predecessor: Option<RendererOutputFence>,
) -> RuntimeDeferredReplyAdvance {
    let command_id = pending.pending.command_id();
    let session_id = pending.pending.session_id().map(str::to_owned);
    let completed = pending
        .pending
        .complete_scheduler_deferred_inspector_reply(&mut scheduler.conn);
    match scheduler
        .complete_pending_command_dispatch_with_context(completed, &mut pending.command_context)
        .await
    {
        CommandTaskStep::Complete(output) => {
            RuntimeDeferredReplyAdvance::Complete(Box::new(RuntimeDeferredReplyCompletion {
                output: pending
                    .dispatch
                    .complete_with_turn_output(*output)
                    .with_renderer_output_predecessor(
                        pending.command_context.take_renderer_output_predecessor(),
                    )
                    .with_renderer_output_predecessor(renderer_output_predecessor)
                    .with_output_release_permit(pending.output_release_permit),
                output_session_id: pending.output_session_id,
            }))
        }
        CommandTaskStep::Pending(next_pending)
            if next_pending.waits_for_scheduler_deferred_inspector_reply() =>
        {
            let next_state = PendingRuntimeDeferredReplyState::new(
                *next_pending,
                pending.dispatch,
                pending.metadata,
                pending.output_release_permit,
                pending.command_context,
            );
            RuntimeDeferredReplyAdvance::Pending(Box::new(next_state))
        }
        CommandTaskStep::Pending(next_pending) => {
            let pending_kind = next_pending.kind_name();
            tracing::warn!(
                pending_kind,
                "runtime deferred reply completion returned an unexpected pending command"
            );
            RuntimeDeferredReplyAdvance::Complete(Box::new(RuntimeDeferredReplyCompletion {
                output: pending
                    .dispatch
                    .complete_with_turn_output(runtime_deferred_reply_unexpected_pending_output(
                        command_id,
                        session_id.as_deref(),
                        pending_kind,
                    ))
                    .with_renderer_output_predecessor(
                        pending.command_context.take_renderer_output_predecessor(),
                    )
                    .with_renderer_output_predecessor(renderer_output_predecessor)
                    .with_output_release_permit(pending.output_release_permit),
                output_session_id: pending.output_session_id,
            }))
        }
    }
}

fn runtime_deferred_reply_unexpected_pending_output(
    command_id: Option<u64>,
    session_id: Option<&str>,
    pending_kind: &str,
) -> CommandTurnOutput {
    CommandTurnOutput::new(
        ProtocolOutputSequence::from_background_event(BackgroundProtocolEvent::command_error(
            command_id,
            session_id,
            -32000,
            "RuntimeDeferredReplyUnexpectedPending".to_owned(),
            Some(json!({
                "pendingKind": pending_kind,
            })),
        )),
        Vec::new(),
    )
}

async fn handle_adapter_scheduler_input(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    input: ProtocolAdapterSchedulerInput,
) -> bool {
    let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
    let advance = adapter_scheduler.advance_input(scheduler, input).await;
    let (kind, ok) = match advance {
        ProtocolAdapterSchedulerAdvance::Idle => ("idle", true),
        ProtocolAdapterSchedulerAdvance::ClientTurnYielded => ("client_turn_yielded", true),
        ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(output) => {
            let ok = flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await;
            ("protocol_residence_completed", ok)
        }
    };
    if let Some(started) = trace_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "protocol_adapter_input_done",
            advance = kind,
            ok,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
    ok
}

fn enqueue_pending_runtime_deferred_reply_state(
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    mut pending: PendingRuntimeDeferredReplyState,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
) {
    if let Some(command_id) = pending.pending.command_id()
        && let Some(response_rx) = pending
            .pending
            .take_scheduler_deferred_inspector_reply_receiver()
    {
        let session_id = pending.pending.session_id().map(str::to_owned);
        pending.response_task = Some(start_deferred_runtime_inspector_response_wait(
            command_id,
            session_id,
            response_rx,
            deferred_runtime_response_tx,
        ));
    }
    pending_runtime_deferred_replies.push_back(pending);
}

fn take_runtime_deferred_initial_protocol_output(
    pending: &mut PendingRuntimeDeferredReplyState,
) -> ProtocolOutputSequence {
    ProtocolOutputSequence::from_background_events(
        pending
            .pending
            .take_scheduler_deferred_inspector_reply_events(),
    )
}

fn start_deferred_runtime_inspector_response_wait(
    command_id: u64,
    session_id: Option<String>,
    response_rx: RuntimeInspectorAsyncCompletionReceiver,
    response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
) -> CommandWaitTask {
    let response_tx = response_tx.clone();
    spawn_owned_wait(response_rx, move |response| {
        let response = match response {
            Ok(response) => {
                response.map_err(|_| "RuntimeDeferredInspectorResponseCanceled".to_owned())
            }
            Err(failure) => Err(failure.message().to_owned()),
        };
        let _ = response_tx.send(RuntimeInspectorResponseReady::new(
            command_id,
            session_id.as_deref(),
            response,
        ));
    })
}

async fn handle_deferred_runtime_inspector_response_result(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    result: RuntimeInspectorResponseReady,
) -> bool {
    match complete_runtime_deferred_reply_for_renderer_response(
        scheduler,
        pending_runtime_deferred_replies,
        result,
    )
    .await
    {
        Ok(advance) => {
            flush_runtime_deferred_reply_advance(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                deferred_runtime_response_tx,
                advance,
            )
            .await
        }
        Err(response) => {
            let output = route_unmatched_runtime_inspector_response(scheduler, response);
            if output.is_empty() {
                return true;
            }
            flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await
        }
    }
}

fn route_unmatched_runtime_inspector_response(
    scheduler: &mut CdpScheduler,
    response: RuntimeInspectorResponseReady,
) -> ProtocolOutputSequence {
    let command_id = response.command_id();
    let output = scheduler.route_registered_runtime_inspector_response(response);
    if output.is_empty() {
        tracing::debug!(
            command_id,
            "dropping renderer runtime response without pending command or registered listener"
        );
    }
    output
}

fn command_metadata(command: &ParsedCdpCommand) -> InFlightCommandMetadata {
    let request = command.request();
    InFlightCommandMetadata {
        method: Some(command.method().to_owned()),
        id: Some(request.id()),
        session_id: request.session_id().map(str::to_owned),
        started: moli_trace::cdp_runtime_trace_enabled().then(Instant::now),
        executes_page_javascript: command.runtime_command_executes_page_javascript(),
        command_output_session_id: command.command_output_session_id().map(str::to_owned),
        internal_response: None,
    }
}

fn take_next_in_flight_command_token(next_token: &mut u64) -> u64 {
    let token = *next_token;
    *next_token = next_token
        .checked_add(1)
        .expect("in-flight CDP command token space exhausted");
    token
}

fn start_pending_command_wait(
    token: u64,
    turn: u64,
    pending: PendingCdpCommandDispatch,
    completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
) -> CommandWaitTask {
    spawn_command_wait(
        CommandWaitIdentity { token, turn },
        async move { CompletedActorWork::Command(pending.wait().await) },
        completion_tx,
    )
}

fn trace_in_flight_command(
    metadata: &InFlightCommandMetadata,
    stage: &'static str,
    elapsed: Option<std::time::Duration>,
    messages: Option<usize>,
    ok: Option<bool>,
) {
    if !moli_trace::cdp_runtime_trace_enabled() {
        return;
    }
    let elapsed_us = elapsed.map(|duration| duration.as_micros());
    tracing::info!(
        target: "moli_cdp_runtime",
        stage = stage,
        method = metadata.method.as_deref().unwrap_or("<parse-error>"),
        id = ?metadata.id,
        session_id = ?metadata.session_id,
        elapsed_us = ?elapsed_us,
        messages = ?messages,
        ok = ?ok,
    );
}

#[allow(clippy::too_many_arguments)]
async fn handle_frontend_command(
    frontend_command: CdpFrontendCommand,
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    blocked_commands: &mut VecDeque<BlockedCommandDispatch>,
    next_in_flight_command_token: &mut u64,
    owner_lifecycle: Option<&CdpOwnerActorLifecycle>,
) -> bool {
    let CdpFrontendCommand {
        frontend_id,
        command,
    } = frontend_command;
    let Some(prepared) = frontend_router.prepare_command(frontend_id, command) else {
        return true;
    };
    let command = match prepared {
        CdpPreparedFrontendCommand::Command(command) => command,
        CdpPreparedFrontendCommand::TargetMessage {
            frontend_id,
            acknowledgements,
            command,
        } => {
            for message in acknowledgements {
                frontend_router.enqueue_immediate_response(frontend_id, message);
            }
            let Some(command) = command else {
                return true;
            };
            command
        }
        CdpPreparedFrontendCommand::ImmediateResponse {
            frontend_id,
            message,
        } => {
            frontend_router.enqueue_immediate_response(frontend_id, message);
            return true;
        }
    };
    trace_command(&command, "frontend_command_received", None, None, None);
    if let Some(owner_lifecycle) = owner_lifecycle
        && super::instance_command::handle_instance_command(
            &command,
            frontend_router,
            &owner_lifecycle.instance_control,
        )
        .await
    {
        return true;
    }
    let metadata = command_metadata(&command);
    handle_client_command_with_interleaved_output(
        frontend_router,
        scheduler,
        scheduler_input_rx,
        command,
        metadata,
        pending_runtime_deferred_replies,
        deferred_runtime_response_tx,
        adapter_scheduler,
        pending_command_completion_tx,
        in_flight_commands,
        blocked_commands,
        next_in_flight_command_token,
    )
    .await
}

async fn handle_client_command_with_interleaved_output(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    command: ParsedCdpCommand,
    metadata: InFlightCommandMetadata,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    blocked_commands: &mut VecDeque<BlockedCommandDispatch>,
    next_in_flight_command_token: &mut u64,
) -> bool {
    let probe_method = moli_trace::command_probe_enabled().then(|| command.method().to_owned());
    if let Some(method) = probe_method.as_deref() {
        tracing::info!(method, "CMD_PROBE_COMMAND_RECEIVED");
    }
    if !blocked_commands.is_empty() && scheduler.command_waits_for_navigation_flush(&command) {
        trace_command(
            &command,
            "command_queued_behind_background_navigation_blocked_command",
            None,
            None,
            None,
        );
        blocked_commands.push_back(BlockedCommandDispatch { command, metadata });
        return drain_blocked_commands_after_navigation_gate(
            frontend_router,
            scheduler,
            scheduler_input_rx,
            pending_runtime_deferred_replies,
            deferred_runtime_response_tx,
            adapter_scheduler,
            pending_command_completion_tx,
            in_flight_commands,
            blocked_commands,
            next_in_flight_command_token,
        )
        .await;
    }
    match scheduler.start_command_or_request_background_navigation_flush(&command) {
        CommandStartAction::NeedsBackgroundNavigationFlush => {
            trace_command(
                &command,
                "command_blocked_by_background_navigation",
                None,
                None,
                None,
            );
            blocked_commands.push_back(BlockedCommandDispatch { command, metadata });
            true
        }
        CommandStartAction::Dispatch {
            step,
            output_release_permit,
            command_context,
        } => {
            start_ready_command_dispatch(
                frontend_router,
                scheduler,
                scheduler_input_rx,
                &command,
                metadata,
                step,
                output_release_permit,
                command_context,
                pending_runtime_deferred_replies,
                deferred_runtime_response_tx,
                adapter_scheduler,
                pending_command_completion_tx,
                in_flight_commands,
                next_in_flight_command_token,
            )
            .await
        }
    }
}

async fn start_ready_command_dispatch(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    command: &ParsedCdpCommand,
    metadata: InFlightCommandMetadata,
    step: CommandTaskStep,
    output_release_permit: CommandOutputReleasePermit,
    mut command_context: CommandDispatchContext,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    next_in_flight_command_token: &mut u64,
) -> bool {
    trace_command(command, "command_dispatch_selected", None, None, None);
    match step {
        CommandTaskStep::Complete(output) => {
            trace_command(command, "command_dispatch_complete_sync", None, None, None);
            let output = CommandDispatchState::pending_command()
                .complete_with_turn_output(*output)
                .with_renderer_output_predecessor(
                    command_context.take_renderer_output_predecessor(),
                )
                .with_output_release_permit(output_release_permit);
            let ok = flush_completed_command_output(
                frontend_router,
                scheduler,
                scheduler_input_rx,
                pending_runtime_deferred_replies,
                adapter_scheduler,
                &metadata,
                output,
            )
            .await;
            trace_in_flight_command(
                &metadata,
                "command_done",
                metadata.started.map(|started| started.elapsed()),
                None,
                Some(ok),
            );
            ok
        }
        CommandTaskStep::Pending(pending)
            if pending.waits_for_scheduler_deferred_inspector_reply() =>
        {
            trace_command(
                command,
                "command_dispatch_pending_deferred_runtime_reply",
                None,
                None,
                None,
            );
            let mut pending = PendingRuntimeDeferredReplyState::new(
                *pending,
                CommandDispatchState::pending_command(),
                metadata,
                output_release_permit,
                command_context,
            );
            let session_id = pending.pending.session_id().map(str::to_owned);
            let mut output = take_runtime_deferred_initial_protocol_output(&mut pending);
            output.append(
                scheduler.project_protocol_local_command_outputs_now(session_id.as_deref()),
            );
            enqueue_pending_runtime_deferred_reply_state(
                pending_runtime_deferred_replies,
                pending,
                deferred_runtime_response_tx,
            );
            flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await
        }
        CommandTaskStep::Pending(pending) => {
            trace_command(command, "command_dispatch_pending", None, None, None);
            let token = take_next_in_flight_command_token(next_in_flight_command_token);
            let owner_session_id = if pending.continues_after_session_detachment() {
                None
            } else {
                pending
                    .session_id()
                    .map(str::to_owned)
                    .or_else(|| metadata.session_id.clone())
            };
            let wait_lifetime = pending.wait_lifetime();
            let wait_task =
                start_pending_command_wait(token, 0, *pending, pending_command_completion_tx);
            in_flight_commands.insert(
                token,
                InFlightWork::Command(Box::new(InFlightCommandState {
                    metadata,
                    dispatch: CommandDispatchState::pending_command(),
                    output_release_permit,
                    command_context,
                    pending_turn: 0,
                    owner_session_id,
                    wait_lifetime,
                    wait_task,
                })),
            );
            true
        }
    }
}

async fn drain_blocked_commands_after_navigation_gate(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    blocked_commands: &mut VecDeque<BlockedCommandDispatch>,
    next_in_flight_command_token: &mut u64,
) -> bool {
    loop {
        let Some(blocked) = blocked_commands.pop_front() else {
            return true;
        };
        if scheduler.command_waits_for_navigation_flush(&blocked.command) {
            trace_command(
                &blocked.command,
                "blocked_command_still_waiting_for_background_navigation",
                None,
                None,
                None,
            );
            blocked_commands.push_front(blocked);
            return true;
        }
        match scheduler.start_command_or_request_background_navigation_flush(&blocked.command) {
            CommandStartAction::NeedsBackgroundNavigationFlush => {
                trace_command(
                    &blocked.command,
                    "blocked_command_still_waiting_for_background_navigation",
                    None,
                    None,
                    None,
                );
                blocked_commands.push_front(blocked);
                return true;
            }
            CommandStartAction::Dispatch {
                step,
                output_release_permit,
                command_context,
            } => {
                if !start_ready_command_dispatch(
                    frontend_router,
                    scheduler,
                    scheduler_input_rx,
                    &blocked.command,
                    blocked.metadata,
                    step,
                    output_release_permit,
                    command_context,
                    pending_runtime_deferred_replies,
                    deferred_runtime_response_tx,
                    adapter_scheduler,
                    pending_command_completion_tx,
                    in_flight_commands,
                    next_in_flight_command_token,
                )
                .await
                {
                    return false;
                }
            }
        }
    }
}

async fn flush_completed_command_output(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    metadata: &InFlightCommandMetadata,
    mut output: CommandTurnOutput,
) -> bool {
    let renderer_output_predecessor = output.take_renderer_output_predecessor();
    if !flush_renderer_publication_predecessor(
        frontend_router,
        scheduler,
        scheduler_input_rx,
        pending_runtime_deferred_replies,
        adapter_scheduler,
        renderer_output_predecessor.as_ref(),
    )
    .await
    {
        return false;
    }

    let mut causal_prefix = ProtocolOutputSequence::empty();
    if let Some(predecessor) = renderer_output_predecessor {
        causal_prefix.append(
            scheduler
                .complete_renderer_output_predecessor_before_runtime_response(&predecessor)
                .await,
        );
    }
    causal_prefix.append(
        scheduler.project_protocol_local_command_outputs_now(
            metadata.command_output_session_id.as_deref(),
        ),
    );
    output.prepend_protocol_output(causal_prefix);
    let (
        mut protocol_output,
        post_renderer_output,
        renderer_output_boundary,
        post_response_events,
        post_flush_scheduler_events,
        renderer_output_predecessor,
        output_release_permit,
    ) = output.into_parts();
    debug_assert!(
        renderer_output_predecessor.is_none(),
        "command response consumed its exact renderer output predecessor before decomposition"
    );
    let internal_response =
        take_internal_command_response(metadata, &mut protocol_output, frontend_router, scheduler);
    let messages = protocol_output.len();
    let post_response_event_count = post_response_events.len();
    let post_flush_event_count = post_flush_scheduler_events.len();
    let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
    if trace_started.is_some() {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "command_output_flush_start",
            messages,
            post_response_events = post_response_event_count,
            post_flush_events = post_flush_event_count,
        );
    }
    if !flush_protocol_output_with_runtime_deferred_reply_routing(
        frontend_router,
        scheduler,
        pending_runtime_deferred_replies,
        protocol_output,
    )
    .await
    {
        return false;
    }
    if !flush_renderer_publication_predecessor(
        frontend_router,
        scheduler,
        scheduler_input_rx,
        pending_runtime_deferred_replies,
        adapter_scheduler,
        renderer_output_boundary.as_ref(),
    )
    .await
    {
        return false;
    }
    if let Some(renderer_output_boundary) = renderer_output_boundary {
        let boundary_output = scheduler
            .complete_renderer_output_predecessor_before_runtime_response(&renderer_output_boundary)
            .await;
        if !boundary_output.is_empty()
            && !flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                boundary_output,
            )
            .await
        {
            return false;
        }
    }
    if !post_renderer_output.is_empty()
        && !flush_protocol_output_with_runtime_deferred_reply_routing(
            frontend_router,
            scheduler,
            pending_runtime_deferred_replies,
            post_renderer_output,
        )
        .await
    {
        return false;
    }
    if !post_response_events.is_empty()
        && !flush_protocol_output_with_runtime_deferred_reply_routing(
            frontend_router,
            scheduler,
            pending_runtime_deferred_replies,
            ProtocolOutputSequence::from_background_events(post_response_events),
        )
        .await
    {
        return false;
    }
    let released_output = scheduler
        .finish_command_dispatch_output_flush(post_flush_scheduler_events, output_release_permit)
        .await;
    if !released_output.is_empty()
        && !flush_protocol_output_with_runtime_deferred_reply_routing(
            frontend_router,
            scheduler,
            pending_runtime_deferred_replies,
            released_output,
        )
        .await
    {
        return false;
    }
    let followup_output = scheduler
        .complete_ready_protocol_residences_after_command()
        .await;
    if !followup_output.is_empty()
        && !flush_protocol_output_with_runtime_deferred_reply_routing(
            frontend_router,
            scheduler,
            pending_runtime_deferred_replies,
            followup_output,
        )
        .await
    {
        return false;
    }
    if let Some(started) = trace_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "command_output_flush_done",
            messages,
            post_response_events = post_response_event_count,
            post_flush_events = post_flush_event_count,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
    if let Some(mut response) = internal_response
        && let Some(completion) = metadata
            .internal_response
            .as_ref()
            .and_then(|slot| slot.borrow_mut().take())
    {
        if let Ok(result) = &mut response {
            result.registration = completion.registration.map(|registration| {
                register_internal_command_frontend(
                    registration,
                    &result.response,
                    frontend_router,
                    scheduler,
                )
            });
        }
        let _ = completion.sender.send(response);
    }
    true
}

fn register_internal_command_frontend(
    registration: CdpFrontendRegistration,
    response: &Value,
    router: &CdpFrontendRouter,
    scheduler: &CdpScheduler,
) -> Result<()> {
    let session = response
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .context("CDP frontend attach returned no session")?;
    anyhow::ensure!(
        scheduler.conn.has_attached_devtools_session(session)
            && !scheduler.conn.session_is_closing(session),
        "CDP frontend session closed before registration"
    );
    match registration {
        CdpFrontendRegistration::Browser { frontend_id, sink } => {
            router.register_browser_frontend(frontend_id, session.to_owned(), sink)
        }
        CdpFrontendRegistration::Target {
            frontend_id,
            target_id,
            sink,
        } => router.register_target_frontend(frontend_id, target_id, session.to_owned(), sink),
    }
}

fn take_internal_command_response(
    metadata: &InFlightCommandMetadata,
    output: &mut ProtocolOutputSequence,
    frontend_router: &CdpFrontendRouter,
    scheduler: &CdpScheduler,
) -> Option<Result<CdpInternalCommandResult>> {
    metadata.internal_response.as_ref()?;
    let mut response = None;
    let mut passthrough = Vec::new();
    for event in std::mem::take(output).into_background_events() {
        if event.protocol_message_id() == metadata.id {
            assert!(response.is_none(), "an internal command completes once");
            response = Some(event.into_protocol_message());
        } else {
            passthrough.push(event);
        }
    }
    *output = ProtocolOutputSequence::from_background_events(passthrough);
    Some((|| {
        let response = response.context("CDP internal command returned no response")?;
        let resource = match metadata.method.as_deref() {
            Some("Target.attachToBrowserTarget" | "Target.attachToTarget") => response
                .pointer("/result/sessionId")
                .and_then(Value::as_str)
                .map(|session| CdpControlResource::Session(session.to_owned())),
            Some("Target.createTarget") => response
                .pointer("/result/targetId")
                .and_then(Value::as_str)
                .map(|target| CdpControlResource::Target(target.to_owned())),
            _ => None,
        };
        let rollback = resource.map(|resource| {
            let completion = metadata.internal_response.as_ref().unwrap().borrow();
            let completion = completion.as_ref().unwrap();
            let resource = match (resource, completion.registration.as_ref()) {
                (
                    CdpControlResource::Session(session_id),
                    Some(CdpFrontendRegistration::Browser { frontend_id, .. }),
                ) => CdpControlResource::BrowserFrontend {
                    session_id,
                    frontend_id: *frontend_id,
                },
                (
                    CdpControlResource::Session(session_id),
                    Some(CdpFrontendRegistration::Target { frontend_id, .. }),
                ) => CdpControlResource::TargetFrontend {
                    session_id,
                    frontend_id: *frontend_id,
                },
                (resource, _) => resource,
            };
            CdpControlResourceLease::new(completion.owner.clone(), resource)
        });
        if metadata.method.as_deref() == Some("Target.attachToBrowserTarget")
            && let Some(session) = response
                .pointer("/result/sessionId")
                .and_then(Value::as_str)
        {
            frontend_router
                .register_private_session(session.to_owned())
                .context("failed to register CDP internal control session")?;
        }
        let created_target = if metadata.method.as_deref() == Some("Target.createTarget") {
            response
                .pointer("/result/targetId")
                .and_then(Value::as_str)
                .map(|target_id| {
                    let tab_target_id = scheduler
                        .conn
                        .tab_target_id_for_page_target_id(target_id)
                        .context("created page target has no tab target")?;
                    Ok::<_, anyhow::Error>(CdpCreatedTarget {
                        page_target_id: target_id.to_owned(),
                        tab_target_id: tab_target_id.to_owned(),
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(CdpInternalCommandResult {
            response,
            created_target,
            registration: None,
            rollback,
        })
    })())
}

async fn handle_pending_command_completion(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    completion: PendingCommandCompletion,
) -> bool {
    let identity = completion.identity;
    if in_flight_commands
        .get(&identity.token)
        .is_none_or(|state| state.turn() != identity.turn)
    {
        tracing::debug!(
            token = identity.token,
            turn = identity.turn,
            "dropping stale pending command completion without matching actor state"
        );
        return true;
    }
    let work = in_flight_commands
        .remove(&identity.token)
        .expect("matching in-flight work");
    let mut state = match work {
        InFlightWork::FrontendControl(_) => {
            if let Err(error) = completion.result {
                tracing::warn!(token = identity.token, %error, "frontend control workflow failed");
            }
            return true;
        }
        InFlightWork::Command(state) => state,
    };
    let completion = CommandWaitCompletion {
        identity,
        result: completion.result.map(|completed| match completed {
            CompletedActorWork::Command(completed) => completed,
            CompletedActorWork::FrontendControl => {
                unreachable!("command wait completed as control work")
            }
        }),
    };
    // A successful result may already be queued when its handler is disabled.
    // Aborting a finished task cannot revoke that result; validate the same
    // lifetime again before any domain completion can mutate protocol state.
    let result = if state
        .wait_lifetime
        .as_ref()
        .is_some_and(|lifetime| !lifetime.is_active())
    {
        drop(completion.result);
        Err(CommandWaitFailure::Cancelled)
    } else {
        completion.result
    };
    let completion_kind = result
        .as_ref()
        .map_or("WaitFailure", |completed| completed.kind_name());
    if moli_trace::cdp_runtime_trace_enabled() {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "command_pending_wait_done",
            pending_turn = state.pending_turn,
            pending_kind = completion_kind,
        );
    }
    let complete_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
    let step = match result {
        Ok(completed) => {
            scheduler
                .complete_pending_command_dispatch_with_context(
                    completed,
                    &mut state.command_context,
                )
                .await
        }
        Err(failure) => scheduler.complete_failed_command_wait_with_context(
            state.metadata.id,
            state.metadata.session_id.as_deref(),
            failure.message(),
            &mut state.command_context,
        ),
    };
    if let Some(started) = complete_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "command_pending_complete_done",
            pending_turn = state.pending_turn,
            pending_kind = completion_kind,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
    match step {
        CommandTaskStep::Pending(pending)
            if pending.waits_for_scheduler_deferred_inspector_reply() =>
        {
            let mut pending = PendingRuntimeDeferredReplyState::new(
                *pending,
                state.dispatch,
                state.metadata,
                state.output_release_permit,
                state.command_context,
            );
            let session_id = pending.pending.session_id().map(str::to_owned);
            let mut output = take_runtime_deferred_initial_protocol_output(&mut pending);
            output.append(
                scheduler.project_protocol_local_command_outputs_now(session_id.as_deref()),
            );
            enqueue_pending_runtime_deferred_reply_state(
                pending_runtime_deferred_replies,
                pending,
                deferred_runtime_response_tx,
            );
            flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await
        }
        CommandTaskStep::Pending(pending) => {
            if moli_trace::cdp_runtime_trace_enabled() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "command_pending_requeued",
                    pending_turn = state.pending_turn,
                    pending_kind = pending.kind_name(),
                );
            }
            state.pending_turn = state
                .pending_turn
                .checked_add(1)
                .expect("pending command turn space exhausted");
            if pending.continues_after_session_detachment() {
                state.owner_session_id = None;
            }
            // A protected completion/recovery owns its admitted resources.
            // Do not keep cancelling it with the previous wait's retired owner.
            state.wait_lifetime = pending.wait_lifetime();
            let token = identity.token;
            state.wait_task = start_pending_command_wait(
                token,
                state.pending_turn,
                *pending,
                pending_command_completion_tx,
            );
            in_flight_commands.insert(token, InFlightWork::Command(state));
            true
        }
        CommandTaskStep::Complete(output) => {
            if moli_trace::cdp_runtime_trace_enabled() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "command_pending_complete_final",
                    pending_turn = state.pending_turn,
                );
            }
            let output = state
                .dispatch
                .complete_with_turn_output(*output)
                .with_renderer_output_predecessor(
                    state.command_context.take_renderer_output_predecessor(),
                )
                .with_output_release_permit(state.output_release_permit);
            let ok = flush_completed_command_output(
                frontend_router,
                scheduler,
                scheduler_input_rx,
                pending_runtime_deferred_replies,
                adapter_scheduler,
                &state.metadata,
                output,
            )
            .await;
            trace_in_flight_command(
                &state.metadata,
                "command_done",
                state.metadata.started.map(|started| started.elapsed()),
                None,
                Some(ok),
            );
            ok
        }
    }
}

async fn materialize_background_navigation_completion_output(
    scheduler: &mut CdpScheduler,
    completion: BackgroundNavigationCompletion,
    background_event_rx: &mut CdpBackgroundEventReceiver,
) -> (
    ProtocolOutputSequence,
    ProtocolOutputSequence,
    Option<moli_core::RendererOutputFence>,
) {
    let timing_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
    if timing_started.is_some() {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %completion.requested_url(),
            kind = completion.kind(),
            stage = "background_completion_actor_recv",
        );
    }
    let (prefix, completion, renderer_output_predecessor) = scheduler
        .materialize_background_navigation_completion_with_progress_barrier(
            completion,
            background_event_rx,
        )
        .await;
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            stage = "background_completion_actor_materialized",
            prefix_messages = prefix.len(),
            completion_messages = completion.len(),
            phase_ms = started.elapsed().as_millis(),
        );
    }
    (prefix, completion, renderer_output_predecessor)
}

fn trace_scheduler_input(input: &SchedulerInput, stage: &'static str) {
    if !moli_trace::cdp_runtime_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_cdp_runtime",
        stage = stage,
        input = scheduler_input_kind(input),
    );
}

fn scheduler_input_kind(input: &SchedulerInput) -> &'static str {
    match input {
        SchedulerInput::ProtocolStateRequest(_) => "protocol_state_request",
        SchedulerInput::ProtocolWorkCompletion(_) => "protocol_work_completion",
        SchedulerInput::DocumentActivityCompletion(_) => "document_activity_completion",
        SchedulerInput::BackgroundNavigationCompletion(_) => "background_navigation_completion",
        SchedulerInput::BackgroundEvent(_) => "background_event",
        SchedulerInput::DeferredRuntimeInspectorResponse(_) => {
            "deferred_runtime_inspector_response"
        }
        SchedulerInput::RendererPublication(_) => "renderer_publication",
        SchedulerInput::AdapterScheduler(ProtocolAdapterSchedulerInput::Turn) => {
            "protocol_adapter_turn"
        }
    }
}

fn trace_command(
    command: &ParsedCdpCommand,
    stage: &'static str,
    elapsed: Option<std::time::Duration>,
    messages: Option<usize>,
    ok: Option<bool>,
) {
    if !moli_trace::cdp_runtime_trace_enabled() {
        return;
    }
    let request = command.request();
    let method = command.method();
    let id = request.id();
    let session_id = request.session_id();
    let elapsed_us = elapsed.map(|duration| duration.as_micros());
    tracing::info!(
        target: "moli_cdp_runtime",
        stage = stage,
        method = method,
        id = ?id,
        session_id = ?session_id,
        elapsed_us = ?elapsed_us,
        messages = ?messages,
        ok = ?ok,
    );
}

#[cfg(test)]
mod tests {
    use moli_core::RendererRuntimeInspectorAsyncCompletion;
    use moli_protocol::CdpConnection;
    use serde_json::json;

    use super::*;
    use crate::cdp_scheduler::command_task::CommandWaitFailure;

    fn empty_scheduler_inputs() -> SchedulerInputReceivers {
        let (_background_event_tx, background_event_rx) = mpsc::unbounded_channel();
        let (_navigation_tx, background_navigation_completion_rx) = mpsc::unbounded_channel();
        let (_renderer_tx, renderer_publication_rx) =
            moli_core::renderer_output_transport_channel();
        let (_activity_tx, document_activity_completion_rx) = mpsc::unbounded_channel();
        SchedulerInputReceivers::new(CdpSchedulerEventReceivers {
            protocol_state_request_rx: mpsc::unbounded_channel().1,
            protocol_work_completion_rx: mpsc::unbounded_channel().1,
            document_activity_completion_rx,
            background_event_rx,
            background_navigation_completion_rx,
            renderer_publication_rx,
        })
    }

    #[tokio::test]
    async fn internal_attach_registers_only_a_session_still_open_at_flush_confirmation() {
        for phase in ["open", "closing", "closed"] {
            let mut scheduler = CdpScheduler::new(CdpConnection::new());
            let attached = scheduler
                .conn
                .process_message_with_turn_outcome_async(
                    &json!({"id": 1, "method": "Target.attachToBrowserTarget", "params": {}})
                        .to_string(),
                )
                .await;
            let response = attached
                .into_protocol_event_parts()
                .0
                .into_iter()
                .find(|event| event.protocol_message_id() == Some(1))
                .unwrap()
                .into_protocol_message();
            let session_id = response["result"]["sessionId"].as_str().unwrap();
            if phase == "closed" {
                let _ = scheduler.conn.process_message_with_turn_outcome_async(
                    &json!({"id": 2, "method": "Target.detachFromTarget", "params": {"sessionId": session_id}}).to_string(),
                ).await;
            } else if phase == "closing" {
                let command = ParsedCdpCommand::parse_str(json!({"id": 2, "method": "Target.detachFromTarget", "params": {"sessionId": session_id}}).to_string()).unwrap();
                let mut context = CommandDispatchContext::default();
                let step = scheduler
                    .conn
                    .start_parsed_command_dispatch_with_context(&command, &mut context);
                assert!(matches!(
                    step,
                    moli_protocol::CdpCommandTaskStep::Pending(_)
                ));
            }
            let router = CdpFrontendRouter::new();
            let (sink, _writer) = crate::cdp_writer::CdpSocketSink::with_stalled_writer_for_test(8);
            let registration = register_internal_command_frontend(
                CdpFrontendRegistration::Browser {
                    frontend_id: 1,
                    sink,
                },
                &response,
                &router,
                &scheduler,
            );
            assert_eq!(registration.is_ok(), phase == "open");
            assert_eq!(
                router.unregister_browser_frontend(1).is_some(),
                phase == "open"
            );
        }
    }

    #[tokio::test]
    async fn failed_wait_uses_normal_response_routing_and_finishes_once() {
        tokio::task::LocalSet::new()
            .run_until(async {
                for (failure, writer_available) in [
                    (CommandWaitFailure::Panicked, true),
                    (CommandWaitFailure::Cancelled, true),
                    (CommandWaitFailure::Panicked, false),
                    (CommandWaitFailure::Cancelled, false),
                ] {
                    let router = CdpFrontendRouter::new();
                    let (sink, writer) =
                        crate::cdp_writer::CdpSocketSink::with_stalled_writer_for_test(8);
                    let mut writer = writer_available.then_some(writer);
                    router
                        .register_browser_frontend(1, "browser-session".to_owned(), sink)
                        .unwrap();
                    let command =
                        ParsedCdpCommand::parse_str(r#"{"id":42,"method":"Browser.getVersion"}"#)
                            .unwrap();
                    let Some(CdpPreparedFrontendCommand::Command(command)) =
                        router.prepare_command(1, Ok(command))
                    else {
                        panic!("frontend command must be admitted");
                    };
                    let metadata = command_metadata(&command);
                    let mut scheduler = CdpScheduler::new(CdpConnection::new());
                    let (permit, context) = scheduler.conn.begin_command_response_flush_permit();
                    let identity = CommandWaitIdentity { token: 10, turn: 2 };
                    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
                    let wait_task = spawn_command_wait(
                        identity,
                        std::future::pending::<CompletedActorWork>(),
                        &completion_tx,
                    );
                    let mut in_flight = InFlightCommands::from([(
                        identity.token,
                        InFlightWork::Command(Box::new(InFlightCommandState {
                            metadata,
                            dispatch: CommandDispatchState::pending_command(),
                            output_release_permit: CommandOutputReleasePermit::new(permit, None),
                            command_context: CommandDispatchContext::new(context),
                            pending_turn: identity.turn,
                            owner_session_id: None,
                            wait_lifetime: None,
                            wait_task,
                        })),
                    )]);
                    let mut inputs = empty_scheduler_inputs();
                    let mut deferred = VecDeque::new();
                    let (response_tx, _response_rx) = mpsc::unbounded_channel();
                    let mut adapter = ProtocolAdapterScheduler::default();
                    let continued = handle_pending_command_completion(
                        &router,
                        &mut scheduler,
                        &mut inputs,
                        &mut deferred,
                        &response_tx,
                        &mut adapter,
                        &completion_tx,
                        &mut in_flight,
                        CommandWaitCompletion {
                            identity,
                            result: Err(failure),
                        },
                    )
                    .await;
                    assert!(
                        continued,
                        "a disconnected frontend must not stop other clients"
                    );
                    assert!(in_flight.is_empty());
                    if let Some(writer) = &mut writer {
                        let message = writer.take_message();
                        assert_eq!(message["id"], 42);
                        assert_eq!(message["error"]["code"], -32000);
                        assert_eq!(message["error"]["message"], failure.message());
                    }

                    // Retiring the entry aborts its wait. Its cancellation report
                    // is now stale and cannot publish a second command response.
                    let late = completion_rx.recv().await.unwrap();
                    assert!(
                        handle_pending_command_completion(
                            &router,
                            &mut scheduler,
                            &mut inputs,
                            &mut deferred,
                            &response_tx,
                            &mut adapter,
                            &completion_tx,
                            &mut in_flight,
                            late,
                        )
                        .await
                    );
                    assert!(in_flight.is_empty());
                }
            })
            .await;
    }

    #[tokio::test]
    async fn stale_wait_round_cannot_consume_current_in_flight_state() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let router = CdpFrontendRouter::new();
                let mut scheduler = CdpScheduler::new(CdpConnection::new());
                let (permit, context) = scheduler.conn.begin_command_response_flush_permit();
                let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
                let identity = CommandWaitIdentity { token: 10, turn: 2 };
                let wait_task = spawn_command_wait(
                    identity,
                    std::future::pending::<CompletedActorWork>(),
                    &completion_tx,
                );
                let mut in_flight = InFlightCommands::from([(
                    identity.token,
                    InFlightWork::Command(Box::new(InFlightCommandState {
                        metadata: InFlightCommandMetadata {
                            method: Some("Browser.getVersion".to_owned()),
                            id: Some(42),
                            session_id: None,
                            started: None,
                            executes_page_javascript: false,
                            command_output_session_id: None,
                            internal_response: None,
                        },
                        dispatch: CommandDispatchState::pending_command(),
                        output_release_permit: CommandOutputReleasePermit::new(permit, None),
                        command_context: CommandDispatchContext::new(context),
                        pending_turn: identity.turn,
                        owner_session_id: None,
                        wait_lifetime: None,
                        wait_task,
                    })),
                )]);
                let mut inputs = empty_scheduler_inputs();
                let mut deferred = VecDeque::new();
                let (response_tx, _response_rx) = mpsc::unbounded_channel();
                let mut adapter = ProtocolAdapterScheduler::default();
                assert!(
                    handle_pending_command_completion(
                        &router,
                        &mut scheduler,
                        &mut inputs,
                        &mut deferred,
                        &response_tx,
                        &mut adapter,
                        &completion_tx,
                        &mut in_flight,
                        CommandWaitCompletion {
                            identity: CommandWaitIdentity {
                                turn: 1,
                                ..identity
                            },
                            result: Err(CommandWaitFailure::Cancelled),
                        },
                    )
                    .await
                );
                assert_eq!(in_flight.len(), 1);
                assert_eq!(in_flight[&identity.token].turn(), 2);
            })
            .await;
    }

    #[test]
    #[should_panic(expected = "in-flight CDP command token space exhausted")]
    fn in_flight_command_tokens_never_wrap() {
        let mut next = u64::MAX;
        let _ = take_next_in_flight_command_token(&mut next);
    }

    #[tokio::test]
    async fn revoked_fetch_handler_rejects_a_success_already_queued_before_reenable() {
        tokio::task::LocalSet::new().run_until(async {
            for legacy_transport in [false, true] {
            let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
                moli_protocol::CdpInitialStoragePartition::memory(),
                moli_core::runtime::NavigationRuntimeConfig::default(),
            );
            let (attached, _, predecessor) = scheduler.conn.process_message_with_turn_outcome_async(&json!({
                "id": 1, "method": "Target.attachToTarget",
                "params": {"targetId": moli_protocol::DEFAULT_CDP_PAGE_TARGET_ID, "flatten": true},
            }).to_string()).await.into_protocol_event_parts();
            assert!(predecessor.is_none());
            let attached: Vec<_> = attached.into_iter().map(BackgroundProtocolEvent::into_protocol_message).collect();
            let session_id = attached.iter().find(|message| message["id"] == 1).unwrap()["result"]["sessionId"].as_str().unwrap().to_owned();
            let enable = json!({"id": 2, "sessionId": session_id, "method": "Fetch.enable", "params": {
                "patterns": [{"urlPattern": "*held", "requestStage": "Request"}],
            }});
            let _ = scheduler.conn.process_message_with_turn_outcome_async(&enable.to_string()).await;
            let (paused, _, predecessor) = scheduler.conn.process_message_with_turn_outcome_async(&json!({
                "id": 3, "sessionId": session_id, "method": "Page.navigate",
                "params": {"url": "http://unused.example/held"},
            }).to_string()).await.into_protocol_event_parts();
            assert!(predecessor.is_none());
            let paused: Vec<_> = paused.into_iter().map(BackgroundProtocolEvent::into_protocol_message).collect();
            let request_id = paused.iter().find(|message| message["method"] == "Fetch.requestPaused").unwrap()["params"]["requestId"].clone();
            let mut command = json!({
                "id": 4, "sessionId": session_id, "method": "Fetch.fulfillRequest", "params": {
                    "requestId": request_id, "responseCode": 200,
                    "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
                    "body": "PGh0bWw+PHRpdGxlPnF1ZXVlZDwvdGl0bGU+PC9odG1sPg==",
                },
            });
            if legacy_transport {
                command.as_object_mut().unwrap().remove("sessionId");
                command = json!({"id": 4,"method":"Target.sendMessageToTarget","params":{
                    "sessionId":session_id,"message":command.to_string(),
                }});
            }
            let command = ParsedCdpCommand::parse_str(command.to_string()).unwrap();
            let (step, output_release_permit, command_context) = scheduler.start_command_dispatch(&command);
            let CommandTaskStep::Pending(pending) = step else { panic!("continuation must own a wait"); };
            let lifetime = pending.wait_lifetime().expect("Fetch handler lifetime");
            let completed = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                let wait = pending.wait();
                tokio::pin!(wait);
                loop {
                    tokio::select! {
                        completed = &mut wait => break completed,
                        request = receivers.protocol_state_request_rx.recv() => {
                            scheduler.apply_protocol_state_request(request.expect("the protocol owner is live"));
                        }
                    }
                }
            }).await.expect("the owned fulfillment completes while state requests are driven");
            // The wait already succeeded. Abort cannot remove this queued
            // completion, and the same wire session will immediately re-enable.
            let _ = scheduler.conn.process_message_with_turn_outcome_async(&json!({
                "id": 5, "sessionId": session_id, "method": "Fetch.disable",
            }).to_string()).await;
            let _ = scheduler.conn.process_message_with_turn_outcome_async(&enable.to_string()).await;
            assert!(!lifetime.is_active());
            let identity = CommandWaitIdentity { token: 10, turn: 0 };
            let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
            let wait_task = spawn_command_wait(identity, std::future::pending::<CompletedActorWork>(), &completion_tx);
            let mut in_flight = InFlightCommands::from([(identity.token, InFlightWork::Command(Box::new(InFlightCommandState {
                metadata: command_metadata(&command),
                dispatch: CommandDispatchState::pending_command(),
                output_release_permit, command_context, pending_turn: identity.turn,
                owner_session_id: Some(session_id.clone()), wait_lifetime: Some(lifetime), wait_task,
            })))]);
            let mut inputs = SchedulerInputReceivers::new(receivers);
            let mut deferred = VecDeque::new();
            let (response_tx, _response_rx) = mpsc::unbounded_channel();
            let mut adapter = ProtocolAdapterScheduler::default();
            assert!(handle_pending_command_completion(
                &CdpFrontendRouter::new(), &mut scheduler, &mut inputs, &mut deferred,
                &response_tx, &mut adapter, &completion_tx, &mut in_flight,
                CommandWaitCompletion { identity, result: Ok(CompletedActorWork::Command(completed)) },
            ).await);
            if legacy_transport {
                let InFlightWork::Command(recovery) = &in_flight[&identity.token] else { panic!("the same entry owns recovery"); };
                assert!(recovery.wait_lifetime.is_none(), "protected cleanup must not inherit the revoked Fetch lifetime");
                assert!(recovery.owner_session_id.is_none());
                let recovery = tokio::time::timeout(std::time::Duration::from_secs(30),async {
                    loop {
                        tokio::select! {
                            completion = completion_rx.recv() => {
                                let completion = completion.unwrap();
                                // Dropping the obsolete dummy waiter reports its old turn.
                                if completion.identity.turn != identity.turn { break completion; }
                            }
                            request = inputs.protocol_state_request_rx.recv() => scheduler.apply_protocol_state_request(request.unwrap()),
                        }
                    }
                }).await.expect("protected cleanup finishes even while the old lifetime stays revoked");
                assert!(handle_pending_command_completion(
                    &CdpFrontendRouter::new(),&mut scheduler,&mut inputs,&mut deferred,&response_tx,
                    &mut adapter,&completion_tx,&mut in_flight,recovery,
                ).await);
            }
            assert!(in_flight.is_empty(), "a revoked queued success cannot start another network wait or cancel recovery forever");
            if legacy_transport {
                // The adapter's ordinary workflow already committed the
                // source before its completed outer reply was queued. Recovery
                // must release that reply, without inventing a second failure.
                assert!(!scheduler.conn.has_pending_document_navigation_for_session_owner(Some(&session_id)));
                assert!(inputs.background_navigation_completion_rx.try_recv().is_err());
            } else {
                let failed_navigation = inputs.background_navigation_completion_rx.try_recv().expect("dropping the revoked result must settle its source navigation");
                assert_eq!(failed_navigation.requested_url(), "http://unused.example/held");
            }
            }
        }).await;
    }

    #[tokio::test]
    async fn ready_background_events_precede_runtime_response_without_starving_it() {
        let (background_event_tx, background_event_rx) = mpsc::unbounded_channel();
        let (_background_navigation_tx, background_navigation_completion_rx) =
            mpsc::unbounded_channel();
        let (_renderer_publication_tx, renderer_publication_rx) =
            moli_core::renderer_output_transport_channel();
        let (_activity_tx, document_activity_completion_rx) = mpsc::unbounded_channel();
        let mut receivers = SchedulerInputReceivers::new(CdpSchedulerEventReceivers {
            protocol_state_request_rx: mpsc::unbounded_channel().1,
            protocol_work_completion_rx: mpsc::unbounded_channel().1,
            document_activity_completion_rx,
            background_event_rx,
            background_navigation_completion_rx,
            renderer_publication_rx,
        });
        let (runtime_response_tx, mut runtime_response_rx) = mpsc::unbounded_channel();
        let mut adapter_scheduler = ProtocolAdapterScheduler::default();

        background_event_tx
            .send(BackgroundProtocolEvent::immediate(json!({
                "method": "Network.requestWillBeSent",
                "params": {}
            })))
            .expect("background event receiver should be alive");
        runtime_response_tx
            .send(RuntimeInspectorResponseReady::new(
                42,
                None,
                Err("test response".to_owned()),
            ))
            .expect("runtime response receiver should be alive");

        let first = receivers
            .recv(
                &mut runtime_response_rx,
                true,
                &mut adapter_scheduler,
                false,
            )
            .await
            .expect("ready background event should be received");
        assert!(matches!(first, SchedulerInput::BackgroundEvent(_)));

        background_event_tx
            .send(BackgroundProtocolEvent::immediate(json!({
                "method": "Network.responseReceived",
                "params": {}
            })))
            .expect("background event receiver should remain alive");
        let second = receivers
            .recv(
                &mut runtime_response_rx,
                true,
                &mut adapter_scheduler,
                false,
            )
            .await
            .expect("snapshotted runtime response should be received");
        let SchedulerInput::DeferredRuntimeInspectorResponse(response) = second else {
            panic!("new events must not starve the snapshotted runtime response");
        };
        assert_eq!(response.command_id(), 42);

        let third = receivers
            .recv(
                &mut runtime_response_rx,
                false,
                &mut adapter_scheduler,
                false,
            )
            .await
            .expect("later background event should remain queued");
        assert!(matches!(third, SchedulerInput::BackgroundEvent(_)));
    }

    #[test]
    fn unmatched_runtime_inspector_response_is_not_downgraded_to_protocol_output() {
        let mut scheduler = CdpScheduler::new(CdpConnection::new());

        let output = route_unmatched_runtime_inspector_response(
            &mut scheduler,
            RuntimeInspectorResponseReady::new(
                42,
                None,
                Ok(
                    RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                        42,
                        json!({
                            "id": 42,
                            "result": {}
                        }),
                    ),
                ),
            ),
        );

        assert!(
            output.is_empty(),
            "typed runtime inspector completions without a pending command or registered await must stay internal"
        );
    }
}
