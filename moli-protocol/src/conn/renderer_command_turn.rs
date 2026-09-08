use moli_core::page::{RendererCommandTurnCompletion, RendererCommandTurnOutput};

use super::CommandDispatchContext;

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
