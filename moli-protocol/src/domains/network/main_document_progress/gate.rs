use parking_lot::{Mutex, MutexGuard};
use std::sync::Arc;

use crate::devtools_runtime::AutomationEvent;
use serde_json::Value;

#[cfg(test)]
use crate::conn::BackgroundEventSender;
use crate::conn::{BackgroundProtocolEvent, build_event};

use super::MainDocumentNavigationProgressEvent;

pub(crate) struct MainDocumentProgressGate {
    queue: MainDocumentProgressQueueHandle,
}

pub(crate) struct MainDocumentProgressBackgroundEventBarrier<'a> {
    out: &'a mut Vec<BackgroundProtocolEvent>,
    progress_gate: &'a mut MainDocumentProgressGate,
}

pub(super) struct MainDocumentProgressDrain {
    output_queue: MainDocumentProgressOutputQueue,
}

#[derive(Clone)]
pub(super) struct MainDocumentProgressQueueHandle {
    drain: Arc<Mutex<MainDocumentProgressDrain>>,
}

pub(super) struct MainDocumentProgressEmission {
    ready_at: MainDocumentProgressPhase,
    batch: MainDocumentProgressEventBatch,
}

#[derive(Default)]
struct MainDocumentProgressOutputQueue {
    source_ready_until: MainDocumentProgressPhase,
    response_received_emitted: bool,
    pending: Vec<MainDocumentProgressEmission>,
}

pub(super) enum MainDocumentProgressOutputTarget<'a> {
    BackgroundEvents(&'a mut Vec<BackgroundProtocolEvent>),
    #[cfg(test)]
    BackgroundSender(&'a BackgroundEventSender),
}

pub(super) struct MainDocumentProgressEventBatch {
    events: Vec<MainDocumentNavigationProgressEvent>,
}

pub(super) enum MainDocumentProgressSourceKind {
    Streaming,
    FailedNavigation(Box<MainDocumentFailedNavigationProgressSource>),
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Default)]
pub(super) enum MainDocumentProgressPhase {
    #[default]
    Created,
    RequestStarted,
    ResponseReceived,
    BodyFinished,
}

pub(super) struct MainDocumentFailedNavigationProgressSource {
    pub(super) failure_event: Option<MainDocumentNavigationProgressEvent>,
}

pub(super) struct MainDocumentProgressSource {
    source: MainDocumentProgressSourceKind,
}

impl MainDocumentProgressSource {
    pub(super) fn streaming() -> Self {
        Self::new(MainDocumentProgressSourceKind::Streaming)
    }

    pub(super) fn failed_navigation(
        failure_event: Option<MainDocumentNavigationProgressEvent>,
    ) -> Self {
        Self::new(MainDocumentProgressSourceKind::FailedNavigation(Box::new(
            MainDocumentFailedNavigationProgressSource { failure_event },
        )))
    }

    pub(super) fn new(source: MainDocumentProgressSourceKind) -> Self {
        Self { source }
    }

    fn append_to_drain(self, drain: &mut MainDocumentProgressDrain) {
        self.source.append_to_drain(drain);
    }
}

impl MainDocumentProgressGate {
    pub(super) fn from_queue(queue: MainDocumentProgressQueueHandle) -> Self {
        Self { queue }
    }

    fn drain_into_background_events(&mut self, out: &mut Vec<BackgroundProtocolEvent>) {
        self.queue.drain_into_background_events(out);
    }
}

impl<'a> MainDocumentProgressBackgroundEventBarrier<'a> {
    pub(crate) fn background_events(
        out: &'a mut Vec<BackgroundProtocolEvent>,
        progress_gate: &'a mut MainDocumentProgressGate,
    ) -> Self {
        Self { out, progress_gate }
    }

    pub(crate) fn drain_progress(&mut self) {
        self.progress_gate.drain_into_background_events(self.out);
    }
}

impl MainDocumentProgressDrain {
    pub(super) fn new() -> Self {
        Self {
            output_queue: MainDocumentProgressOutputQueue::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn from_source(source: MainDocumentProgressSource) -> Self {
        let mut drain = Self::new();
        drain.append_source(source);
        drain
    }

    fn append_source(&mut self, source: MainDocumentProgressSource) {
        source.append_to_drain(self);
    }

    pub(super) fn append_ready_emission(
        &mut self,
        ready_until: MainDocumentProgressPhase,
        emission: MainDocumentProgressEmission,
    ) {
        self.output_queue.mark_source_ready_until(ready_until);
        self.output_queue.push(emission);
    }

    #[cfg(test)]
    pub(super) fn drain_into_output_target(
        &mut self,
        output: &mut MainDocumentProgressOutputTarget<'_>,
    ) {
        for emission in self.take_ready_emissions() {
            emission.emit_to(output);
        }
    }

    fn take_ready_emissions(&mut self) -> Vec<MainDocumentProgressEmission> {
        self.output_queue.drain_ready()
    }
}

impl MainDocumentProgressQueueHandle {
    pub(super) fn from_source(source: MainDocumentProgressSource) -> Self {
        let queue = Self::new(MainDocumentProgressDrain::new());
        queue.append_source(source);
        queue
    }

    pub(super) fn new(drain: MainDocumentProgressDrain) -> Self {
        Self {
            drain: Arc::new(Mutex::new(drain)),
        }
    }

    pub(super) fn append_ready_emission(
        &self,
        ready_until: MainDocumentProgressPhase,
        emission: MainDocumentProgressEmission,
    ) {
        self.lock_drain()
            .append_ready_emission(ready_until, emission);
    }

    fn append_source(&self, source: MainDocumentProgressSource) {
        self.lock_drain().append_source(source);
    }

    fn drain_into_background_events(&self, out: &mut Vec<BackgroundProtocolEvent>) {
        let ready = self.take_ready_emissions();
        let mut output = MainDocumentProgressOutputTarget::background_events(out);
        for emission in ready {
            emission.emit_to(&mut output);
        }
    }

    pub(super) fn drain_into_output_target(
        &self,
        output: &mut MainDocumentProgressOutputTarget<'_>,
    ) {
        for emission in self.take_ready_emissions() {
            emission.emit_to(output);
        }
    }

    fn take_ready_emissions(&self) -> Vec<MainDocumentProgressEmission> {
        self.lock_drain().take_ready_emissions()
    }

    fn lock_drain(&self) -> MutexGuard<'_, MainDocumentProgressDrain> {
        self.drain.lock()
    }
}

impl MainDocumentProgressOutputQueue {
    fn mark_source_ready_until(&mut self, phase: MainDocumentProgressPhase) {
        self.source_ready_until = self.source_ready_until.max(phase);
    }

