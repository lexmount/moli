use moli_protocol::{BackgroundProtocolEvent, CdpSchedulerEvent};

use super::{CommandOutputReleasePermit, ProtocolOutputSequence};

pub(crate) struct CommandDispatchState {
    replies: ProtocolOutputSequence,
}

pub(crate) struct CommandTurnOutput {
    protocol_output: ProtocolOutputSequence,
    post_renderer_output: ProtocolOutputSequence,
    renderer_output_boundary: Option<moli_core::RendererOutputFence>,
    post_response_events: Vec<BackgroundProtocolEvent>,
    post_flush_scheduler_events: Vec<CdpSchedulerEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    output_release_permit: Option<CommandOutputReleasePermit>,
}

impl CommandDispatchState {
    pub(crate) fn pending_command() -> Self {
        Self {
            replies: ProtocolOutputSequence::empty(),
        }
    }

    pub(crate) fn complete_with_turn_output(
        mut self,
        turn_output: CommandTurnOutput,
    ) -> CommandTurnOutput {
        let (
            replies,
            post_renderer_output,
            renderer_output_boundary,
            post_response_events,
            scheduler_events,
            renderer_output_predecessor,
            output_release_permit,
        ) = turn_output.into_parts();
        self.replies = replies;
        CommandTurnOutput::new_with_post_response_events_and_permit(
            self.replies,
            post_response_events,
            scheduler_events,
            output_release_permit,
        )
        .with_renderer_output_boundary(renderer_output_boundary, post_renderer_output)
        .with_renderer_output_predecessor(renderer_output_predecessor)
    }
}

impl CommandTurnOutput {
    pub(super) fn new(
        protocol_output: ProtocolOutputSequence,
        post_flush_scheduler_events: Vec<CdpSchedulerEvent>,
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
        post_flush_scheduler_events: Vec<CdpSchedulerEvent>,
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
        post_flush_scheduler_events: Vec<CdpSchedulerEvent>,
        output_release_permit: Option<CommandOutputReleasePermit>,
    ) -> Self {
        Self {
            protocol_output,
            post_renderer_output: ProtocolOutputSequence::empty(),
            renderer_output_boundary: None,
            post_response_events,
            post_flush_scheduler_events,
            renderer_output_predecessor: None,
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
            predecessor.merge_into_same_stream_tail(&mut self.renderer_output_predecessor);
        }
        self
    }

    pub(crate) fn take_renderer_output_predecessor(
        &mut self,
    ) -> Option<moli_core::RendererOutputFence> {
        self.renderer_output_predecessor.take()
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
        Vec<CdpSchedulerEvent>,
        Option<moli_core::RendererOutputFence>,
        Option<CommandOutputReleasePermit>,
    ) {
        (
            self.protocol_output,
            self.post_renderer_output,
            self.renderer_output_boundary,
            self.post_response_events,
            self.post_flush_scheduler_events,
            self.renderer_output_predecessor,
            self.output_release_permit,
        )
    }
}
