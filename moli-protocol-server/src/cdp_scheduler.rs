use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, atomic::AtomicU64},
    time::{Duration, Instant},
};

use moli_cookie_jar::StoredCookie;
use moli_core::{
    RendererOutputTransportMessage, page::RendererDocumentLifecycleMilestone,
    runtime::NavigationRuntimeConfig,
};
use moli_protocol::{
    BackgroundNavigationCompletion, BackgroundProtocolEvent, CdpCommandTaskStep, CdpConnection,
    CdpInitialStoragePartition, CdpRendererCommandAccess, CdpSchedulerEvent,
    CdpTargetHostLifecycleObserver, CommandDispatchContext, CompletedCdpCommandDispatch,
    CompletedDeferredMainDocumentLoadCompletion, DeferredMainDocumentLoadCompletionOutputAction,
    DeferredMainDocumentLoadCompletionOutputInterest, DeferredMainDocumentLoadObservationId,
    DeferredMainDocumentLoadPredecessorCandidate, DevToolsPageResidenceIdentity,
    PageScreencastCaptureCompletion, PageScreencastCaptureStart, PageScreencastRegistration,
    PageScreencastSubscriptionStatus, ParsedCdpCommand, PendingCdpCommandDispatch,
    PendingDeferredMainDocumentLoadCompletion, PendingPageScreencastCapture,
    ProtocolSchedulerWorkKind, RuntimeCommandOutputBarriers,
    conn::{RuntimeInspectorResponseReady, RuntimeInspectorResponseReadySender},
    devtools_runtime::{
        DevToolsCommand, DevToolsCommandResult, DevToolsError, DevToolsNavigationWait,
    },
};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::Instant as TokioInstant;

const PAGE_SCREENCAST_RETRY_INTERVAL: Duration = Duration::from_secs(1);

mod actor;
mod adapter_scheduler;
mod command_dispatch;
mod frontend_control;
mod protocol_residence;
mod runtime_command_barrier;
mod runtime_dispatch;

pub(crate) use actor::spawn_cdp_scheduler_actor;
pub(crate) use adapter_scheduler::{
    ProtocolAdapterScheduler, ProtocolAdapterSchedulerAdvance, ProtocolAdapterSchedulerInput,
};
pub(crate) use command_dispatch::{
    CommandDispatchState, CommandDispatchStepOutput, CommandTurnOutput,
};
pub(crate) use frontend_control::{CdpCookieSnapshot, CdpOwnerActorLifecycle};
use protocol_residence::{
    ClientTurnPredecessor, ProtocolSchedulerResidence, ProtocolSchedulerStep, SchedulerQueues,
};
use runtime_command_barrier::CommandOutputReleasePermit;
pub(crate) use runtime_dispatch::{
    DevToolsRuntimeCommandProgress, PendingDevToolsRuntimeDeferredReplyExecution,
};

pub(crate) struct CdpTargetHostIntegration {
    target_id_allocator: Arc<AtomicU64>,
    lifecycle_observer: CdpTargetHostLifecycleObserver,
}

impl CdpTargetHostIntegration {
    pub(crate) fn new(
        target_id_allocator: Arc<AtomicU64>,
        lifecycle_observer: CdpTargetHostLifecycleObserver,
    ) -> Self {
        Self {
            target_id_allocator,
            lifecycle_observer,
        }
    }

    fn install(self, conn: &mut CdpConnection) {
        conn.set_shared_target_id_allocator(self.target_id_allocator);
        conn.set_target_host_lifecycle_observer(self.lifecycle_observer);
    }
}

pub(crate) enum CommandTaskStep {
    Pending(Box<PendingCdpCommandDispatch>),
    Complete(Box<CommandTurnOutput>),
}

pub(crate) struct DevToolsCommandExecution {
    pub(crate) result: Result<DevToolsCommandResult, DevToolsError>,
    pub(crate) protocol_output: ProtocolOutputSequence,
}

pub(crate) struct DevToolsPageCommandExecution {
    pub(crate) execution: DevToolsCommandExecution,
    pub(crate) page_residence: Option<DevToolsPageResidenceIdentity>,
}

// `Dispatch` is the common, short-lived command-start result. Boxing it only
// to match the rare no-payload variant would add one allocation to every
// dispatched protocol command; the bounded stack value is intentional.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CommandStartAction {
    NeedsBackgroundNavigationFlush,
    Dispatch {
        step: CommandTaskStep,
        output_release_permit: CommandOutputReleasePermit,
        command_context: CommandDispatchContext,
    },
}

pub(crate) struct CdpScheduler {
    conn: CdpConnection,
    pending_navigation_background_events: VecDeque<PendingNavigationBackgroundEvent>,
    runtime_command_output_barriers: RuntimeCommandOutputBarriers,
    queues: SchedulerQueues,
    page_screencasts: HashMap<Option<String>, PageScreencastSchedule>,
}

impl CdpScheduler {
    pub(crate) fn page_residence_identity_for_devtools_context(
        &mut self,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
    ) -> Option<DevToolsPageResidenceIdentity> {
        self.conn
            .page_residence_identity_for_devtools_context(context)
    }
}

#[derive(Debug)]
struct PendingNavigationBackgroundEvent {
    target_id: Option<String>,
    event: BackgroundProtocolEvent,
}

#[derive(Debug)]
struct PageScreencastSchedule {
    registration: PageScreencastRegistration,
    interval: Duration,
    next_due_at: TokioInstant,
}

pub(crate) struct ScheduledPageScreencastFrame {
    event: BackgroundProtocolEvent,
    session_id: Option<String>,
    generation: i32,
}

fn page_screencast_interval(every_nth_frame: u32) -> Duration {
    debug_assert!(every_nth_frame > 0);
    Duration::from_secs(u64::from(every_nth_frame))
}

fn next_page_screencast_deadline(now: TokioInstant, interval: Duration) -> TokioInstant {
    now + interval
}

fn append_unique_target_ids(target_ids: &mut Vec<String>, additional: Vec<String>) {
    for target_id in additional {
        if !target_ids.contains(&target_id) {
            target_ids.push(target_id);
        }
    }
}

#[derive(Debug, Default)]
struct ForegroundNavigationNetworkBarrier {
    wait_for_document_response_started: bool,
    pending_subresource_events: VecDeque<BackgroundProtocolEvent>,
}

impl ForegroundNavigationNetworkBarrier {
    fn for_navigation_wait(wait: Option<DevToolsNavigationWait>) -> Self {
        Self {
            wait_for_document_response_started: matches!(wait, Some(DevToolsNavigationWait::Load)),
            pending_subresource_events: VecDeque::new(),
        }
    }

    fn route_event(&mut self, event: BackgroundProtocolEvent) -> ProtocolOutputSequence {
        if !self.wait_for_document_response_started {
            return ProtocolOutputSequence::from_background_event(event);
        }
        if event.is_document_network_response_started() {
            self.wait_for_document_response_started = false;
            let mut output = ProtocolOutputSequence::from_background_event(event);
            output.append(self.drain_pending());
            return output;
        }
        if event.is_non_document_network_event() {
            self.pending_subresource_events.push_back(event);
            return ProtocolOutputSequence::empty();
        }
        ProtocolOutputSequence::from_background_event(event)
    }

    fn route_output(&mut self, output: ProtocolOutputSequence) -> ProtocolOutputSequence {
        let mut routed = ProtocolOutputSequence::empty();
        for event in output.into_background_events() {
            routed.append(self.route_event(event));
        }
        routed
    }

    fn drain_pending(&mut self) -> ProtocolOutputSequence {
        ProtocolOutputSequence::from_background_events(
            self.pending_subresource_events.drain(..).collect(),
        )
    }

    fn finish(mut self) -> ProtocolOutputSequence {
        self.wait_for_document_response_started = false;
        self.drain_pending()
    }
}

pub(crate) type CdpBackgroundEventReceiver = mpsc::UnboundedReceiver<BackgroundProtocolEvent>;
pub(crate) type CdpBackgroundNavigationCompletionReceiver =
    mpsc::UnboundedReceiver<BackgroundNavigationCompletion>;
pub(crate) type CdpRendererPublicationReceiver = moli_core::RendererOutputTransportReceiver;
pub(crate) struct CdpSchedulerEventReceivers {
    pub(crate) background_event_rx: CdpBackgroundEventReceiver,
    pub(crate) background_navigation_completion_rx: CdpBackgroundNavigationCompletionReceiver,
    pub(crate) renderer_publication_rx: CdpRendererPublicationReceiver,
}

/// One move-owned scheduler input selected from the independent producer
/// channels used by direct CDP/WebDriver command execution.
///
/// Receiving an input and applying it are deliberately separate operations.
/// Several command waits race input readiness against a renderer reply or a
/// deadline. If such a race also awaited projection inside the selected
/// future, a later-ready reply could cancel that future after `recv()` had
/// removed a concrete publication from its channel. Keeping the selected
/// value in the caller makes ownership unambiguous: once dequeued, the input
/// is completed before the command wait selects again.
pub(crate) enum CdpSchedulerInterleavedInput {
    BackgroundNavigationCompletion(BackgroundNavigationCompletion),
    BackgroundEvent(BackgroundProtocolEvent),
    RendererPublication(RendererOutputTransportMessage),
}

