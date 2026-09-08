use moli_protocol::{CommandResponseFlushPermit, RendererCommandResponsePermit};

/// The single-use authority that finishes one command's protocol-output
/// boundary.
///
/// The frontend flush permit and optional renderer-response permit are one
/// value so a command cannot publish its response while forgetting to release
/// the concrete Page output held on its behalf. Commands that do not execute
/// Page JavaScript still carry the frontend permit and simply have no
/// renderer response-order permit.
#[must_use = "a command output release permit must be completed exactly once"]
pub(crate) struct CommandOutputReleasePermit {
    response_flush: CommandResponseFlushPermit,
    renderer_response_permit: Option<RendererCommandResponsePermit>,
}

impl CommandOutputReleasePermit {
    pub(super) fn new(
        response_flush: CommandResponseFlushPermit,
        renderer_response_permit: Option<RendererCommandResponsePermit>,
    ) -> Self {
        Self {
            response_flush,
            renderer_response_permit,
        }
    }

    pub(super) fn finish_response(self) -> Option<RendererCommandResponsePermit> {
        self.response_flush.finish();
        self.renderer_response_permit
    }
}
