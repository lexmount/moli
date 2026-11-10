use moli_protocol::{BackgroundProtocolEvent, CdpSchedulerEvent, RendererOutputDependencies};
use std::{cell::Cell, rc::Rc};

use super::{CdpScheduler, CommandOutputReleasePermit, ProtocolOutputSequence};

#[derive(Clone, Debug, Default)]
pub(super) struct CommandOutputFlushPredecessor(Rc<Cell<bool>>);

impl CommandOutputFlushPredecessor {
    pub(super) fn is_satisfied(&self) -> bool {
        self.0.get()
    }
}

/// Work is admitted before waiting for renderer output, but retains the
/// original command response as an execution predecessor. Losing the response
/// releases that predecessor without losing the already-accepted owner work.
#[derive(Default)]
pub(crate) struct CommandPostFlushActions {
    events: Vec<CdpSchedulerEvent>,
    predecessor: Option<CommandOutputFlushPredecessor>,
    count: usize,
}

impl CommandPostFlushActions {
    pub(super) fn admit(scheduler: &mut CdpScheduler, events: Vec<CdpSchedulerEvent>) -> Self {
        let mut actions = Self {
            events: Vec::new(),
            predecessor: None,
            count: events.len(),
        };
        for event in events {
            match event {
                CdpSchedulerEvent::ProtocolWorkPublished { work } => {
                    let predecessor = actions.predecessor.get_or_insert_with(Default::default);
                    scheduler
                        .queues
                        .enqueue_command_protocol_work(work, predecessor.clone());
                }
                event => actions.events.push(event),
            }
        }
        actions
    }

    pub(super) fn len(&self) -> usize {
        self.count
    }

    pub(super) fn finish(mut self) -> Vec<CdpSchedulerEvent> {
        self.release();
        std::mem::take(&mut self.events)
    }

    fn release(&self) {
        if let Some(predecessor) = &self.predecessor {
            predecessor.0.set(true);
        }
    }
}

impl Drop for CommandPostFlushActions {
    fn drop(&mut self) {
        self.release();
    }
}

pub(crate) struct CommandTurnOutput {
    protocol_output: ProtocolOutputSequence,
    post_renderer_output: ProtocolOutputSequence,
    renderer_output_boundary: Option<moli_core::RendererOutputFence>,
    post_response_events: Vec<BackgroundProtocolEvent>,
    post_flush_scheduler_events: CommandPostFlushActions,
    renderer_output_dependencies: RendererOutputDependencies,
    output_release_permit: Option<CommandOutputReleasePermit>,
}

impl CommandTurnOutput {
    pub(super) fn new(
        protocol_output: ProtocolOutputSequence,
        post_flush_scheduler_events: CommandPostFlushActions,
    ) -> Self {
        Self::new_with_post_response_events(
            protocol_output,
            Vec::new(),
            post_flush_scheduler_events,
        )
    }

    pub(super) fn new_with_post_response_events(
        protocol_output: ProtocolOutputSequence,
        post_response_events: Vec<BackgroundProtocolEvent>,
        post_flush_scheduler_events: CommandPostFlushActions,
    ) -> Self {
        Self::new_with_post_response_events_and_permit(
            protocol_output,
            post_response_events,
            post_flush_scheduler_events,
            None,
        )
    }

    pub(super) fn new_with_post_response_events_and_permit(
        protocol_output: ProtocolOutputSequence,
        post_response_events: Vec<BackgroundProtocolEvent>,
        post_flush_scheduler_events: CommandPostFlushActions,
        output_release_permit: Option<CommandOutputReleasePermit>,
    ) -> Self {
        Self {
            protocol_output,
            post_renderer_output: ProtocolOutputSequence::empty(),
            renderer_output_boundary: None,
            post_response_events,
            post_flush_scheduler_events,
            renderer_output_dependencies: RendererOutputDependencies::default(),
            output_release_permit,
        }
    }

    pub(crate) fn with_output_release_permit(mut self, permit: CommandOutputReleasePermit) -> Self {
        self.output_release_permit = Some(permit);
        self
    }

    pub(crate) fn with_renderer_output_boundary(
        mut self,
        boundary: Option<moli_core::RendererOutputFence>,
        post_renderer_output: ProtocolOutputSequence,
    ) -> Self {
        assert!(
            self.renderer_output_boundary.is_none(),
            "one command turn cannot contain multiple renderer insertion boundaries"
        );
        if boundary.is_none() {
            assert!(
                post_renderer_output.is_empty(),
                "post-renderer protocol output requires an exact boundary"
            );
        }
        self.renderer_output_boundary = boundary;
        self.post_renderer_output = post_renderer_output;
        self
    }

    pub(crate) fn with_renderer_output_predecessor(
        mut self,
        predecessor: Option<moli_core::RendererOutputFence>,
    ) -> Self {
        if let Some(predecessor) = predecessor {
            self.renderer_output_dependencies
                .add_same_stream(predecessor);
        }
        self
    }

    pub(crate) fn with_renderer_output_dependencies(
        mut self,
        dependencies: RendererOutputDependencies,
    ) -> Self {
        self.renderer_output_dependencies.merge(dependencies);
        self
    }

    pub(crate) fn take_renderer_output_dependencies(&mut self) -> RendererOutputDependencies {
        std::mem::take(&mut self.renderer_output_dependencies)
    }

    /// Place output projected from the command's exact renderer cause before
    /// the command response carried by this turn.
    ///
    /// The frozen sequence has already been split by its domain-declared
    /// `BeforeResponse`/`AfterResponse` order. Only the former reaches this
    /// method; the latter remains in the command barrier until the response
    /// flush consumes its single-use permit.
    pub(crate) fn prepend_protocol_output(&mut self, output: ProtocolOutputSequence) {
        if output.is_empty() {
            return;
        }
        let mut combined = output;
        combined.append(std::mem::take(&mut self.protocol_output));
        self.protocol_output = combined;
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ProtocolOutputSequence,
        ProtocolOutputSequence,
        Option<moli_core::RendererOutputFence>,
        Vec<BackgroundProtocolEvent>,
        CommandPostFlushActions,
        RendererOutputDependencies,
        Option<CommandOutputReleasePermit>,
    ) {
        (
            self.protocol_output,
            self.post_renderer_output,
            self.renderer_output_boundary,
            self.post_response_events,
            self.post_flush_scheduler_events,
            self.renderer_output_dependencies,
            self.output_release_permit,
        )
    }
}