impl CdpSchedulerEventReceivers {
    pub(crate) async fn recv_interleaved_input(&mut self) -> Option<CdpSchedulerInterleavedInput> {
        tokio::select! {
            biased;
            maybe_completion = self.background_navigation_completion_rx.recv() => {
                maybe_completion.map(
                    CdpSchedulerInterleavedInput::BackgroundNavigationCompletion,
                )
            }
            maybe_event = self.background_event_rx.recv() => {
                maybe_event.map(CdpSchedulerInterleavedInput::BackgroundEvent)
            }
            maybe_publication = self.renderer_publication_rx.recv() => {
                maybe_publication.map(CdpSchedulerInterleavedInput::RendererPublication)
            }
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ProtocolOutputSequence {
    events: Vec<BackgroundProtocolEvent>,
}

/// Terminal renderer transport together with the concrete protocol prefix
/// already projected before that boundary.
///
/// Direct CDP/WebDriver paths buffer their protocol output until the command
/// boundary. Dropping that prefix while returning the terminal error would
/// violate the same notification-before-response rule enforced by the actor.
pub(crate) struct RendererOutputTransportFailure {
    protocol_output: ProtocolOutputSequence,
    error: DevToolsError,
}

impl RendererOutputTransportFailure {
    fn new(protocol_output: ProtocolOutputSequence, error: DevToolsError) -> Self {
        Self {
            protocol_output,
            error,
        }
    }

    pub(crate) fn into_parts(self) -> (ProtocolOutputSequence, DevToolsError) {
        (self.protocol_output, self.error)
    }

    fn without_output(error: DevToolsError) -> Self {
        Self::new(ProtocolOutputSequence::empty(), error)
    }
}

impl ProtocolOutputSequence {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn from_background_event(event: BackgroundProtocolEvent) -> Self {
        Self {
            events: vec![event],
        }
    }

    pub(crate) fn from_background_events(events: Vec<BackgroundProtocolEvent>) -> Self {
        Self { events }
    }

    #[cfg(test)]
    pub(crate) fn from_messages(messages: Vec<Value>) -> Self {
        Self {
            events: messages
                .into_iter()
                .map(BackgroundProtocolEvent::immediate)
                .collect(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    fn contains_document_load_for_since(
        &self,
        key: &moli_protocol::DevToolsDocumentLifecycleWaitKey,
        start_index: usize,
    ) -> bool {
        self.events
            .iter()
            .skip(start_index)
            .any(|event| event.matches_document_load_wait_key(key))
    }

    fn contains_download_start_for_frame_since(&self, frame_id: &str, start_index: usize) -> bool {
        self.events.iter().skip(start_index).any(|event| {
            event
                .download_will_begin_frame_id()
                .is_some_and(|event_frame_id| event_frame_id == frame_id)
        })
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        self.events.append(&mut other.events);
    }

    fn navigation_gate_target_ids(&self, conn: &CdpConnection) -> Vec<String> {
        let mut target_ids = Vec::new();
        for event in &self.events {
            let Some(target_id) = conn.background_navigation_target_id_for_event(event) else {
                // A publication is released atomically. If even one event has
                // no exact owner, keep the whole batch behind the conservative
                // connection-wide gate rather than attributing it to a known
                // sibling event's target.
                return Vec::new();
            };
            if !target_ids.contains(&target_id) {
                target_ids.push(target_id);
            }
        }
        target_ids
    }

    fn split_network_observations(self) -> (Self, Self) {
        let mut network = Vec::new();
        let mut remaining = Vec::new();
        for event in self.events {
            if event.is_network_protocol_observation() {
                network.push(event);
            } else {
                remaining.push(event);
            }
        }
        (
            Self::from_background_events(network),
            Self::from_background_events(remaining),
        )
    }

    pub(crate) fn take_protocol_events_with_id(
        &mut self,
        command_id: u64,
    ) -> Vec<BackgroundProtocolEvent> {
        let mut retained = Vec::new();
        let mut matches = Vec::new();
        for event in self.events.drain(..) {
            if event.protocol_message_id() == Some(command_id) {
                matches.push(event);
            } else {
                retained.push(event);
            }
        }
        self.events = retained;
        matches
    }

    pub(crate) fn split_next_protocol_message_with_any_id(
        &mut self,
        command_ids: &[u64],
    ) -> Option<(Self, u64, BackgroundProtocolEvent)> {
        let mut events = std::mem::take(&mut self.events).into_iter();
        let mut prefix = Vec::new();
        while let Some(event) = events.next() {
            if let Some(command_id) = event.protocol_message_id()
                && command_ids.contains(&command_id)
            {
                self.events = events.collect();
                return Some((Self::from_background_events(prefix), command_id, event));
            }
            prefix.push(event);
        }
        self.events = prefix;
        None
    }

    pub(crate) fn split_next_runtime_response_ready(
        &mut self,
    ) -> Option<(Self, RuntimeInspectorResponseReady)> {
        let mut events = std::mem::take(&mut self.events).into_iter();
        let mut prefix = Vec::new();
        while let Some(event) = events.next() {
            match event.take_runtime_inspector_response_ready() {
                Ok(response) => {
                    self.events = events.collect();
                    return Some((Self::from_background_events(prefix), response));
                }
                Err(event) => prefix.push(event),
            }
        }
        self.events = prefix;
        None
    }

    pub(crate) fn len(&self) -> usize {
        self.events.len()
    }

    #[cfg(test)]
    pub(crate) fn into_messages(self) -> Vec<Value> {
        self.events
            .into_iter()
            .filter_map(|event| {
                event
                    .has_protocol_wire_message()
                    .then(|| event.into_protocol_message())
            })
            .collect()
    }

    pub(crate) fn into_deliveries(self) -> Vec<BackgroundProtocolEvent> {
        self.events
    }

    pub(crate) fn into_background_events(self) -> Vec<BackgroundProtocolEvent> {
        self.events
    }
}

#[cfg(test)]
pub(crate) fn drain_pending_background_events(
    background_event_rx: &mut CdpBackgroundEventReceiver,
) -> ProtocolOutputSequence {
    let mut events = Vec::new();
    while let Ok(event) = background_event_rx.try_recv() {
        events.push(event);
    }
    ProtocolOutputSequence::from_background_events(events)
}

impl CdpScheduler {
    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        self.conn.has_pending_javascript_dialog()
    }

    pub(crate) fn set_automation_javascript_dialog_handler_enabled(
        &mut self,
        enabled: bool,
    ) -> bool {
        self.conn
            .set_automation_javascript_dialog_handler_enabled(enabled)
    }

    pub(crate) fn set_runtime_inspector_response_ready_sender(
        &mut self,
        sender: RuntimeInspectorResponseReadySender,
    ) {
        self.conn
            .set_runtime_inspector_response_ready_sender(sender);
    }

    fn register_page_screencast(
        &mut self,
        registration: PageScreencastRegistration,
        now: TokioInstant,
    ) {
        let session_id = registration.session_id().map(str::to_owned);
        let interval = page_screencast_interval(registration.every_nth_frame());
        self.page_screencasts.insert(
            session_id,
            PageScreencastSchedule {
                registration,
                interval,
                next_due_at: now,
            },
        );
    }

    fn page_screencast_schedule_matches(
        &self,
        session_id: &Option<String>,
        generation: i32,
    ) -> bool {
        self.page_screencasts
            .get(session_id)
            .is_some_and(|schedule| schedule.registration.generation() == generation)
    }

    pub(crate) fn next_page_screencast_deadline(&mut self) -> Option<TokioInstant> {
        let schedules = self
            .page_screencasts
            .iter()
            .map(|(session_id, schedule)| {
                (
                    session_id.clone(),
                    schedule.registration.clone(),
                    schedule.next_due_at,
                )
            })
            .collect::<Vec<_>>();
        let mut next_deadline = None;
        for (session_id, registration, deadline) in schedules {
            match self.conn.page_screencast_subscription_status(&registration) {
                PageScreencastSubscriptionStatus::Inactive => {
                    if self.page_screencast_schedule_matches(&session_id, registration.generation())
                    {
                        self.page_screencasts.remove(&session_id);
                    }
                }
                PageScreencastSubscriptionStatus::Ready => {
                    next_deadline = Some(
                        next_deadline
                            .map_or(deadline, |current: TokioInstant| current.min(deadline)),
                    );
                }
                PageScreencastSubscriptionStatus::CaptureInProgress
                | PageScreencastSubscriptionStatus::AwaitingAck => {}
            }
        }
        next_deadline
    }

    pub(crate) fn start_due_page_screencast_captures(
        &mut self,
        now: TokioInstant,
    ) -> Vec<PendingPageScreencastCapture> {
        let due = self
            .page_screencasts
            .iter()
            .filter(|(_, schedule)| schedule.next_due_at <= now)
            .map(|(session_id, schedule)| (session_id.clone(), schedule.registration.clone()))
            .collect::<Vec<_>>();
        let mut pending = Vec::with_capacity(due.len());
        for (session_id, registration) in due {
            match self.conn.page_screencast_subscription_status(&registration) {
                PageScreencastSubscriptionStatus::Inactive => {
                    if self.page_screencast_schedule_matches(&session_id, registration.generation())
                    {
                        self.page_screencasts.remove(&session_id);
                    }
                }
                PageScreencastSubscriptionStatus::CaptureInProgress
                | PageScreencastSubscriptionStatus::AwaitingAck => {}
                PageScreencastSubscriptionStatus::Ready => {
                    match self.conn.start_page_screencast_frame_capture(&registration) {
                        PageScreencastCaptureStart::Pending(capture) => pending.push(capture),
                        PageScreencastCaptureStart::Retry => {
                            if let Some(schedule) = self.page_screencasts.get_mut(&session_id)
                                && schedule.registration.generation() == registration.generation()
                            {
                                schedule.next_due_at = next_page_screencast_deadline(
                                    now,
                                    PAGE_SCREENCAST_RETRY_INTERVAL,
                                );
                            }
                        }
                        PageScreencastCaptureStart::Stale => {
                            if self.page_screencast_schedule_matches(
                                &session_id,
                                registration.generation(),
                            ) {
                                self.page_screencasts.remove(&session_id);
                            }
                        }
                    }
                }
            }
        }
        pending
    }

    pub(crate) fn complete_page_screencast_capture(
        &mut self,
        completed: moli_protocol::CompletedPageScreencastCapture,
        now: TokioInstant,
    ) -> Option<ScheduledPageScreencastFrame> {
        let session_id = completed.session_id().map(str::to_owned);
        let generation = completed.generation();
        let completion = self.conn.complete_page_screencast_frame_capture(completed);
        if !self.page_screencast_schedule_matches(&session_id, generation) {
            return None;
        }
        match completion {
            PageScreencastCaptureCompletion::Frame(event) => Some(ScheduledPageScreencastFrame {
                event,
                session_id,
                generation,
            }),
            PageScreencastCaptureCompletion::Retry => {
                let schedule = self
                    .page_screencasts
                    .get_mut(&session_id)
                    .expect("matching screencast schedule must exist");
                schedule.next_due_at =
                    next_page_screencast_deadline(now, PAGE_SCREENCAST_RETRY_INTERVAL);
                None
            }
            PageScreencastCaptureCompletion::Stale => {
                self.page_screencasts.remove(&session_id);
                None
            }
        }
    }

    pub(crate) fn note_page_screencast_frame_emitted(
        &mut self,
        session_id: &Option<String>,
        generation: i32,
        now: TokioInstant,
    ) {
        if let Some(schedule) = self.page_screencasts.get_mut(session_id)
            && schedule.registration.generation() == generation
        {
            schedule.next_due_at = next_page_screencast_deadline(now, schedule.interval);
        }
    }

    pub(crate) fn route_registered_runtime_inspector_response(
        &mut self,
        response: RuntimeInspectorResponseReady,
    ) -> ProtocolOutputSequence {
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        self.conn.route_registered_runtime_inspector_response_into(
            response,
            &mut response_events,
            &mut background_events,
        );
        let mut output = ProtocolOutputSequence::from_background_events(background_events);
        output.append(ProtocolOutputSequence::from_background_events(
            response_events,
        ));
        output
    }

    fn new(conn: CdpConnection) -> Self {
        Self {
            conn,
            pending_navigation_background_events: VecDeque::new(),
            runtime_command_output_barriers: RuntimeCommandOutputBarriers::default(),
            queues: SchedulerQueues::default(),
            page_screencasts: HashMap::new(),
        }
    }

    pub(crate) fn new_with_initial_state_runtime_config(
        initial_storage_partition: CdpInitialStoragePartition,
        navigation_runtime_config: NavigationRuntimeConfig,
    ) -> (Self, CdpSchedulerEventReceivers) {
        Self::new_with_initial_state_runtime_config_and_target_host_integration(
            initial_storage_partition,
            navigation_runtime_config,
            None,
        )
    }

    pub(crate) fn new_with_initial_state_runtime_config_and_target_host_integration(
        initial_storage_partition: CdpInitialStoragePartition,
        navigation_runtime_config: NavigationRuntimeConfig,
        target_host_integration: Option<CdpTargetHostIntegration>,
    ) -> (Self, CdpSchedulerEventReceivers) {
        let mut scheduler = Self::new(
            CdpConnection::new_with_initial_storage_partition_and_runtime_config(
                initial_storage_partition,
                navigation_runtime_config,
            ),
        );
        if let Some(target_host_integration) = target_host_integration {
            target_host_integration.install(&mut scheduler.conn);
        }
        scheduler.conn.install_default_browser_target();
        scheduler.conn.enable_default_target_on_auto_attach();
        let (background_event_tx, background_event_rx) = mpsc::unbounded_channel();
        scheduler
            .conn
            .set_background_event_sender(background_event_tx);
        let (background_navigation_completion_tx, background_navigation_completion_rx) =
            mpsc::unbounded_channel();
        scheduler
            .conn
            .set_background_navigation_completion_sender(background_navigation_completion_tx);
        let (renderer_publication_tx, renderer_publication_rx) =
            moli_core::renderer_output_transport_channel();
        scheduler
            .conn
            .set_renderer_publication_sender(renderer_publication_tx);
        (
            scheduler,
            CdpSchedulerEventReceivers {
                background_event_rx,
                background_navigation_completion_rx,
                renderer_publication_rx,
            },
        )
    }

    pub(crate) fn start_command_or_request_background_navigation_flush(
        &mut self,
        command: &ParsedCdpCommand,
    ) -> CommandStartAction {
        if self.command_waits_for_navigation_flush(command) {
            return CommandStartAction::NeedsBackgroundNavigationFlush;
        }
        let (step, output_release_permit, command_context) = self.start_command_dispatch(command);
        CommandStartAction::Dispatch {
            step,
            output_release_permit,
            command_context,
        }
    }

    fn start_command_dispatch(
        &mut self,
        command: &ParsedCdpCommand,
    ) -> (
        CommandTaskStep,
        CommandOutputReleasePermit,
        CommandDispatchContext,
    ) {
        let (response_flush_permit, response_flush_context) =
            self.conn.begin_command_response_flush_permit();
        let mut command_context = CommandDispatchContext::new(response_flush_context);
        let dispatch_step = self
            .conn
            .start_parsed_command_dispatch_with_context(command, &mut command_context);
        // Dispatch registers a session-local renderer call id before the
        // renderer task can be observed by this actor. The response barrier
        // must use that exact id rather than infer one from the frontend CDP
        // request id.
        let runtime_output_barrier = if command.runtime_command_executes_page_javascript() {
            self.runtime_command_output_barriers.admit(
                &self.conn,
                command.request().id(),
                command.command_output_session_id(),
            )
        } else {
            None
        };
        let output_release_permit =
            CommandOutputReleasePermit::new(response_flush_permit, runtime_output_barrier);
        let step = match dispatch_step {
            CdpCommandTaskStep::Pending(mut pending) => {
                let scheduler_events = pending.take_scheduler_events();
                self.apply_scheduler_events(scheduler_events);
                CommandTaskStep::Pending(pending)
            }
            CdpCommandTaskStep::Complete(result) => {
                let (
                    events,
                    post_renderer_output_events,
                    renderer_output_boundary,
                    post_response_events,
                    scheduler_events,
                    renderer_output_predecessor,
                ) = result.into_renderer_owner_turn_parts();
                CommandTaskStep::Complete(Box::new(
                    CommandTurnOutput::new_with_post_response_events(
                        self.route_background_events_around_inflight_navigation(events),
                        self.route_background_events_around_inflight_navigation(
                            post_response_events,
                        )
                        .into_background_events(),
                        scheduler_events,
                    )
                    .with_renderer_output_boundary(
                        renderer_output_boundary,
                        self.route_background_events_around_inflight_navigation(
                            post_renderer_output_events,
                        ),
                    )
                    .with_renderer_output_predecessor(renderer_output_predecessor),
                ))
            }
        };
        (step, output_release_permit, command_context)
    }

    pub(crate) async fn complete_pending_command_dispatch_with_context(
        &mut self,
        completed: CompletedCdpCommandDispatch,
        command_context: &mut CommandDispatchContext,
    ) -> CommandTaskStep {
        match self
            .conn
            .complete_pending_command_dispatch_with_context(completed, command_context)
            .await
        {
            CdpCommandTaskStep::Pending(mut pending) => {
                let scheduler_events = pending.take_scheduler_events();
                self.apply_scheduler_events(scheduler_events);
                CommandTaskStep::Pending(pending)
            }
            CdpCommandTaskStep::Complete(result) => {
                let (
                    events,
                    post_renderer_output_events,
                    renderer_output_boundary,
                    post_response_events,
                    scheduler_events,
                    renderer_output_predecessor,
                ) = result.into_renderer_owner_turn_parts();
                CommandTaskStep::Complete(Box::new(
                    CommandTurnOutput::new_with_post_response_events(
                        self.route_background_events_around_inflight_navigation(events),
                        self.route_background_events_around_inflight_navigation(
                            post_response_events,
                        )
                        .into_background_events(),
                        scheduler_events,
                    )
                    .with_renderer_output_boundary(
                        renderer_output_boundary,
                        self.route_background_events_around_inflight_navigation(
                            post_renderer_output_events,
                        ),
                    )
                    .with_renderer_output_predecessor(renderer_output_predecessor),
                ))
            }
        }
    }

    pub(crate) fn snapshot_profile_backed_cookies(&mut self) -> Option<Vec<StoredCookie>> {
        self.conn.snapshot_profile_backed_cookies()
    }

    pub(crate) async fn execute_devtools_command_with_protocol_messages(
        &mut self,
        command: DevToolsCommand,
    ) -> DevToolsCommandExecution {
        self.execute_devtools_command_with_protocol_messages_inner(None, command, true, None)
            .await
    }

    async fn execute_devtools_command_with_protocol_messages_inner(
        &mut self,
        receivers: Option<&mut CdpSchedulerEventReceivers>,
        command: DevToolsCommand,
        drain_load_completion: bool,
        background_command_id: Option<u64>,
    ) -> DevToolsCommandExecution {
        let navigation_wait = devtools_navigation_wait(&command);
        let navigation_context = command.context().clone();
        let outcome = self
            .conn
            .execute_devtools_command_with_protocol_events_with_background_command_id(
                command,
                background_command_id,
            )
            .await;
        let (mut result, scheduler_events, protocol_events, renderer_output_predecessor) =
            outcome.into_complete_parts();
        self.apply_scheduler_events(scheduler_events);
        let mut protocol_output = ProtocolOutputSequence::empty();
        if let Some(predecessor) = renderer_output_predecessor {
            if let Some(receivers) = receivers {
                match self
                    .project_renderer_output_predecessor_before_devtools_result(
                        receivers,
                        &predecessor,
                    )
                    .await
                {
                    Ok(output) => protocol_output.append(output),
                    Err(failure) => {
                        let (output, error) = failure.into_parts();
                        protocol_output.append(output);
                        result = Err(error);
                    }
                }
            } else if self
                .conn
                .renderer_output_cursor_is_projected(predecessor.cursor())
            {
                protocol_output.append(
                    self.complete_renderer_output_predecessor_before_runtime_response(&predecessor)
                        .await,
                );
            } else {
                result = Err(DevToolsError::new(
                    moli_protocol::devtools_runtime::DevToolsErrorKind::Internal,
                    "DevTools command produced renderer output without an ingress receiver",
                ));
            }
        }
        protocol_output
            .append(self.route_background_events_around_inflight_navigation(protocol_events));
        if drain_load_completion
            && result.is_ok()
            && matches!(navigation_wait, Some(DevToolsNavigationWait::Load))
        {
            protocol_output.append(
                self.drain_deferred_main_document_load_completion_for_wait(&navigation_context)
                    .await,
            );
        }
        DevToolsCommandExecution {
            result,
            protocol_output,
        }
    }

    pub(crate) async fn execute_internal_protocol_message(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        message: Value,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let outcome = self
            .conn
            .process_message_with_turn_outcome_async(&message.to_string())
            .await;
        self.apply_renderer_owner_turn_outcome(receivers, outcome)
            .await
    }

    pub(crate) fn enable_network_listener_for_target(&mut self, target_id: &str) -> bool {
        self.conn.enable_network_listener_for_target(target_id)
    }

    pub(crate) fn disable_network_listener_for_target(&mut self, target_id: &str) -> bool {
        self.conn.disable_network_listener_for_target(target_id)
    }

    pub(crate) fn enable_file_dialog_opened_listener_for_target(
        &mut self,
        target_id: &str,
    ) -> bool {
        self.conn
            .enable_file_dialog_opened_listener_for_target(target_id)
    }

    pub(crate) fn disable_file_dialog_opened_listener_for_target(
        &mut self,
        target_id: &str,
    ) -> bool {
        self.conn
            .disable_file_dialog_opened_listener_for_target(target_id)
    }

    pub(crate) fn enable_webdriver_bidi_download_events(&mut self) -> bool {
        self.conn.enable_webdriver_bidi_download_events()
    }

    pub(crate) fn disable_webdriver_bidi_download_events(&mut self) -> bool {
        self.conn.disable_webdriver_bidi_download_events()
    }

    pub(crate) fn worker_target_id_for_session(&self, session_id: Option<&str>) -> Option<String> {
        self.conn.worker_target_id_for_session(session_id)
    }

    pub(crate) async fn enable_runtime_listener_for_target(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        target_id: &str,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let Some(outcome) = self
            .conn
            .enable_runtime_listener_for_target(target_id)
            .await
        else {
            return Ok(ProtocolOutputSequence::empty());
        };
        self.apply_renderer_owner_turn_outcome(receivers, outcome)
            .await
    }

    pub(crate) async fn disable_runtime_listener_for_target(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        target_id: &str,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let Some(outcome) = self
            .conn
            .disable_runtime_listener_for_target(target_id)
            .await
        else {
            return Ok(ProtocolOutputSequence::empty());
        };
        self.apply_renderer_owner_turn_outcome(receivers, outcome)
            .await
    }

    pub(crate) fn replace_target_discovery_enabled(&mut self, enabled: bool) -> bool {
        self.conn.replace_root_target_discovery_enabled(enabled)
    }

    pub(crate) async fn execute_devtools_command_with_external_load_wait(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        self.execute_devtools_command_with_external_load_wait_and_protocol_messages(
            receivers, command,
        )
        .await
        .result
    }

    pub(crate) async fn execute_devtools_command_with_external_load_wait_and_protocol_messages(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
    ) -> DevToolsCommandExecution {
        self.execute_devtools_command_with_external_load_wait_and_protocol_messages_inner(
            receivers, command, None, None, None,
        )
        .await
        .execution
    }

    pub(crate) async fn execute_devtools_command_with_external_load_wait_and_protocol_messages_background_command_id(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
        background_command_id: Option<u64>,
    ) -> DevToolsCommandExecution {
        self.execute_devtools_command_with_external_load_wait_and_protocol_messages_inner(
            receivers,
            command,
            None,
            background_command_id,
            None,
        )
        .await
        .execution
    }

    pub(crate) async fn execute_devtools_command_with_external_load_wait_and_page_residence(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
        timeout: Option<std::time::Duration>,
        expected_page: Option<&DevToolsPageResidenceIdentity>,
    ) -> DevToolsPageCommandExecution {
        self.execute_devtools_command_with_external_load_wait_and_protocol_messages_inner(
            receivers,
            command,
            timeout,
            None,
            expected_page,
        )
        .await
    }

    async fn execute_devtools_command_with_external_load_wait_and_protocol_messages_inner(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
        timeout: Option<std::time::Duration>,
        background_command_id: Option<u64>,
        expected_page: Option<&DevToolsPageResidenceIdentity>,
    ) -> DevToolsPageCommandExecution {
        let navigation_wait = devtools_navigation_wait(&command);
        let navigation_lifecycle_milestone =
            devtools_navigation_lifecycle_milestone(navigation_wait);
        let navigation_context = command.context().clone();
        let validate_root_document_lifecycle = navigation_lifecycle_milestone.is_some()
            && self
                .conn
                .devtools_context_routes_to_top_level_target(&navigation_context);
        let mut foreground_navigation_network_barrier =
            ForegroundNavigationNetworkBarrier::for_navigation_wait(navigation_wait);
        let mut protocol_output = match self
            .drain_inflight_background_navigation_before_internal_command(
                receivers,
                &navigation_context,
            )
            .await
        {
            Ok(output) => output,
            Err(failure) => {
                let (protocol_output, error) = failure.into_parts();
                return DevToolsPageCommandExecution {
                    execution: DevToolsCommandExecution {
                        result: Err(error),
                        protocol_output,
                    },
                    page_residence: None,
                };
            }
        };
        // Background navigation completion is deliberately drained before a
        // command starts. Capture and authorize the Page only after that
        // drain, so a DOM reference cannot pass a pre-drain check and then be
        // dispatched to the replacement Page.
        let page_residence = self.page_residence_identity_for_devtools_context(&navigation_context);
        if expected_page.is_some_and(|expected| page_residence.as_ref() != Some(expected)) {
            return DevToolsPageCommandExecution {
                execution: DevToolsCommandExecution {
                    result: Err(DevToolsError::new(
                        moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchNode,
                        "DOM reference belongs to a replaced Page",
                    )),
                    protocol_output,
                },
                page_residence,
            };
        }
        let navigation_command_output_start = protocol_output.len();
        let mut execution =
            if runtime_dispatch::devtools_command_uses_interleaved_runtime_dispatch(&command) {
                match timeout {
                    Some(timeout) => {
                        self.execute_devtools_runtime_command_with_interleaved_progress_timeout(
                            receivers, command, timeout,
                        )
                        .await
                    }
                    None => {
                        self.execute_devtools_runtime_command_with_interleaved_progress(
                            receivers, command,
                        )
                        .await
                    }
                }
            } else {
                match timeout {
                    Some(timeout) => {
                        match tokio::time::timeout(
                            timeout,
                            self.execute_devtools_command_with_protocol_messages_inner(
                                Some(&mut *receivers),
                                command,
                                false,
                                background_command_id,
                            ),
                        )
                        .await
                        {
                            Ok(execution) => execution,
                            Err(_) => DevToolsCommandExecution {
                                result: Err(DevToolsError::new(
                                    moli_protocol::devtools_runtime::DevToolsErrorKind::Timeout,
                                    "script timed out",
                                )),
                                protocol_output: ProtocolOutputSequence::empty(),
                            },
                        }
                    }
                    None => {
                        self.execute_devtools_command_with_protocol_messages_inner(
                            Some(&mut *receivers),
                            command,
                            false,
                            background_command_id,
                        )
                        .await
                    }
                }
            };
        let expected_document_loader_id = devtools_navigation_result_loader_id(&execution.result);
        let mut document_lifecycle_wait_key =
            if validate_root_document_lifecycle && execution.result.is_ok() {
                expected_document_loader_id
                    .as_deref()
                    .zip(navigation_lifecycle_milestone)
                    .and_then(|(loader_id, milestone)| {
                        self.conn.capture_devtools_document_lifecycle_wait_key(
                            &navigation_context,
                            loader_id,
                            milestone,
                        )
                    })
            } else {
                None
            };
        protocol_output
            .append(foreground_navigation_network_barrier.route_output(execution.protocol_output));
        execution.protocol_output = protocol_output;
        let output = self
            .complete_ready_protocol_residences_after_command()
            .await;
        execution
            .protocol_output
            .append(foreground_navigation_network_barrier.route_output(output));
        if execution.result.is_ok() && matches!(navigation_wait, Some(DevToolsNavigationWait::Load))
        {
            let output = self
                .drain_deferred_main_document_load_completion_until_complete(
                    receivers,
                    &navigation_context,
                )
                .await;
            let output = match output {
                Ok(output) => output,
                Err(failure) => {
                    let (output, error) = failure.into_parts();
                    execution.result = Err(error);
                    output
                }
            };
            execution
                .protocol_output
                .append(foreground_navigation_network_barrier.route_output(output));
            let output = self
                .complete_ready_protocol_residences_after_command()
                .await;
            execution
                .protocol_output
                .append(foreground_navigation_network_barrier.route_output(output));
        }
        if execution.result.is_ok()
            && validate_root_document_lifecycle
            && document_lifecycle_wait_key.is_none()
        {
            document_lifecycle_wait_key = expected_document_loader_id
                .as_deref()
                .zip(navigation_lifecycle_milestone)
                .and_then(|(loader_id, milestone)| {
                    self.conn.capture_devtools_document_lifecycle_wait_key(
                        &navigation_context,
                        loader_id,
                        milestone,
                    )
                });
        }
        if execution.result.is_ok()
            && matches!(
                navigation_lifecycle_milestone,
                Some(RendererDocumentLifecycleMilestone::DomContentLoaded)
            )
            && let Some(key) = document_lifecycle_wait_key.as_ref()
        {
            let output = self
                .wait_for_document_lifecycle_observer(receivers, &navigation_context, key)
                .await;
            let output = match output {
                Ok(output) => output,
                Err(failure) => {
                    let (output, error) = failure.into_parts();
                    execution.result = Err(error);
                    output
                }
            };
            execution
                .protocol_output
                .append(foreground_navigation_network_barrier.route_output(output));
        }
        if execution.result.is_ok() && validate_root_document_lifecycle {
            let expected_download_frame_id = document_lifecycle_wait_key
                .as_ref()
                .map(|key| key.frame_id())
                .or_else(|| {
                    navigation_context
                        .target_id
                        .as_ref()
                        .map(|target_id| target_id.as_str())
                });
            let observed_download = expected_download_frame_id.is_some_and(|frame_id| {
                execution
                    .protocol_output
                    .contains_download_start_for_frame_since(
                        frame_id,
                        navigation_command_output_start,
                    )
            });
            let observed_lifecycle_protocol_event =
                matches!(
                    navigation_lifecycle_milestone,
                    Some(RendererDocumentLifecycleMilestone::Load)
                ) && document_lifecycle_wait_key.as_ref().is_some_and(|key| {
                    execution
                        .protocol_output
                        .contains_document_load_for_since(key, navigation_command_output_start)
                });
            // A pre-commit navigation can legitimately have no renderer key yet
            // (for example while an auth challenge owns the response). Its
            // background command response remains the completion authority.
            if !observed_download && !observed_lifecycle_protocol_event {
                if let Some(key) = document_lifecycle_wait_key.as_ref() {
                    let wait_state = self
                        .conn
                        .devtools_document_lifecycle_wait_state(&navigation_context, key);
                    if let Some(error) =
                        devtools_document_lifecycle_wait_error(wait_state, key.milestone())
                    {
                        execution.result = Err(error);
                    }
                } else if !self
                    .conn
                    .devtools_context_routes_to_top_level_target(&navigation_context)
                {
                    execution.result = Err(DevToolsError::new(
                        moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchTarget,
                        "Target closed before navigation load",
                    ));
                }
            }
            if let Some(key) = document_lifecycle_wait_key.as_ref() {
                self.conn
                    .release_devtools_document_lifecycle_wait_key(&navigation_context, key);
            }
        }
        execution
            .protocol_output
            .append(foreground_navigation_network_barrier.finish());
        DevToolsPageCommandExecution {
            execution,
            page_residence,
        }
    }

    async fn drain_inflight_background_navigation_before_internal_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let mut out = ProtocolOutputSequence::empty();
        while self
            .conn
            .has_inflight_background_navigation_for_devtools_context(context)
        {
            let Some(input) = receivers.recv_interleaved_input().await else {
                return Err(RendererOutputTransportFailure::new(
                    out,
                    renderer_output_transport_terminal_error(
                        &receivers.renderer_publication_rx,
                        "the in-flight navigation completed",
                    ),
                ));
            };
            out.append(
                self.complete_interleaved_scheduler_input(receivers, input)
                    .await?,
            );
        }
        Ok(out)
    }

    async fn wait_for_document_lifecycle_observer(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
        key: &moli_protocol::DevToolsDocumentLifecycleWaitKey,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let mut out = ProtocolOutputSequence::empty();
        while self
            .conn
            .devtools_document_lifecycle_wait_state(context, key)
            == moli_protocol::DevToolsDocumentLifecycleWaitState::Pending
        {
            let Some(input) = receivers.recv_interleaved_input().await else {
                return Err(RendererOutputTransportFailure::new(
                    out,
                    renderer_output_transport_terminal_error(
                        &receivers.renderer_publication_rx,
                        "the document lifecycle observation completed",
                    ),
                ));
            };
            out.append(
                self.complete_interleaved_scheduler_input(receivers, input)
                    .await?,
            );
        }
        Ok(out)
    }

    /// Waits for one exact target's current Document to reach `milestone`.
    ///
    /// This is the protocol-neutral equivalent of ChromeDriver's
    /// `WaitForPendingNavigations`: first wait until any in-flight navigation
    /// commits, then register against that exact committed Document. If a
    /// successor Document replaces it before the milestone, restart from the
    /// target route instead of observing or commanding the stale renderer.
    pub(crate) async fn wait_for_devtools_context_document_lifecycle(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
        milestone: RendererDocumentLifecycleMilestone,
        timeout: Option<std::time::Duration>,
    ) -> DevToolsCommandExecution {
        use moli_protocol::devtools_runtime::DevToolsErrorKind;
        use moli_protocol::{DevToolsDocumentLifecycleWaitState, DevToolsDocumentNavigationState};

        let started = Instant::now();
        let mut protocol_output = ProtocolOutputSequence::empty();

        loop {
            let navigation_state = self
                .conn
                .devtools_context_document_navigation_state(context);
            let loader_id = match navigation_state.clone() {
                DevToolsDocumentNavigationState::Unavailable => {
                    return DevToolsCommandExecution {
                        result: Err(DevToolsError::new(
                            DevToolsErrorKind::NoSuchTarget,
                            "Target closed while waiting for document navigation",
                        )),
                        protocol_output,
                    };
                }
                DevToolsDocumentNavigationState::PendingNavigation
                | DevToolsDocumentNavigationState::AwaitingCommit => {
                    let ready = self
                        .complete_ready_protocol_residences_for_external_load_wait()
                        .await;
                    let ready_was_empty = ready.is_empty();
                    protocol_output.append(ready);
                    if !ready_was_empty
                        || self
                            .conn
                            .devtools_context_document_navigation_state(context)
                            != navigation_state
                    {
                        continue;
                    }
                    match self
                        .wait_for_interleaved_scheduler_progress_before_deadline(
                            receivers, started, timeout,
                        )
                        .await
                    {
                        Ok(progress) => protocol_output.append(progress),
                        Err(failure) => {
                            let (progress, error) = failure.into_parts();
                            protocol_output.append(progress);
                            return DevToolsCommandExecution {
                                result: Err(error),
                                protocol_output,
                            };
                        }
                    }
                    continue;
                }
                DevToolsDocumentNavigationState::Committed { loader_id } => loader_id,
            };

            let Some(key) = self
                .conn
                .capture_devtools_document_lifecycle_wait_key(context, &loader_id, milestone)
            else {
                // The target route and lifecycle journal are published by
                // distinct owner turns. Let ordered ingress settle them, then
                // resolve the target again; never infer a Document from an old
                // loader id.
                let ready = self
                    .complete_ready_protocol_residences_for_external_load_wait()
                    .await;
                let ready_was_empty = ready.is_empty();
                protocol_output.append(ready);
                if !ready_was_empty
                    || self
                        .conn
                        .devtools_context_document_navigation_state(context)
                        != (DevToolsDocumentNavigationState::Committed {
                            loader_id: loader_id.clone(),
                        })
                {
                    continue;
                }
                match self
                    .wait_for_interleaved_scheduler_progress_before_deadline(
                        receivers, started, timeout,
                    )
                    .await
                {
                    Ok(progress) => protocol_output.append(progress),
                    Err(failure) => {
                        let (progress, error) = failure.into_parts();
                        protocol_output.append(progress);
                        return DevToolsCommandExecution {
                            result: Err(error),
                            protocol_output,
                        };
                    }
                }
                continue;
            };

            loop {
                let wait_state = self
                    .conn
                    .devtools_document_lifecycle_wait_state(context, &key);
                match wait_state {
                    DevToolsDocumentLifecycleWaitState::Reached => {
                        self.conn
                            .release_devtools_document_lifecycle_wait_key(context, &key);
                        return DevToolsCommandExecution {
                            result: Ok(DevToolsCommandResult::Empty),
                            protocol_output,
                        };
                    }
                    DevToolsDocumentLifecycleWaitState::Superseded => {
                        self.conn
                            .release_devtools_document_lifecycle_wait_key(context, &key);
                        break;
                    }
                    DevToolsDocumentLifecycleWaitState::Interrupted
                    | DevToolsDocumentLifecycleWaitState::Unavailable => {
                        self.conn
                            .release_devtools_document_lifecycle_wait_key(context, &key);
                        return DevToolsCommandExecution {
                            result: Err(devtools_document_lifecycle_wait_error(
                                wait_state, milestone,
                            )
                            .expect("terminal lifecycle wait state should produce an error")),
                            protocol_output,
                        };
                    }
                    DevToolsDocumentLifecycleWaitState::Pending => {}
                }

                let ready = self
                    .complete_ready_protocol_residences_for_external_load_wait()
                    .await;
                let ready_was_empty = ready.is_empty();
                protocol_output.append(ready);
                if !ready_was_empty
                    || self
                        .conn
                        .devtools_document_lifecycle_wait_state(context, &key)
                        != wait_state
                {
                    continue;
                }
                match self
                    .wait_for_interleaved_scheduler_progress_before_deadline(
                        receivers, started, timeout,
                    )
                    .await
                {
                    Ok(progress) => protocol_output.append(progress),
                    Err(failure) => {
                        let (progress, error) = failure.into_parts();
                        protocol_output.append(progress);
                        self.conn
                            .release_devtools_document_lifecycle_wait_key(context, &key);
                        return DevToolsCommandExecution {
                            result: Err(error),
                            protocol_output,
                        };
                    }
                }
            }
        }
    }

    async fn wait_for_interleaved_scheduler_progress_before_deadline(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        started: Instant,
        timeout: Option<std::time::Duration>,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        use moli_protocol::devtools_runtime::DevToolsErrorKind;

        let input = match timeout {
            Some(timeout) => {
                let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                    return Err(RendererOutputTransportFailure::without_output(
                        DevToolsError::new(DevToolsErrorKind::Timeout, "navigation wait timed out"),
                    ));
                };
                match tokio::time::timeout(remaining, receivers.recv_interleaved_input()).await {
                    Ok(progress) => progress,
                    Err(_) => {
                        return Err(RendererOutputTransportFailure::without_output(
                            DevToolsError::new(
                                DevToolsErrorKind::Timeout,
                                "navigation wait timed out",
                            ),
                        ));
                    }
                }
            }
            None => receivers.recv_interleaved_input().await,
        };
        let input = input.ok_or_else(|| {
            RendererOutputTransportFailure::without_output(DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Protocol runtime stopped while waiting for document navigation",
            ))
        })?;
        self.complete_interleaved_scheduler_input(receivers, input)
            .await
    }

    async fn drain_deferred_main_document_load_completion_for_wait(
        &mut self,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
    ) -> ProtocolOutputSequence {
        let mut out = ProtocolOutputSequence::empty();
        loop {
            if self
                .conn
                .has_inflight_background_navigation_for_devtools_context(context)
                || !self.front_protocol_residence_is_main_document_load_action_for_context(context)
            {
                return out;
            }
            if self.queues.front_needs_client_turn_predecessor() {
                self.queues.satisfy_front_client_turn_predecessor();
                continue;
            }
            if !self.queues.should_complete_next_residence()
                || !self
                    .queues
                    .protocol_residences
                    .front()
                    .is_some_and(|residence| {
                        matches!(
                            residence,
                            ProtocolSchedulerResidence::ProtocolWork { work, .. }
                                if work.kind()
                                    == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
                                    && work.is_ready()
                        )
                    })
            {
                return out;
            }
            let Some(residence) = self.queues.pop_next_protocol_residence() else {
                return out;
            };
            out.append(self.complete_protocol_residence(residence).await);
        }
    }

    async fn drain_deferred_main_document_load_completion_until_complete(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let mut out = ProtocolOutputSequence::empty();
        loop {
            out.append(self.drain_background_events_around_inflight_navigation(
                &mut receivers.background_event_rx,
            ));
            out.append(
                self.drain_deferred_main_document_load_completion_for_wait(context)
                    .await,
            );
            out.append(self.drain_background_events_around_inflight_navigation(
                &mut receivers.background_event_rx,
            ));
            if !self.has_deferred_main_document_load_completion_for_devtools_context(context) {
                return Ok(out);
            }
            out.append(
                self.complete_ready_protocol_residences_for_external_load_wait()
                    .await,
            );
            if !self.has_deferred_main_document_load_completion_for_devtools_context(context) {
                return Ok(out);
            }
            let Some(input) = receivers.recv_interleaved_input().await else {
                return Err(RendererOutputTransportFailure::new(
                    out,
                    renderer_output_transport_terminal_error(
                        &receivers.renderer_publication_rx,
                        "the deferred document load completed",
                    ),
                ));
            };
            out.append(
                self.complete_interleaved_scheduler_input(receivers, input)
                    .await?,
            );
            if !self.has_deferred_main_document_load_completion_for_devtools_context(context) {
                return Ok(out);
            }
        }
    }

    pub(crate) async fn complete_ready_protocol_residences_for_external_load_wait(
        &mut self,
    ) -> ProtocolOutputSequence {
        let mut out = ProtocolOutputSequence::empty();
        let mut snapshot = self.queues.take_external_load_wait_snapshot();
        while let Some(mut residence) = snapshot.pop_front() {
            self.queues
                .satisfy_checked_out_client_turn_predecessor(&mut residence);
            let has_pending_scheduler_predecessor = !residence.is_ready_to_complete();
            let pending_load_observation = matches!(
                &residence,
                ProtocolSchedulerResidence::ProtocolWork { work, .. }
                    if work.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
                        && !work.is_ready()
            );
            if has_pending_scheduler_predecessor || pending_load_observation {
                snapshot.push_front(residence);
                self.queues.restore_snapshot_to_front(snapshot);
                return out;
            }
            out.append(self.complete_protocol_residence(residence).await);
        }
        out
    }

    pub(crate) async fn complete_ready_protocol_residences_after_command(
        &mut self,
    ) -> ProtocolOutputSequence {
        if self.has_pending_javascript_dialog() {
            return ProtocolOutputSequence::empty();
        }
        let snapshot = self.queues.take_command_followup_snapshot();
        self.complete_protocol_residence_snapshot(snapshot).await
    }

    /// Completes frozen outputs admitted from one exact renderer stream before
    /// exposing the Runtime response fenced by `predecessor`.
    pub(crate) async fn complete_renderer_output_predecessor_before_runtime_response(
        &mut self,
        predecessor: &moli_core::RendererOutputFence,
    ) -> ProtocolOutputSequence {
        let cursor = predecessor.cursor();
        let snapshot = self
            .queues
            .take_renderer_output_predecessor_snapshot(cursor);
        self.complete_protocol_residence_snapshot(snapshot).await
    }

    /// Projects the exact renderer stream position owned by a DevTools
    /// command before its protocol-neutral result leaves the scheduler.
    ///
    /// The renderer reply and concrete publication use independent channels.
    /// Merely completing the command future therefore does not imply that the
    /// owner actions produced by that turn are visible in protocol state. In
    /// particular, a `window.open()` result must not be serialized for
    /// WebDriver until its popup target has been created. This is the direct
    /// scheduler counterpart of the frontend actor's response fence.
    pub(crate) async fn project_renderer_output_predecessor_before_devtools_result(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        predecessor: &moli_core::RendererOutputFence,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let mut output = ProtocolOutputSequence::empty();
        while !self
            .conn
            .renderer_output_cursor_is_projected(predecessor.cursor())
        {
            let Some(publication) = receivers.renderer_publication_rx.recv().await else {
                return Err(RendererOutputTransportFailure::new(
                    output,
                    renderer_output_transport_terminal_error(
                        &receivers.renderer_publication_rx,
                        "the command predecessor was projected",
                    ),
                ));
            };
            output.append(self.ingest_renderer_publication_now(publication).await);
        }
        output.append(
            self.complete_renderer_output_predecessor_before_runtime_response(predecessor)
                .await,
        );
        Ok(output)
    }

    async fn complete_protocol_residence_snapshot(
        &mut self,
        mut snapshot: VecDeque<ProtocolSchedulerResidence>,
    ) -> ProtocolOutputSequence {
        let mut out = ProtocolOutputSequence::empty();
        let mut retained = VecDeque::new();
        let mut blocked_target_ids = Vec::new();
        while let Some(mut residence) = snapshot.pop_front() {
            if self.has_pending_javascript_dialog() {
                retained.push_back(residence);
                retained.append(&mut snapshot);
                self.queues.restore_snapshot_to_front(retained);
                return out;
            }
            let target_ids = self.protocol_residence_navigation_gate_target_ids(&residence);
            let blocked_by_prior_residence = target_ids
                .iter()
                .any(|target_id| blocked_target_ids.contains(target_id));
            let blocked_by_navigation =
                self.protocol_targets_have_inflight_background_navigation(&target_ids);
            if blocked_by_prior_residence || blocked_by_navigation {
                if target_ids.is_empty() {
                    retained.push_back(residence);
                    retained.append(&mut snapshot);
                    self.queues.restore_snapshot_to_front(retained);
                    return out;
                }
                append_unique_target_ids(&mut blocked_target_ids, target_ids);
                retained.push_back(residence);
                continue;
            }
            self.queues
                .satisfy_checked_out_client_turn_predecessor(&mut residence);
            if !residence.is_ready_to_complete() {
                if target_ids.is_empty() {
                    retained.push_back(residence);
                    retained.append(&mut snapshot);
                    self.queues.restore_snapshot_to_front(retained);
                    return out;
                }
                append_unique_target_ids(&mut blocked_target_ids, target_ids);
                retained.push_back(residence);
                continue;
            }
            out.append(self.complete_protocol_residence(residence).await);
        }
        if !retained.is_empty() {
            self.queues.restore_snapshot_to_front(retained);
        }
        out
    }

    fn protocol_residence_navigation_gate_target_ids(
        &self,
        residence: &ProtocolSchedulerResidence,
    ) -> Vec<String> {
        match residence {
            ProtocolSchedulerResidence::RendererOutputPublication(work) => {
                work.output.navigation_gate_target_ids(&self.conn)
            }
            ProtocolSchedulerResidence::ProtocolWork { work, .. } => work
                .navigation_gate_target_id()
                .map(str::to_owned)
                .into_iter()
                .collect(),
        }
    }

    fn protocol_targets_have_inflight_background_navigation(&self, target_ids: &[String]) -> bool {
        if target_ids.is_empty() {
            return self.has_inflight_background_navigation();
        }
        target_ids.iter().any(|target_id| {
            self.conn
                .has_inflight_background_navigation_for_target(target_id)
        })
    }

    fn next_ungated_protocol_residence_index(&self) -> Option<usize> {
        // A target-local navigation is an ordering barrier only for later
        // work from the same target. Keep those lanes ordered while allowing
        // an independent target to advance, matching Chromium's per-frame
        // NavigationRequest ownership.
        let mut blocked_target_ids = Vec::new();
        for (index, residence) in self.queues.protocol_residences.iter().enumerate() {
            let target_ids = self.protocol_residence_navigation_gate_target_ids(residence);
            if target_ids
                .iter()
                .any(|target_id| blocked_target_ids.contains(target_id))
            {
                continue;
            }
            if self.protocol_targets_have_inflight_background_navigation(&target_ids) {
                if target_ids.is_empty() {
                    return None;
                }
                append_unique_target_ids(&mut blocked_target_ids, target_ids);
                continue;
            }
            if !residence.should_yield_to_client_turn() && !residence.is_ready_to_complete() {
                if target_ids.is_empty() {
                    return None;
                }
                append_unique_target_ids(&mut blocked_target_ids, target_ids);
                continue;
            }
            return Some(index);
        }
        None
    }

    pub(crate) async fn complete_interleaved_scheduler_input(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        input: CdpSchedulerInterleavedInput,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        match input {
            CdpSchedulerInterleavedInput::BackgroundNavigationCompletion(completion) => {
                self.drain_background_navigation_completion_with_progress_barrier(
                    completion, receivers,
                )
                .await
            }
            CdpSchedulerInterleavedInput::BackgroundEvent(event) => {
                Ok(self.route_background_event_around_inflight_navigation(event))
            }
            CdpSchedulerInterleavedInput::RendererPublication(publication) => {
                Ok(self.ingest_renderer_publication_now(publication).await)
            }
        }
    }

    fn front_protocol_residence_is_main_document_load_action_for_context(
        &self,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
    ) -> bool {
        matches!(
            self.queues.protocol_residences.front(),
            Some(ProtocolSchedulerResidence::ProtocolWork { work, .. })
                if work.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
                    && work.observes_main_document_load_for_devtools_context(&self.conn, context)
        )
    }

    fn has_deferred_main_document_load_completion_for_devtools_context(
        &self,
        context: &moli_protocol::devtools_runtime::DevToolsCommandContext,
    ) -> bool {
        self.queues.protocol_residences.iter().any(|residence| {
            matches!(
                residence,
                ProtocolSchedulerResidence::ProtocolWork { work, .. }
                    if work.observes_main_document_load_for_devtools_context(&self.conn, context)
            )
        })
    }

    pub(crate) fn has_inflight_background_navigation(&self) -> bool {
        self.conn.has_inflight_background_navigation()
    }

    pub(crate) fn command_waits_for_navigation_flush(&self, command: &ParsedCdpCommand) -> bool {
        command.renderer_access() == CdpRendererCommandAccess::MainThread
            && self
                .conn
                .renderer_document_navigation_is_suspended_for_session_owner(command.session_id())
    }

    pub(crate) fn route_background_event_around_inflight_navigation(
        &mut self,
        event: BackgroundProtocolEvent,
    ) -> ProtocolOutputSequence {
        if !event.route_is_current(&self.conn) {
            return ProtocolOutputSequence::empty();
        }
        let should_wait = event.should_wait_for_background_navigation_completion();
        let navigation_target_id = should_wait
            .then(|| self.conn.background_navigation_target_id_for_event(&event))
            .flatten();
        let has_inflight_navigation = should_wait
            && navigation_target_id.as_deref().map_or_else(
                || self.has_inflight_background_navigation(),
                |target_id| {
                    self.conn
                        .has_inflight_background_navigation_for_target(target_id)
                },
            );
        if moli_trace::cdp_runtime_trace_enabled()
            && let Some((method, resource_type, request_id, url)) = event.trace_network_summary()
        {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "background_network_event_navigation_gate_route",
                method,
                resource_type,
                request_id,
                url,
                has_inflight_navigation,
                should_wait,
            );
        }
        if has_inflight_navigation {
            if moli_trace::cdp_runtime_trace_enabled() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "background_event_deferred_for_navigation_completion",
                    pending_background_events = self.pending_navigation_background_events.len() + 1,
                );
            }
            self.pending_navigation_background_events
                .push_back(PendingNavigationBackgroundEvent {
                    target_id: navigation_target_id,
                    event,
                });
            return ProtocolOutputSequence::empty();
        }
        ProtocolOutputSequence::from_background_event(event)
    }

    fn route_background_events_around_inflight_navigation(
        &mut self,
        events: Vec<BackgroundProtocolEvent>,
    ) -> ProtocolOutputSequence {
        let mut out = ProtocolOutputSequence::empty();
        for event in events {
            out.append(self.route_background_event_around_inflight_navigation(event));
        }
        out
    }

    pub(crate) fn drain_background_events_around_inflight_navigation(
        &mut self,
        background_event_rx: &mut CdpBackgroundEventReceiver,
    ) -> ProtocolOutputSequence {
        let mut out = ProtocolOutputSequence::empty();
        while let Ok(event) = background_event_rx.try_recv() {
            out.append(self.route_background_event_around_inflight_navigation(event));
        }
        out
    }

    fn drain_pending_navigation_background_events(&mut self) -> ProtocolOutputSequence {
        let mut events = Vec::new();
        let mut retained = VecDeque::new();
        while let Some(pending) = self.pending_navigation_background_events.pop_front() {
            // The navigation gate deliberately extends an event's residence
            // beyond its projection turn. Reauthorize its frozen route at
            // the actual release boundary: the in-flight navigation may have
            // replaced the root Document or detached its session meanwhile.
            if !pending.event.route_is_current(&self.conn) {
                continue;
            }
            let remains_gated = pending.target_id.as_deref().map_or_else(
                || self.has_inflight_background_navigation(),
                |target_id| {
                    self.conn
                        .has_inflight_background_navigation_for_target(target_id)
                },
            );
            if remains_gated {
                retained.push_back(pending);
            } else {
                events.push(pending.event);
            }
        }
        self.pending_navigation_background_events = retained;
        ProtocolOutputSequence::from_background_events(events)
    }

    fn append_navigation_gate_release_before_renderer_boundary(
        &mut self,
        prefix: &mut ProtocolOutputSequence,
    ) {
        prefix.append(self.drain_pending_navigation_background_events());
    }

    fn apply_scheduler_events(&mut self, events: Vec<CdpSchedulerEvent>) {
        self.apply_scheduler_events_with_load_predecessors(events, &[], None);
    }

    fn apply_scheduler_events_with_load_predecessors(
        &mut self,
        events: Vec<CdpSchedulerEvent>,
        load_predecessors: &[DeferredMainDocumentLoadObservationId],
        future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
    ) {
        for event in events {
            if moli_trace::cdp_runtime_trace_enabled() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "scheduler_event_apply_start",
                    event = ?event,
                    protocol_residence_len = self.queues.protocol_residence_len(),
                );
            }
            match event {
                CdpSchedulerEvent::ProtocolWorkPublished { work } => {
                    if moli_trace::cdp_nav_timing_enabled() {
                        tracing::info!(
                            publish_sequence = work.publish_sequence().get(),
                            ?work,
                            kind = ?work.kind(),
                            stage = "scheduler_protocol_work_published"
                        );
                    }
                    self.queues.enqueue_protocol_work(
                        work,
                        load_predecessors.to_vec(),
                        future_load_predecessor,
                    );
                }
                CdpSchedulerEvent::PageScreencastStarted { registration } => {
                    self.register_page_screencast(registration, TokioInstant::now());
                }
            }
            if moli_trace::cdp_runtime_trace_enabled() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "scheduler_event_apply_done",
                    protocol_residence_len = self.queues.protocol_residence_len(),
                );
            }
        }
    }

    fn apply_protocol_only_turn_outcome(
        &mut self,
        outcome: moli_protocol::CdpTurnOutcome,
    ) -> ProtocolOutputSequence {
        let (mut output, post_renderer_output, renderer_output_boundary) =
            self.materialize_protocol_only_turn_outcome(outcome);
        assert!(
            renderer_output_boundary.is_none(),
            "non-command turn must consume its renderer insertion boundary at its owner boundary"
        );
        output.append(post_renderer_output);
        output
    }

    async fn apply_renderer_owner_turn_outcome(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        outcome: moli_protocol::CdpRendererOwnerTurnOutcome,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let (output, post_renderer_output, renderer_output_boundary, renderer_output_predecessor) =
            self.materialize_renderer_owner_turn_outcome(outcome);
        assert!(
            renderer_output_boundary.is_none(),
            "non-navigation owner turn must not carry a renderer insertion boundary"
        );

        let mut causal_output = ProtocolOutputSequence::empty();
        if let Some(predecessor) = renderer_output_predecessor {
            causal_output.append(
                self.project_renderer_output_predecessor_before_devtools_result(
                    receivers,
                    &predecessor,
                )
                .await?,
            );
        }
        causal_output.append(output);
        causal_output.append(post_renderer_output);
        Ok(causal_output)
    }

    fn materialize_protocol_only_turn_outcome(
        &mut self,
        outcome: moli_protocol::CdpTurnOutcome,
    ) -> (
        ProtocolOutputSequence,
        ProtocolOutputSequence,
        Option<moli_core::RendererOutputFence>,
    ) {
        let (
            events,
            mut post_renderer_output_events,
            renderer_output_boundary,
            mut post_response_events,
            scheduler_events,
        ) = outcome.into_command_turn_parts();
        post_renderer_output_events.append(&mut post_response_events);
        self.apply_scheduler_events(scheduler_events);
        (
            self.route_background_events_around_inflight_navigation(events),
            self.route_background_events_around_inflight_navigation(post_renderer_output_events),
            renderer_output_boundary,
        )
    }

    fn materialize_renderer_owner_turn_outcome(
        &mut self,
        outcome: moli_protocol::CdpRendererOwnerTurnOutcome,
    ) -> (
        ProtocolOutputSequence,
        ProtocolOutputSequence,
        Option<moli_core::RendererOutputFence>,
        Option<moli_core::RendererOutputFence>,
    ) {
        let (
            events,
            mut post_renderer_output_events,
            renderer_output_boundary,
            mut post_response_events,
            scheduler_events,
            renderer_output_predecessor,
        ) = outcome.into_renderer_owner_turn_parts();
        post_renderer_output_events.append(&mut post_response_events);
        self.apply_scheduler_events(scheduler_events);
        (
            self.route_background_events_around_inflight_navigation(events),
            self.route_background_events_around_inflight_navigation(post_renderer_output_events),
            renderer_output_boundary,
            renderer_output_predecessor,
        )
    }

    async fn ingest_renderer_publication(
        &mut self,
        publication: RendererOutputTransportMessage,
        mut load_predecessors: Vec<DeferredMainDocumentLoadObservationId>,
        mut future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
    ) -> ProtocolOutputSequence {
        let pending_scheduler_events = self.conn.take_scheduler_events();
        self.apply_scheduler_events(pending_scheduler_events);
        let renderer_output_cursor = match &publication {
            RendererOutputTransportMessage::Publication(output) => Some(output.cursor()),
            RendererOutputTransportMessage::StreamControl(_)
            | RendererOutputTransportMessage::PageReservationReleased { .. }
            | RendererOutputTransportMessage::CursorLeaseDeclared { .. }
            | RendererOutputTransportMessage::CursorLeaseReleased { .. } => None,
        };
        for predecessor in self.queued_load_predecessors_for_renderer_output(&publication) {
            if !load_predecessors.contains(&predecessor) {
                load_predecessors.push(predecessor);
            }
        }
        if !load_predecessors.is_empty() {
            // An exact observation supersedes the short command-completion
            // binding window. One residence must never wait on both forms of
            // the same causal boundary.
            future_load_predecessor = None;
        }
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "renderer_output_ingress_start",
                residence = ?publication.residence(),
                load_predecessors = load_predecessors.len(),
                protocol_residence_len = self.queues.protocol_residence_len(),
            );
        }
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let outcome = self
            .conn
            .ingest_renderer_output_turn_async(
                publication,
                &mut self.runtime_command_output_barriers,
            )
            .await;
        let (
            mut events,
            mut post_renderer_output_events,
            renderer_output_boundary,
            mut post_response_events,
            scheduler_events,
        ) = outcome.into_command_turn_parts();
        assert!(
            renderer_output_boundary.is_none(),
            "renderer output ingress cannot recursively insert another renderer cursor"
        );
        events.append(&mut post_renderer_output_events);
        events.append(&mut post_response_events);
        let output = ProtocolOutputSequence::from_background_events(events);
        // The concrete event batch is admitted before work published by the
        // same ingress turn. That preserves the "project frozen output, then
        // run owner continuation" boundary without rescanning its source. A
        // load-ordered event batch and its produced work inherit the exact
        // predecessor; already-observed Network facts are split below because
        // they are prerequisites of browser load visibility, not Page effects
        // produced after that boundary.
        let requires_output_residence =
            !load_predecessors.is_empty() || future_load_predecessor.is_some();
        let (immediate_output, resident_output) = if requires_output_residence {
            // A timer publication can contain both Page-side effects that must
            // remain after the exact load boundary and Network-domain facts
            // that Chromium has already exposed. Keep the load predecessor on
            // the former without delaying the latter behind Page.loadEventFired.
            output.split_network_observations()
        } else {
            (ProtocolOutputSequence::empty(), output)
        };
        if !resident_output.is_empty() && requires_output_residence {
            self.queues.enqueue_renderer_output_publication(
                renderer_output_cursor.expect(
                    "only a concrete renderer publication can produce resident protocol output",
                ),
                resident_output,
                load_predecessors.clone(),
                future_load_predecessor,
            );
            self.apply_scheduler_events_with_load_predecessors(
                scheduler_events,
                &load_predecessors,
                future_load_predecessor,
            );
            if let Some(started) = trace_started {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "renderer_output_ingress_deferred",
                    renderer_output_cursor = ?renderer_output_cursor,
                    protocol_residence_len = self.queues.protocol_residence_len(),
                    elapsed_us = %started.elapsed().as_micros(),
                );
            }
            return self.route_background_events_around_inflight_navigation(
                immediate_output.into_background_events(),
            );
        }

        self.apply_scheduler_events_with_load_predecessors(
            scheduler_events,
            &load_predecessors,
            future_load_predecessor,
        );
        let mut output = immediate_output;
        output.append(resident_output);
        let output = self
            .route_background_events_around_inflight_navigation(output.into_background_events());
        if let Some(started) = trace_started {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "renderer_output_ingress_done",
                renderer_output_cursor = ?renderer_output_cursor,
                messages = output.len(),
                protocol_residence_len = self.queues.protocol_residence_len(),
                elapsed_us = %started.elapsed().as_micros(),
            );
        }
        output
    }

    pub(crate) async fn ingest_renderer_publication_now(
        &mut self,
        publication: RendererOutputTransportMessage,
    ) -> ProtocolOutputSequence {
        self.ingest_renderer_publication(publication, Vec::new(), None)
            .await
    }

    /// Consumes one renderer publication now.
    ///
    /// Only a typed post-load candidate (currently a timer or an exact
    /// after-load lifecycle action output) may briefly wait for the biased
    /// command-completion turn to publish its exact load predecessor. Parser,
    /// module, child-frame, lifecycle-prerequisite and ordinary resource
    /// output is returned from this ingress turn.
    pub(crate) async fn ingest_renderer_publication_for_scheduler(
        &mut self,
        publication: RendererOutputTransportMessage,
    ) -> ProtocolOutputSequence {
        let future_load_predecessor =
            DeferredMainDocumentLoadPredecessorCandidate::from_renderer_publication(&publication);
        self.ingest_renderer_publication(publication, Vec::new(), future_load_predecessor)
            .await
    }

    pub(crate) async fn ingest_renderer_publication_after_load(
        &mut self,
        publication: RendererOutputTransportMessage,
        observation_id: DeferredMainDocumentLoadObservationId,
    ) -> ProtocolOutputSequence {
        self.ingest_renderer_publication(publication, vec![observation_id], None)
            .await
    }

    pub(crate) async fn finish_command_dispatch_output_flush(
        &mut self,
        post_flush_scheduler_events: Vec<CdpSchedulerEvent>,
        output_release_permit: Option<CommandOutputReleasePermit>,
    ) -> ProtocolOutputSequence {
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "command_post_flush_scheduler_events",
                events = post_flush_scheduler_events.len(),
                protocol_residence_len = self.queues.protocol_residence_len(),
            );
        }
        self.apply_scheduler_events(post_flush_scheduler_events);
        let Some(permit) = output_release_permit else {
            return ProtocolOutputSequence::empty();
        };
        let Some(runtime_barrier) = permit.finish_response() else {
            return ProtocolOutputSequence::empty();
        };
        let completion = self
            .conn
            .release_runtime_command_output_barrier_turn_async(
                &mut self.runtime_command_output_barriers,
                runtime_barrier,
            )
            .await;
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "runtime_command_output_barrier_terminal",
                terminal = ?completion.terminal(),
            );
        }
        self.apply_protocol_only_turn_outcome(completion.into_outcome())
    }

    fn next_protocol_scheduler_step(&self) -> ProtocolSchedulerStep {
        let Some(index) = self.next_ungated_protocol_residence_index() else {
            return ProtocolSchedulerStep::Wait;
        };
        let residence = self
            .queues
            .protocol_residences
            .get(index)
            .expect("selected protocol residence must exist");
        if residence.should_yield_to_client_turn() {
            return ProtocolSchedulerStep::SatisfyClientTurnPredecessor;
        }
        if residence.is_ready_to_complete() {
            return ProtocolSchedulerStep::CompleteReadyResidence;
        }
        ProtocolSchedulerStep::Wait
    }

    fn satisfy_front_protocol_residence_client_turn_predecessor(&mut self) {
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "protocol_residence_client_turn_predecessor_satisfied",
                protocol_residence_len = self.queues.protocol_residence_len(),
            );
        }
        let Some(index) = self.next_ungated_protocol_residence_index() else {
            return;
        };
        self.queues.satisfy_client_turn_predecessor_at(index);
    }

    fn next_ready_protocol_residence_is_main_document_load_action(&self) -> bool {
        let Some(index) = self.next_ungated_protocol_residence_index() else {
            return false;
        };
        matches!(
            self.queues.protocol_residences.get(index),
            Some(ProtocolSchedulerResidence::ProtocolWork {
                work,
                client_turn_predecessor: ClientTurnPredecessor::Satisfied,
                load_predecessors,
                ..
            }) if load_predecessors.is_empty()
                && work.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
        )
    }

    pub(crate) fn route_renderer_output_for_deferred_load_completion(
        &self,
        output: &RendererOutputTransportMessage,
        interest: &DeferredMainDocumentLoadCompletionOutputInterest,
    ) -> DeferredMainDocumentLoadCompletionOutputAction {
        interest.route_output_while_waiting(output)
    }

    pub(crate) async fn complete_next_protocol_residence(&mut self) -> ProtocolOutputSequence {
        let Some(index) = self.next_ungated_protocol_residence_index() else {
            return ProtocolOutputSequence::empty();
        };
        let Some(residence) = self.queues.take_protocol_residence_at(index) else {
            return ProtocolOutputSequence::empty();
        };
        self.complete_protocol_residence(residence).await
    }

    pub(crate) async fn project_protocol_local_command_outputs_now(
        &mut self,
        session_id: Option<&str>,
    ) -> ProtocolOutputSequence {
        let outcome = self
            .conn
            .project_protocol_local_command_outputs_turn_async(session_id)
            .await;
        self.apply_protocol_only_turn_outcome(outcome)
    }

    async fn complete_protocol_residence(
        &mut self,
        residence: ProtocolSchedulerResidence,
    ) -> ProtocolOutputSequence {
        let mut out = ProtocolOutputSequence::empty();
        let runtime_trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        if runtime_trace_started.is_some() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "protocol_residence_completion_start",
                residence = ?residence,
                protocol_residence_len = self.queues.protocol_residence_len(),
            );
        }
        let probe_started = moli_trace::command_probe_enabled().then(Instant::now);
        if probe_started.is_some() {
            tracing::info!(?residence, "CMD_PROBE_PROTOCOL_RESIDENCE_START");
        }
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(?residence, stage = "scheduler_protocol_residence_start");
        }
        match residence {
            ProtocolSchedulerResidence::RendererOutputPublication(work) => {
                assert!(
                    work.load_predecessors.is_empty(),
                    "scheduler selected renderer output before its exact load predecessor"
                );
                if moli_trace::cdp_runtime_trace_enabled() {
                    tracing::info!(
                        target: "moli_cdp_runtime",
                        stage = "renderer_output_publication_release",
                        renderer_output_cursor = ?work.renderer_output_cursor,
                    );
                }
                out.append(self.route_background_events_around_inflight_navigation(
                    work.output.into_background_events(),
                ));
            }
            ProtocolSchedulerResidence::ProtocolWork {
                work,
                load_predecessors,
                ..
            } => {
                assert!(
                    load_predecessors.is_empty(),
                    "scheduler selected protocol work before its exact load predecessor"
                );
                let load_observation_id = work.main_document_load_observation_id();
                let outcome = self
                    .conn
                    .complete_ready_protocol_scheduler_work_turn(work)
                    .await;
                out.append(self.apply_protocol_only_turn_outcome(outcome));
                if let Some(observation_id) = load_observation_id {
                    self.queues.satisfy_load_predecessor(observation_id);
                }
            }
        }
        if let Some(started) = probe_started {
            tracing::info!(
                elapsed_us = %started.elapsed().as_micros(),
                "CMD_PROBE_PROTOCOL_RESIDENCE_DONE"
            );
        }
        if let Some(started) = runtime_trace_started {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "protocol_residence_completion_done",
                messages = out.len(),
                protocol_residence_len = self.queues.protocol_residence_len(),
                elapsed_us = %started.elapsed().as_micros(),
            );
        }
        out
    }

    /// Returns every exact load observation that must precede output projected
    /// from this renderer publication.
    ///
    /// The publication itself is consumed immediately. The returned identities
    /// are stored on the concrete event batch, so a later scheduler turn never
    /// needs the wake source to rediscover either payload or ordering.
    fn queued_load_predecessors_for_renderer_output(
        &self,
        output: &RendererOutputTransportMessage,
    ) -> Vec<DeferredMainDocumentLoadObservationId> {
        self.queues
            .protocol_residences
            .iter()
            .filter_map(|residence| match residence {
                ProtocolSchedulerResidence::ProtocolWork { work, .. }
                    if work.route_renderer_output_while_main_document_load_waits(output)
                        == Some(DeferredMainDocumentLoadCompletionOutputAction::Queue) =>
                {
                    work.main_document_load_observation_id()
                }
                _ => None,
            })
            .collect()
    }

    pub(crate) fn start_next_deferred_load_completion(
        &mut self,
    ) -> Option<PendingDeferredMainDocumentLoadCompletion> {
        let index = self.next_ungated_protocol_residence_index()?;
        let should_start = matches!(
            self.queues.protocol_residences.get(index),
            Some(ProtocolSchedulerResidence::ProtocolWork {
                work,
                client_turn_predecessor: ClientTurnPredecessor::Satisfied,
                load_predecessors,
                ..
            }) if load_predecessors.is_empty()
                && work.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
        );
        if !should_start {
            return None;
        }
        let Some(ProtocolSchedulerResidence::ProtocolWork { work, .. }) =
            self.queues.take_protocol_residence_at(index)
        else {
            return None;
        };
        if moli_trace::command_probe_enabled() {
            tracing::info!(
                observation_sequence = work.publish_sequence().get(),
                "CMD_PROBE_DEFERRED_LOAD_START"
            );
        }
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "deferred_load_completion_start",
                publish_sequence = work.publish_sequence().get(),
                protocol_residence_len = self.queues.protocol_residence_len(),
            );
        }
        Some(work.start_main_document_load_wait())
    }

    pub(crate) async fn complete_deferred_load_completion(
        &mut self,
        completion: CompletedDeferredMainDocumentLoadCompletion,
    ) -> ProtocolOutputSequence {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let observation_id = completion.observation_id();
        let outcome = self
            .conn
            .complete_deferred_main_document_load_completion_for_scheduler(completion)
            .await;
        let output = self.apply_protocol_only_turn_outcome(outcome);
        self.queues.satisfy_load_predecessor(observation_id);
        if let Some(started) = trace_started {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "deferred_load_completion_done",
                messages = output.len(),
                elapsed_us = %started.elapsed().as_micros(),
            );
        }
        output
    }

    pub(crate) async fn drain_background_navigation_completion(
        &mut self,
        completion: BackgroundNavigationCompletion,
    ) -> (
        ProtocolOutputSequence,
        ProtocolOutputSequence,
        Option<moli_core::RendererOutputFence>,
        Option<moli_core::RendererOutputFence>,
    ) {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let outcome = self
            .conn
            .drain_background_navigation_completion_turn_async(completion)
            .await;
        let (out, post_renderer_output, renderer_output_boundary, renderer_output_predecessor) =
            self.materialize_renderer_owner_turn_outcome(outcome);
        if let Some(started) = trace_started {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "background_navigation_completion_done",
                messages = out.len(),
                elapsed_us = %started.elapsed().as_micros(),
            );
        }
        (
            out,
            post_renderer_output,
            renderer_output_boundary,
            renderer_output_predecessor,
        )
    }

    async fn materialize_background_navigation_completion_with_progress_barrier(
        &mut self,
        completion: BackgroundNavigationCompletion,
        background_event_rx: &mut CdpBackgroundEventReceiver,
    ) -> (
        ProtocolOutputSequence,
        ProtocolOutputSequence,
        Option<moli_core::RendererOutputFence>,
    ) {
        // Navigation start, response-head progress and the early
        // `Page.navigate` response were produced before the renderer Page
        // commit. Keep them in a distinct prefix. The completion's renderer
        // cursor orders only the new Page's concrete output before the commit
        // tail; it must never pull that output in front of the earlier prefix.
        let mut prefix =
            self.drain_background_events_around_inflight_navigation(background_event_rx);
        let (completion_prefix, mut suffix, renderer_output_boundary, renderer_output_predecessor) =
            self.drain_background_navigation_completion(completion)
                .await;
        assert!(
            renderer_output_predecessor.is_none(),
            "navigation completion must use its exact insertion boundary, not a command predecessor"
        );
        prefix.append(completion_prefix);
        // The completion can still carry a renderer insertion boundary. While
        // that boundary is projected, later publications may contain the
        // response or terminal for a request whose start is parked behind the
        // navigation gate. Release the parked FIFO into the pre-boundary
        // prefix so those later publications cannot overtake it.
        self.append_navigation_gate_release_before_renderer_boundary(&mut prefix);
        suffix.append(self.drain_background_events_around_inflight_navigation(background_event_rx));
        (prefix, suffix, renderer_output_boundary)
    }

    /// Completes one navigation owner turn together with the exact concrete
    /// renderer publication produced by that commit.
    ///
    /// Navigation completion and renderer output travel over independent
    /// channels. The completion therefore carries a cursor instead of relying
    /// on channel arrival order. Transport records up to that cursor are fully
    /// projected here; their frozen output and owner actions precede the
    /// completion output.
    pub(crate) async fn drain_background_navigation_completion_with_progress_barrier(
        &mut self,
        completion: BackgroundNavigationCompletion,
        receivers: &mut CdpSchedulerEventReceivers,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        let (mut output, completion_output, renderer_output_boundary) = self
            .materialize_background_navigation_completion_with_progress_barrier(
                completion,
                &mut receivers.background_event_rx,
            )
            .await;
        let Some(predecessor) = renderer_output_boundary else {
            output.append(completion_output);
            return Ok(output);
        };

        while !self
            .conn
            .renderer_output_cursor_is_projected(predecessor.cursor())
        {
            let Some(publication) = receivers.renderer_publication_rx.recv().await else {
                return Err(RendererOutputTransportFailure::new(
                    output,
                    renderer_output_transport_terminal_error(
                        &receivers.renderer_publication_rx,
                        "navigation completion",
                    ),
                ));
            };
            output.append(self.ingest_renderer_publication_now(publication).await);
        }
        output.append(
            self.complete_renderer_output_predecessor_before_runtime_response(&predecessor)
                .await,
        );
        output.append(completion_output);
        Ok(output)
    }
}

