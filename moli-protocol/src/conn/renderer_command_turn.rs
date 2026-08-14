use moli_core::page::{
    CompletedPageCommand, RendererCommandTurnCompletion, RendererCommandTurnOutput,
};

use super::{CdpConnection, CommandDispatchContext, TargetPageResidenceIdentity};

impl CdpConnection {
    /// Settles a renderer command turn against the Page residence that admitted it.
    ///
    /// A completed renderer turn is immutable even if navigation replaces its
    /// originating Page before protocol delivery resumes. Refresh the Page cache
    /// only while that exact residence is still current; otherwise preserve the
    /// output for domain-specific reply handling without installing stale state
    /// into the replacement Page.
    pub(crate) fn settle_page_command_turn_for_owner(
        &mut self,
        session_id: Option<&str>,
        owner: &TargetPageResidenceIdentity,
        completion: CompletedPageCommand,
    ) -> RendererCommandTurnOutput {
        if !self.target_page_residence_identity_is_current_for_session(session_id, owner) {
            return completion.into_output();
        }

        if let Ok(page) = self.loaded_page_mut_for_interruptible_protocol_access(session_id) {
            return page.finish_page_command_turn(completion);
        }

        // The renderer has already settled this immutable result. Losing the
        // protocol-side Page cache must not turn its acknowledgement into an
        // error or apply its state to a future replacement Page.
        completion.into_output()
    }
}

impl CommandDispatchContext {
    /// Captures the exact concrete-stream predecessor and consumes the unique
    /// renderer completion boundary.
    ///
    /// Concrete records have already entered ordered ingress; this boundary
    /// must never project or transport them a second time.
    pub(crate) fn consume_renderer_command_turn_output(
        &mut self,
        output: RendererCommandTurnOutput,
    ) -> RendererCommandTurnCompletion {
        let (mut completion, renderer_output_predecessor) =
            output.into_completion_and_predecessor();
        if let Some(predecessor) = renderer_output_predecessor {
            self.set_renderer_output_predecessor(predecessor);
        }
        if let Some(continuation) = completion.take_post_response_continuation() {
            self.response_flush()
                .defer_until_response_flush(move || continuation.release());
        }
        completion
    }
}
