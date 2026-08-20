use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};

use moli_core::{RendererOutputFence, RendererOutputTransportMessage};
use moli_protocol::{
    BackgroundNavigationCompletion, BackgroundProtocolEvent, CdpSchedulerEvent,
    CommandDispatchContext, CompletedCdpCommandDispatch, CompletedPageScreencastCapture,
    DeferredMainDocumentLoadObservationId, ParsedCdpCommand, PendingCdpCommandDispatch,
    conn::{RuntimeInspectorAsyncCompletionReceiver, RuntimeInspectorResponseReady},
};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;

use crate::{
    cdp_frontend::{CdpFrontendCommand, CdpFrontendReceivers},
    cdp_frontend_router::{CdpFrontendRouter, CdpPreparedFrontendCommand},
};

use super::frontend_control::CdpFrontendControlState;
use super::{
    CdpBackgroundEventReceiver, CdpBackgroundNavigationCompletionReceiver, CdpCookieSnapshot,
    CdpOwnerActorLifecycle, CdpRendererPublicationReceiver, CdpScheduler,
    CdpSchedulerEventReceivers, CommandDispatchState, CommandDispatchStepOutput,
    CommandOutputReleasePermit, CommandStartAction, CommandTaskStep, CommandTurnOutput,
    ProtocolAdapterScheduler, ProtocolAdapterSchedulerAdvance, ProtocolAdapterSchedulerInput,
    ProtocolOutputSequence,
};

struct PendingRuntimeDeferredReplyState {
    pending: PendingCdpCommandDispatch,
    dispatch: CommandDispatchState,
    metadata: InFlightCommandMetadata,
    output_release_permit: CommandOutputReleasePermit,
    command_context: CommandDispatchContext,
    output_session_id: Option<String>,
}

#[derive(Clone)]
struct InFlightCommandMetadata {
    method: Option<String>,
    id: Option<u64>,
    session_id: Option<String>,
    started: Option<Instant>,
    executes_page_javascript: bool,
    command_output_session_id: Option<String>,
}

struct InFlightCommandState {
    metadata: InFlightCommandMetadata,
    dispatch: CommandDispatchState,
    output_release_permit: CommandOutputReleasePermit,
    command_context: CommandDispatchContext,
    pending_turn: u64,
}

struct PendingCommandCompletion {
    token: u64,
    completed: CompletedCdpCommandDispatch,
}

struct BlockedCommandDispatch {
    command: ParsedCdpCommand,
    metadata: InFlightCommandMetadata,
}

type InFlightCommands = HashMap<u64, InFlightCommandState>;

fn page_javascript_owner_is_blocked(
    scheduler: &CdpScheduler,
    in_flight_commands: &InFlightCommands,
) -> bool {
    scheduler.has_pending_javascript_dialog()
        || in_flight_commands
            .values()
            .any(|state| state.metadata.executes_page_javascript)
}

enum SchedulerInput {
    BackgroundNavigationCompletion(BackgroundNavigationCompletion),
    BackgroundEvent(BackgroundProtocolEvent),
    RendererPublication(RendererOutputTransportMessage),
    DeferredRuntimeInspectorResponse(Box<RuntimeInspectorResponseReady>),
    AdapterScheduler(ProtocolAdapterSchedulerInput),
}

struct SchedulerInputReceivers {
    background_event_rx: CdpBackgroundEventReceiver,
    background_navigation_completion_rx: CdpBackgroundNavigationCompletionReceiver,
    renderer_publication_rx: CdpRendererPublicationReceiver,
    buffered_renderer_publications: VecDeque<RendererOutputTransportMessage>,
    ready_background_inputs_before_runtime_response: VecDeque<SchedulerInput>,
}