fn renderer_output_transport_terminal_error(
    receiver: &moli_core::RendererOutputTransportReceiver,
    boundary: &str,
) -> DevToolsError {
    let diagnostics = receiver.diagnostics();
    let reason = if diagnostics.terminal {
        "exceeded its bounded admission budget"
    } else {
        "closed"
    };
    DevToolsError::new(
        moli_protocol::devtools_runtime::DevToolsErrorKind::Internal,
        format!("Renderer output transport {reason} before {boundary}"),
    )
}

fn devtools_navigation_wait(command: &DevToolsCommand) -> Option<DevToolsNavigationWait> {
    match command {
        DevToolsCommand::Navigate(command) => Some(command.wait),
        DevToolsCommand::Reload(command) => Some(command.wait),
        DevToolsCommand::TraverseHistory(command) => Some(command.wait),
        _ => None,
    }
}

fn devtools_navigation_lifecycle_milestone(
    wait: Option<DevToolsNavigationWait>,
) -> Option<RendererDocumentLifecycleMilestone> {
    match wait {
        Some(DevToolsNavigationWait::DomContentLoaded) => {
            Some(RendererDocumentLifecycleMilestone::DomContentLoaded)
        }
        Some(DevToolsNavigationWait::Load) => Some(RendererDocumentLifecycleMilestone::Load),
        _ => None,
    }
}