    fn push(&mut self, emission: MainDocumentProgressEmission) {
        if emission.is_empty() {
            return;
        }
        self.pending.push(emission);
    }

    fn drain_ready(&mut self) -> Vec<MainDocumentProgressEmission> {
        let mut ready_candidates = Vec::new();
        let mut pending = Vec::new();
        let drain_until = self.source_ready_until;
        for emission in std::mem::take(&mut self.pending) {
            if emission.ready_at <= drain_until {
                ready_candidates.push(emission);
            } else {
                pending.push(emission);
            }
        }
        ready_candidates.sort_by_key(|emission| emission.ready_at);
        let mut ready = Vec::new();
        for emission in ready_candidates {
            if emission.ready_at == MainDocumentProgressPhase::BodyFinished
                && !self.response_received_emitted
            {
                pending.push(emission);
                continue;
            }
            if emission.ready_at == MainDocumentProgressPhase::ResponseReceived {
                self.response_received_emitted = true;
            }
            ready.push(emission);
        }
        self.pending = pending;
        ready
    }
}

impl MainDocumentProgressEventBatch {
    pub(super) fn from_events(events: Vec<MainDocumentNavigationProgressEvent>) -> Self {
        Self { events }
    }

    pub(super) fn into_events(self) -> Vec<MainDocumentNavigationProgressEvent> {
        self.events
    }

    fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl MainDocumentProgressEmission {
    pub(super) fn new(
        ready_at: MainDocumentProgressPhase,
        batch: MainDocumentProgressEventBatch,
    ) -> Self {
        Self { ready_at, batch }
    }

    fn is_empty(&self) -> bool {
        self.batch.is_empty()
    }

    pub(super) fn emit_to(self, output: &mut MainDocumentProgressOutputTarget<'_>) {
        output.emit_batch(self.batch);
    }
}

impl<'a> MainDocumentProgressOutputTarget<'a> {
    pub(super) fn background_events(out: &'a mut Vec<BackgroundProtocolEvent>) -> Self {
        Self::BackgroundEvents(out)
    }

    #[cfg(test)]
    pub(super) fn background_sender(sender: &'a BackgroundEventSender) -> Self {
        Self::BackgroundSender(sender)
    }

    pub(super) fn emit_batch(&mut self, batch: MainDocumentProgressEventBatch) {
        self.emit_events(batch.into_events());
    }

    fn emit_events(&mut self, events: Vec<MainDocumentNavigationProgressEvent>) {
        for event in events {
            self.emit_event(event);
        }
    }

    pub(super) fn emit_event(&mut self, event: MainDocumentNavigationProgressEvent) {
        event.emit_into(self);
    }

    pub(super) fn push_background_event(&mut self, event: BackgroundProtocolEvent) {
        match self {
            Self::BackgroundEvents(out) => {
                out.push(event);
            }
            #[cfg(test)]
            Self::BackgroundSender(sender) => {
                let _ = (*sender).send(event);
            }
        }
    }

    pub(super) fn push_automation_event(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        event: AutomationEvent,
    ) {
        let protocol_event = build_event(method, params, session_id);
        self.push_background_event(BackgroundProtocolEvent::immediate_automation_event(
            protocol_event,
            event,
        ));
    }
}

impl MainDocumentProgressSourceKind {
    fn append_to_drain(self, drain: &mut MainDocumentProgressDrain) {
        match self {
            Self::Streaming => {}
            Self::FailedNavigation(source) => source.append_to_drain(drain),
        }
    }
}

impl MainDocumentFailedNavigationProgressSource {
    fn append_to_drain(self, drain: &mut MainDocumentProgressDrain) {
        if let Some(event) = self.failure_event {
            drain.append_ready_emission(
                MainDocumentProgressPhase::RequestStarted,
                MainDocumentProgressEmission::new(
                    MainDocumentProgressPhase::RequestStarted,
                    MainDocumentProgressEventBatch::from_events(vec![event]),
                ),
            );
        }
    }
}