impl SchedulerInputReceivers {
    fn new(receivers: CdpSchedulerEventReceivers) -> Self {
        Self {
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

    /// Receives the next concrete stream message needed by an exact response
    /// cursor fence.
    ///
    /// The transport contains only typed stream controls and concrete
    /// publications. Every message is safe to admit while a Page JavaScript
    /// stack is blocked because protocol never re-enters renderer state to
    /// discover its payload.
    async fn recv_concrete_renderer_transport(&mut self) -> Option<RendererOutputTransportMessage> {
        if let Some(publication) = self.buffered_renderer_publications.pop_front() {
            return Some(publication);
        }
        self.renderer_publication_rx.recv().await
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
        adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
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
                maybe_response = deferred_runtime_response_rx.recv() => {
                    let response = maybe_response?;
                    self.queue_ready_background_inputs_before_runtime_response(response);
                    self.ready_background_inputs_before_runtime_response.pop_front()
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
    let mut adapter_scheduler = ProtocolAdapterScheduler::<CommandDispatchState>::default();
    let mut pending_runtime_deferred_replies: VecDeque<PendingRuntimeDeferredReplyState> =
        VecDeque::new();
    let (pending_command_completion_tx, mut pending_command_completion_rx) =
        mpsc::unbounded_channel();
    let (page_screencast_completion_tx, mut page_screencast_completion_rx) =
        mpsc::unbounded_channel::<CompletedPageScreencastCapture>();
    let mut in_flight_commands = InFlightCommands::new();
    let mut blocked_commands = VecDeque::new();
    let mut next_in_flight_command_token = 0_u64;
    let mut frontend_control = CdpFrontendControlState::default();

    loop {
        if scheduler_input_rx.renderer_publication_rx.is_closed() {
            break;
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
                    let output = route_top_level_background_event(
                        &mut adapter_scheduler,
                        &mut scheduler,
                        event,
                    );
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
                if !frontend_control.handle_request(
                    request,
                    &frontend_router,
                    &mut scheduler,
                    owner_lifecycle.as_ref(),
                )
                .await
                {
                    break;
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
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
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
            let output = route_top_level_background_event(adapter_scheduler, scheduler, event);
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

async fn flush_renderer_publication_predecessor(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
    predecessor: Option<&RendererOutputFence>,
) -> bool {
    let Some(predecessor) = predecessor else {
        return true;
    };
    while !scheduler
        .conn
        .renderer_output_cursor_is_projected(predecessor.cursor())
    {
        let Some(publication) = scheduler_input_rx.recv_concrete_renderer_transport().await else {
            return false;
        };
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
    }
    true
}

async fn ingest_and_flush_renderer_publication(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
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
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
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
        let output = scheduler
            .project_protocol_local_command_outputs_now(self.output_session_id.as_deref())
            .await;
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
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    input: ProtocolAdapterSchedulerInput,
) -> bool {
    let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
    let advance = adapter_scheduler
        .advance_input(scheduler, input, CommandDispatchState::pending_command)
        .await;
    let (kind, observation_id, ok) = match advance {
        ProtocolAdapterSchedulerAdvance::Idle => ("idle", None, true),
        ProtocolAdapterSchedulerAdvance::ClientTurnYielded => ("client_turn_yielded", None, true),
        ProtocolAdapterSchedulerAdvance::DeferredLoadStarted { observation_id } => {
            ("deferred_load_started", Some(observation_id), true)
        }
        ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(output) => {
            let ok = flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await;
            ("protocol_residence_completed", None, ok)
        }
        ProtocolAdapterSchedulerAdvance::DeferredLoadCompleted {
            observation_id,
            attachment,
            output,
        } => {
            let output = attachment.complete_protocol_output(output);
            let ok = flush_protocol_output_with_runtime_deferred_reply_routing(
                frontend_router,
                scheduler,
                pending_runtime_deferred_replies,
                output,
            )
            .await;
            ("deferred_load_completed", Some(observation_id), ok)
        }
        ProtocolAdapterSchedulerAdvance::StaleDeferredLoadCompletion { observation_id } => {
            tracing::debug!(
                ?observation_id,
                "dropping stale shared adapter load completion"
            );
            ("stale_deferred_load_completion", Some(observation_id), true)
        }
    };
    if let Some(started) = trace_started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "protocol_adapter_input_done",
            advance = kind,
            ?observation_id,
            ok,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
    ok
}

fn route_top_level_background_event(
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
    scheduler: &mut CdpScheduler,
    event: BackgroundProtocolEvent,
) -> ProtocolOutputSequence {
    let output = scheduler.route_background_event_around_inflight_navigation(event);
    if output.is_empty() {
        return output;
    }
    let mut events = output.into_background_events();
    let Some(event) = events.pop() else {
        return ProtocolOutputSequence::empty();
    };
    let Some(dispatch) = adapter_scheduler.pending_load_attachment_mut() else {
        return ProtocolOutputSequence::from_background_event(event);
    };
    match dispatch.route_pending_background_event(event) {
        CommandDispatchStepOutput::Emit(output) => output,
    }
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
        start_deferred_runtime_inspector_response_wait(
            command_id,
            session_id,
            response_rx,
            deferred_runtime_response_tx,
        );
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
) {
    let response_tx = response_tx.clone();
    tokio::task::spawn_local(async move {
        let response = response_rx
            .await
            .map_err(|_| "RuntimeDeferredInspectorResponseCanceled".to_owned());
        let _ = response_tx.send(RuntimeInspectorResponseReady::new(
            command_id,
            session_id.as_deref(),
            response,
        ));
    });
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
    pending: PendingCdpCommandDispatch,
    completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
) {
    let completion_tx = completion_tx.clone();
    tokio::task::spawn_local(async move {
        let completed = pending.wait().await;
        let _ = completion_tx.send(PendingCommandCompletion { token, completed });
    });
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
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    blocked_commands: &mut VecDeque<BlockedCommandDispatch>,
    next_in_flight_command_token: &mut u64,
) -> bool {
    let CdpFrontendCommand { frontend_id, raw } = frontend_command;
    let Some(prepared) = frontend_router.prepare_command_str(frontend_id, raw) else {
        return true;
    };
    let command = match prepared {
        CdpPreparedFrontendCommand::Command(command) => command,
        CdpPreparedFrontendCommand::ImmediateResponse {
            frontend_id,
            message,
        } => {
            frontend_router.enqueue_immediate_response(frontend_id, message);
            return true;
        }
    };
    trace_command(&command, "frontend_command_received", None, None, None);
    handle_client_command_with_interleaved_output(
        frontend_router,
        scheduler,
        scheduler_input_rx,
        command,
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
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    blocked_commands: &mut VecDeque<BlockedCommandDispatch>,
    next_in_flight_command_token: &mut u64,
) -> bool {
    let metadata = command_metadata(&command);
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
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
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
                scheduler
                    .project_protocol_local_command_outputs_now(session_id.as_deref())
                    .await,
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
            in_flight_commands.insert(
                token,
                InFlightCommandState {
                    metadata,
                    dispatch: CommandDispatchState::pending_command(),
                    output_release_permit,
                    command_context,
                    pending_turn: 0,
                },
            );
            start_pending_command_wait(token, *pending, pending_command_completion_tx);
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
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
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
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
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
        scheduler
            .project_protocol_local_command_outputs_now(
                metadata.command_output_session_id.as_deref(),
            )
            .await,
    );
    output.prepend_protocol_output(causal_prefix);
    let (
        protocol_output,
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
    true
}

async fn handle_pending_command_completion(
    frontend_router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    scheduler_input_rx: &mut SchedulerInputReceivers,
    pending_runtime_deferred_replies: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    deferred_runtime_response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
    adapter_scheduler: &mut ProtocolAdapterScheduler<CommandDispatchState>,
    pending_command_completion_tx: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight_commands: &mut InFlightCommands,
    completion: PendingCommandCompletion,
) -> bool {
    let Some(mut state) = in_flight_commands.remove(&completion.token) else {
        tracing::debug!(
            token = completion.token,
            "dropping stale pending command completion without actor state"
        );
        return true;
    };
    let completion_kind = completion.completed.kind_name();
    if moli_trace::cdp_runtime_trace_enabled() {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = "command_pending_wait_done",
            pending_turn = state.pending_turn,
            pending_kind = completion_kind,
        );
    }
    let complete_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
    let step = scheduler
        .complete_pending_command_dispatch_with_context(
            completion.completed,
            &mut state.command_context,
        )
        .await;
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
                scheduler
                    .project_protocol_local_command_outputs_now(session_id.as_deref())
                    .await,
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
            state.pending_turn = state.pending_turn.wrapping_add(1);
            let token = completion.token;
            in_flight_commands.insert(token, state);
            start_pending_command_wait(token, *pending, pending_command_completion_tx);
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
        observation_id = ?scheduler_input_observation_id(input),
    );
}

fn scheduler_input_kind(input: &SchedulerInput) -> &'static str {
    match input {
        SchedulerInput::BackgroundNavigationCompletion(_) => "background_navigation_completion",
        SchedulerInput::BackgroundEvent(_) => "background_event",
        SchedulerInput::DeferredRuntimeInspectorResponse(_) => {
            "deferred_runtime_inspector_response"
        }
        SchedulerInput::RendererPublication(_) => "renderer_publication",
        SchedulerInput::AdapterScheduler(ProtocolAdapterSchedulerInput::Turn) => {
            "protocol_adapter_turn"
        }
        SchedulerInput::AdapterScheduler(
            ProtocolAdapterSchedulerInput::DeferredLoadCompletion(_),
        ) => "deferred_load_completion",
    }
}

fn scheduler_input_observation_id(
    input: &SchedulerInput,
) -> Option<DeferredMainDocumentLoadObservationId> {
    match input {
        SchedulerInput::AdapterScheduler(
            ProtocolAdapterSchedulerInput::DeferredLoadCompletion(completion),
        ) => Some(completion.observation_id()),
        _ => None,
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

    #[test]
    #[should_panic(expected = "in-flight CDP command token space exhausted")]
    fn in_flight_command_tokens_never_wrap() {
        let mut next = u64::MAX;
        let _ = take_next_in_flight_command_token(&mut next);
    }

    #[tokio::test]
    async fn ready_background_events_precede_runtime_response_without_starving_it() {
        let (background_event_tx, background_event_rx) = mpsc::unbounded_channel();
        let (_background_navigation_tx, background_navigation_completion_rx) =
            mpsc::unbounded_channel();
        let (_renderer_publication_tx, renderer_publication_rx) =
            moli_core::renderer_output_transport_channel();
        let mut receivers = SchedulerInputReceivers::new(CdpSchedulerEventReceivers {
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