fn devtools_navigation_result_loader_id(
    result: &Result<DevToolsCommandResult, DevToolsError>,
) -> Option<String> {
    let Ok(DevToolsCommandResult::Navigate(result)) = result else {
        return None;
    };
    result
        .loader_id
        .as_ref()
        .map(|loader_id| loader_id.as_str().to_owned())
        .or_else(|| {
            result
                .navigation_id
                .as_ref()
                .and_then(|navigation_id| navigation_id.as_str().strip_prefix("navigation-"))
                .map(str::to_owned)
        })
}

fn devtools_document_lifecycle_wait_error(
    state: moli_protocol::DevToolsDocumentLifecycleWaitState,
    milestone: RendererDocumentLifecycleMilestone,
) -> Option<DevToolsError> {
    use moli_protocol::DevToolsDocumentLifecycleWaitState;
    use moli_protocol::devtools_runtime::DevToolsErrorKind;

    let milestone_name = match milestone {
        RendererDocumentLifecycleMilestone::DomContentLoaded => "DOMContentLoaded",
        RendererDocumentLifecycleMilestone::Load => "load",
    };
    match state {
        DevToolsDocumentLifecycleWaitState::Reached => None,
        DevToolsDocumentLifecycleWaitState::Interrupted => Some(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Navigation interrupted before {milestone_name}"),
        )),
        DevToolsDocumentLifecycleWaitState::Superseded => Some(DevToolsError::new(
            DevToolsErrorKind::NavigationChangingDocument,
            format!("Navigation was superseded before {milestone_name}"),
        )),
        DevToolsDocumentLifecycleWaitState::Unavailable => Some(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            format!("Target closed before navigation {milestone_name}"),
        )),
        DevToolsDocumentLifecycleWaitState::Pending => Some(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Navigation {milestone_name} wait was cancelled"),
        )),
    }
}

#[cfg(test)]
mod tests;
