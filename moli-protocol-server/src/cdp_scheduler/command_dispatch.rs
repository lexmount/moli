use moli_protocol::{BackgroundProtocolEvent, RendererOutputDependencies};

use super::{CommandOutputReleasePermit, ProtocolOutputSequence};

pub(crate) struct CommandTurnOutput {
    protocol_output: ProtocolOutputSequence,
    post_renderer_output: ProtocolOutputSequence,
    renderer_output_boundary: Option<moli_core::RendererOutputFence>,
    post_response_events: Vec<BackgroundProtocolEvent>,
    renderer_output_dependencies: RendererOutputDependencies,
    output_release_permit: Option<CommandOutputReleasePermit>,
}

impl CommandTurnOutput {
    pub(super) fn new(protocol_output: ProtocolOutputSequence) -> Self {
        Self::new_with_post_response_events(protocol_output, Vec::new())
    }

    pub(super) fn new_with_post_response_events(
        protocol_output: ProtocolOutputSequence,
        post_response_events: Vec<BackgroundProtocolEvent>,
    ) -> Self {
        Self {
            protocol_output,
            post_renderer_output: ProtocolOutputSequence::empty(),
            renderer_output_boundary: None,
            post_response_events,
            renderer_output_dependencies: RendererOutputDependencies::default(),
            output_release_permit: None,
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
        RendererOutputDependencies,
        Option<CommandOutputReleasePermit>,
    ) {
        (
            self.protocol_output,
            self.post_renderer_output,
            self.renderer_output_boundary,
            self.post_response_events,
            self.renderer_output_dependencies,
            self.output_release_permit,
        )
    }
}
